//! A page is one folder opened on its own canvas (shown as a tab).

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

/// One row of the explorer tree.
pub struct TreeRow {
    pub depth: u16,
    pub is_dir: bool,
    pub open: bool,
    /// Unchecked: left out of the canvas.
    pub hidden: bool,
    /// An ancestor is hidden, so this row's checkbox has no effect.
    pub locked: bool,
    pub name: String,
    pub path: String,
    /// Files below (directories).
    pub count: u32,
}

pub struct Ready {
    pub repo: Repo,
    /// Which file imports which, from the import statements.
    pub deps: canvas_core::deps::Deps,
    pub layout: Layout,
    pub gpu: SceneGpu,
    pub tokens: TokenCache,
    pub load_ms: u128,
    pub tree: Vec<TreeRow>,
    pub tree_dirty: bool,
}

pub enum PageState {
    /// Opened but not read yet: a folder restored from the last session
    /// that has not been shown. It is scanned when the user goes to it.
    Waiting,
    Loading {
        progress: Arc<ScanProgress>,
        rx: Receiver<Loaded>,
        started: Instant,
    },
    Ready(Box<Ready>),
    Failed(String),
}

/// Results of the last "search in files".
pub struct SearchResults {
    pub query: String,
    pub hits: Vec<Site>,
    pub truncated: bool,
    pub ms: f32,
}

/// A symbol the user picked, with everything related to it.
pub struct Selection {
    pub sym: SymRef,
    pub rel: Relations,
    /// Relations whose other end is not on the canvas right now.
    pub outside: usize,
}

/// Which relation kinds are drawn, and how strongly.
#[derive(Clone, Copy, Debug, PartialEq)]
/// What to draw of the dependencies between files.
pub struct DepFilters {
    /// Edges of the selected file or folder.
    pub selected: bool,
    /// Every edge of the repository, faint.
    pub all: bool,
    /// Per `DepKind::index()`.
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
    /// Per kind: highlight the bodies of the functions at the far end
    /// (for definitions: the selected symbol's own body).
    pub bodies: [bool; 5],
    pub body_opacity: [f32; 5],
}

impl Default for RelFilters {
    fn default() -> Self {
        let mut opacity = [1.0; 5];
        // Outgoing calls are usually the noisiest kind.
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
    /// Where the camera is animating to, if anywhere.
    pub target: Option<Camera>,
    /// Node under the pointer (canvas or explorer).
    pub hover: Option<usize>,
    pub selected: Option<usize>,
    /// `(file, line, col)` under the pointer when it is over code.
    pub hover_cell: Option<(u32, u32, u32)>,
    /// The camera was framed once the page finished loading.
    pub framed: bool,
    /// Camera to restore from the previous session instead of framing.
    pub restore_camera: Option<Camera>,
    /// Collapsed and hidden paths (the user's choice; `only_files` is derived).
    pub filter: LayoutFilter,
    /// Directories expanded in the explorer.
    pub tree_open: HashSet<String>,
    pub selections: Vec<Selection>,
    pub only_connected: bool,
    /// Relation curves must be rebuilt (selection, filters or layout changed).
    pub curves_dirty: bool,
    pub search: Option<SearchResults>,
    /// What git says changed in this folder; `None` outside a repository.
    pub git: Option<GitState>,
    git_rx: Option<Receiver<Option<GitState>>>,
    /// Show only the changed files on the canvas.
    pub only_changed: bool,
    /// Files with edits in memory, by path. They stay here until saved,
    /// so leaving a file does not lose what was typed.
    pub editors: std::collections::HashMap<String, Editor>,
    /// The file the caret is in.
    pub editing: Option<String>,
    /// The node whose dependency edges are in the curves on the GPU.
    pub curves_node: Option<usize>,
    /// `(drawn, total)` dependency edges of the selection, for the panel.
    pub dep_edges: (usize, usize),
    /// The commit message being written in the source control panel.
    pub commit_message: String,
    /// A git command started from the panel is running.
    git_op: Option<Receiver<(crate::git::GitOp, Result<String, String>)>>,
    /// How the last one went: `(ok, text)`.
    pub git_note: Option<(bool, String)>,
    /// History of the selected file.
    pub timeline: Timeline,
    timeline_rx: Option<Receiver<(String, Vec<Commit>)>>,
}

impl SearchResults {
    /// One place to mark on the canvas (where a definition was found).
    pub fn single(site: Site) -> Self {
        SearchResults { query: String::new(), hits: vec![site], truncated: false, ms: 0.0 }
    }
}

/// The commits that touched one file, newest first.
#[derive(Default)]
pub struct Timeline {
    /// The file this is (or will be) the history of.
    pub path: Option<String>,
    pub commits: Vec<Commit>,
    pub loading: bool,
}

impl Page {
    /// Open `root`. With `scan` false it only takes its place in the list
    /// and is read when first shown ([`Page::ensure_scanned`]).
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

    /// Start reading the folder if that has not happened yet.
    pub fn ensure_scanned(&mut self) {
        if matches!(self.state, PageState::Waiting) {
            self.start_scan();
        }
    }

    pub fn is_waiting(&self) -> bool {
        matches!(self.state, PageState::Waiting)
    }

    /// Git is being asked and has not answered yet.
    pub fn git_loading(&self) -> bool {
        self.git_rx.is_some() && self.git.is_none()
    }

    pub fn to_session(&self, scale: f32) -> PageSession {
        let sorted = |set: &HashSet<String>| {
            let mut v: Vec<String> = set.iter().cloned().collect();
            v.sort();
            v
        };
        PageSession {
            path: self.root.clone(),
            camera: self.framed.then_some((self.camera.center, self.camera.zoom / scale)),
            collapsed: sorted(&self.filter.collapsed),
            hidden: sorted(&self.filter.hidden),
            tree_open: sorted(&self.tree_open),
        }
    }

