use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_core::scan::{self, Dir, ScanProgress};
use canvas_core::{Layout, LayoutFilter, RelKind, Relations, Repo, ScanOptions, Site, SymRef, TokenCache};
use canvas_render::{Renderer, SceneData, SceneGpu};

use crate::camera::Camera;
use crate::editor::Editor;
use crate::ext::Found;
use crate::git::{Change, Commit, CommitFile, FileChange, GitState, Row};
use crate::session::PageSession;

pub struct Gpu<'a> {
    pub renderer: &'a Renderer,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

pub struct Loaded {
    pub repo: Repo,
    pub deps: canvas_core::deps::Deps,
    pub layout: Layout,
    pub order: Option<Arc<canvas_core::Order>>,
    pub scene: SceneData,
    pub total_ms: u128,
}

pub struct TreeRow {
    pub depth: u16,
    pub is_dir: bool,
    pub open: bool,
    pub hidden: bool,
    pub locked: bool,
    pub ignored: bool,
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

impl Ready {
    /// A grammar's tokens for a file, and from the whole file its far-away bars,
    /// written on the GPU when they are not the ones there. The chunks newly covered.
    pub fn paint(&mut self, index: usize, painted: Arc<canvas_core::Painted>, gpu: &Gpu) -> Vec<u32> {
        let blocks = self.repo.files.get(index).and_then(|f| painted.blocks(f));
        let fresh = self.tokens.paint(index, painted);
        if let Some(blocks) = blocks {
            let file = &mut self.repo.files[index];
            if file.analysis.painted.as_deref() != Some(&blocks[..]) {
                file.analysis.painted = Some(blocks.into());
                self.gpu.blocks_changed(gpu.device, gpu.queue, &self.repo, &self.layout, index);
            }
        }
        fresh
    }

    /// Forgets the far-away bars grammars gave; whether there were any.
    pub fn unpaint(&mut self) -> bool {
        let mut had = false;
        for file in &mut self.repo.files {
            had |= file.analysis.painted.take().is_some();
        }
        had
    }
}

pub enum PageState {
    Waiting,
    Loading { progress: Arc<ScanProgress>, rx: Receiver<Loaded>, started: Instant },
    Ready(Box<Ready>),
    Failed(String),
}

/// The marks on the canvas of the Search panel, the find bar or a place gone to.
// The query, the cut and the time are not shown anywhere since the Search panel keeps its own results.
#[allow(dead_code)]
pub struct SearchResults {
    pub query: String,
    pub hits: Vec<Site>,
    pub truncated: bool,
    pub ms: f32,
    pub current: Option<usize>,
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
    pub version_hover: Option<(String, u32)>,
    pub framed: bool,
    pub restore_camera: Option<Camera>,
    pub pane_center: Option<[f32; 2]>,
    pub pane_viewport: [f32; 2],
    pub extent: Option<canvas_core::Rect>,
    pub filter: LayoutFilter,
    pub arranged: bool,
    pub tree_open: HashSet<String>,
    pub selections: Vec<Selection>,
    pub want_exact: Vec<SymRef>,
    pub only_connected: bool,
    pub curves_dirty: bool,
    pub search: Option<SearchResults>,
    /// What the Search panel found in the folder's files.
    pub grep: crate::search::Grep,
    pub git: Option<GitState>,
    pub history: crate::history::HistoryView,
    /// The interactive rebase being arranged in its card.
    pub rebase: Option<crate::rebase::Plan>,
    git_rx: Option<Receiver<Option<GitState>>>,
    pub only_changed: bool,
    /// With only the changed files shown: also every file of a folder that has a changed file.
    pub changed_folders: bool,
    /// With only the changed files shown: also the files that import or are imported by a changed file.
    pub changed_neighbours: bool,
    /// How many files the canvas was last narrowed to (None: it shows them all).
    pub narrowed_to: Option<usize>,
    pub editors: std::collections::HashMap<String, Editor>,
    pub doodles: Vec<crate::doodle::Stroke>,
    pub editing: Option<String>,
    pub curves_node: Option<usize>,
    pub dep_edges: (usize, usize),
    pub dep_marks: Vec<crate::overlay::DepMark>,
    pub dep_zoom: Option<i32>,
    pub commit_message: String,
    git_op: Option<Receiver<(crate::git::GitOp, Result<String, String>)>>,
    pub git_note: Option<GitNote>,
    pub git_said: Option<(bool, String)>,
    message_rx: Option<Receiver<Result<String, String>>>,
    pub timeline: Timeline,
    timeline_rx: Option<Receiver<(String, Vec<Commit>)>>,
    timeline_stale: bool,
    heads: HashMap<String, Option<String>>,
    heads_asked: HashSet<String>,
    heads_round: u32,
    heads_tx: mpsc::Sender<(u32, String, Option<String>)>,
    heads_rx: Receiver<(u32, String, Option<String>)>,
    previewed: Vec<(String, Option<FileChange>)>,
    pub changed_dirs: HashSet<String>,
    pub touched: Vec<Touch>,
    pub reveal: Option<String>,
    heads_at: String,
    pub diff: DiffView,
    pub showing: Option<Showing>,
    showing_rx: Option<Receiver<(String, Vec<CommitFile>, usize)>>,
    showing_changed: bool,
    pub frame_wanted: bool,
    pub diffs: HashMap<String, Diffed>,
    diff_stale: bool,
    diff_editing: Option<String>,
    pub read_again: bool,
    pub warm_at: usize,
    pub warm_batch: usize,
    pub only: bool,
    pub shared: Option<u32>,
    pub read_only: bool,
    pub refused_edit: bool,
    pub no_git: bool,
    pub extra: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DiffView {
    pub on: bool,
    pub split: bool,
    pub fold: bool,
    pub around: u32,
}

impl Default for DiffView {
    fn default() -> Self {
        DiffView { on: false, split: false, fold: true, around: 3 }
    }
}

pub struct Showing {
    pub commit: Commit,
    pub files: Vec<CommitFile>,
    pub total: usize,
    pub here: usize,
    pub loading: bool,
}

pub struct Diffed {
    stamp: u64,
    pub rows: Vec<Row>,
    lines: Vec<u32>,
    pub plain: String,
    committed: Option<String>,
    symbols: Vec<(u32, u32)>,
    /// Some unchanged lines of the file are folded away.
    folded: bool,
}

impl Diffed {
    pub fn row(&self, line: u32) -> u32 {
        crate::git::row_of(&self.lines, self.rows.len(), line)
    }

    pub fn shows(&self, line: u32) -> bool {
        crate::git::shows(&self.lines, &self.rows, line)
    }

    fn text(&self) -> &str {
        self.committed.as_deref().unwrap_or(&self.plain)
    }

