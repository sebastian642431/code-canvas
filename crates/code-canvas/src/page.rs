use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use canvas_core::scan::{self, Dir, ScanProgress};
use canvas_core::{Layout, LayoutFilter, RelKind, Relations, Repo, ScanOptions, Site, SymRef, TokenCache};
use canvas_render::{Renderer, SceneData, SceneGpu};

use crate::camera::Camera;
use crate::editor::Editor;
use crate::git::{Commit, GitState};
use crate::session::PageSession;

pub struct Loaded {
    pub repo: Repo,
    pub deps: canvas_core::deps::Deps,
    pub layout: Layout,
    pub scene: SceneData,
    pub total_ms: u128,
}

pub struct TreeRow {
    pub depth: u16,
    pub is_dir: bool,
    pub open: bool,
    pub hidden: bool,
    pub locked: bool,
    pub name: String,
    pub path: String,
    pub count: u32,
}

pub struct Ready {
    pub repo: Repo,
    pub deps: canvas_core::deps::Deps,
    pub layout: Layout,
    pub gpu: SceneGpu,
    pub tokens: TokenCache,
    pub load_ms: u128,
    pub tree: Vec<TreeRow>,
    pub tree_dirty: bool,
}

pub enum PageState {
    Waiting,
    Loading { progress: Arc<ScanProgress>, rx: Receiver<Loaded>, started: Instant },
    Ready(Box<Ready>),
    Failed(String),
}

pub struct SearchResults {
    pub query: String,
    pub hits: Vec<Site>,
    pub truncated: bool,
    pub ms: f32,
}