    /// Ask git what changed, on a background thread.
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

    /// The editor of the file the caret is in.
    pub fn editor(&self) -> Option<&Editor> {
        self.editors.get(self.editing.as_deref()?)
    }

    pub fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.editors.get_mut(self.editing.as_deref()?)
    }

    /// Put the caret in `rel_path`, reading the file as it is on disk the
    /// first time. False when it cannot be edited (unreadable, not text).
    pub fn begin_edit(&mut self, rel_path: &str) -> bool {
        if !self.editors.contains_key(rel_path) {
            let Ok(bytes) = std::fs::read(self.root.join(rel_path)) else { return false };
            // Only text that survives a round trip: nothing is lost on save.
            let Ok(source) = String::from_utf8(bytes) else { return false };
            if source.contains('\0') {
                return false;
            }
            self.editors.insert(rel_path.to_string(), Editor::open(rel_path.to_string(), &source));
        }
        self.editing = Some(rel_path.to_string());
        true
    }

    /// Show what the editor of `rel_path` holds: the card follows every
    /// keystroke. The caller drops this page's cached text first.
    pub fn show_edit(&mut self, rel_path: &str, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) {
        let Some(source) = self.editors.get(rel_path).map(|e| e.display_source()) else { return };
        let PageState::Ready(r) = &mut self.state else { return };
        let Some(index) = r.repo.file_index(rel_path) else { return };
        let before = (r.repo.files[index].line_count(), r.repo.files[index].max_cols);
        r.repo.replace_source(index, &source, &ScanOptions::default());
        r.tokens = Default::default();
        let after = (r.repo.files[index].line_count(), r.repo.files[index].max_cols);
        // Symbols of this file moved: selections pointing at them are stale.
        if !self.selections.is_empty() {
            self.selections.clear();
            self.curves_dirty = true;
        }
        self.search = None;
        // The card only changes size when a line is added, removed or outgrows it.
        if before != after {
            self.relayout(renderer, device, filters);
        }
    }

    pub fn git_busy(&self) -> bool {
        self.git_op.is_some()
    }

    /// Run a source control command on a background thread; the changes
    /// are read again when it finishes.
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

    /// Take in a finished `run_git`. True when one finished.
    pub fn poll_git_op(&mut self) -> bool {
        let Some(rx) = &self.git_op else { return false };
        let Ok((op, result)) = rx.try_recv() else { return false };
        self.git_op = None;
        match (&op, result) {
            (crate::git::GitOp::Commit(_) | crate::git::GitOp::CommitAll(_), Ok(line)) => {
                self.commit_message.clear();
                self.git_note = Some((true, line));
            }
            (crate::git::GitOp::Sync, Ok(_)) => self.git_note = Some((true, "Synced with the remote.".into())),
            (crate::git::GitOp::Switch(branch), Ok(_)) => self.git_note = Some((true, format!("On branch {branch}."))),
            (_, Ok(_)) => {}
            (_, Err(e)) => self.git_note = Some((false, e)),
        }
        self.refresh_git();
        true
    }

    /// Take in a finished `refresh_git`. True when the changes were updated.
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

    /// Make the timeline follow `path` (the selected file): its history is
    /// read on a background thread when it is not the one already shown.
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

    /// Files git reports as changed that exist on disk (deleted ones do not).
    pub fn changed_files(&self) -> HashSet<u32> {
        let (Some(git), Some(r)) = (&self.git, self.ready()) else { return HashSet::new() };
        git.changes.iter().filter_map(|c| r.repo.file_index(&c.path)).map(|i| i as u32).collect()
    }

    /// (Re)scan the folder on a background thread.
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

    /// Run a text search over every file of the page.
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

    /// Poll a running scan; returns the finished data when it is ready.
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

    /// Files touched by the selected symbols (their definitions and every
    /// relation of an enabled kind).
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

    /// Build the GPU scene again from the layout as it is: its colours come
    /// from the theme, so a new theme needs a new scene.
    pub fn rebuild_scene(&mut self, renderer: &Renderer, device: &wgpu::Device) {
        if let PageState::Ready(r) = &mut self.state {
            let scene = SceneData::build(&r.repo, &r.layout);
            r.gpu = renderer.create_scene(device, &r.repo.name, scene);
            self.curves_dirty = true;
        }
    }

    /// Rebuild the layout and the GPU scene after the filter changed
    /// (collapse, hide, "only connected"). Keeps the selection by path.
    pub fn relayout(&mut self, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) {
        let changed = self.only_changed.then(|| self.changed_files()).filter(|set| !set.is_empty());
        let connected = (self.only_connected && !self.selections.is_empty()).then(|| self.connected_files(filters));
        // Both on: the changed files and what the selection connects them to.
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

    /// `rel_path` changed on disk: read it again and rebuild what shows it.
    /// The caller drops this page's cached text first. False when the file
    /// is not part of this page.
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
        // Symbols and lines of that file moved: what pointed at them is stale.
        self.selections.clear();
        self.search = None;
        self.relayout(renderer, device, filters);
        true
    }

    /// Add `sym` to the selection, or remove it if it is already there.
    /// Returns true when it was added.
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

/// Flatten the explorer tree into the rows that are currently visible.
pub fn build_tree(repo: &Repo, open: &HashSet<String>, hidden: &HashSet<String>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    push_dir(&repo.tree, 0, false, &TreeView { repo, open, hidden }, &mut rows);
    rows
}