    fn place(&self, line: u32) -> (u32, bool) {
        row_seen(&self.rows, &self.lines, line)
    }
}

fn row_seen(rows: &[Row], lines: &[u32], line: u32) -> (u32, bool) {
    use crate::git::RowKind;
    let row = crate::git::row_of(lines, rows.len(), line);
    let seen = |at: &Row| matches!(at.kind, RowKind::Same | RowKind::Added) && at.line == line;
    (row, rows.get(row as usize).is_some_and(seen))
}

const MAX_DIFFS: usize = 400;

const SCANS_KEPT_DAYS: u64 = 30;

fn scans_folder() -> PathBuf {
    crate::session::data_dir().join("scans")
}

pub fn forget_scan(root: &std::path::Path) {
    if let Some(kept) = scan_cache(root) {
        let _ = std::fs::remove_file(kept);
    }
}

fn scan_cache(root: &std::path::Path) -> Option<PathBuf> {
    if std::env::var_os("CODE_CANVAS_NO_CACHE").is_some() {
        return None;
    }
    let name = root.to_string_lossy().to_lowercase();
    let hash = canvas_core::store::names_hash(std::iter::once(name.as_str()));
    Some(scans_folder().join(format!("{hash:016x}.bin")))
}

pub fn forget_old_scans() {
    std::thread::spawn(|| {
        let Ok(entries) = std::fs::read_dir(scans_folder()) else { return };
        for entry in entries.flatten() {
            let age = entry.metadata().and_then(|m| m.modified()).ok().and_then(|at| at.elapsed().ok());
            if age.is_some_and(|age| age.as_secs() > SCANS_KEPT_DAYS * 86_400) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    });
}

type Skipped = (Option<Vec<String>>, Option<Vec<String>>);

static NEVER_READ: std::sync::Mutex<Skipped> = std::sync::Mutex::new((None, None));

pub fn never_read(dirs: Option<Vec<String>>, files: Option<Vec<String>>) {
    *NEVER_READ.lock().unwrap() = (dirs, files);
}

pub fn scan_options() -> ScanOptions {
    let mut options = ScanOptions::default();
    let (dirs, files) = NEVER_READ.lock().unwrap().clone();
    options.skip_dirs = dirs.unwrap_or(options.skip_dirs);
    options.skip_files = files.unwrap_or(options.skip_files);
    options
}

fn arrange(repo: &Repo, deps: &canvas_core::deps::Deps) -> canvas_core::Order {
    let t = Instant::now();
    let order = canvas_core::Order::build(repo, deps);
    log::info!(
        "arranged '{}' by its imports: {} layers at most, {} cycles kept together, in {:.1} ms",
        repo.name,
        order.layers + 1,
        order.tied,
        t.elapsed().as_secs_f64() * 1000.0
    );
    order
}

pub fn text_budget() -> usize {
    let megabytes = std::env::var("CODE_CANVAS_TEXT_MB").ok().and_then(|mb| mb.parse::<usize>().ok());
    megabytes.unwrap_or(768) << 20
}

pub struct Touch {
    pub path: String,
    pub lines: Vec<(u32, u32)>,
    pub at: Instant,
}

impl Touch {
    pub const SHOWN: f32 = 6.0;

    pub fn strength(&self) -> f32 {
        let age = self.at.elapsed().as_secs_f32();
        ((Self::SHOWN - age) / (Self::SHOWN * 0.5)).clamp(0.0, 1.0)
    }
}

pub struct GitNote {
    pub ok: bool,
    pub text: String,
    pub since: Instant,
}

impl GitNote {
    pub const DONE_SHOWN: Duration = Duration::from_secs(6);
    pub const ERROR_SHOWN: Duration = Duration::from_secs(20);

    pub fn new(ok: bool, text: String) -> Self {
        GitNote { ok, text, since: Instant::now() }
    }

    pub fn left(&self, now: Instant) -> Option<Duration> {
        let shown = if self.ok { Self::DONE_SHOWN } else { Self::ERROR_SHOWN };
        shown.checked_sub(now.saturating_duration_since(self.since)).filter(|left| !left.is_zero())
    }
}

impl SearchResults {
    pub fn single(site: Site) -> Self {
        SearchResults { query: String::new(), hits: vec![site], truncated: false, ms: 0.0, current: None }
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
        let (heads_tx, heads_rx) = mpsc::channel();
        let no_git = root.starts_with(crate::session::data_dir().join("shared"));
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
            version_hover: None,
            framed: false,
            restore_camera: None,
            pane_center: None,
            pane_viewport: [0.0, 0.0],
            extent: None,
            filter: LayoutFilter::default(),
            arranged: false,
            tree_open: HashSet::new(),
            selections: Vec::new(),
            want_exact: Vec::new(),
            only_connected: false,
            curves_dirty: true,
            search: None,
            grep: Default::default(),
            git: None,
            history: Default::default(),
            rebase: None,
            git_rx: None,
            only_changed: false,
            changed_folders: false,
            changed_neighbours: false,
            narrowed_to: None,
            editors: Default::default(),
            doodles: Vec::new(),
            editing: None,
            curves_node: None,
            dep_edges: (0, 0),
            dep_marks: Vec::new(),
            dep_zoom: None,
            commit_message: String::new(),
            git_op: None,
            git_note: None,
            git_said: None,
            message_rx: None,
            timeline: Timeline::default(),
            timeline_rx: None,
            timeline_stale: false,
            heads: HashMap::new(),
            heads_asked: HashSet::new(),
            heads_round: 0,
            heads_tx,
            heads_rx,
            previewed: Vec::new(),
            changed_dirs: HashSet::new(),
            touched: Vec::new(),
            reveal: None,
            heads_at: String::new(),
            diff: DiffView::default(),
            showing: None,
            showing_rx: None,
            showing_changed: false,
            frame_wanted: false,
            diffs: HashMap::new(),
            diff_stale: false,
            diff_editing: None,
            read_again: false,
            warm_at: 0,
            warm_batch: 256,
            only: false,
            shared: None,
            read_only: false,
            refused_edit: false,
            no_git,
            extra: Vec::new(),
        };
        if let Some(s) = saved {
            page.filter.collapsed = s.collapsed.iter().cloned().collect();
            page.filter.hidden = s.hidden.iter().cloned().collect();
            page.tree_open = s.tree_open.iter().cloned().collect();
            page.arranged = s.arranged;
            page.restore_camera = s.camera.map(|(center, zoom)| Camera { center, zoom });
        }
        if scan {
            page.start_scan();
        } else {
            page.state = PageState::Waiting;
        }
        page
    }

    pub fn of_files(id: u32, root: PathBuf, files: Vec<String>) -> Page {
        let mut page = Page::open(id, root, None, false);
        (page.only, page.extra) = (true, files);
        page.start_scan();
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
        let window = if self.pane_center.is_some() { self.pane_viewport } else { viewport };
        let half = [window[0] * 0.5, window[1] * 0.5];
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
            arranged: self.arranged,
        }
    }