pub struct Selection {
    pub sym: SymRef,
    pub rel: Relations,
    pub outside: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepFilters {
    pub selected: bool,
    pub all: bool,
    pub kinds: [bool; 5],
}

impl Default for DepFilters {
    fn default() -> Self {
        DepFilters { selected: true, all: false, kinds: [true; 5] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelFilters {
    pub enabled: [bool; 5],
    pub opacity: [f32; 5],
    pub bodies: [bool; 5],
    pub body_opacity: [f32; 5],
}

impl Default for RelFilters {
    fn default() -> Self {
        let mut opacity = [1.0; 5];
        opacity[RelKind::OutgoingCall.index()] = 0.45;
        Self { enabled: [true; 5], opacity, bodies: [true; 5], body_opacity: [1.0; 5] }
    }
}

pub struct Page {
    pub id: u32,
    pub root: PathBuf,
    pub title: String,
    pub state: PageState,
    pub camera: Camera,
    pub target: Option<Camera>,
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    pub hover_cell: Option<(u32, u32, u32)>,
    pub framed: bool,
    pub restore_camera: Option<Camera>,
    pub pane_center: Option<[f32; 2]>,
    pub filter: LayoutFilter,
    pub tree_open: HashSet<String>,
    pub selections: Vec<Selection>,
    pub only_connected: bool,
    pub curves_dirty: bool,
    pub search: Option<SearchResults>,
    pub git: Option<GitState>,
    git_rx: Option<Receiver<Option<GitState>>>,
    pub only_changed: bool,
    pub editors: std::collections::HashMap<String, Editor>,
    pub editing: Option<String>,
    pub curves_node: Option<usize>,
    pub dep_edges: (usize, usize),
    pub commit_message: String,
    git_op: Option<Receiver<(crate::git::GitOp, Result<String, String>)>>,
    pub git_note: Option<(bool, String)>,
    message_rx: Option<Receiver<Result<String, String>>>,
    pub timeline: Timeline,
    timeline_rx: Option<Receiver<(String, Vec<Commit>)>>,
}

impl SearchResults {
    pub fn single(site: Site) -> Self {
        SearchResults { query: String::new(), hits: vec![site], truncated: false, ms: 0.0 }
    }
}

#[derive(Default)]
pub struct Timeline {
    pub path: Option<String>,
    pub commits: Vec<Commit>,
    pub loading: bool,
}

impl Page {
    pub fn open(id: u32, root: PathBuf, saved: Option<&PageSession>, scan: bool) -> Page {
        let title = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());
        let mut page = Page {
            id,
            root,
            title,
            state: PageState::Failed(String::new()),
            camera: Camera::default(),
            target: None,
            hover: None,
            selected: None,
            hover_cell: None,
            framed: false,
            restore_camera: None,
            pane_center: None,
            filter: LayoutFilter::default(),
            tree_open: HashSet::new(),
            selections: Vec::new(),
            only_connected: false,
            curves_dirty: true,
            search: None,
            git: None,
            git_rx: None,
            only_changed: false,
            editors: Default::default(),
            editing: None,
            curves_node: None,
            dep_edges: (0, 0),
            commit_message: String::new(),
            git_op: None,
            git_note: None,
            message_rx: None,
            timeline: Timeline::default(),
            timeline_rx: None,
        };
        if let Some(s) = saved {
            page.filter.collapsed = s.collapsed.iter().cloned().collect();
            page.filter.hidden = s.hidden.iter().cloned().collect();
            page.tree_open = s.tree_open.iter().cloned().collect();
            page.restore_camera = s.camera.map(|(center, zoom)| Camera { center, zoom });
        }
        if scan {
            page.start_scan();
        } else {
            page.state = PageState::Waiting;
        }
        page
    }

    pub fn ensure_scanned(&mut self) {
        if matches!(self.state, PageState::Waiting) {
            self.start_scan();
        }
    }

    pub fn is_waiting(&self) -> bool {
        matches!(self.state, PageState::Waiting)
    }

    pub fn git_loading(&self) -> bool {
        self.git_rx.is_some() && self.git.is_none()
    }

    pub fn portable_camera(&self, scale: f32, viewport: [f32; 2]) -> Option<Camera> {
        if !self.framed {
            return None;
        }
        let z = self.camera.zoom;
        let half = [viewport[0] * 0.5, viewport[1] * 0.5];
        let middle = self.pane_center.unwrap_or(half);
        let center =
            [self.camera.center[0] + (middle[0] - half[0]) / z, self.camera.center[1] + (middle[1] - half[1]) / z];
        Some(Camera { center, zoom: z / scale })
    }

    pub fn place_camera(portable: Camera, scale: f32, viewport: [f32; 2], middle: [f32; 2]) -> Camera {
        let z = portable.zoom * scale;
        let center = [
            portable.center[0] - (middle[0] - viewport[0] * 0.5) / z,
            portable.center[1] - (middle[1] - viewport[1] * 0.5) / z,
        ];
        Camera { center, zoom: z }
    }

    pub fn to_session(&self, scale: f32, viewport: [f32; 2]) -> PageSession {
        let sorted = |set: &HashSet<String>| {
            let mut v: Vec<String> = set.iter().cloned().collect();
            v.sort();
            v
        };
        PageSession {
            path: self.root.clone(),
            camera: self.portable_camera(scale, viewport).map(|c| (c.center, c.zoom)),
            collapsed: sorted(&self.filter.collapsed),
            hidden: sorted(&self.filter.hidden),
            tree_open: sorted(&self.tree_open),
        }
    }

    pub fn refresh_git(&mut self) {
        let (tx, rx) = mpsc::channel();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let state = crate::git::load(&root);
            match &state {
                Some(g) => log::debug!("git: {} changes on {} in {} ms", g.changes.len(), g.branch, g.ms),
                None => log::debug!("git: {} is not in a repository", root.display()),
            }
            let _ = tx.send(state);
            crate::ext::wake_ui();
        });
        self.git_rx = Some(rx);
    }

    pub fn editor(&self) -> Option<&Editor> {
        self.editors.get(self.editing.as_deref()?)
    }

    pub fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.editors.get_mut(self.editing.as_deref()?)
    }

    pub fn begin_edit(&mut self, rel_path: &str) -> bool {
        if !self.editors.contains_key(rel_path) {
            let Ok(bytes) = std::fs::read(self.root.join(rel_path)) else { return false };
            let Ok(source) = String::from_utf8(bytes) else { return false };
            if source.contains('\0') {
                return false;
            }
            self.editors.insert(rel_path.to_string(), Editor::open(rel_path.to_string(), &source));
        }
        self.editing = Some(rel_path.to_string());
        true
    }

