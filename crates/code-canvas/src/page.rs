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
use crate::session::PageSession;

pub struct Loaded {
    pub repo: Repo,
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
    pub layout: Layout,
    pub gpu: SceneGpu,
    pub tokens: TokenCache,
    pub load_ms: u128,
    pub tree: Vec<TreeRow>,
    pub tree_dirty: bool,
}

pub enum PageState {
    Loading { progress: Arc<ScanProgress>, rx: Receiver<Loaded>, started: Instant },
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
    /// Conversation with Claude Code about this folder.
    pub chat: crate::claude::Chat,
    pub search: Option<SearchResults>,
}

impl Page {
    pub fn open(id: u32, root: PathBuf, saved: Option<&PageSession>) -> Page {
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
            chat: Default::default(),
            search: None,
        };
        if let Some(s) = saved {
            page.filter.collapsed = s.collapsed.iter().cloned().collect();
            page.filter.hidden = s.hidden.iter().cloned().collect();
            page.tree_open = s.tree_open.iter().cloned().collect();
            page.restore_camera = s.camera.map(|(center, zoom)| Camera { center, zoom });
        }
        page.start_scan();
        page
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

    /// (Re)scan the folder on a background thread.
    pub fn start_scan(&mut self) {
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
                let _ = tx.send(Loaded { repo, layout, scene, total_ms });
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

    /// Rebuild the layout and the GPU scene after the filter changed
    /// (collapse, hide, "only connected"). Keeps the selection by path.
    pub fn relayout(&mut self, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) {
        let only = (self.only_connected && !self.selections.is_empty()).then(|| self.connected_files(filters));
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