    pub fn refresh_git(&mut self) {
        if self.no_git {
            return;
        }
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

    pub fn want_head(&mut self, rel_path: &str) {
        self.want_heads(vec![rel_path.to_string()]);
    }

    fn want_heads(&mut self, paths: Vec<String>) {
        let Some(git) = &self.git else { return };
        let paths: Vec<String> = paths.into_iter().filter(|path| self.heads_asked.insert(path.clone())).collect();
        if paths.is_empty() {
            return;
        }
        let committed_as = |path: &String| git.change(path).and_then(|change| change.from.clone());
        let named: Vec<String> = paths.iter().map(|path| committed_as(path).unwrap_or_else(|| path.clone())).collect();
        let (root, prefix, round, tx) =
            (self.root.clone(), git.prefix.clone(), self.heads_round, self.heads_tx.clone());
        std::thread::spawn(move || {
            let texts = crate::git::head_texts(&root, &prefix, &named);
            for (rel, text) in paths.into_iter().zip(texts) {
                let _ = tx.send((round, rel, text));
            }
            crate::ext::wake_ui();
        });
    }

    pub fn poll_heads(&mut self) -> bool {
        let mut arrived = false;
        while let Ok((round, rel, text)) = self.heads_rx.try_recv() {
            if round == self.heads_round {
                self.heads.insert(rel, text);
                arrived = true;
            }
        }
        self.diff_stale |= arrived;
        arrived
    }

    pub fn rows(&self, rel_path: &str) -> Option<&[Row]> {
        self.diffs.get(rel_path).map(|shown| shown.rows.as_slice())
    }

    pub fn row_of(&self, rel_path: &str, line: u32) -> u32 {
        self.diffs.get(rel_path).map_or(line, |shown| shown.row(line))
    }

    pub fn line_of(&self, rel_path: &str, row: u32) -> Option<u32> {
        crate::git::line_of(self.rows(rel_path), row)
    }

    pub fn set_diff(&mut self, view: DiffView) {
        let (shown, sides) = (if view.on { "shown" } else { "not shown" }, if view.split { 2 } else { 1 });
        let folded =
            if view.fold { format!("{} lines kept around a change", view.around) } else { "nothing folded".into() };
        log::info!("what changed inside the files: {shown}, {sides} sides, {folded}");
        self.diff = view;
        self.diff_stale = true;
    }

    pub fn show_commit(&mut self, commit: Option<Commit>) {
        self.showing_rx = None;
        self.showing_changed = true;
        self.diff_stale = true;
        self.showing = None;
        let Some(commit) = commit else { return };
        let (tx, rx) = mpsc::channel();
        let (root, hash) = (self.root.clone(), commit.hash.clone());
        let prefix = self.git.as_ref().map(|git| git.prefix.clone()).unwrap_or_default();
        std::thread::spawn(move || {
            let (files, total) = crate::git::commit_files(&root, &prefix, &hash, MAX_DIFFS);
            let _ = tx.send((hash, files, total));
            crate::ext::wake_ui();
        });
        self.showing = Some(Showing { commit, files: Vec::new(), total: 0, here: 0, loading: true });
        self.showing_rx = Some(rx);
    }

    pub fn poll_commit(&mut self) -> bool {
        let Some(rx) = &self.showing_rx else { return false };
        let Ok((hash, files, total)) = rx.try_recv() else { return false };
        self.showing_rx = None;
        if let Some(showing) = self.showing.as_mut().filter(|showing| showing.commit.hash == hash) {
            (showing.files, showing.total, showing.loading) = (files, total, false);
            self.showing_changed = true;
            self.diff_stale = true;
        }
        true
    }

    pub fn commit_shown(&self) -> Option<&Showing> {
        self.showing.as_ref().filter(|showing| !showing.loading)
    }

    fn commit_files(&self) -> Option<HashSet<u32>> {
        let (showing, r) = (self.commit_shown()?, self.ready()?);
        Some(showing.files.iter().filter_map(|file| r.repo.file_index(&file.path)).map(|i| i as u32).collect())
    }

    pub fn diff_due(&mut self) -> bool {
        if self.diff_editing != self.editing {
            self.diff_editing = self.editing.clone();
            self.diff_stale = true;
        }
        if self.showing.as_ref().is_some_and(|showing| showing.loading) {
            return false;
        }
        if !self.diff.on && self.diffs.is_empty() && !self.showing_changed && self.showing.is_none() {
            self.diff_stale = false;
        }
        self.diff_stale && self.ready().is_some()
    }

    pub fn diff_settled(&self, rel_path: &str) -> bool {
        let waiting = self.git_rx.is_some()
            || self.diff_stale
            || (self.heads_asked.contains(rel_path) && !self.heads.contains_key(rel_path));
        !self.diff.on || self.diffs.contains_key(rel_path) || !waiting
    }

    pub fn sync_diff(&mut self, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) -> bool {
        use std::hash::{Hash, Hasher};
        self.diff_stale = false;
        let split = self.diff.split;
        let wanted: Vec<String> = match (self.commit_shown(), &self.git, self.diff.on) {
            (Some(showing), ..) => showing
                .files
                .iter()
                .filter(|file| self.editing.as_deref() != Some(file.path.as_str()))
                .map(|file| file.path.clone())
                .collect(),
            (None, Some(git), true) => git
                .changes
                .iter()
                .filter(|c| !matches!(c.change, Change::Untracked | Change::Added | Change::Deleted))
                .filter(|c| self.editing.as_deref() != Some(c.path.as_str()))
                .map(|c| c.path.clone())
                .take(MAX_DIFFS)
                .collect(),
            _ => Vec::new(),
        };
        if self.commit_shown().is_none() {
            let unknown: Vec<String> = wanted.iter().filter(|path| !self.heads.contains_key(*path)).cloned().collect();
            self.want_heads(unknown);
        }
        let around = self.diff.fold.then_some(self.diff.around);
        let laid = std::mem::take(&mut self.showing_changed);
        let committed: Option<HashMap<&str, &CommitFile>> = self
            .showing
            .as_ref()
            .filter(|showing| !showing.loading)
            .map(|showing| showing.files.iter().map(|file| (file.path.as_str(), file)).collect());
        let PageState::Ready(r) = &mut self.state else { return false };
        let options = ScanOptions::default();
        let current = |editors: &HashMap<String, Editor>, rel: &str| match editors.get(rel) {
            Some(editor) => Some(editor.text()),
            None => std::fs::read_to_string(self.root.join(rel)).ok(),
        };
        let mut changed = false;
        let keep: HashSet<&str> = wanted.iter().map(String::as_str).collect();
        let gone: Vec<String> = self.diffs.keys().filter(|rel| !keep.contains(rel.as_str())).cloned().collect();
        for rel in gone {
            self.diffs.remove(&rel);
            if let (Some(index), Some(text)) = (r.repo.file_index(&rel), current(&self.editors, &rel)) {
                r.repo.replace_source(index, &text, &options);
                changed = true;
            }
        }
        let mut here = 0;
        for rel in &wanted {
            let Some(index) = r.repo.file_index(rel) else { continue };
            here += 1;
            let placeholder = r.repo.files[index].note.as_ref().is_some_and(|note| !note.starts_with("truncated"));
            if placeholder && !self.diffs.contains_key(rel) {
                continue;
            }
            let Some(text) = current(&self.editors, rel) else { continue };
            let (old, new): (&str, &str) = match &committed {
                Some(files) => {
                    let Some(file) = files.get(rel.as_str()) else { continue };
                    (file.before.as_deref().unwrap_or(""), file.after.as_deref().unwrap_or(""))
                }
                None => {
                    let Some(Some(head)) = self.heads.get(rel) else { continue };
                    (head, &text)
                }
            };
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (old, new, &text, around, split).hash(&mut hasher);
            let stamp = hasher.finish();
            if self.diffs.get(rel).is_some_and(|shown| shown.stamp == stamp) {
                continue;
            }
            let made = match split {
                true => crate::git::side_by_side(old, new, around),
                false => crate::git::unified(old, new, around),
            };
            match made {
                Some(shown) => {
                    let (rows, lines) = (shown.rows, shown.lines);
                    let place = |line: u32| row_seen(&rows, &lines, line);
                    let symbols = r.repo.replace_shown(index, &shown.text, new, place, &options);
                    r.repo.files[index].fold = shown.fold;
                    let numbered = |row: &Row| match row.kind {
                        crate::git::RowKind::Same | crate::git::RowKind::Added => row.line + 1,
                        _ => 0,
                    };
                    r.repo.files[index].numbers = Some(rows.iter().map(numbered).collect());
                    let plain = canvas_core::scan::normalise(&text).0;
                    let committed = committed.is_some().then(|| canvas_core::scan::normalise(new).0);
                    let folded = rows.iter().any(|row| row.kind == crate::git::RowKind::Gap);
                    self.diffs.insert(rel.clone(), Diffed { stamp, rows, lines, plain, committed, symbols, folded });
                }
                None => {
                    if self.diffs.remove(rel).is_none() {
                        continue;
                    }
                    r.repo.replace_source(index, &text, &options);
                }
            }
            changed = true;
        }
        if changed {
            r.tokens.reset();
            if r.repo.files.len() <= 60_000 {
                r.repo.reindex_symbols();
            }
            self.selections.clear();
            self.want_exact.clear();
            self.search = None;
            self.grep.marks_due = true;
        }
        drop(committed);
        if changed && self.showing.is_none() {
            log::info!("what changed is shown inside {} files", self.diffs.len());
        }
        if let Some(showing) = self.showing.as_mut().filter(|showing| !showing.loading) {
            showing.here = here;
            if changed || laid {
                let (subject, with_diff) = (&showing.commit.subject, self.diffs.len());
                log::info!("commit '{subject}': what changed is shown in {with_diff} of its {} files", showing.total);
            }
        }
        if changed || laid {
            self.relayout(renderer, device, filters);
        }
        self.frame_wanted |= laid;
        changed || laid
    }

    pub fn unsaved(&self) -> impl Iterator<Item = &Editor> {
        self.editors.values().filter(|e| e.dirty)
    }

    pub fn preview_unsaved(&mut self) {
        let Some(git) = self.git.as_mut() else { return };
        let place = |changes: &[FileChange], path: &str| {
            changes.binary_search_by(|c| std::path::Path::new(&c.path).cmp(std::path::Path::new(path)))
        };
        for (path, original) in std::mem::take(&mut self.previewed) {
            match (place(&git.changes, &path), original) {
                (Ok(at), Some(original)) => git.changes[at] = original,
                (Ok(at), None) => {
                    git.changes.remove(at);
                }
                (Err(at), Some(original)) => git.changes.insert(at, original),
                (Err(_), None) => {}
            }
        }
        for editor in self.editors.values().filter(|e| e.dirty) {
            let Some(head) = self.heads.get(&editor.rel_path) else { continue };
            let at = place(&git.changes, &editor.rel_path);
            let original = at.ok().map(|i| git.changes[i].clone());
            let shown = match (head, &original) {
                (None, Some(known)) => Some(FileChange { unsaved: true, ..known.clone() }),
                (None, None) => continue,
                (Some(head), _) => {
                    let (hunks, insertions, deletions) = crate::git::compare(head, &editor.text());
                    let staged = original.as_ref().and_then(|o| o.staged);
                    let differs = !hunks.is_empty();
                    (differs || staged.is_some()).then(|| FileChange {
                        path: editor.rel_path.clone(),
                        change: original.as_ref().map_or(Change::Modified, |o| o.change),
                        staged,
                        unstaged: differs
                            .then(|| original.as_ref().and_then(|o| o.unstaged).unwrap_or(Change::Modified)),
                        hunks,
                        insertions,
                        deletions,
                        unsaved: true,
                        from: original.as_ref().and_then(|o| o.from.clone()),
                    })
                }
            };
            match (at, shown) {
                (Ok(i), Some(shown)) => git.changes[i] = shown,
                (Ok(i), None) => {
                    git.changes.remove(i);
                }
                (Err(i), Some(shown)) => git.changes.insert(i, shown),
                (Err(_), None) => continue,
            }
            self.previewed.push((editor.rel_path.clone(), original));
        }
        self.changed_dirs.clear();
        for change in &git.changes {
            let mut path = change.path.as_str();
            while let Some((parent, _)) = path.rsplit_once('/') {
                if !self.changed_dirs.insert(parent.to_string()) {
                    break;
                }
                path = parent;
            }
        }
    }

    pub fn touch(&mut self, rel_path: &str, lines: Vec<(u32, u32)>) {
        self.touched.retain(|t| t.path != rel_path);
        self.touched.push(Touch { path: rel_path.to_string(), lines, at: Instant::now() });
    }

    pub fn editor(&self) -> Option<&Editor> {
        self.editors.get(self.editing.as_deref()?)
    }

    pub fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.editors.get_mut(self.editing.as_deref()?)
    }