    pub fn show_edit(&mut self, rel_path: &str, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) {
        let Some(source) = self.editors.get(rel_path).map(|e| e.display_source()) else { return };
        let PageState::Ready(r) = &mut self.state else { return };
        let Some(index) = r.repo.file_index(rel_path) else { return };
        let before = (r.repo.files[index].line_count(), r.repo.files[index].max_cols);
        r.repo.replace_source(index, &source, &ScanOptions::default());
        r.tokens = Default::default();
        let after = (r.repo.files[index].line_count(), r.repo.files[index].max_cols);
        if !self.selections.is_empty() {
            self.selections.clear();
            self.curves_dirty = true;
        }
        self.search = None;
        if before != after {
            self.relayout(renderer, device, filters);
        }
    }

    pub fn git_busy(&self) -> bool {
        self.git_op.is_some()
    }

    pub fn run_git(&mut self, op: crate::git::GitOp) {
        if self.git_op.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let result = crate::git::run(&root, &op);
            let _ = tx.send((op, result));
            crate::ext::wake_ui();
        });
        self.git_op = Some(rx);
        self.git_note = None;
    }

    pub fn writing_message(&self) -> bool {
        self.message_rx.is_some()
    }

    pub fn write_message(&mut self, settings: crate::commit_ai::Settings) {
        if self.message_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::commit_ai::write(&root, &settings));
            crate::ext::wake_ui();
        });
        self.message_rx = Some(rx);
        self.git_note = None;
    }

    pub fn poll_message(&mut self) -> bool {
        let Some(rx) = &self.message_rx else { return false };
        match rx.try_recv() {
            Ok(Ok(message)) => self.commit_message = message,
            Ok(Err(why)) => self.git_note = Some((false, why)),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {}
        }
        self.message_rx = None;
        true
    }

    pub fn poll_git_op(&mut self) -> bool {
        let Some(rx) = &self.git_op else { return false };
        let Ok((op, result)) = rx.try_recv() else { return false };
        self.git_op = None;
        match (&op, result) {
            (crate::git::GitOp::Commit { .. }, Ok(line)) => {
                self.commit_message.clear();
                self.git_note = Some((true, line));
            }
            (crate::git::GitOp::Sync, Ok(_)) => self.git_note = Some((true, "Synced with the remote.".into())),
            (crate::git::GitOp::Push | crate::git::GitOp::Publish(_), Ok(_)) => {
                self.git_note = Some((true, "Pushed.".into()))
            }
            (crate::git::GitOp::Pull, Ok(_)) => self.git_note = Some((true, "Pulled.".into())),
            (crate::git::GitOp::Fetch, Ok(_)) => self.git_note = Some((true, "Fetched.".into())),
            (crate::git::GitOp::Stash, Ok(_)) => self.git_note = Some((true, "Changes stashed.".into())),
            (crate::git::GitOp::Switch(branch), Ok(_)) => self.git_note = Some((true, format!("On branch {branch}."))),
            (_, Ok(_)) => {}
            (_, Err(e)) => self.git_note = Some((false, e)),
        }
        self.refresh_git();
        true
    }

    pub fn poll_git(&mut self) -> bool {
        let Some(rx) = &self.git_rx else { return false };
        match rx.try_recv() {
            Ok(state) => {
                self.git = state;
                self.git_rx = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.git_rx = None;
                false
            }
        }
    }

    pub fn want_timeline(&mut self, path: Option<String>) {
        if self.timeline.path == path {
            return;
        }
        self.timeline = Timeline { path: path.clone(), commits: Vec::new(), loading: path.is_some() };
        self.timeline_rx = None;
        let Some(path) = path else { return };
        let (tx, rx) = mpsc::channel();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let commits = crate::git::log(&root, &path, 60);
            let _ = tx.send((path, commits));
            crate::ext::wake_ui();
        });
        self.timeline_rx = Some(rx);
    }

    pub fn poll_timeline(&mut self) -> bool {
        let Some(rx) = &self.timeline_rx else { return false };
        let Ok((path, commits)) = rx.try_recv() else { return false };
        self.timeline_rx = None;
        if self.timeline.path.as_deref() == Some(path.as_str()) {
            self.timeline.commits = commits;
            self.timeline.loading = false;
        }
        true
    }

    pub fn changed_files(&self) -> HashSet<u32> {
        let (Some(git), Some(r)) = (&self.git, self.ready()) else { return HashSet::new() };
        git.changes.iter().filter_map(|c| r.repo.file_index(&c.path)).map(|i| i as u32).collect()
    }

    pub fn start_scan(&mut self) {
        self.refresh_git();
        let progress = Arc::new(ScanProgress::default());
        let (tx, rx) = mpsc::channel();
        let root = self.root.clone();
        let p = progress.clone();
        let filter = LayoutFilter { only_files: None, ..self.filter.clone() };
        std::thread::Builder::new()
            .name(format!("scan {}", self.title))
            .spawn(move || {
                let t = Instant::now();
                let repo = scan::scan(&root, &ScanOptions::default(), Some(p));
                let deps = canvas_core::deps::analyze(&repo);
                log::info!(
                    "dependencies of '{}': {} edges between {} pairs of files in {} ms ({} unresolved, {} external)",
                    repo.name,
                    deps.edges.len(),
                    deps.pairs(),
                    deps.ms,
                    deps.unresolved,
                    deps.external
                );
                let layout = Layout::build_filtered(&repo, &filter);
                let scene = SceneData::build(&repo, &layout);
                let total_ms = t.elapsed().as_millis();
                log::info!(
                    "scanned '{}': {} files, {} lines, {} symbols, scan {} ms (index {} ms), total {} ms",
                    repo.name,
                    repo.files.len(),
                    repo.total_lines,
                    repo.symbols.count,
                    repo.scan_ms,
                    repo.timings.index_ms,
                    total_ms
                );
                let _ = tx.send(Loaded { repo, deps, layout, scene, total_ms });
            })
            .expect("spawn scan thread");
        self.state = PageState::Loading { progress, rx, started: Instant::now() };
        self.hover = None;
        self.hover_cell = None;
        self.selected = None;
        self.selections.clear();
        self.only_connected = false;
        self.curves_dirty = true;
        self.search = None;
    }

    pub fn run_search(&mut self, query: String) {
        self.search = None;
        let Some(r) = self.ready() else { return };
        if query.trim().is_empty() {
            return;
        }
        let t = Instant::now();
        let (hits, truncated) = r.repo.search_text(&query, 5000);
        let ms = t.elapsed().as_secs_f32() * 1000.0;
        log::info!("search '{query}': {} hits in {ms:.1} ms", hits.len());
        self.search = Some(SearchResults { query, hits, truncated, ms });
    }

    pub fn poll(&mut self) -> Option<Loaded> {
        if let PageState::Loading { rx, .. } = &self.state {
            match rx.try_recv() {
                Ok(loaded) => return Some(loaded),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.state = PageState::Failed("the scan thread crashed".into());
                }
            }
        }
        None
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state, PageState::Loading { .. })
    }

    pub fn ready(&self) -> Option<&Ready> {
        match &self.state {
            PageState::Ready(r) => Some(r),
            _ => None,
        }
    }

    pub fn ready_mut(&mut self) -> Option<&mut Ready> {
        match &mut self.state {
            PageState::Ready(r) => Some(r),
            _ => None,
        }
    }

    pub fn connected_files(&self, filters: &RelFilters) -> HashSet<u32> {
        let mut out = HashSet::new();
        for s in &self.selections {
            out.insert(s.sym.file);
            for r in &s.rel.items {
                if filters.enabled[r.kind.index()] {
                    out.insert(r.from.file);
                    out.insert(r.to.file);
                }
            }
        }
        out
    }

    pub fn rebuild_scene(&mut self, renderer: &Renderer, device: &wgpu::Device) {
        if let PageState::Ready(r) = &mut self.state {
            let scene = SceneData::build(&r.repo, &r.layout);
            r.gpu = renderer.create_scene(device, &r.repo.name, scene);
            self.curves_dirty = true;
        }
    }

    pub fn relayout(&mut self, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) {
        let changed = self.only_changed.then(|| self.changed_files()).filter(|set| !set.is_empty());
        let connected = (self.only_connected && !self.selections.is_empty()).then(|| self.connected_files(filters));
        let only = match (changed, connected) {
            (Some(a), Some(b)) => Some(a.union(&b).copied().collect()),
            (a, b) => a.or(b),
        };
        let filter = LayoutFilter { only_files: only, ..self.filter.clone() };
        let selected = self.selected;
        let PageState::Ready(r) = &mut self.state else { return };
        let t = Instant::now();
        let keep = selected.and_then(|n| r.layout.nodes.get(n)).map(|n| n.rel_path.clone());
        r.layout = Layout::build_filtered(&r.repo, &filter);
        let scene = SceneData::build(&r.repo, &r.layout);
        r.gpu = renderer.create_scene(device, &r.repo.name, scene);
        r.tree_dirty = true;
        self.selected = keep.and_then(|p| r.layout.node_for_path(&r.repo, &p));
        self.hover = None;
        self.hover_cell = None;
        self.curves_dirty = true;
        log::debug!("relayout: {} nodes in {:.1} ms", r.layout.nodes.len(), t.elapsed().as_secs_f32() * 1000.0);
    }

    pub fn reload_file(
        &mut self,
        rel_path: &str,
        renderer: &Renderer,
        device: &wgpu::Device,
        filters: &RelFilters,
    ) -> bool {
        let PageState::Ready(r) = &mut self.state else { return false };
        let Some(index) = r.repo.file_index(rel_path) else { return false };
        r.repo.reload_file(index, &ScanOptions::default());
        r.tokens = Default::default();
        self.selections.clear();
        self.search = None;
        self.relayout(renderer, device, filters);
        true
    }

    pub fn toggle_symbol(&mut self, sym: SymRef, clicked: Option<Site>) -> bool {
        self.curves_dirty = true;
        if let Some(i) = self.selections.iter().position(|s| s.sym == sym) {
            self.selections.remove(i);
            return false;
        }
        let Some(r) = self.ready() else { return false };
        let rel = r.repo.relations(sym, clicked);
        log::info!(
            "relations of '{}': {} items in {:.1} ms{}",
            r.repo.sym_name(sym),
            rel.items.len(),
            rel.search_ms,
            if rel.truncated { " (truncated)" } else { "" }
        );
        self.selections.push(Selection { sym, rel, outside: 0 });
        true
    }
}

fn count_files(dir: &Dir) -> u32 {
    dir.files.len() as u32 + dir.dirs.iter().map(count_files).sum::<u32>()
}

struct TreeView<'a> {
    repo: &'a Repo,
    open: &'a HashSet<String>,
    hidden: &'a HashSet<String>,
}

fn push_dir(dir: &Dir, depth: u16, locked: bool, view: &TreeView, rows: &mut Vec<TreeRow>) {
    for d in &dir.dirs {
        let hidden = view.hidden.contains(&d.rel_path);
        let open = view.open.contains(&d.rel_path);
        rows.push(TreeRow {
            depth,
            is_dir: true,
            open,
            hidden,
            locked,
            name: d.name.clone(),
            path: d.rel_path.clone(),
            count: count_files(d),
        });
        if open {
            push_dir(d, depth + 1, locked || hidden, view, rows);
        }
    }
    for &fi in &dir.files {
        let f = &view.repo.files[fi];
        rows.push(TreeRow {
            depth,
            is_dir: false,
            open: false,
            hidden: view.hidden.contains(&f.rel_path),
            locked,
            name: f.name.clone(),
            path: f.rel_path.clone(),
            count: 0,
        });
    }
}

pub fn build_tree(repo: &Repo, open: &HashSet<String>, hidden: &HashSet<String>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    push_dir(&repo.tree, 0, false, &TreeView { repo, open, hidden }, &mut rows);
    rows
}