    pub fn begin_edit(&mut self, rel_path: &str) -> bool {
        if self.read_only {
            self.refused_edit = true;
            return false;
        }
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

    pub fn show_edit(&mut self, rel_path: &str, why: &str, gpu: &Gpu, filters: &RelFilters) {
        let started = Instant::now();
        if self.diffs.remove(rel_path).is_some() {
            self.diff_stale = true;
        }
        let Some(source) = self.editors.get(rel_path).map(|e| e.display_source()) else { return };
        let PageState::Ready(r) = &mut self.state else { return };
        let Some(index) = r.repo.file_index(rel_path) else { return };
        r.repo.replace_source(index, &source, &ScanOptions::default());
        r.tokens.forget_file(index);
        if !self.selections.is_empty() {
            self.selections.clear();
            self.want_exact.clear();
            self.curves_dirty = true;
        }
        self.search = None;
        self.grep.touched(rel_path);
        self.card_changed(index, rel_path, why, started, gpu, filters, "");
    }

    pub fn edited_wrap(&self) -> Option<Arc<canvas_core::Wrap>> {
        let (editor, r) = (self.editor()?, self.ready()?);
        let node = *r.layout.file_node.get(r.repo.file_index(&editor.rel_path)?)?;
        r.layout.nodes.get(node as usize)?.wrap.clone()
    }

    pub fn discarded(&self, only: Option<&str>) -> Vec<(String, crate::git::Back)> {
        let Some(git) = &self.git else { return Vec::new() };
        let asked = git.changes.iter().filter(|change| only.is_none_or(|one| one == change.path));
        asked.filter_map(|change| Some((change.path.clone(), change.goes_back()?))).collect()
    }

    pub fn git_busy(&self) -> bool {
        self.git_op.is_some()
    }

    pub fn run_git(&mut self, op: crate::git::GitOp) {
        if self.git_op.is_some() || self.no_git {
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
            Ok(Err(why)) => self.git_note = Some(GitNote::new(false, why)),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {}
        }
        self.message_rx = None;
        true
    }

    pub fn poll_git_op(&mut self) -> bool {
        let Some(rx) = &self.git_op else { return false };
        let (op, result) = match rx.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.git_op = None;
                self.git_note = Some(GitNote::new(false, "git stopped before it was done".to_string()));
                self.refresh_git();
                return true;
            }
        };
        self.git_op = None;
        match (&op, result) {
            (crate::git::GitOp::Part { .. }, result) => {
                self.git_said = Some(match result {
                    Ok(said) => (true, said),
                    Err(said) => (false, said),
                })
            }
            (crate::git::GitOp::Commit { .. }, Ok(line)) => {
                self.commit_message.clear();
                self.git_note = Some(GitNote::new(true, line));
            }
            (crate::git::GitOp::Sync, Ok(_)) => {
                self.git_note = Some(GitNote::new(true, "Synced with the remote.".into()))
            }
            (crate::git::GitOp::Push | crate::git::GitOp::Publish(_), Ok(_)) => {
                self.git_note = Some(GitNote::new(true, "Pushed.".into()))
            }
            (crate::git::GitOp::Pull, Ok(_)) => self.git_note = Some(GitNote::new(true, "Pulled.".into())),
            (crate::git::GitOp::Fetch, Ok(_)) => self.git_note = Some(GitNote::new(true, "Fetched.".into())),
            (crate::git::GitOp::Stash, Ok(_)) => self.git_note = Some(GitNote::new(true, "Changes stashed.".into())),
            (crate::git::GitOp::Switch(branch), Ok(_)) => {
                self.git_note = Some(GitNote::new(true, format!("On branch {branch}.")))
            }
            (crate::git::GitOp::Track { name, .. }, Ok(_)) => {
                self.git_note = Some(GitNote::new(true, format!("On branch {name}.")))
            }
            (crate::git::GitOp::NewBranch { name, .. }, Ok(_)) => {
                self.git_note = Some(GitNote::new(true, format!("On the new branch {name}.")))
            }
            (crate::git::GitOp::NewTag { name, .. }, Ok(_)) => {
                self.git_note = Some(GitNote::new(true, format!("Tag {name} made.")))
            }
            (crate::git::GitOp::Merge(branch), Ok(_)) => {
                self.git_note = Some(GitNote::new(true, format!("Merged {branch}.")))
            }
            (crate::git::GitOp::Rebase(branch), Ok(_)) => {
                self.git_note = Some(GitNote::new(true, format!("Rebased onto {branch}.")))
            }
            (crate::git::GitOp::InteractiveRebase { stash, .. }, Ok(_)) => {
                self.rebase = None;
                let kept = if *stash { " Your changes are in Stashes: pop them to bring them back." } else { "" };
                self.git_note = Some(GitNote::new(true, format!("Rebased.{kept}")));
            }
            (crate::git::GitOp::Carry { operation, how }, Ok(_)) => {
                let done = match how {
                    crate::git::Carry::Continue => "finished",
                    crate::git::Carry::Skip => "went on without that step",
                    crate::git::Carry::Abort => "given up: everything is as it was",
                };
                self.git_note = Some(GitNote::new(true, format!("{} {done}.", operation.noun())));
            }
            (crate::git::GitOp::CherryPick(_), Ok(_)) => {
                self.git_note = Some(GitNote::new(true, "Cherry-picked.".into()))
            }
            (crate::git::GitOp::Revert(_), Ok(_)) => self.git_note = Some(GitNote::new(true, "Reverted.".into())),
            (_, Ok(_)) => {}
            (_, Err(e)) => self.git_note = Some(GitNote::new(false, e)),
        }
        self.refresh_git();
        true
    }

    pub fn poll_git(&mut self) -> bool {
        let Some(rx) = &self.git_rx else { return false };
        match rx.try_recv() {
            Ok(state) => {
                let head = state.as_ref().map(|git| git.head.clone()).unwrap_or_default();
                self.git = state;
                self.git_rx = None;
                if head != self.heads_at {
                    self.heads_at = head;
                    self.heads.clear();
                    self.heads_asked.clear();
                    self.heads_round += 1;
                    self.timeline_stale = true;
                }
                let moved = self.git.iter().flat_map(|git| &git.changes).filter(|change| change.from.is_some());
                let moved: Vec<String> = moved.map(|change| change.path.clone()).collect();
                for path in moved.iter().filter(|path| self.heads.get(*path) == Some(&None)) {
                    self.heads_asked.remove(path);
                }
                self.heads.retain(|path, text| text.is_some() || !moved.contains(path));
                let unsaved: Vec<String> = self.unsaved().map(|e| e.rel_path.clone()).collect();
                self.want_heads(unsaved);
                self.previewed.clear();
                self.changed_dirs.clear();
                self.preview_unsaved();
                self.diff_stale = true;
                if let PageState::Ready(r) = &mut self.state {
                    r.tree_dirty = true;
                }
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
        let same = self.timeline.path == path;
        if (same && !self.timeline_stale) || self.no_git {
            return;
        }
        self.timeline_stale = false;
        if !same {
            self.timeline = Timeline { path: path.clone(), commits: Vec::new(), loading: path.is_some() };
        }
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
            log::info!("timeline of {path}: {} commits", commits.len());
            self.timeline.commits = commits;
            self.timeline.loading = false;
        }
        true
    }

    pub fn changed_files(&self) -> HashSet<u32> {
        let (Some(git), Some(r)) = (&self.git, self.ready()) else { return HashSet::new() };
        git.changes.iter().filter_map(|c| r.repo.file_index(&c.path)).map(|i| i as u32).collect()
    }

    /// The changed files, and with the options of the Diffs section on also the files of
    /// their folders and the files one import away from them.
    fn changed_and_around(&self) -> HashSet<u32> {
        let changed = self.changed_files();
        let Some(r) = self.ready().filter(|_| !changed.is_empty()) else { return changed };
        let mut files = changed.clone();
        if self.changed_folders {
            let paths: Vec<&str> = r.repo.files.iter().map(|f| f.rel_path.as_str()).collect();
            files.extend(files_in_folders_of(&changed, &paths));
        }
        if self.changed_neighbours {
            let kinds = DepFilters::default().kinds;
            let mut edges = Vec::new();
            for &file in &changed {
                edges.extend(r.deps.outgoing(file).iter().filter(|e| kinds[e.kind.index()]).map(|e| (e.from, e.to)));
                edges.extend(r.deps.incoming(file).filter(|e| kinds[e.kind.index()]).map(|e| (e.from, e.to)));
            }
            files.extend(with_neighbours(&changed, &edges));
        }
        files
    }

    /// What the Diffs section of Source Control counts: the files with changes, the files the
    /// canvas shows while narrowed to them (nothing when it is narrowed to something else),
    /// and the files whose unchanged lines are folded away.
    pub fn diffs_info(&self) -> crate::ui::DiffsInfo {
        let clean = self.only_changed && !self.only_connected && self.showing.is_none();
        crate::ui::DiffsInfo {
            changed: self.changed_files().len(),
            shown: self.narrowed_to.filter(|_| clean),
            folded: self.diffs.values().filter(|shown| shown.folded).count(),
        }
    }

    pub fn start_scan(&mut self) {
        self.diffs.clear();
        self.diff_stale = true;
        self.showing_changed = self.showing.is_some();
        self.refresh_git();
        let progress = Arc::new(ScanProgress::default());
        let (tx, rx) = mpsc::channel();
        let root = self.root.clone();
        let p = progress.clone();
        let mut filter = LayoutFilter { only_files: None, order: None, ..self.filter.clone() };
        let arranged = self.arranged;
        let reuse = !std::mem::take(&mut self.read_again);
        (self.warm_at, self.warm_batch) = (0, 256);
        let (only, extra) = (self.only, self.extra.clone());
        std::thread::Builder::new()
            .name(format!("scan {}", self.title))
            .spawn(move || {
                let t = Instant::now();
                let cache = scan_cache(&root).filter(|_| !only);
                let options = ScanOptions { cache, reuse, only, extra, ..scan_options() };
                let mut repo = scan::scan(&root, &options, Some(p));
                let (deps, per_file) = canvas_core::deps::analyze_keeping(&repo);
                if let Err(why) = canvas_core::store::keep(&mut repo, &per_file, &options) {
                    log::warn!("what the scan found was not kept: {why}");
                }
                let freed = repo.trim_text(text_budget());
                if freed > 0 {
                    log::info!("'{}': {} MB of text let go, read again when it is looked at", repo.name, freed >> 20);
                }
                log::info!(
                    "dependencies of '{}': {} edges between {} pairs of files in {} ms ({} unresolved, {} external)",
                    repo.name,
                    deps.edges.len(),
                    deps.pairs(),
                    deps.ms,
                    deps.unresolved,
                    deps.external
                );
                if arranged {
                    filter.order = Some(Arc::new(arrange(&repo, &deps)));
                }
                let layout = Layout::build_filtered(&repo, &filter);
                let scene = SceneData::build(&repo, &layout);
                let total_ms = t.elapsed().as_millis();
                log::info!(
                    "scanned '{}': {} files ({} not read again), {} lines, {} symbols, scan {} ms (index {} ms), total {} ms",
                    repo.name,
                    repo.files.len(),
                    repo.reused,
                    repo.total_lines,
                    repo.symbols.count,
                    repo.scan_ms,
                    repo.timings.index_ms,
                    total_ms
                );
                let _ = tx.send(Loaded { repo, deps, layout, order: filter.order, scene, total_ms });
            })
            .expect("spawn scan thread");
        self.state = PageState::Loading { progress, rx, started: Instant::now() };
        self.hover = None;
        self.hover_cell = None;
        self.selected = None;
        self.selections.clear();
        self.want_exact.clear();
        self.only_connected = false;
        self.curves_dirty = true;
        self.search = None;
        self.grep.everything();
    }

    /// The files the Search panel looks in (only some of them when `only` names them), with
    /// the text of those whose card does not show what is on disk.
    pub fn search_sources(&self, only: Option<&[String]>) -> Vec<canvas_core::grep::Source> {
        let Some(r) = self.ready() else { return Vec::new() };
        let source = |f: &canvas_core::SourceFile| {
            if f.note.as_ref().is_some_and(|note| !note.starts_with("truncated")) {
                return None;
            }
            let rel = f.rel_path.clone();
            let lines = self.line_count(&rel).unwrap_or(f.line_count()) as u32;
            let unsaved = self.editors.get(&rel).filter(|editor| editor.dirty).map(|editor| editor.display_source());
            let committed = self.diffs.get(&rel).and_then(|shown| shown.committed.clone());
            Some(canvas_core::grep::Source { text: unsaved.or(committed), rel, lines })
        };
        match only {
            Some(rels) => {
                let files = rels.iter().filter_map(|rel| r.repo.file_index(rel));
                files.filter_map(|index| source(&r.repo.files[index])).collect()
            }
            None => r.repo.files.iter().filter_map(source).collect(),
        }
    }

    pub fn start_search(&mut self, asked: crate::search::Request, only: Option<Vec<String>>) {
        let sources = self.search_sources(only.as_deref());
        self.grep.start(self.root.clone(), sources, asked, only);
    }

    /// Where a match of the Search panel is on the canvas.
    pub fn grep_site(&self, rel: &str, hit: &canvas_core::grep::Hit) -> Option<Site> {
        let r = self.ready()?;
        let file = r.repo.file_index(rel)?;
        self.site_in(r, file, rel, hit)
    }

    fn site_in(&self, r: &Ready, file: usize, rel: &str, hit: &canvas_core::grep::Hit) -> Option<Site> {
        let f = &r.repo.files[file];
        let (row, seen) = self.diffs.get(rel).map_or((hit.line, true), |shown| shown.place(hit.line));
        if row as usize >= f.line_count() {
            return None;
        }
        let clip = |n: u32| n.min(u16::MAX as u32) as u16;
        Some(match seen {
            true => Site { file: file as u32, line: row, col: clip(hit.col), len: clip(hit.len.max(1)) },
            false => {
                Site { file: file as u32, line: row, col: 0, len: clip(f.line(row as usize).chars().count() as u32) }
            }
        })
    }

    /// Marks every match of the Search panel on the canvas, the one shown last brighter.
    pub fn mark_search(&mut self) {
        if !self.grep.marks_due || self.ready().is_none() {
            return;
        }
        self.grep.marks_due = false;
        let Some(r) = self.ready() else { return };
        let Some(results) = &self.grep.results else {
            self.search = None;
            return;
        };
        let (mut hits, mut current) = (Vec::new(), None);
        for (fi, found) in results.files.iter().enumerate() {
            let Some(file) = r.repo.file_index(&found.rel) else { continue };
            for (hi, hit) in found.hits.iter().enumerate() {
                let Some(site) = self.site_in(r, file, &found.rel, hit) else { continue };
                if self.grep.at == Some((fi, hi)) {
                    current = Some(hits.len());
                }
                hits.push(site);
            }
        }
        let query = results.asked.query.text.clone();
        let (truncated, ms) = (results.truncated, results.ms);
        self.search = (!hits.is_empty()).then_some(SearchResults { query, hits, truncated, ms, current });
    }

    pub fn line_count(&self, rel_path: &str) -> Option<usize> {
        self.diffs.get(rel_path).map(|shown| shown.lines.len())
    }

    pub fn symbol_line(&self, sym: SymRef) -> u32 {
        let Some(r) = self.ready() else { return 0 };
        let shown = self.diffs.get(&r.repo.files[sym.file as usize].rel_path);
        let real = shown.and_then(|shown| shown.symbols.get(sym.index as usize));
        real.map_or(r.repo.sym(sym).line, |(line, _)| *line)
    }

    pub fn site_line(&self, site: Site, of: Option<SymRef>) -> u32 {
        let Some(r) = self.ready() else { return site.line };
        let rel = &r.repo.files[site.file as usize].rel_path;
        match of.filter(|sym| r.repo.sym_site(*sym) == site) {
            Some(sym) => self.symbol_line(sym),
            None => self.line_of(rel, site.line).unwrap_or(site.line),
        }
    }

    pub fn signature(&self, sym: SymRef) -> String {
        let Some(r) = self.ready() else { return String::new() };
        let shown = self.diffs.get(&r.repo.files[sym.file as usize].rel_path);
        match shown.and_then(|shown| Some((shown, shown.symbols.get(sym.index as usize)?))) {
            Some((shown, &(line, end))) => {
                let lines = shown.text().lines().skip(line as usize).take(end.saturating_sub(line) as usize);
                canvas_core::refs::signature(lines)
            }
            None => r.repo.sym_signature(sym),
        }
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

    pub fn layout_filter(&self, filters: &RelFilters) -> LayoutFilter {
        let changed = match self.commit_files() {
            Some(of_commit) => Some(of_commit),
            None => self.only_changed.then(|| self.changed_and_around()),
        };
        let changed = changed.filter(|set| !set.is_empty());
        let connected = (self.only_connected && !self.selections.is_empty()).then(|| self.connected_files(filters));
        let only = match (changed, connected) {
            (Some(a), Some(b)) => Some(a.union(&b).copied().collect()),
            (a, b) => a.or(b),
        };
        LayoutFilter { only_files: only, ..self.filter.clone() }
    }

    pub fn relayout(&mut self, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) {
        let filter = self.layout_filter(filters);
        let narrowed_to = filter.only_files.as_ref().map(|set| set.len());
        if self.commit_shown().is_none() && self.only_changed {
            let (folders, neighbours) = (self.changed_folders, self.changed_neighbours);
            let what = match (folders, neighbours) {
                (false, false) => "changed",
                (true, false) => "changed, their folders",
                (false, true) => "changed, their neighbours",
                (true, true) => "changed, their folders, their neighbours",
            };
            log::info!("showing {} files: {what}", narrowed_to.unwrap_or(0));
        }
        self.narrowed_to = narrowed_to;
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

    pub fn set_arranged(&mut self, on: bool, renderer: &Renderer, device: &wgpu::Device, filters: &RelFilters) {
        self.arranged = on;
        let Some(r) = self.ready() else { return };
        self.filter.order = on.then(|| Arc::new(arrange(&r.repo, &r.deps)));
        self.relayout(renderer, device, filters);
    }

    /// The order of the folders made again after imports changed; the layout follows
    /// it when it is next updated (`Layout::resize_file` and the others), alone or
    /// with everything laid out again. How many folders are in another order.
    fn rearrange(&mut self) -> usize {
        let Some(r) = self.ready().filter(|_| self.arranged) else { return 0 };
        let order = arrange(&r.repo, &r.deps);
        let moved = r.layout.order.as_deref().map_or(1, |old| old.changed(&order).len());
        self.filter.order = Some(Arc::new(order));
        moved
    }

    pub fn imports_typed(&mut self, typed: &std::collections::BTreeSet<u32>, gpu: &Gpu, filters: &RelFilters) {
        let started = Instant::now();
        let moved = self.rearrange();
        if moved == 0 {
            return;
        }
        let one = typed.iter().next().filter(|_| typed.len() == 1).map(|&f| f as usize);
        let rel = one.and_then(|f| Some(self.ready()?.repo.files.get(f)?.rel_path.clone()));
        match (one, rel) {
            (Some(index), Some(rel)) => {
                let more = format!(", {moved} folders in another order");
                self.card_changed(index, &rel, "imports typed", started, gpu, filters, &more)
            }
            _ => self.relayout(gpu.renderer, gpu.device, filters),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn card_changed(
        &mut self,
        index: usize,
        rel_path: &str,
        why: &str,
        started: Instant,
        gpu: &Gpu,
        filters: &RelFilters,
        more: &str,
    ) {
        let filter = self.layout_filter(filters);
        let PageState::Ready(r) = &mut self.state else { return };
        let t = Instant::now();
        let Some(changed) = r.layout.resize_file(&r.repo, &filter, index) else {
            self.relayout(gpu.renderer, gpu.device, filters);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            return log::info!("updated '{rel_path}' ({why}) with everything laid out again in {ms:.1} ms");
        };
        let laid = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        r.gpu.card_changed(gpu.device, gpu.queue, &gpu.renderer.origins_layout, &r.repo, &r.layout, index, &changed);
        let drawn = t.elapsed().as_secs_f64() * 1000.0;
        self.curves_dirty = true;
        let placed: u32 = changed.iter().map(|range| range.end - range.start).sum();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let boxes = r.layout.nodes.len();
        log::info!(
            "updated '{rel_path}' ({why}) alone in {ms:.2} ms: layout {laid:.2} ms ({placed} of {boxes} boxes placed again), scene {drawn:.2} ms{more}"
        );
    }

    pub fn reload_file(&mut self, rel_path: &str, why: &str, gpu: &Gpu, filters: &RelFilters) -> bool {
        let started = Instant::now();
        self.diffs.remove(rel_path);
        self.diff_stale = true;
        let selected = self.selected_names();
        let PageState::Ready(r) = &mut self.state else { return false };
        let Some(index) = r.repo.file_index(rel_path) else { return false };
        r.repo.reload_file(index, &ScanOptions::default());
        let read = started.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let moved = r.deps.update_files(&r.repo, &[index as u32].into());
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let edges = if moved { " (edges changed)" } else { "" };
        r.tokens.forget_file(index);
        let reordered = match moved {
            true => self.rearrange(),
            false => 0,
        };
        let reordered = match reordered {
            0 => String::new(),
            n => format!(", {n} folders in another order"),
        };
        let imports = format!(", read and indexed {read:.2} ms, imports {ms:.2} ms{edges}{reordered}");
        self.search = None;
        self.grep.touched(rel_path);
        self.select_again(selected);
        self.card_changed(index, rel_path, why, started, gpu, filters, &imports);
        true
    }

    pub fn file_made(&mut self, rel_path: &str, gpu: &Gpu, filters: &RelFilters) -> bool {
        self.file_came_or_went(rel_path, true, gpu, filters)
    }

    pub fn file_gone(&mut self, rel_path: &str, gpu: &Gpu, filters: &RelFilters) -> bool {
        self.file_came_or_went(rel_path, false, gpu, filters)
    }

    fn file_came_or_went(&mut self, rel_path: &str, made: bool, gpu: &Gpu, filters: &RelFilters) -> bool {
        let started = Instant::now();
        let selected = self.selected_names();
        let filter = self.layout_filter(filters);
        let keep = self.selected.and_then(|n| self.ready()?.layout.nodes.get(n)).map(|n| n.rel_path.clone());
        let PageState::Ready(r) = &mut self.state else { return false };
        let (index, old) = match made {
            true => match r.repo.add_file(rel_path, &scan_options()) {
                Some(index) => (index, None),
                None => return false,
            },
            false => match r.repo.file_index(rel_path) {
                Some(index) => (index, Some(r.repo.remove_file(index))),
                None => return false,
            },
        };
        let t = Instant::now();
        let resolved = match &old {
            None => r.deps.insert_file(&r.repo, index as u32),
            Some(old) => r.deps.remove_file(&r.repo, index as u32, old),
        };
        let resolved = resolved.unwrap_or_else(|| {
            r.deps = canvas_core::deps::analyze(&r.repo);
            r.repo.files.len()
        });
        let imports = t.elapsed().as_secs_f64() * 1000.0;
        let reordered = self.rearrange();
        let filter = LayoutFilter { order: self.filter.order.clone(), ..filter };
        let PageState::Ready(r) = &mut self.state else { return false };
        let t = Instant::now();
        let from = match made {
            true => r.layout.insert_file(&r.repo, &filter, index),
            false => r.layout.remove_file(&r.repo, &filter, index, rel_path),
        };
        let laid = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let alone = match &from {
            Some(from) => {
                let (device, queue, origins) = (gpu.device, gpu.queue, &gpu.renderer.origins_layout);
                r.gpu.relaid(device, queue, origins, &r.repo, &r.layout, from, index, made);
                true
            }
            None => {
                r.layout = Layout::build_filtered(&r.repo, &filter);
                let scene = SceneData::build(&r.repo, &r.layout);
                r.gpu = gpu.renderer.create_scene(gpu.device, &r.repo.name, scene);
                false
            }
        };
        let drawn = t.elapsed().as_secs_f64() * 1000.0;
        r.tokens = Default::default();
        r.tree_dirty = true;
        self.selected = keep.and_then(|p| r.layout.node_for_path(&r.repo, &p));
        let boxes = r.layout.nodes.len();
        self.hover = None;
        self.hover_cell = None;
        self.search = None;
        self.grep.touched(rel_path);
        self.curves_dirty = true;
        self.diffs.remove(rel_path);
        self.diff_stale = true;
        self.select_again(selected);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let why = if made { "made" } else { "deleted" };
        let how = if alone { "alone" } else { "with everything laid out again" };
        let reordered = match reordered {
            0 => String::new(),
            n => format!(", {n} folders in another order"),
        };
        log::info!(
            "updated '{rel_path}' ({why}) {how} in {ms:.2} ms: imports of {resolved} files resolved again in {imports:.2} ms, layout {laid:.2} ms ({boxes} boxes), scene {drawn:.2} ms{reordered}"
        );
        true
    }

    pub fn toggle_symbol(&mut self, sym: SymRef, clicked: Option<Site>) -> bool {
        self.curves_dirty = true;
        if let Some(i) = self.selections.iter().position(|s| s.sym == sym) {
            self.selections.remove(i);
            return false;
        }
        let Some(r) = self.ready() else { return false };
        let mut rel = r.repo.relations(sym, clicked);
        let of_file = |at: &Site| self.line_of(&r.repo.files[at.file as usize].rel_path, at.line).is_some();
        rel.items
            .retain(|item| matches!(item.kind, RelKind::Definition | RelKind::Implementation) || of_file(&item.from));
        log::info!(
            "relations of '{}': {} items in {:.1} ms{}",
            r.repo.sym_name(sym),
            rel.items.len(),
            rel.search_ms,
            if rel.truncated { " (truncated)" } else { "" }
        );
        self.selections.push(Selection { sym, rel, outside: 0 });
        self.want_exact.push(sym);
        true
    }

    pub fn use_site(&self, sym: SymRef) -> Option<(PathBuf, u32, u32)> {
        let r = self.ready().filter(|_| self.showing.is_none())?;
        let file = r.repo.files.get(sym.file as usize)?;
        file.analysis.symbols.get(sym.index as usize)?;
        let site = r.repo.sym_site(sym);
        Some((self.root.join(&file.rel_path), self.symbol_line(sym), site.col as u32))
    }

    fn selected_names(&self) -> Vec<(String, String, u32)> {
        let Some(r) = self.ready() else { return Vec::new() };
        let named = |s: &Selection| {
            let rel = r.repo.files[s.sym.file as usize].rel_path.clone();
            (rel, r.repo.sym_name(s.sym).to_string(), r.repo.sym_site(s.sym).line)
        };
        self.selections.iter().map(named).collect()
    }

    fn select_again(&mut self, names: Vec<(String, String, u32)>) {
        self.selections.clear();
        self.want_exact.clear();
        self.want_exact.clear();
        for (rel, name, line) in names {
            let found = self.ready().and_then(|r| {
                let file = r.repo.file_index(&rel)? as u32;
                let defs = r.repo.symbols.defs(&name);
                let here = defs.iter().copied().filter(|sym| sym.file == file);
                here.min_by_key(|sym| r.repo.sym_site(*sym).line.abs_diff(line))
            });
            if let Some(sym) = found {
                self.toggle_symbol(sym, None);
            }
        }
        self.curves_dirty = true;
    }

    /// Puts what a language server said about a selected name (places relative to the
    /// folder, in lines of the file) in place of what was guessed by name.
    pub fn exact_relations(&mut self, of: (u32, u32), found: &Found<String>) -> bool {
        let sym = SymRef { file: of.0, index: of.1 };
        let Some(at) = self.selections.iter().position(|s| s.sym == sym) else { return false };
        let PageState::Ready(r) = &self.state else { return false };
        let place = |(rel, line, col): &(String, u32, u32)| {
            let row = self.diffs.get(rel).map_or(*line, |shown| shown.row(*line));
            Some((r.repo.file_index(rel)? as u32, row, (*col).min(u16::MAX as u32) as u16))
        };
        let rel = &mut self.selections[at].rel;
        let (what, before) = match found {
            Found::Uses(places) => {
                let uses: Vec<(u32, u32, u16)> = places.iter().filter_map(place).collect();
                if uses.is_empty() {
                    return false;
                }
                r.repo.refine(rel, &uses);
                ("uses", RelKind::Reference)
            }
            Found::Calls(calls) => {
                let calls: Vec<_> = calls.iter().filter_map(|(from, to)| Some((place(from)?, place(to)?))).collect();
                r.repo.refine_calls(rel, &calls);
                ("outgoing calls", RelKind::OutgoingCall)
            }
            Found::Implementations(places) => {
                let places: Vec<(u32, u32, u16)> = places.iter().filter_map(place).collect();
                r.repo.refine_implementations(rel, &places);
                ("implementations", RelKind::Implementation)
            }
        };
        let count = match before {
            RelKind::Reference => rel.count(RelKind::Reference) + rel.count(RelKind::IncomingCall),
            kind => rel.count(kind),
        };
        log::info!("exact {what} of '{}': {count}", r.repo.sym_name(sym));
        self.curves_dirty = true;
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
    ignored: HashMap<String, Vec<(String, bool)>>,
}

/// The folder a path is directly in ("" at the top of the folder).
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// Every file that is in the same folder as one of the `changed` files. `paths` are the
/// relative paths of all files, the index of a path being the number of its file.
fn files_in_folders_of(changed: &HashSet<u32>, paths: &[&str]) -> HashSet<u32> {
    let folders: HashSet<&str> = changed.iter().filter_map(|&i| paths.get(i as usize)).map(|p| folder_of(p)).collect();
    paths.iter().enumerate().filter(|(_, p)| folders.contains(folder_of(p))).map(|(i, _)| i as u32).collect()
}

/// The `changed` files and the files at the other end of an import (`edges` are
/// `(importing file, imported file)`) of one of them.
fn with_neighbours(changed: &HashSet<u32>, edges: &[(u32, u32)]) -> HashSet<u32> {
    let mut out = changed.clone();
    for &(from, to) in edges {
        if changed.contains(&from) {
            out.insert(to);
        }
        if changed.contains(&to) {
            out.insert(from);
        }
    }
    out
}

fn folders_of(dir: &Dir, folders: &mut HashSet<String>) {
    folders.insert(dir.rel_path.clone());
    for inner in &dir.dirs {
        folders_of(inner, folders);
    }
}

fn ignored_by_folder(tree: &Dir, ignored: &[String]) -> HashMap<String, Vec<(String, bool)>> {
    let mut folders = HashSet::new();
    folders_of(tree, &mut folders);
    folders.insert(String::new());
    let mut by_folder: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for entry in ignored {
        let path = entry.trim_end_matches('/');
        let up = |path: &str| path.rsplit_once('/').map_or(String::new(), |(parent, _)| parent.to_string());
        let mut parent = up(path);
        while !folders.contains(&parent) {
            parent = up(&parent);
        }
        let name = if parent.is_empty() { path } else { &path[parent.len() + 1..] };
        by_folder.entry(parent).or_default().push((name.to_string(), entry.ends_with('/')));
    }
    by_folder
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
            ignored: false,
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
            ignored: false,
            name: f.name.clone(),
            path: f.rel_path.clone(),
            count: 0,
        });
    }
    for (name, is_dir) in view.ignored.get(&dir.rel_path).into_iter().flatten() {
        let path = if dir.rel_path.is_empty() { name.clone() } else { format!("{}/{name}", dir.rel_path) };
        rows.push(TreeRow {
            depth,
            is_dir: *is_dir,
            open: false,
            hidden: false,
            locked: true,
            ignored: true,
            name: name.clone(),
            path,
            count: 0,
        });
    }
}

pub fn build_tree(repo: &Repo, open: &HashSet<String>, hidden: &HashSet<String>, ignored: &[String]) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    let aside: Vec<String> =
        ignored.iter().filter(|path| repo.file_index(path.trim_end_matches('/')).is_none()).cloned().collect();
    let view = TreeView { repo, open, hidden, ignored: ignored_by_folder(&repo.tree, &aside) };
    push_dir(&repo.tree, 0, false, &view, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(rel_path: &str, dirs: Vec<Dir>) -> Dir {
        let name = rel_path.rsplit('/').next().unwrap_or_default().to_string();
        Dir { name, rel_path: rel_path.to_string(), dirs, files: Vec::new() }
    }

    #[test]
    fn a_git_note_goes_away_sooner_when_it_is_good_news() {
        let done = GitNote::new(true, "Pushed.".into());
        let at = done.since;
        let error = GitNote { since: at, ..GitNote::new(false, "error: nope".into()) };
        assert_eq!(done.left(at), Some(GitNote::DONE_SHOWN));
        assert_eq!(done.left(at + Duration::from_secs(5)), Some(Duration::from_secs(1)));
        assert_eq!(done.left(at + GitNote::DONE_SHOWN), None);
        assert_eq!(error.left(at + Duration::from_secs(10)), Some(Duration::from_secs(10)));
        assert_eq!(error.left(at + GitNote::ERROR_SHOWN + Duration::from_secs(1)), None);
    }

    #[test]
    fn changed_files_bring_the_other_files_of_their_folder() {
        let paths = ["a/one.rs", "a/two.rs", "b/three.rs", "b/four.rs", "top.rs", "c/deep/five.rs", "c/six.rs"];
        let changed: HashSet<u32> = [0, 5].into_iter().collect();
        let got = files_in_folders_of(&changed, &paths);
        assert_eq!(got, [0, 1, 5].into_iter().collect::<HashSet<u32>>());
        let top: HashSet<u32> = [4].into_iter().collect();
        assert_eq!(files_in_folders_of(&top, &paths), top);
        assert!(files_in_folders_of(&HashSet::new(), &paths).is_empty());
    }

    #[test]
    fn changed_files_bring_the_files_one_import_away() {
        let changed: HashSet<u32> = [1].into_iter().collect();
        let edges = [(0, 1), (1, 2), (2, 3), (4, 0)];
        let got = with_neighbours(&changed, &edges);
        assert_eq!(got, [0, 1, 2].into_iter().collect::<HashSet<u32>>(), "importers and imported, one hop only");
        assert_eq!(with_neighbours(&changed, &[]), changed);
    }

    #[test]
    fn ignored_paths_hang_from_the_nearest_folder_on_the_canvas() {
        let tree = folder("", vec![folder("docs", vec![folder("docs/img", Vec::new())]), folder("src", Vec::new())]);
        let ignored: Vec<String> =
            ["clip.mp4", "target/", "docs/img/shot.png", "media/raw/take.mov"].map(String::from).to_vec();
        let by_folder = ignored_by_folder(&tree, &ignored);
        let at = |parent: &str| by_folder.get(parent).cloned().unwrap_or_default();
        assert_eq!(
            at(""),
            [("clip.mp4".to_string(), false), ("target".to_string(), true), ("media/raw/take.mov".to_string(), false)]
        );
        assert_eq!(at("docs/img"), [("shot.png".to_string(), false)]);
        assert!(at("docs").is_empty() && at("src").is_empty());
    }
}
