use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_core::layout::{NodeKind, FILE_LABEL_H};
use canvas_core::{Rect, Site, SymRef, LINE_H};
use canvas_render::{FrameStats, Label, Lod, Overlay, Pane, Renderer, ScenePass};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use crate::bench::Bench;
use crate::camera::Camera;
use crate::ext::{self, Extensions};
use crate::keys::{Chord, Command, Keymap, Scope};
use crate::overlay::{self, CanvasOverlay, SYMBOL_HOVER_MIN_PX};
use crate::page::{Page, PageState, Ready};
use crate::script::{Script, Step};
use crate::session::{self, PageSession, Session};
use crate::ui::{self, Action, FileOp, FrameInfo, Panel, QuickOpen, Side, UiState};

mod assistants;
mod sharing;

struct Gfx {
    renderer: Renderer,
    egui_renderer: egui_wgpu::Renderer,
    egui_state: egui_winit::State,
    egui: egui::Context,
    queue: wgpu::Queue,
    device: wgpu::Device,
    config: wgpu::SurfaceConfiguration,
    surface: wgpu::Surface<'static>,
    resized: bool,
    instance: wgpu::Instance,
    window: Arc<Window>,
}

fn key_name(key: &Key) -> Option<String> {
    let name = match key {
        Key::Character(c) if !c.is_empty() => return Some(c.to_lowercase()),
        Key::Named(named) => match named {
            NamedKey::F1 => "f1",
            NamedKey::F2 => "f2",
            NamedKey::F3 => "f3",
            NamedKey::F4 => "f4",
            NamedKey::F5 => "f5",
            NamedKey::F6 => "f6",
            NamedKey::F7 => "f7",
            NamedKey::F8 => "f8",
            NamedKey::F9 => "f9",
            NamedKey::F10 => "f10",
            NamedKey::F11 => "f11",
            NamedKey::F12 => "f12",
            NamedKey::Tab => "tab",
            NamedKey::Enter => "enter",
            NamedKey::Escape => "escape",
            NamedKey::Space => "space",
            NamedKey::Backspace => "backspace",
            NamedKey::Delete => "delete",
            NamedKey::Insert => "insert",
            NamedKey::Home => "home",
            NamedKey::End => "end",
            NamedKey::PageUp => "pageup",
            NamedKey::PageDown => "pagedown",
            NamedKey::ArrowLeft => "left",
            NamedKey::ArrowRight => "right",
            NamedKey::ArrowUp => "up",
            NamedKey::ArrowDown => "down",
            _ => return None,
        },
        _ => return None,
    };
    Some(name.to_string())
}

const CHECKERS: [(&[&str], &str, &str, &str); 2] = [
    (&["rs"], "Rust", "rust-lang.rust-analyzer", "rust-analyzer"),
    (
        &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
        "TypeScript and JavaScript",
        "vscode.typescript-language-features",
        "VS Code's TypeScript extension",
    ),
];
const INSTALL_IT: &str = "Install it";
const NEVER: &str = "Do not ask again";

struct Place {
    page: u32,
    anchor: String,
    offset: [f32; 2],
    zoom: f32,
    selected: Option<String>,
}

const PLACES_KEPT: usize = 200;

enum Offered {
    Paths(Vec<crate::paths::Candidate>),
    Code { serial: u64, line: usize, items: Vec<(usize, ext::Suggestion)> },
}

impl Offered {
    fn len(&self) -> usize {
        match self {
            Offered::Paths(items) => items.len(),
            Offered::Code { items, .. } => items.len(),
        }
    }
}

struct Completion {
    start: usize,
    items: Offered,
    selected: usize,
}

fn theme_of_extension(ext: &Extensions, key: &str) -> Option<canvas_render::theme::Theme> {
    let chosen = ext.color_themes().find(|theme| theme.key == key)?;
    let made = crate::themes::from_vscode(&chosen.file, chosen.light);
    if made.is_none() {
        log::warn!("the theme {} could not be read from {}", chosen.label, chosen.file.display());
    }
    made
}

type Awaited = (PathBuf, Option<(u32, u32, u32)>);

const WHEEL_NOTCH: f32 = 110.0;

fn can_show(size: winit::dpi::PhysicalSize<u32>) -> bool {
    size.width >= 320 && size.height >= 200
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn typed_in_order(typed: &str, label: &str) -> bool {
    let mut rest = label.chars().flat_map(char::to_lowercase);
    typed.chars().flat_map(char::to_lowercase).all(|c| rest.any(|l| l == c))
}

struct Drag {
    last: [f32; 2],
    start: [f32; 2],
    moved: bool,
    button: MouseButton,
    select: Option<String>,
    anchored: bool,
}

pub struct App {
    gfx: Option<Gfx>,
    pages: Vec<Page>,
    active: usize,
    next_id: u32,
    initial: Vec<(PathBuf, Option<PageSession>)>,
    initial_active: usize,
    window_size: Option<[f32; 2]>,
    window_pos: Option<[i32; 2]>,
    start_maximized: bool,
    cursor: [f32; 2],
    drag: Option<Drag>,
    mods: ModifiersState,
    last_click: Option<(Instant, [f32; 2])>,
    ui: UiState,
    canvas: Rect,
    canvas_full: Rect,
    was_shared: bool,
    chip_hits: Vec<(Rect, SymRef)>,
    toggle_hits: Vec<(Rect, String)>,
    preview_hits: Vec<(Rect, String)>,
    preview: Option<String>,
    curves_for: Option<u32>,
    last_frame: Instant,
    frame_ms: f32,
    stats: FrameStats,
    bench: Option<Bench>,
    next_repaint: Option<Instant>,
    script: Option<Script>,
    exit_requested: bool,
    exit_confirmed: bool,
    resize_edge: Option<ResizeDirection>,
    session_dirty: bool,
    last_save: Instant,
    ext: Extensions,
    symbols_stale: Option<Instant>,
    unsent_edit: Option<(PathBuf, Instant)>,
    hover_probe: Option<((u32, u32, u32), Instant, bool)>,
    watcher: crate::watch::Watcher,
    auto_rescan: Option<Instant>,
    rescan_later: HashSet<u32>,
    changed_under: HashSet<(u32, String)>,
    merged_in: HashSet<(u32, String)>,
    webviews: crate::webview::WebViews,
    webview_roots: Vec<PathBuf>,
    copies: Vec<crate::files::Copying>,
    ui_events: VecDeque<Vec<egui::Event>>,
    updater: crate::update::Updater,
    restart_as: Option<PathBuf>,
    completion: Option<Completion>,
    versions: crate::versions::Checker,
    manifests: HashMap<PathBuf, Vec<crate::versions::Dependency>>,
    versions_due: Vec<u32>,
    versions_told: HashSet<PathBuf>,
    theme_revision: u64,
    theme_dirty: bool,
    last_input: Instant,
    checkers_offered: HashSet<&'static str>,
    following: Option<(usize, String, u32, Instant)>,
    text_checked: Instant,
    show_when_read: Option<Awaited>,
    back: Vec<Place>,
    forward: Vec<Place>,
    moved_by_itself: bool,
    pictures: crate::pictures::Pictures,
    playing: Option<PathBuf>,
    player_shown: bool,
    commit_from: Option<Place>,
    sharing: sharing::Sharing,
    workspace: Vec<session::Known>,
    frame_all_when_read: bool,
    agents: crate::agents::Agents,
    agent_followed: Option<(u64, Option<u32>)>,
    claude_checked: Option<Instant>,
    claude_offer_due: Option<Instant>,
    last_move_by_itself: bool,
}

impl App {
    pub fn new(paths: Vec<PathBuf>, waker: ext::Waker) -> Self {
        let saved: Session = session::load();
        let mut initial: Vec<(PathBuf, Option<PageSession>)> =
            saved.pages.iter().filter(|s| s.path.is_dir()).map(|s| (s.path.clone(), Some(s.clone()))).collect();
        let mut initial_active = saved.active;
        for (n, p) in paths.into_iter().enumerate() {
            let p = canvas_core::scan::clean_path(p.canonicalize().unwrap_or(p));
            let at = match initial.iter().position(|(q, _)| *q == p) {
                Some(i) => i,
                None => {
                    initial.push((p, None));
                    initial.len() - 1
                }
            };
            if n == 0 {
                initial_active = at;
            }
        }
        crate::page::forget_old_scans();
        let mut chosen = crate::themes::load();
        let mut ext = Extensions::new(session::data_dir(), waker);
        chosen.from_extension = theme_of_extension(&ext, &chosen.theme);
        canvas_render::theme::set(chosen.resolve());
        if std::env::var_os("CODE_CANVAS_SCRIPT").is_none() && std::env::var_os("CODE_CANVAS_BENCH").is_none() {
            ext.install_defaults();
        }
        let mut ui = UiState::default();
        ui.theme = chosen;
        ui.prefs = crate::prefs::load();
        canvas_core::layout::set_line_numbers(ui.prefs.line_numbers);
        crate::page::never_read(ui.prefs.skip_dirs.clone(), ui.prefs.skip_files.clone());
        let quiet = std::env::var_os("CODE_CANVAS_SCRIPT").is_some() || std::env::var_os("CODE_CANVAS_BENCH").is_some();
        let updates = (ui.prefs.check_updates && !quiet) || std::env::var_os("CODE_CANVAS_CHECK_UPDATES").is_some();
        let follow_chat = ui.prefs.follow_chat;
        ui.commit_ai = crate::commit_ai::load();
        ui.key_file = crate::keys::load();
        ui.keys = Keymap::new(&ui.key_file);
        ui.rails = saved.rails.clone();
        ui.rails_at_home();
        ui.restore(Side::Left, saved.left_open, &saved.left_last);
        ui.restore(Side::Right, saved.right_open, &saved.right_last);
        ui.single = saved.single;
        let mut app = Self {
            gfx: None,
            pages: Vec::new(),
            active: 0,
            next_id: 1,
            initial,
            initial_active,
            window_size: saved.window,
            window_pos: saved.window_pos,
            start_maximized: saved.maximized,
            cursor: [-1.0, -1.0],
            drag: None,
            mods: ModifiersState::empty(),
            last_click: None,
            ui,
            canvas: Rect::new(0.0, 0.0, 1.0, 1.0),
            canvas_full: Rect::new(0.0, 0.0, 1.0, 1.0),
            was_shared: false,
            chip_hits: Vec::new(),
            toggle_hits: Vec::new(),
            preview_hits: Vec::new(),
            preview: None,
            curves_for: None,
            last_frame: Instant::now(),
            frame_ms: 0.0,
            stats: FrameStats::default(),
            bench: Bench::from_env(),
            next_repaint: None,
            script: Script::from_env(),
            exit_requested: false,
            exit_confirmed: false,
            resize_edge: None,
            session_dirty: false,
            last_save: Instant::now(),
            ext,
            symbols_stale: None,
            unsent_edit: None,
            hover_probe: None,
            watcher: crate::watch::Watcher::new(),
            auto_rescan: None,
            rescan_later: HashSet::new(),
            changed_under: HashSet::new(),
            merged_in: HashSet::new(),
            webviews: {
                let mut webviews = crate::webview::WebViews::new();
                webviews.set_follow(follow_chat);
                webviews
            },
            webview_roots: Vec::new(),
            copies: Vec::new(),
            ui_events: VecDeque::new(),
            updater: crate::update::Updater::new(updates, ext::wake_ui),
            restart_as: None,
            completion: None,
            versions: crate::versions::Checker::new(ext::wake_ui),
            manifests: HashMap::new(),
            versions_due: Vec::new(),
            versions_told: HashSet::new(),
            theme_revision: canvas_render::theme::revision(),
            theme_dirty: false,
            last_input: Instant::now(),
            checkers_offered: HashSet::new(),
            following: None,
            text_checked: Instant::now(),
            show_when_read: None,
            back: Vec::new(),
            forward: Vec::new(),
            moved_by_itself: false,
            pictures: crate::pictures::Pictures::new(),
            playing: None,
            player_shown: false,
            commit_from: None,
            sharing: Default::default(),
            workspace: saved.workspace.clone(),
            frame_all_when_read: false,
            agents: crate::agents::Agents::listen(),
            agent_followed: None,
            claude_checked: None,
            claude_offer_due: Some(Instant::now() + Duration::from_secs(6)),
            last_move_by_itself: false,
        };
        app.share_setup();
        app.know_folders();
        app
    }

    fn scale(&self) -> f32 {
        self.gfx.as_ref().map(|g| g.window.scale_factor() as f32).unwrap_or(1.0)
    }

    fn viewport(&self) -> [f32; 2] {
        self.gfx.as_ref().map(|g| [g.config.width as f32, g.config.height as f32]).unwrap_or([1.0, 1.0])
    }

    fn request_redraw(&self) {
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }

    fn update_title(&self) {
        if let Some(g) = &self.gfx {
            let title = match self.pages.get(self.active) {
                Some(p) => format!("{} — Code Canvas", p.title),
                None => "Code Canvas".into(),
            };
            g.window.set_title(&title);
        }
    }

    fn shares_canvas(&self) -> bool {
        !self.ui.single && self.pages.len() > 1
    }

    fn shown(&self) -> Vec<usize> {
        if self.shares_canvas() {
            (0..self.pages.len()).collect()
        } else {
            vec![self.active]
        }
    }

    fn origins(&self) -> Vec<[f32; 2]> {
        let extents: Vec<Option<Rect>> = self.pages.iter().map(|page| page.extent).collect();
        crate::camera::origins(&extents)
    }

    fn follow_active_camera(&mut self) {
        if !self.shares_canvas() {
            return;
        }
        let origins = self.origins();
        let Some(leader) = self.pages.get(self.active).map(|page| page.camera) else { return };
        let own = origins[self.active];
        for (i, page) in self.pages.iter_mut().enumerate() {
            if i != self.active {
                page.camera = leader.seen_from(own, origins[i]);
                page.target = None;
            }
        }
    }

    fn focus_folder_at(&mut self, p: [f32; 2]) {
        if !self.shares_canvas() || !self.canvas_full.contains(p) || self.over_ui(p) {
            return;
        }
        self.follow_active_camera();
        let vp = self.viewport();
        let over = self
            .pages
            .iter()
            .position(|page| page.extent.is_some_and(|extent| extent.contains(page.camera.view(vp).to_world(p))));
        if let Some(i) = over.filter(|i| *i != self.active) {
            self.switch_to(i);
            self.session_dirty = true;
        }
    }

    fn show_folder(&mut self, i: usize) {
        self.switch_to(i);
        if !self.shares_canvas() {
            return;
        }
        let vp = self.viewport();
        let out_of_sight = self.pages.get(self.active).is_some_and(|page| {
            page.extent.is_some_and(|extent| !extent.intersects(&page.camera.view(vp).world_rect()))
        });
        if out_of_sight {
            self.frame_all();
        }
    }

    fn over_ui(&self, p: [f32; 2]) -> bool {
        if !self.canvas.contains(p) {
            return true;
        }
        let s = self.scale();
        let pt = egui::Pos2::new(p[0] / s, p[1] / s);
        self.ui.floating.iter().any(|r| r.contains(pt))
    }

    fn theme_changed(&mut self) {
        canvas_render::theme::set(self.ui.theme.resolve());
        self.theme_dirty = true;
        self.request_redraw();
    }

    fn follow_theme(&mut self) {
        self.webviews.retheme();
        let revision = canvas_render::theme::revision();
        if self.theme_revision == revision {
            return;
        }
        self.theme_revision = revision;
        if let Some(g) = &self.gfx {
            ui::apply_theme(&g.egui);
            for page in &mut self.pages {
                page.rebuild_scene(&g.renderer, &g.device);
            }
        }
        self.request_redraw();
    }

    fn save_session(&mut self) {
        if std::mem::take(&mut self.theme_dirty) {
            crate::themes::save(&self.ui.theme);
        }
        self.session_dirty = false;
        self.last_save = Instant::now();
        let s = self.scale();
        if let Some(g) = &self.gfx {
            let (maximized, minimized) = (g.window.is_maximized(), g.window.is_minimized().unwrap_or(false));
            self.start_maximized = maximized;
            if !maximized && !minimized {
                let size = g.window.inner_size().to_logical::<f32>(g.window.scale_factor());
                self.window_size = Some([size.width, size.height]);
                self.window_pos = g.window.outer_position().ok().map(|p| [p.x, p.y]);
            }
        }
        let (window, window_pos, maximized) = (self.window_size, self.window_pos, self.start_maximized);
        session::save(&Session {
            pages: self
                .pages
                .iter()
                .filter(|p| !p.only && p.shared.is_none())
                .map(|p| p.to_session(s, self.viewport()))
                .collect(),
            active: self.pages.iter().take(self.active).filter(|p| !p.only && p.shared.is_none()).count(),
            left_open: self.ui.left.is_some(),
            right_open: self.ui.right.is_some(),
            left_last: self.ui.last_left.key(),
            right_last: self.ui.last_right.key(),
            rails: self.ui.rails.clone(),
            workspace: self.workspace.clone(),
            window,
            window_pos,
            maximized,
            single: self.ui.single,
        });
    }

    fn edge_at(&self, p: [f32; 2]) -> Option<ResizeDirection> {
        let g = self.gfx.as_ref()?;
        if g.window.is_maximized() {
            return None;
        }
        let m = 5.0 * self.scale();
        let [w, h] = self.viewport();
        let (l, r, t, b) = (p[0] < m, p[0] > w - m, p[1] < m, p[1] > h - m);
        Some(match (l, r, t, b) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return None,
        })
    }

    fn drop_file(&mut self, file: PathBuf) {
        let Some(page) = self.pages.get(self.active) else { return };
        let selected = page.selected.and_then(|n| page.ready().and_then(|r| r.layout.nodes.get(n)));
        let rel = match selected {
            Some(n) if n.is_dir() => n.rel_path.clone(),
            Some(n) => n.rel_path.rsplit_once('/').map_or(String::new(), |(dir, _)| dir.to_string()),
            None => String::new(),
        };
        let dir = crate::files::inside(&page.root, &rel).unwrap_or_else(|_| page.root.clone());
        let shown = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let place: PathBuf = dir.components().collect();
        let already_there = dir.join(&shown);
        match crate::files::copy_into(file, &dir) {
            Ok(copying) => {
                let text = format!("Copying \"{shown}\" into {}", place.display());
                self.ext.notice(ext::Level::Info, "files", text);
                self.copies.push(copying);
            }
            Err(why) if why.is_empty() => {}
            Err(why) => {
                if !self.show_dropped(&already_there, "was already in") {
                    self.ext.notice(ext::Level::Warn, "files", format!("\"{shown}\" was not copied: {why}."));
                }
            }
        }
        self.request_redraw();
    }

    fn show_dropped(&mut self, target: &Path, arrival: &str) -> bool {
        if !target.is_file() {
            return false;
        }
        let inside = self.locate_file(target);
        if let Some((i, _)) = &inside {
            self.pages[*i].refresh_git();
        }
        let unlisted =
            inside.is_some_and(|(i, rel)| crate::git::not_ignored(&self.pages[i].root, vec![rel]).is_empty());
        let opened = self.open_media(target);
        if !unlisted && !opened {
            return false;
        }
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let place: PathBuf = target.parent().unwrap_or(target).components().collect();
        let mut text = format!("\"{name}\" {arrival} {}.", place.display());
        if unlisted {
            text.push_str(" Git ignores it there: it is not on the canvas, and the file list shows it dimmed.");
        }
        if opened {
            text.push_str(" It is open in the viewer.");
        }
        self.ext.notice(ext::Level::Info, "files", text);
        true
    }

    fn poll_copies(&mut self) -> bool {
        let mut arrived = Vec::new();
        self.copies.retain(|copying| match copying.finished.try_recv() {
            Ok(result) => {
                arrived.push((copying.target.clone(), result));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
        for (target, result) in arrived {
            match result {
                Ok(()) => {
                    self.show_dropped(&target, "is now in");
                }
                Err(why) => {
                    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.ext.notice(ext::Level::Warn, "files", format!("\"{name}\" was not copied: {why}."));
                }
            }
            self.request_redraw();
        }
        !self.copies.is_empty()
    }

    fn file_op(&mut self, op: FileOp) {
        let Some(root) = self.pages.get(self.active).map(|p| p.root.clone()) else { return };
        let changes = !matches!(op, FileOp::Reveal(_) | FileOp::Terminal(_) | FileOp::CopyPath { .. });
        if changes && self.pages[self.active].shared.is_some() && self.pages[self.active].read_only {
            let text = "You are looking, not editing: the one who shares can let you make, rename and delete files.";
            return self.ext.notice(ext::Level::Info, "Sharing", text);
        }
        let at = |rel: &str| crate::files::inside(&root, rel);
        let done: Result<(), String> = match op {
            FileOp::Reveal(path) => at(&path).and_then(|p| crate::files::reveal(&p)),
            FileOp::Terminal(dir) => at(&dir).and_then(|p| crate::files::terminal(&p)),
            FileOp::CopyPath { path, relative } => at(&path).and_then(|full| {
                let full: PathBuf = full.components().collect();
                let text = if relative { path.clone() } else { full.display().to_string() };
                arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|e| e.to_string())
            }),
            FileOp::NewFile { dir, name } => at(&dir).and_then(|d| crate::files::create_file(&d, &name)).map(|_| ()),
            FileOp::NewFolder { dir, name } => {
                at(&dir).and_then(|d| crate::files::create_folder(&d, &name)).map(|_| ())
            }
            FileOp::Rename { path, name } => at(&path).and_then(|p| crate::files::rename(&p, &name)).map(|_| ()),
            FileOp::Delete(path) if path.is_empty() => Err("the open folder itself is not deleted from here".into()),
            FileOp::Delete(path) => at(&path).map(|p| {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                let what = if p.is_dir() { "the folder and everything in it" } else { "the file" };
                let bin = if cfg!(windows) { "the Recycle Bin" } else { "the trash" };
                let inside = format!("{path}/");
                let page = &self.pages[self.active];
                let unsaved = page.unsaved().any(|e| e.rel_path == path || e.rel_path.starts_with(&inside));
                let lost = if unsaved { " What is not saved in it is lost." } else { "" };
                let text = format!("It moves {what} to {bin}, where it can be brought back from.{lost}");
                let about = ui::Asked::Delete { folder: root.clone(), path: path.clone() };
                let mut ask = ui::Ask::new(about, &format!("Delete {name}?"), text)
                    .choice("Delete", ui::ChoiceLook::Danger, ui::Answer::Yes)
                    .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
                ask.items = vec![path];
                self.ui.ask = Some(ask);
            }),
        };
        if let Err(why) = done {
            self.ext.notice(ext::Level::Error, "files", why);
        }
        self.request_redraw();
    }

    fn open_folder(&mut self, path: PathBuf, saved: Option<PageSession>) {
        let path = canvas_core::scan::clean_path(path.canonicalize().unwrap_or(path));
        if let Some(i) = self.pages.iter().position(|p| p.root == path) {
            self.switch_to(i);
            return;
        }
        log::info!("opening {}", path.display());
        let page = Page::open(self.next_id, path, saved.as_ref(), true);
        self.next_id += 1;
        self.pages.push(page);
        self.switch_to(self.pages.len() - 1);
        self.know_folders();
    }

    fn know_folders(&mut self) {
        for page in self.pages.iter().filter(|page| !page.only && page.shared.is_none()) {
            let files = page.ready().map(|r| r.repo.files.len() as u32);
            match self.workspace.iter_mut().find(|known| known.path == page.root) {
                Some(known) => known.files = files.or(known.files),
                None => self.workspace.push(session::Known { path: page.root.clone(), files }),
            }
        }
    }

    fn workspace_rows(&self) -> Vec<ui::WorkspaceRow> {
        let row = |known: &session::Known| {
            let page = self.pages.iter().position(|page| page.root == known.path);
            let name = known.path.file_name().map(|name| name.to_string_lossy().into_owned());
            ui::WorkspaceRow {
                path: known.path.clone(),
                title: name.unwrap_or_else(|| known.path.display().to_string()),
                files: known.files,
                page,
                active: page == Some(self.active),
                loading: page.is_some_and(|page| self.pages[page].is_loading()),
            }
        };
        self.workspace.iter().map(row).collect()
    }

    fn workspace_op(&mut self, op: ui::WorkspaceOp) {
        use ui::WorkspaceOp;
        let on = |me: &App, path: &Path| me.pages.iter().position(|page| page.root == path);
        match op {
            WorkspaceOp::Put(path) => match on(self, &path) {
                Some(page) => {
                    let apart = std::mem::take(&mut self.ui.single);
                    self.show_folder(page);
                    if apart {
                        self.frame_all_when_read = true;
                    }
                }
                None if path.is_dir() => {
                    self.ui.single = false;
                    self.open_folder(path, None);
                }
                None => {
                    let text = format!("{} is not there any more.", path.display());
                    self.ext.notice(ext::Level::Warn, "Workspace", text);
                }
            },
            WorkspaceOp::TakeOff(path) => {
                if let Some(page) = on(self, &path) {
                    self.know_folders();
                    self.close_folder(page, false);
                }
            }
            WorkspaceOp::Forget(path) => match on(self, &path) {
                Some(page) => self.close_folder(page, true),
                None => self.workspace.retain(|known| known.path != path),
            },
            WorkspaceOp::PutAll => {
                let off: Vec<PathBuf> = self.workspace.iter().map(|known| known.path.clone()).collect();
                self.ui.single = false;
                for path in off.into_iter().filter(|path| path.is_dir()) {
                    if on(self, &path).is_none() {
                        self.open_folder(path, None);
                    }
                }
                self.frame_all_when_read = true;
            }
        }
        self.save_session();
        self.request_redraw();
    }

    fn pick_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Open another folder");
        if let Some(g) = &self.gfx {
            dialog = dialog.set_parent(&g.window);
        }
        if let Some(path) = dialog.pick_folder() {
            self.open_folder(path, None);
            self.save_session();
        }
    }

    fn switch_to(&mut self, i: usize) {
        if i < self.pages.len() {
            if i != self.active {
                self.completion = None;
            }
            if i != self.active && self.shares_canvas() {
                self.follow_active_camera();
                let origins = self.origins();
                let flying_to = self.pages.get_mut(self.active).and_then(|page| page.target.take());
                if let Some(target) = flying_to {
                    self.pages[i].target = Some(target.seen_from(origins[self.active], origins[i]));
                }
            }
            self.active = i;
            self.pages[i].ensure_scanned();
            self.drag = None;
            self.update_title();
            self.request_redraw();
        }
    }

    fn close_folder(&mut self, i: usize, forget: bool) {
        let Some(page) = self.pages.get(i) else { return };
        let folder = page.root.clone();
        let unsaved: Vec<String> = match page.shared {
            Some(_) => Vec::new(),
            None => page.editors.values().filter(|e| e.dirty).map(|e| e.rel_path.clone()).collect(),
        };
        if unsaved.is_empty() {
            self.close_page(i);
            if forget {
                self.workspace.retain(|known| known.path != folder);
            }
            return;
        }
        let title = format!("Save the changes in {} before closing it?", page.title);
        let text = match unsaved.len() {
            1 => "One file has changes that are not saved. Without saving they are lost.".to_string(),
            n => format!("{n} files have changes that are not saved. Without saving they are lost."),
        };
        let mut ask = ui::Ask::new(ui::Asked::Close { folder, forget }, &title, text)
            .choice("Save and close", ui::ChoiceLook::Main, ui::Answer::Yes)
            .choice("Close without saving", ui::ChoiceLook::Plain, ui::Answer::Other)
            .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
        ask.items = unsaved;
        ask.danger = false;
        self.ui.ask = Some(ask);
        self.request_redraw();
    }

    fn close_page(&mut self, i: usize) {
        if i >= self.pages.len() {
            return;
        }
        if self.pages[i].shared.is_some() && self.sharing.session.is_some() {
            return self.share_stop(None);
        }
        let page = self.pages.remove(i);
        if let Some(g) = &mut self.gfx {
            g.renderer.text.drop_page(page.id);
        }
        if self.active >= self.pages.len() {
            self.active = self.pages.len().saturating_sub(1);
        } else if i < self.active {
            self.active -= 1;
        }
        self.update_title();
        self.save_session();
        self.request_redraw();
    }

    fn cycle(&mut self, delta: isize) {
        let n = self.pages.len() as isize;
        if n > 0 {
            self.show_folder(((self.active as isize + delta).rem_euclid(n)) as usize);
        }
    }

    fn rescan(&mut self) {
        let (s, vp) = (self.scale(), self.viewport());
        if let Some(p) = self.pages.get_mut(self.active) {
            if !p.is_loading() {
                p.restore_camera = p.portable_camera(s, vp);
                p.framed = false;
                p.read_again = true;
                p.start_scan();
                self.request_redraw();
            }
        }
    }

    fn relayout(&mut self) {
        if let (Some(g), Some(page)) = (self.gfx.as_ref(), self.pages.get_mut(self.active)) {
            page.relayout(&g.renderer, &g.device, &self.ui.filters);
        }
        self.request_redraw();
    }

    fn locate_file(&self, path: &Path) -> Option<(usize, String)> {
        let key = ext::path_key(&path.to_string_lossy());
        let inside = self.pages.iter().enumerate().filter_map(|(i, p)| {
            let mut root = ext::path_key(&p.root.to_string_lossy());
            root.push('/');
            let whole = path.to_string_lossy().replace('\\', "/");
            let rel = whole.get(root.len()..).filter(|_| key.starts_with(&root) && key.len() == whole.len())?;
            Some((i, rel.to_string()))
        });
        let on_canvas = |(i, rel): &(usize, String)| {
            self.pages[*i].ready().is_some_and(|r| r.layout.node_for_path(&r.repo, rel).is_some())
        };
        inside.clone().find(on_canvas).or_else(|| inside.clone().next())
    }

    fn show_path(&mut self, path: &Path, at: Option<(u32, u32, u32)>) {
        let path = canvas_core::scan::clean_path(path.to_path_buf());
        let found = self.locate_file(&path);
        let shown = found.as_ref().is_some_and(|(i, rel)| {
            self.pages[*i].ready().is_some_and(|r| r.layout.node_for_path(&r.repo, rel).is_some())
        });
        if let (true, Some((i, rel))) = (shown, &found) {
            self.switch_to(*i);
            self.show_when_read = None;
            let file = self.pages[*i].ready().and_then(|r| r.repo.file_index(rel));
            match (at, file) {
                (Some((line, col, len)), Some(file)) => {
                    let line = self.pages[*i].row_of(rel, line);
                    let (col, len) = (col.min(u16::MAX as u32) as u16, len.clamp(1, 400) as u16);
                    let site = Site { file: file as u32, line, col, len };
                    self.frame_site(site);
                    self.pages[*i].search = Some(crate::page::SearchResults::single(site));
                }
                _ => self.apply(Action::FlyToPath(rel.clone())),
            }
            return;
        }
        if !path.exists() {
            let text = format!("{} is not there any more.", path.display());
            return self.ext.notice(ext::Level::Warn, "Files", text);
        }
        if path.is_dir() {
            return self.open_folder(path, None);
        }
        let waiting = found.as_ref().is_some_and(|(i, _)| self.pages[*i].ready().is_none());
        match found {
            Some((i, _)) if waiting => self.pages[i].ensure_scanned(),
            Some((i, rel)) => {
                let (s, vp) = (self.scale(), self.viewport());
                let page = &mut self.pages[i];
                let read = page.ready().is_some_and(|r| r.repo.file_index(&rel).is_some());
                let narrowed = page.only_changed || page.only_connected || page.showing.is_some();
                if read && narrowed && !page.filter.hidden.contains(&rel) {
                    page.only_changed = false;
                    page.only_connected = false;
                    if page.showing.is_some() {
                        page.show_commit(None);
                    }
                    self.switch_to(i);
                    self.relayout();
                    self.show_when_read = Some((path, at));
                    return self.request_redraw();
                }
                let page = &mut self.pages[i];
                if page.extra.contains(&rel) {
                    let text = format!("{rel} is hidden on the canvas: tick it in the explorer to see it.");
                    return self.ext.notice(ext::Level::Info, "Files", text);
                }
                page.extra.push(rel);
                page.restore_camera = page.portable_camera(s, vp);
                page.framed = false;
                page.start_scan();
            }
            None => {
                let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else { return };
                let name = name.to_string_lossy().into_owned();
                match self.pages.iter().position(|p| p.only && p.root == folder) {
                    Some(i) => {
                        self.pages[i].extra.push(name);
                        self.pages[i].start_scan();
                        self.switch_to(i);
                    }
                    None => {
                        log::info!("showing {} from outside the open folders", path.display());
                        self.pages.push(Page::of_files(self.next_id, folder.to_path_buf(), vec![name]));
                        self.next_id += 1;
                        self.switch_to(self.pages.len() - 1);
                    }
                }
            }
        }
        self.show_when_read = Some((path, at));
        self.request_redraw();
    }

    fn write_file(&mut self, path: &Path, text: &str) {
        self.watcher.expect(path);
        if let Err(e) = std::fs::write(path, text) {
            log::error!("cannot write {}: {e}", path.display());
            return;
        }
        log::info!("wrote {} ({} bytes)", path.display(), text.len());
        self.ext.file_changed(path);
        if let Some((i, rel)) = self.locate_file(path) {
            if self.sharing.session.is_some() {
                self.sharing.touched.push((self.pages[i].id, rel.clone()));
            }
            if let Some(page) = self.pages.get_mut(i) {
                if page.editors.get(&rel).is_some_and(|e| !e.dirty) {
                    let caret = page.editors.remove(&rel).map(|e| e.caret).unwrap_or_default();
                    if page.editing.as_deref() == Some(rel.as_str()) && page.begin_edit(&rel) {
                        if let Some(e) = page.editor_mut() {
                            e.move_to(caret, false);
                        }
                    }
                }
            }
            if let (Some(g), Some(page)) = (self.gfx.as_mut(), self.pages.get_mut(i)) {
                g.renderer.text.drop_page(page.id);
                page.reload_file(&rel, &g.renderer, &g.device, &self.ui.filters);
                page.refresh_git();
            }
        }
        self.request_redraw();
    }

    fn go_to_definition(&mut self) {
        let Some(page) = self.pages.get(self.active) else { return };
        let Some(r) = page.ready() else { return };
        let at = match page.editor() {
            Some(e) => {
                r.repo.file_index(&e.rel_path).map(|f| (f as u32, e.caret.line as u32, e.display_col(e.caret) as u32))
            }
            None => page.hover_cell,
        };
        let Some((file, line, col)) = at else { return };
        let rel = &r.repo.files[file as usize].rel_path;
        let path = page.root.join(rel);
        let by_index = r.repo.resolve_at(file, line, col).map(|(sym, _)| r.repo.sym_site(sym));
        let asked = page.line_of(rel, line).is_some_and(|line| self.ext.request_definition(&path, line, col));
        if !asked {
            if let Some(site) = by_index {
                self.frame_site(site);
            }
        }
    }

    fn on_disk_changes(&mut self, batch: crate::watch::Batch) {
        const ONE_BY_ONE: usize = 24;
        log::debug!(
            "changed on disk: {} files {:?}, git of {} folders",
            batch.files.len(),
            batch.files.iter().take(4).collect::<Vec<_>>(),
            batch.git.len()
        );
        let skip = crate::page::scan_options().skip_dirs;
        for root in &batch.git {
            if let Some(page) = self.pages.iter_mut().find(|p| &p.root == root) {
                page.refresh_git();
            }
        }
        let mut known: Vec<Vec<String>> = vec![Vec::new(); self.pages.len()];
        let mut other: Vec<Vec<String>> = vec![Vec::new(); self.pages.len()];
        let mut seen: Option<(usize, String, u32)> = None;
        for path in &batch.files {
            let Some((i, rel)) = self.locate_file(path) else { continue };
            if rel.split('/').any(|part| skip.iter().any(|s| s == part)) {
                continue;
            }
            if self.sharing.session.is_some() {
                self.sharing.touched.push((self.pages[i].id, rel.clone()));
            }
            let on_canvas = self.pages[i].ready().is_some_and(|r| r.repo.file_index(&rel).is_some());
            match (on_canvas, path.is_file()) {
                (true, true) => {
                    self.pictures.forget(path);
                    known[i].push(rel)
                }
                (true, false) => {
                    let page = &self.pages[i];
                    let unsaved = page.editors.get(&rel).is_some_and(|e| e.dirty);
                    if unsaved && self.changed_under.insert((page.id, rel.clone())) {
                        let text = format!(
                            "{rel} was deleted by another program while it has changes not saved here. Saving will write it again."
                        );
                        self.ext.notice(ext::Level::Warn, "Files", text);
                    }
                    other[i].push(rel)
                }
                (false, true) => other[i].push(rel),
                (false, false) => {}
            }
        }
        for i in 0..self.pages.len() {
            if self.pages[i].only {
                other[i].clear();
            }
            let root = self.pages[i].root.clone();
            let fresh = if other[i].is_empty() {
                Vec::new()
            } else {
                crate::git::not_ignored(&root, std::mem::take(&mut other[i]))
            };
            if known[i].is_empty() && fresh.is_empty() {
                continue;
            }
            self.pages[i].refresh_git();
            let rescan = !fresh.is_empty() || known[i].len() > ONE_BY_ONE;
            if rescan {
                let reveal = fresh.iter().find(|rel| root.join(rel).is_file()).cloned();
                if self.rescan_quietly(i, reveal) {
                    continue;
                }
                self.rescan_later.insert(self.pages[i].id);
                known[i].truncate(ONE_BY_ONE * 2);
            }
            for rel in std::mem::take(&mut known[i]) {
                let page = &mut self.pages[i];
                if page.editors.get(&rel).is_some_and(|e| e.dirty) {
                    let theirs = std::fs::read_to_string(root.join(&rel)).ok().map(|text| crate::share::body(&text));
                    let merged = theirs.and_then(|text| page.editors.get_mut(&rel)?.take_in(&text));
                    if let Some((first, count)) = merged {
                        if let Some(g) = self.gfx.as_mut() {
                            g.renderer.text.drop_page(page.id);
                            page.show_edit(&rel, &g.renderer, &g.device, &self.ui.filters);
                        }
                        page.want_head(&rel);
                        page.preview_unsaved();
                        if count > 0 {
                            page.touch(&rel, vec![(first, count)]);
                        }
                        if i == self.active && page.editing.as_deref() == Some(rel.as_str()) {
                            self.completion = None;
                        }
                        self.symbols_stale = Some(Instant::now());
                        self.unsent_edit = Some((root.join(&rel), Instant::now()));
                        if count > 0 && self.merged_in.insert((page.id, rel.clone())) {
                            let text = format!(
                                "{rel} was changed by another program while it has changes not saved here: what it wrote is in the text now, with yours, and the file is still not saved."
                            );
                            self.ext.notice(ext::Level::Info, "Files", text);
                        }
                        continue;
                    }
                    if self.changed_under.insert((page.id, rel.clone())) {
                        let text = format!(
                            "{rel} was changed by another program while it has changes not saved here. Saving will replace what that program wrote."
                        );
                        self.ext.notice(ext::Level::Warn, "Files", text);
                    }
                    continue;
                }
                let caret = page.editors.remove(&rel).map(|e| e.caret);
                let text_of = |page: &Page| {
                    let r = page.ready()?;
                    let shown = &r.repo.files[r.repo.file_index(&rel)?];
                    match page.diffs.get(&rel) {
                        Some(diffed) => Some(diffed.plain.clone()),
                        None => shown.has_text().then(|| shown.text().to_string()),
                    }
                };
                let before = text_of(page);
                if let Some(g) = self.gfx.as_mut() {
                    g.renderer.text.drop_page(page.id);
                    page.reload_file(&rel, &g.renderer, &g.device, &self.ui.filters);
                }
                match (before, text_of(page)) {
                    (Some(before), Some(after)) => {
                        let (hunks, _, _) = crate::git::compare(&before, &after);
                        let lines: Vec<(u32, u32)> = hunks.iter().map(|h| (h.line, h.count.max(1))).collect();
                        if let Some(&(first, _)) = lines.first() {
                            page.touch(&rel, lines);
                            seen = Some((i, rel.clone(), first));
                        }
                    }
                    (None, Some(_)) => {
                        page.touch(&rel, Vec::new());
                        seen = Some((i, rel.clone(), 0));
                    }
                    _ => {}
                }
                if rel.ends_with("package.json") || rel.ends_with("Cargo.toml") {
                    self.versions_due.push(page.id);
                }
                if let (Some(caret), true) = (caret, page.editing.as_deref() == Some(rel.as_str())) {
                    if page.begin_edit(&rel) {
                        if let Some(e) = page.editor_mut() {
                            e.move_to(caret, false);
                        }
                    }
                }
                self.ext.file_changed(&root.join(&rel));
            }
        }
        if let Some((index, rel, line)) = seen {
            self.follow_change(index, &rel, line);
        }
        self.request_redraw();
    }

    fn rescan_quietly(&mut self, i: usize, reveal: Option<String>) -> bool {
        let Some(page) = self.pages.get(i) else { return true };
        let big = page.ready().is_some_and(|r| r.repo.files.len() > 60_000);
        let recent = self.auto_rescan.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        if big {
            return true;
        }
        if recent || page.is_loading() {
            return false;
        }
        log::info!("{}: files were added or removed, scanning again", page.root.display());
        self.auto_rescan = Some(Instant::now());
        let (s, vp) = (self.scale(), self.viewport());
        let p = &mut self.pages[i];
        p.restore_camera = p.portable_camera(s, vp);
        p.framed = false;
        p.editors.retain(|_, editor| editor.dirty);
        if p.editing.as_ref().is_some_and(|rel| !p.editors.contains_key(rel)) {
            p.editing = None;
        }
        p.reveal = reveal;
        p.start_scan();
        true
    }

    fn rescan_what_waited(&mut self) {
        for id in self.rescan_later.clone() {
            match self.pages.iter().position(|page| page.id == id) {
                Some(i) => {
                    if self.rescan_quietly(i, None) {
                        self.rescan_later.remove(&id);
                    }
                }
                None => {
                    self.rescan_later.remove(&id);
                }
            }
        }
    }

    fn offer_checker(&mut self, rel: &str) {
        let kind = Path::new(rel).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some((_, language, id, name)) = CHECKERS.iter().find(|(kinds, ..)| kinds.contains(&kind.as_str())) else {
            return;
        };
        let known = self.ext.is_installed(id) || self.ui.prefs.unwanted.iter().any(|u| u == id);
        if known || !self.checkers_offered.insert(id) {
            return;
        }
        let text =
            format!("Nothing checks {language} while you type yet. {name} underlines mistakes as they are written.");
        self.ext.ask("Extensions", text, &[INSTALL_IT, "Not now", NEVER], &format!("checker:{id}"));
    }

    fn follow_change(&mut self, index: usize, rel: &str, line: u32) {
        let idle = self.drag.is_none() && self.last_input.elapsed() > Duration::from_millis(1500);
        let editing = self.pages.get(self.active).is_some_and(|p| p.editing.is_some());
        let asking = self.ui.ask.is_some() || self.ui.card_menu.is_some();
        if !self.ui.prefs.follow_changes || !idle || editing || asking {
            return;
        }
        if index != self.active {
            if !self.shares_canvas() {
                return;
            }
            self.switch_to(index);
        }
        self.following = Some((index, rel.to_string(), line, Instant::now()));
    }

    fn code_under(&self, point: [f32; 2]) -> Option<String> {
        let page = self.pages.get(self.active)?;
        let r = page.ready()?;
        if Lod::for_zoom(page.camera.zoom) != Lod::Text {
            return None;
        }
        let world = page.camera.view(self.viewport()).to_world(point);
        let n = &r.layout.nodes[r.layout.hit(world)?];
        let NodeKind::File(fi) = n.kind else { return None };
        let f = &r.repo.files[fi];
        n.cell_at(world, f.line_count())?;
        (f.note.is_none() && page.rows(&f.rel_path).is_none()).then(|| f.rel_path.clone())
    }

    fn card_menu(&mut self, point: [f32; 2]) {
        self.ui.card_menu = None;
        let scale = self.scale();
        let bare = |me: &mut App| {
            me.ui.card_menu = Some(ui::CardMenu {
                at: egui::Pos2::new(point[0] / scale, point[1] / scale),
                folder: None,
                on: ui::MenuOn::Canvas,
                line: None,
                staged: false,
                unstaged: false,
                untracked: false,
                place: None,
            });
            me.request_redraw();
        };
        let Some(page) = self.pages.get(self.active) else { return bare(self) };
        let Some(r) = page.ready() else { return bare(self) };
        let world = page.camera.view(self.viewport()).to_world(point);
        let scale = self.scale();
        let folder = (!page.only && page.shared.is_none()).then(|| (page.root.clone(), page.title.clone()));
        let hit = r.layout.hit(world).map(|n| &r.layout.nodes[n]);
        let file = hit.and_then(|n| match n.kind {
            NodeKind::File(fi) => Some((n, fi)),
            NodeKind::Dir => None,
        });
        let Some((n, fi)) = file else {
            let on = match hit {
                Some(n) => ui::MenuOn::Folder { path: n.rel_path.clone(), collapsed: n.collapsed },
                None => ui::MenuOn::Canvas,
            };
            self.ui.card_menu = Some(ui::CardMenu {
                at: egui::Pos2::new(point[0] / scale, point[1] / scale),
                folder,
                on,
                line: None,
                staged: false,
                unstaged: false,
                untracked: false,
                place: None,
            });
            return self.request_redraw();
        };
        let f = &r.repo.files[fi];
        let change = page.git.as_ref().and_then(|git| git.change(&f.rel_path));
        let readable = Lod::for_zoom(page.camera.zoom) == Lod::Text;
        let row = n.cell_at(world, f.line_count()).filter(|_| readable && page.showing.is_none()).map(|(row, _)| row);
        let line = row.and_then(|row| match page.rows(&f.rel_path) {
            Some(rows) => rows.get(row as usize).filter(|row| row.kind != crate::git::RowKind::Gap).map(|row| row.line),
            None => Some(row),
        });
        let changed_here = |line: &u32| {
            let hunks =
                change.filter(|change| !change.unsaved).map(|change| change.hunks.as_slice()).unwrap_or_default();
            hunks.iter().any(|hunk| crate::git::holds(hunk.line, hunk.count, *line))
        };
        self.ui.card_menu = Some(ui::CardMenu {
            at: egui::Pos2::new(point[0] / scale, point[1] / scale),
            folder,
            on: ui::MenuOn::File { path: f.rel_path.clone() },
            line: line.filter(changed_here),
            staged: change.is_some_and(|change| change.staged.is_some()),
            unstaged: change
                .is_some_and(|change| change.unstaged.is_some() && change.change != crate::git::Change::Untracked),
            untracked: change.is_some_and(|change| change.change == crate::git::Change::Untracked),
            place: None,
        });
        self.request_redraw();
    }

    fn cell_in(&self, rel: &str, point: [f32; 2]) -> Option<(usize, usize)> {
        let page = self.pages.get(self.active)?;
        let r = page.ready()?;
        let fi = r.repo.file_index(rel)?;
        let n = r.layout.file_node.get(fi).filter(|&&n| n != canvas_core::layout::NONE)?;
        let n = &r.layout.nodes[*n as usize];
        let lines = r.repo.files[fi].line_count();
        let world = page.camera.view(self.viewport()).to_world(point);
        let first = n.cell_rect(0, 0, 1);
        let last = n.cell_rect(lines.saturating_sub(1) as u32, 0, 1);
        let inside = [world[0].max(first.x) + first.w * 0.5, world[1].clamp(first.y, last.bottom() - 0.01)];
        n.cell_at(inside, lines).map(|(line, col)| (line as usize, col as usize))
    }

    fn drag_select(&mut self, rel: &str, from: [f32; 2], first: bool) {
        let Some(to) = self.cell_in(rel, self.cursor) else { return };
        let anchor = if first { self.cell_in(rel, from) } else { None };
        let Some(page) = self.pages.get_mut(self.active) else { return };
        if first && !page.begin_edit(rel) {
            return;
        }
        if let Some(editor) = page.editor_mut().filter(|e| e.rel_path == rel) {
            if let Some((line, col)) = anchor {
                let at = editor.pos_at_display(line, col);
                editor.move_to(at, false);
            }
            let at = editor.pos_at_display(to.0, to.1);
            editor.move_to(at, true);
        }
        self.completion = None;
        self.request_redraw();
    }

    fn find(&mut self, op: ui::FindOp) {
        use ui::FindOp;
        match op {
            FindOp::Open => {
                let page = self.pages.get(self.active);
                let editing = page.and_then(|p| p.editing.clone());
                let pointed = self.current_file().and_then(|file| self.locate_file(&file)).map(|(_, rel)| rel);
                let Some(file) = editing.or(pointed) else {
                    let text = "Select a file first: click its card, or its name in the explorer.";
                    return self.ext.notice(ext::Level::Info, "Find", text.to_string());
                };
                let chosen = page.and_then(|p| p.editor()).map(|e| e.selected_text()).unwrap_or_default();
                let before = self.ui.find.take().unwrap_or_default();
                let query = if chosen.is_empty() || chosen.contains('\n') { before.query } else { chosen };
                self.ui.find = Some(ui::Find { query, with: before.with, focus: true, file, ..Default::default() });
                self.find_refresh(true);
            }
            FindOp::Changed => {
                if let Some(find) = self.ui.find.as_mut() {
                    find.at = 0;
                }
                self.find_refresh(true);
            }
            FindOp::Next | FindOp::Previous => {
                if let Some(find) = self.ui.find.as_mut().filter(|find| find.count > 0) {
                    let step = if op == FindOp::Next { 1 } else { find.count - 1 };
                    find.at = (find.at + step) % find.count;
                }
                self.find_refresh(true);
            }
            FindOp::Replace | FindOp::ReplaceAll => self.find_replace(op == FindOp::ReplaceAll),
            FindOp::Close => {
                self.ui.find = None;
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.search = None;
                }
                self.request_redraw();
            }
        }
    }

    fn found_in(page: &Page, file: &str, query: &str, most: usize) -> Vec<Site> {
        let mut hits: Vec<Site> = Vec::new();
        let Some(r) = page.ready() else { return hits };
        let Some(index) = r.repo.file_index(file).filter(|_| !query.is_empty()) else { return hits };
        let f = &r.repo.files[index];
        let exact = query.chars().any(char::is_uppercase);
        let same = |c: char| {
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(one), None) if !exact => one,
                _ => c,
            }
        };
        let wanted: Vec<char> = query.chars().map(same).collect();
        let len = wanted.len().min(400) as u16;
        'lines: for line in 0..f.line_count() {
            let text: Vec<char> = f.line(line).chars().map(same).collect();
            let mut from = 0;
            while from + wanted.len() <= text.len() {
                if text[from..from + wanted.len()] != wanted[..] {
                    from += 1;
                    continue;
                }
                let col = from.min(u16::MAX as usize) as u16;
                hits.push(Site { file: index as u32, line: line as u32, col, len });
                from += wanted.len();
                if hits.len() >= most {
                    break 'lines;
                }
            }
        }
        hits
    }

    fn find_refresh(&mut self, travel: bool) {
        self.find_again(travel, 5000);
    }

    fn find_again(&mut self, travel: bool, most: usize) {
        let Some(find) = self.ui.find.as_mut() else { return };
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let hits = Self::found_in(page, &find.file, &find.query, most);
        find.count = hits.len();
        find.at = find.at.min(hits.len().saturating_sub(1));
        let current = hits.get(find.at).copied();
        page.search = (!hits.is_empty()).then(|| crate::page::SearchResults {
            query: find.query.clone(),
            hits,
            truncated: false,
            ms: 0.0,
            current: Some(find.at),
        });
        if let (true, Some(site)) = (travel, current) {
            self.moved_by_itself = true;
            self.frame_site(site);
            self.moved_by_itself = false;
        }
        self.request_redraw();
    }

    fn find_replace(&mut self, all: bool) {
        let Some(file) = self.ui.find.as_ref().map(|find| find.file.clone()) else { return };
        if !self.ui.prefs.edit {
            let text = "Replacing changes the file: switch to the pencil first.";
            return self.ext.notice(ext::Level::Info, "Find", text.to_string());
        }
        let Some(page) = self.pages.get_mut(self.active) else { return };
        if !page.begin_edit(&file) {
            return;
        }
        self.sync_diffs();
        self.find_again(false, if all { usize::MAX } else { 5000 });
        let Some((with, at)) = self.ui.find.as_ref().map(|find| (find.with.clone(), find.at)) else { return };
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let hits: Vec<Site> = page.search.as_ref().map(|found| found.hits.clone()).unwrap_or_default();
        let chosen: Vec<Site> = if all { hits } else { hits.get(at).copied().into_iter().collect() };
        let Some(editor) = page.editor_mut().filter(|_| !chosen.is_empty()) else { return };
        let place = |editor: &crate::editor::Editor, hit: &Site| {
            let (line, col) = (hit.line as usize, hit.col as usize);
            (editor.pos_at_display(line, col), editor.pos_at_display(line, col + hit.len as usize))
        };
        let mut after = None;
        if all {
            let places: Vec<_> = chosen.iter().map(|hit| place(editor, hit)).collect();
            editor.replace_each(&places, &with);
        } else if let Some(hit) = chosen.first() {
            let (from, to) = place(editor, hit);
            editor.move_to(from, false);
            editor.move_to(to, true);
            editor.insert(&with);
            after = Some((hit.line, editor.display_col(editor.caret) as u16));
        }
        self.after_edit();
        if let (Some(after), Some(find)) = (after, self.ui.find.as_mut()) {
            let hits = self.pages.get(self.active).and_then(|page| page.search.as_ref()).map(|found| &found.hits);
            let next = hits.and_then(|hits| hits.iter().position(|hit| (hit.line, hit.col) >= after));
            find.at = next.unwrap_or(0);
        }
        self.find_refresh(!all);
    }

    fn follow_now(&mut self) -> bool {
        let Some((index, rel, line, since)) = self.following.take() else { return false };
        let Some(page) = self.pages.get(index) else { return false };
        if !page.diff_settled(&rel) && since.elapsed() < Duration::from_millis(1500) {
            self.following = Some((index, rel, line, since));
            return true;
        }
        let file = page.ready().and_then(|r| r.repo.file_index(&rel));
        let mut row = page.row_of(&rel, line);
        let column = page.ready().zip(file).and_then(|(r, file)| {
            let node = r.layout.file_node.get(file).filter(|&&node| node != canvas_core::layout::NONE)?;
            Some(r.layout.nodes[*node as usize].fold)
        });
        if let (true, Some(rows)) = (page.diff.split && page.rows(&rel).is_some(), column) {
            row -= rows.min(row);
        }
        if let (Some(file), true) = (file, index == self.active) {
            self.moved_by_itself = true;
            self.frame_site(Site { file: file as u32, line: row, col: 0, len: 0 });
            self.moved_by_itself = false;
        }
        false
    }

    fn sync_diffs(&mut self) -> bool {
        let Some(g) = self.gfx.as_mut() else { return false };
        let mut changed = false;
        for page in &mut self.pages {
            if page.diff_due() && page.sync_diff(&g.renderer, &g.device, &self.ui.filters) {
                g.renderer.text.drop_page(page.id);
                changed = true;
            }
        }
        let mut framed = Vec::new();
        for (i, page) in self.pages.iter_mut().enumerate() {
            if std::mem::take(&mut page.frame_wanted) {
                framed.push(i);
            }
        }
        if framed.contains(&self.active) {
            match (self.pages[self.active].showing.is_some(), self.commit_from.take()) {
                (false, Some(place)) => self.go_place(&place),
                (true, from) => {
                    self.commit_from = from;
                    self.frame_all();
                }
                (false, None) => self.frame_all(),
            }
        }
        changed
    }

    fn after_edit(&mut self) {
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let Some(rel) = page.editing.clone() else { return };
        if let Some(g) = self.gfx.as_mut() {
            g.renderer.text.drop_page(page.id);
            page.show_edit(&rel, &g.renderer, &g.device, &self.ui.filters);
        }
        page.want_head(&rel);
        self.symbols_stale = Some(Instant::now());
        self.unsent_edit = Some((page.root.join(&rel), Instant::now()));
        self.offer_checker(&rel);
        self.find_refresh(false);
        self.keep_caret_visible();
        self.request_redraw();
    }

    fn caret_cell(&self) -> Option<([f32; 2], [f32; 2])> {
        let page = self.pages.get(self.active)?;
        let editor = page.editor()?;
        let (line, col) = (editor.caret.line as u32, editor.display_col(editor.caret) as u32);
        let r = page.ready()?;
        let node = r.repo.file_index(&editor.rel_path).and_then(|fi| r.layout.file_node.get(fi).copied());
        let n = node.filter(|&n| n != canvas_core::layout::NONE).map(|n| &r.layout.nodes[n as usize])?;
        let cell = n.cell_rect(line, col, 1);
        let view = page.camera.view(self.viewport());
        Some((view.to_screen([cell.x, cell.y]), view.to_screen([cell.right(), cell.bottom()])))
    }

    fn update_completion(&mut self) {
        let before = self.completion.take();
        let Some(page) = self.pages.get(self.active) else { return };
        let Some(editor) = page.editor().filter(|e| e.selection().is_none()) else { return };
        let caret = editor.caret;
        self.completion = self.paths_offered().or_else(|| {
            let Completion { start, items: Offered::Code { serial, line, items }, selected } = before? else {
                return None;
            };
            let typed: String =
                editor.line(caret.line).chars().skip(start).take(caret.col.saturating_sub(start)).collect();
            if line != caret.line || caret.col < start || !typed.chars().all(word_char) {
                return None;
            }
            let chosen = items.get(selected).map(|(index, _)| *index);
            let items: Vec<_> = items.into_iter().filter(|(_, item)| typed_in_order(&typed, &item.label)).collect();
            let selected = items.iter().position(|(index, _)| Some(*index) == chosen).unwrap_or(0);
            (!items.is_empty()).then_some(Completion { start, items: Offered::Code { serial, line, items }, selected })
        });
    }

    fn paths_offered(&self) -> Option<Completion> {
        let page = self.pages.get(self.active).filter(|_| self.ui.prefs.complete_paths)?;
        let editor = page.editor()?;
        let caret = editor.caret;
        let typed = crate::paths::partial(editor.line(caret.line), caret.col)?;
        let folder = crate::paths::folder(&page.root, &editor.rel_path, &typed.dir)?;
        let mut items = crate::paths::candidates(&folder, &typed.prefix);
        let own = page.root.join(&editor.rel_path);
        items.retain(|c| folder.join(&c.name) != own);
        (!items.is_empty()).then_some(Completion { start: typed.start, items: Offered::Paths(items), selected: 0 })
    }

    fn ask_completion(&mut self) {
        if !self.ui.prefs.complete_code || matches!(self.completion, Some(Completion { items: Offered::Paths(_), .. }))
        {
            return;
        }
        let Some(page) = self.pages.get(self.active) else { return };
        let Some(editor) = page.editor().filter(|e| e.selection().is_none()) else { return };
        let caret = editor.caret;
        let Some(last) = editor.line(caret.line).chars().take(caret.col).last() else { return };
        if !word_char(last) && (last.is_whitespace() || ")]};,".contains(last)) {
            return;
        }
        let trigger = (!word_char(last)).then_some(last);
        let path = page.root.join(&editor.rel_path);
        let (text, col) = (editor.text(), editor.display_col(caret) as u32);
        self.ext.file_edited(&path, &text);
        self.ext.request_completion(&path, caret.line as u32, col, trigger);
    }

    fn offer_code(&mut self, found: ext::Suggestions) {
        if matches!(self.completion, Some(Completion { items: Offered::Paths(_), .. })) {
            return;
        }
        let Some(page) = self.pages.get(self.active) else { return };
        let Some(editor) = page.editor().filter(|e| e.selection().is_none()) else { return };
        let caret = editor.caret;
        let here = page.root.join(&editor.rel_path) == found.path
            && caret.line as u32 == found.line
            && editor.display_col(caret) as u32 == found.col;
        if !here {
            return;
        }
        let before: Vec<char> = editor.line(caret.line).chars().take(caret.col).collect();
        let start = caret.col - before.iter().rev().take_while(|c| word_char(**c)).count();
        let chosen = match &self.completion {
            Some(Completion { items: Offered::Code { items, .. }, selected, .. }) => {
                items.get(*selected).map(|(_, item)| item.label.clone())
            }
            _ => None,
        };
        let items: Vec<(usize, ext::Suggestion)> = found.items.into_iter().enumerate().collect();
        let selected = items.iter().position(|(_, item)| Some(&item.label) == chosen.as_ref()).unwrap_or(0);
        let offered = Offered::Code { serial: found.serial, line: caret.line, items };
        self.completion = (offered.len() > 0).then_some(Completion { start, items: offered, selected });
        self.request_redraw();
    }

    fn extra_edits(&mut self, path: &Path, edits: Vec<ext::ExtraEdit>) {
        let Some((i, rel)) = self.locate_file(path) else { return };
        let Some(editor) = self.pages[i].editors.get_mut(&rel) else { return };
        let mut edits = edits;
        edits.sort_by_key(|edit| (edit.line, edit.col));
        let mut changed = false;
        for edit in edits.into_iter().rev() {
            let from = editor.pos_at_display(edit.line as usize, edit.col as usize);
            let to = editor.pos_at_display(edit.end_line as usize, edit.end_col as usize);
            changed |= editor.edit_before(from, to, &edit.text);
        }
        if changed && i == self.active && self.pages[i].editing.as_deref() == Some(rel.as_str()) {
            self.after_edit();
        }
    }

    fn accept_completion(&mut self, index: usize) {
        let Some(offer) = self.completion.take() else { return };
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let root = page.root.clone();
        let Some(editor) = page.editor_mut() else { return };
        match &offer.items {
            Offered::Paths(items) => {
                let Some(candidate) = items.get(index) else { return };
                let text = crate::paths::completion(candidate, &editor.rel_path, editor.line(editor.caret.line));
                let more = candidate.is_dir;
                editor.complete(offer.start, &text);
                self.after_edit();
                if more {
                    self.update_completion();
                }
            }
            Offered::Code { serial, items, .. } => {
                let Some((at, item)) = items.get(index) else { return };
                let line = editor.caret.line;
                let from = editor.pos_at_display(line, item.col as usize);
                let indent: String = editor.line(line).chars().take_while(|c| c.is_whitespace()).collect();
                editor.complete(from.col, &item.text.replace('\n', &format!("\n{indent}")));
                if let Some(caret) = item.caret {
                    let place = |chars: usize| {
                        let before: String = item.text.chars().take(chars).collect();
                        match before.rsplit_once('\n') {
                            Some((above, last)) => crate::editor::Pos::new(
                                line + above.matches('\n').count() + 1,
                                indent.chars().count() + last.chars().count(),
                            ),
                            None => crate::editor::Pos::new(line, from.col + chars),
                        }
                    };
                    editor.move_to(place(caret), false);
                    if item.select > 0 {
                        editor.move_to(place(caret + item.select), true);
                    }
                }
                let path = root.join(&editor.rel_path);
                self.ext.request_completion_extra(&path, *serial, *at);
                self.after_edit();
            }
        }
    }

    fn wheel(&mut self, moved: [f32; 2], at: [f32; 2], zooming: bool, sideways: bool) {
        let (vp, notch) = (self.viewport(), WHEEL_NOTCH * self.scale());
        let [across, along] = moved;
        self.ui.card_menu = None;
        self.share_let_go();
        if let Some(page) = self.pages.get_mut(self.active) {
            page.target = None;
            match (zooming, sideways) {
                (true, _) => page.camera.zoom_at(at, vp, 1.2f32.powf(along / notch)),
                (false, true) => page.camera.pan_pixels(if along != 0.0 { along } else { across }, 0.0),
                (false, false) => page.camera.pan_pixels(across, along),
            }
        }
        self.session_dirty = true;
        self.update_hover();
        self.request_redraw();
    }

    fn keep_caret_visible(&mut self) {
        let area = self.canvas;
        let Some((tl, br)) = self.caret_cell() else { return };
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let margin = 24.0;
        let mut shift = [0.0f32, 0.0f32];
        if tl[0] < area.x + margin {
            shift[0] = tl[0] - (area.x + margin);
        } else if br[0] > area.right() - margin {
            shift[0] = br[0] - (area.right() - margin);
        }
        if tl[1] < area.y + margin {
            shift[1] = tl[1] - (area.y + margin);
        } else if br[1] > area.bottom() - margin {
            shift[1] = br[1] - (area.bottom() - margin);
        }
        if shift != [0.0, 0.0] {
            page.target = None;
            page.camera.center[0] += shift[0] / page.camera.zoom;
            page.camera.center[1] += shift[1] / page.camera.zoom;
        }
    }

    fn save_file(&mut self, index: usize, rel: &str, format: bool) -> bool {
        let Some(page) = self.pages.get_mut(index) else { return false };
        let path = page.root.join(rel);
        let Some(editor) = page.editors.get_mut(rel).filter(|editor| editor.dirty) else { return false };
        self.watcher.expect(&path);
        if let Err(e) = std::fs::write(&path, editor.text()) {
            log::error!("cannot save {}: {e}", path.display());
            self.ext.notice(ext::Level::Error, "Files", format!("{} could not be saved: {e}", path.display()));
            return false;
        }
        editor.mark_saved();
        self.changed_under.remove(&(page.id, rel.to_string()));
        self.merged_in.remove(&(page.id, rel.to_string()));
        log::info!("saved {}", path.display());
        if let (Some(folder), Some(session)) = (page.shared, self.sharing.session.as_mut()) {
            session.saved(folder, rel);
        }
        self.ext.file_changed(&path);
        if let Some(g) = self.gfx.as_mut() {
            g.renderer.text.drop_page(page.id);
            page.reload_file(rel, &g.renderer, &g.device, &self.ui.filters);
        }
        page.refresh_git();
        if format {
            self.ext.format_on_save(&path);
        }
        true
    }

    fn save_edit(&mut self) {
        let unsaved = match self.pages.get(self.active).and_then(|p| p.editing.clone()) {
            Some(rel) => vec![(self.active, rel)],
            None => self.unsaved(),
        };
        self.save_these(unsaved, true);
    }

    fn save_these(&mut self, files: Vec<(usize, String)>, format: bool) {
        let mut saved = false;
        let mut held: Option<(usize, String)> = None;
        for (index, rel) in files {
            let own = self.pages.get(index).filter(|page| page.shared.is_none()).map(|page| page.id);
            if own.is_some_and(|id| self.changed_under.contains(&(id, rel.clone()))) {
                held = held.or(Some((index, rel)));
                continue;
            }
            saved |= self.save_file(index, &rel, format);
        }
        if let Some((index, rel)) = held {
            self.ask_overwrite(index, &rel);
        }
        if saved {
            self.unsent_edit = None;
            self.symbols_stale = None;
        }
        self.request_redraw();
    }

    fn ask_overwrite(&mut self, index: usize, rel: &str) {
        let Some(page) = self.pages.get(index) else { return };
        let there = page.root.join(rel).is_file();
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let (title, text, theirs) = match there {
            true => (
                format!("{name} was changed by another program"),
                "It has changes you have not saved, and the file on disk is not the one you opened any more. Saving yours replaces what the other program wrote.",
                "Take theirs",
            ),
            false => (
                format!("{name} was deleted by another program"),
                "It has changes you have not saved. Saving yours writes the file again.",
                "Leave it deleted",
            ),
        };
        let about = ui::Asked::Overwrite { folder: page.root.clone(), path: rel.to_string() };
        let mut ask = ui::Ask::new(about, &title, text.to_string())
            .choice("Save mine", ui::ChoiceLook::Danger, ui::Answer::Yes)
            .choice(theirs, ui::ChoiceLook::Plain, ui::Answer::Other)
            .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
        ask.items = vec![rel.to_string()];
        self.ui.ask = Some(ask);
        self.request_redraw();
    }

    fn settle_changed_under(&mut self) {
        let mut again = Vec::new();
        self.changed_under.retain(|(id, rel)| {
            let Some(page) = self.pages.iter().find(|page| page.id == *id) else { return false };
            match page.editors.get(rel) {
                Some(editor) if editor.dirty => true,
                Some(_) => {
                    again.push(page.root.join(rel));
                    false
                }
                None => false,
            }
        });
        if !again.is_empty() {
            self.on_disk_changes(crate::watch::Batch { files: again, git: Vec::new() });
        }
    }

    fn drop_edits(&mut self, index: usize, only: Option<&str>) {
        let Some(page) = self.pages.get_mut(index) else { return };
        let reverted = |rel: &String| {
            let change = page.git.as_ref().and_then(|git| git.change(rel));
            only.is_none_or(|one| one == rel) && change.is_none_or(|c| c.change != crate::git::Change::Untracked)
        };
        let dropped: Vec<String> = page.unsaved().map(|e| e.rel_path.clone()).filter(reverted).collect();
        for rel in &dropped {
            page.editors.remove(rel);
            if page.editing.as_deref() == Some(rel.as_str()) {
                page.editing = None;
            }
            if let Some(g) = self.gfx.as_mut() {
                g.renderer.text.drop_page(page.id);
                page.reload_file(rel, &g.renderer, &g.device, &self.ui.filters);
            }
        }
        if !dropped.is_empty() {
            page.preview_unsaved();
            self.unsent_edit = None;
        }
    }

    fn unsaved(&self) -> Vec<(usize, String)> {
        let dirty = |i: usize, p: &Page| {
            let own = p.editors.values().filter(|e| e.dirty && p.shared.is_none());
            own.map(|e| (i, e.rel_path.clone())).collect::<Vec<_>>()
        };
        self.pages.iter().enumerate().flat_map(|(i, p)| dirty(i, p)).collect()
    }

    fn confirm_exit(&mut self) -> bool {
        let unsaved = self.unsaved();
        if unsaved.is_empty() || self.exit_confirmed {
            return true;
        }
        if self.ui.ask.as_ref().is_none_or(|ask| ask.about != ui::Asked::Exit) {
            let (title, text) = match unsaved.len() {
                1 => ("Save the changes before closing?".to_string(), "One file has changes that are not saved."),
                n => {
                    (format!("Save the changes to {n} files before closing?"), "They have changes that are not saved.")
                }
            };
            let mut ask = ui::Ask::new(ui::Asked::Exit, &title, format!("{text} Without saving they are lost."))
                .choice("Save and close", ui::ChoiceLook::Main, ui::Answer::Yes)
                .choice("Close without saving", ui::ChoiceLook::Plain, ui::Answer::Other)
                .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
            ask.items = unsaved.into_iter().map(|(_, rel)| rel).collect();
            ask.danger = false;
            self.ui.ask = Some(ask);
            self.ui.settings.open = false;
            self.request_redraw();
        }
        false
    }

    fn answered(&mut self, about: ui::Asked, answer: ui::Answer) {
        match (about, answer) {
            (ui::Asked::Delete { folder: root, path }, ui::Answer::Yes) => {
                let done = crate::files::inside(&root, &path).and_then(|p| crate::files::recycle(&p));
                match done {
                    Ok(()) => {
                        if let Some(page) = self.pages.iter_mut().find(|page| page.root == root) {
                            let inside = format!("{path}/");
                            let gone = |rel: &str| rel == path || rel.starts_with(&inside);
                            page.editors.retain(|rel, _| !gone(rel));
                            if page.editing.as_deref().is_some_and(gone) {
                                page.editing = None;
                            }
                            page.preview_unsaved();
                        }
                    }
                    Err(why) => self.ext.notice(ext::Level::Error, "files", why),
                }
            }
            (ui::Asked::Overwrite { folder, path }, ui::Answer::Yes | ui::Answer::Other) => {
                let Some(index) = self.pages.iter().position(|page| page.root == folder) else { return };
                self.changed_under.remove(&(self.pages[index].id, path.clone()));
                if answer == ui::Answer::Yes {
                    self.save_these(vec![(index, path)], true);
                } else if let Some(editor) = self.pages[index].editors.get_mut(&path) {
                    editor.mark_saved();
                    let file = self.pages[index].root.join(&path);
                    self.on_disk_changes(crate::watch::Batch { files: vec![file], git: Vec::new() });
                }
            }
            (ui::Asked::Git { folder, op }, ui::Answer::Yes) => {
                match self.pages.iter().position(|page| page.root == folder) {
                    Some(index) => self.run_git(index, op),
                    None => self.ext.notice(ext::Level::Warn, "Source Control", "That folder is not open any more."),
                }
            }
            (ui::Asked::Close { folder, forget }, ui::Answer::Yes | ui::Answer::Other) => {
                let Some(i) = self.pages.iter().position(|page| page.root == folder) else { return };
                if answer == ui::Answer::Yes {
                    let files: Vec<(usize, String)> =
                        self.pages[i].editors.values().filter(|e| e.dirty).map(|e| (i, e.rel_path.clone())).collect();
                    self.save_these(files, false);
                    if self.pages[i].editors.values().any(|e| e.dirty) {
                        return;
                    }
                }
                self.close_page(i);
                if forget {
                    self.workspace.retain(|known| known.path != folder);
                }
                self.save_session();
            }
            (ui::Asked::Share { join }, ui::Answer::Yes) => {
                self.share_op(if join { ui::ShareOp::Join } else { ui::ShareOp::Start });
                self.ui.share.open = true;
            }
            (ui::Asked::Exit, ui::Answer::Yes) => {
                let unsaved = self.unsaved();
                self.save_these(unsaved, false);
                self.exit_requested = self.unsaved().is_empty();
                if !self.exit_requested {
                    self.restart_as = None;
                }
            }
            (ui::Asked::Exit, ui::Answer::Other) => {
                self.exit_confirmed = true;
                self.exit_requested = true;
            }
            (ui::Asked::Exit, ui::Answer::Cancel) => self.restart_as = None,
            _ => {}
        }
        self.request_redraw();
    }

    fn run_git(&mut self, active: usize, op: crate::git::GitOp) {
        use crate::git::GitOp;
        if self.pages.get(active).is_none_or(|page| page.git_busy()) {
            let text = "Git is still at something else in this folder: try again when it is done.";
            return self.ext.notice(ext::Level::Warn, "Source Control", text);
        }
        let unsaved: Vec<(usize, String)> = self.unsaved().into_iter().filter(|(i, _)| *i == active).collect();
        match &op {
            GitOp::Stage(path) => self.save_these(unsaved.into_iter().filter(|(_, rel)| rel == path).collect(), false),
            GitOp::StageAll | GitOp::Stash | GitOp::Commit { all: true, .. } => self.save_these(unsaved, false),
            GitOp::Discard(path) => self.drop_edits(active, Some(path)),
            GitOp::DiscardAll => self.drop_edits(active, None),
            _ => {}
        }
        if let Some(page) = self.pages.get_mut(self.active) {
            page.run_git(op);
        }
        self.request_redraw();
    }

    fn edit_key(&mut self, event: &KeyEvent) -> bool {
        let cmd = (self.mods.control_key() && !self.mods.alt_key()) || self.mods.super_key();
        let shift = self.mods.shift_key();
        if self.pages.get(self.active).and_then(|page| page.editor()).is_none() {
            self.completion = None;
        }
        if let Some(offer) = self.completion.as_mut() {
            let count = offer.items.len();
            let code = matches!(offer.items, Offered::Code { .. });
            let taken = match &event.logical_key {
                Key::Named(NamedKey::ArrowDown) => {
                    offer.selected = (offer.selected + 1) % count;
                    true
                }
                Key::Named(NamedKey::ArrowUp) => {
                    offer.selected = (offer.selected + count - 1) % count;
                    true
                }
                Key::Named(NamedKey::Enter | NamedKey::Tab) if !cmd => {
                    let index = offer.selected;
                    self.accept_completion(index);
                    return true;
                }
                Key::Named(NamedKey::Escape) => {
                    self.completion = None;
                    true
                }
                Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::Home | NamedKey::End) if code => {
                    self.completion = None;
                    false
                }
                _ => false,
            };
            if taken {
                self.request_redraw();
                return true;
            }
        }
        let Some(page) = self.pages.get_mut(self.active) else { return false };
        let Some(editor) = page.editor_mut() else { return false };
        let mut changed = false;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                page.editing = None;
                self.completion = None;
                self.request_redraw();
                return true;
            }
            Key::Named(NamedKey::Enter) => {
                editor.newline();
                changed = true;
            }
            Key::Named(NamedKey::Backspace) => {
                editor.backspace();
                changed = true;
            }
            Key::Named(NamedKey::Delete) => {
                editor.delete();
                changed = true;
            }
            Key::Named(NamedKey::Tab) if !cmd => {
                editor.tab();
                changed = true;
            }
            Key::Named(NamedKey::ArrowLeft) if cmd => editor.word(false, shift),
            Key::Named(NamedKey::ArrowRight) if cmd => editor.word(true, shift),
            Key::Named(NamedKey::ArrowLeft) => editor.left(shift),
            Key::Named(NamedKey::ArrowRight) => editor.right(shift),
            Key::Named(NamedKey::ArrowUp) => editor.vertical(-1, shift),
            Key::Named(NamedKey::ArrowDown) => editor.vertical(1, shift),
            Key::Named(NamedKey::PageUp) => editor.vertical(-20, shift),
            Key::Named(NamedKey::PageDown) => editor.vertical(20, shift),
            Key::Named(NamedKey::Home) if cmd => editor.move_to(crate::editor::Pos::default(), shift),
            Key::Named(NamedKey::End) if cmd => editor.move_to(crate::editor::Pos::new(usize::MAX, usize::MAX), shift),
            Key::Named(NamedKey::Home) => editor.home(shift),
            Key::Named(NamedKey::End) => editor.end(shift),
            Key::Character(c) if cmd => match c.to_lowercase().as_str() {
                "s" => {
                    self.save_edit();
                    return true;
                }
                "z" if shift => changed = editor.redo(),
                "z" => changed = editor.undo(),
                "y" => changed = editor.redo(),
                "a" => editor.select_all(),
                "c" | "x" => {
                    let text = editor.selected_text();
                    if !text.is_empty() {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(text);
                        }
                        if c.eq_ignore_ascii_case("x") {
                            editor.backspace();
                            changed = true;
                        }
                    }
                }
                "v" => {
                    if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        editor.insert(&text);
                        changed = true;
                    }
                }
                _ => return false,
            },
            _ => match event.text.as_deref() {
                Some(text) if !cmd && !text.chars().any(char::is_control) => {
                    editor.insert(text);
                    changed = true;
                }
                _ => return false,
            },
        }
        if changed {
            self.after_edit();
        } else {
            self.keep_caret_visible();
            self.request_redraw();
        }
        self.update_completion();
        if changed {
            self.ask_completion();
        }
        true
    }

    fn open_media(&mut self, path: &Path) -> bool {
        let Some(kind) = crate::media::kind(path).filter(|_| path.is_file()) else { return false };
        let played = canvas_core::scan::media_kind(&path.to_string_lossy()).is_some_and(|what| what != "picture");
        if played && self.playing.as_deref() != Some(path) {
            self.webviews.allow_file(path);
            self.webviews.set_html(crate::media::PLAYER, crate::media::page(path, kind));
            self.playing = Some(path.to_path_buf());
        }
        self.request_redraw();
        true
    }

    fn stop_playing(&mut self) {
        if self.playing.take().is_some() {
            self.webviews.close(crate::media::PLAYER);
            self.player_shown = false;
            self.request_redraw();
        }
    }

    fn card_on_screen(&self, page: &Page, rel: &str) -> Option<Rect> {
        let r = page.ready()?;
        let node = r.layout.file_node.get(r.repo.file_index(rel)?).filter(|&&n| n != canvas_core::layout::NONE)?;
        let inside = r.layout.nodes[*node as usize].rect.expand(-canvas_core::layout::CARD_PAD);
        let view = page.camera.view(self.viewport());
        let (tl, br) = (view.to_screen([inside.x, inside.y]), view.to_screen([inside.right(), inside.bottom()]));
        Some(Rect::new(tl[0], tl[1], br[0] - tl[0], br[1] - tl[1]))
    }

    fn player_place(&self) -> Option<Rect> {
        let (i, rel) = self.locate_file(self.playing.as_deref()?)?;
        let card = self.card_on_screen(self.pages.get(i).filter(|_| self.shown().contains(&i))?, &rel)?;
        let area = self.canvas;
        let (left, top) = (card.x.max(area.x), card.y.max(area.y));
        let (right, bottom) = (card.right().min(area.right()), card.bottom().min(area.bottom()));
        let seen = Rect::new(left.round(), top.round(), (right - left).round(), (bottom - top).round());
        (card.w >= 200.0 && card.h >= 120.0 && seen.w >= 120.0 && seen.h >= 80.0).then_some(seen)
    }

    fn place_pictures(&mut self) {
        const MIN_SIDE: f32 = 24.0;
        let (vp, s) = (self.viewport(), self.scale());
        let area = self.canvas;
        let mut wanted: Vec<(PathBuf, Rect)> = Vec::new();
        for i in self.shown() {
            let Some(page) = self.pages.get(i) else { continue };
            let Some(r) = page.ready() else { continue };
            let view = page.camera.view(vp);
            let z = view.zoom;
            let mut world = view.world_rect();
            (world.x, world.y, world.w, world.h) = (world.x + area.x / z, world.y + area.y / z, area.w / z, area.h / z);
            r.layout.visit_visible(&world, |_, n| {
                let NodeKind::File(fi) = n.kind else { return true };
                let f = &r.repo.files[fi];
                if f.note.as_deref() == Some("picture") && n.rect.h * z >= MIN_SIDE {
                    wanted.extend(self.card_on_screen(page, &f.rel_path).map(|at| (page.root.join(&f.rel_path), at)));
                }
                wanted.len() < 240
            });
        }
        self.ui.pictures.clear();
        for (path, card) in wanted {
            let Some((picture, size)) = self.pictures.get(&path) else { continue };
            let scale = (card.w / size[0] as f32).min(card.h / size[1] as f32);
            let (w, h) = (size[0] as f32 * scale, size[1] as f32 * scale);
            let (x, y) = (card.x + (card.w - w) * 0.5, card.y + (card.h - h) * 0.5);
            let at = egui::Rect::from_min_size(egui::pos2(x / s, y / s), egui::vec2(w / s, h / s));
            self.ui.pictures.push((picture, at));
        }
    }

    fn markdown_text(&self, path: &Path) -> Option<String> {
        let edited = self.locate_file(path).and_then(|(i, rel)| self.pages[i].editors.get(&rel).map(|e| e.text()));
        edited.or_else(|| std::fs::read_to_string(path).ok())
    }

    fn open_preview(&mut self, path: &Path) -> bool {
        if !crate::markdown::is_markdown(path) || !path.is_file() {
            return false;
        }
        let Some(text) = self.markdown_text(path) else { return false };
        self.webviews.allow_file(path);
        self.webviews.set_html(crate::media::HANDLE, crate::markdown::page(&text, path));
        self.preview = Some(text);
        self.ui.media = Some(path.to_path_buf());
        self.ui.show(Panel::Media);
        self.request_redraw();
        true
    }

    fn toggle_preview(&mut self, path: &Path) {
        if self.ui.media.as_deref() == Some(path) && self.ui.is_open(&Panel::Media) {
            self.ui.close_panel(&Panel::Media);
            self.request_redraw();
        } else {
            self.open_preview(path);
        }
    }

    fn refresh_preview(&mut self) {
        let Some(path) = self.ui.media.clone().filter(|_| self.preview.is_some()) else { return };
        let Some(text) = self.markdown_text(&path) else { return };
        if self.preview.as_deref() != Some(text.as_str()) {
            self.webviews.set_html(crate::media::HANDLE, crate::markdown::page(&text, &path));
            self.preview = Some(text);
            self.request_redraw();
        }
    }

    fn follow_link(&mut self, url: &str) {
        match crate::media::file_of(url) {
            Some(file) => {
                if !self.open_preview(&file) {
                    self.reveal_file(&file);
                }
            }
            None => {
                if let Err(why) = crate::files::open_web_link(url) {
                    self.ext.notice(ext::Level::Warn, "markdown", format!("The link was not opened: {why}."));
                }
            }
        }
    }

    fn reveal_file(&mut self, path: &Path) {
        self.open_media(path);
        self.show_path(path, None);
    }

    fn current_file(&self) -> Option<PathBuf> {
        let page = self.pages.get(self.active)?;
        let r = page.ready()?;
        [page.hover, page.selected].into_iter().flatten().find_map(|i| match r.layout.nodes.get(i)?.kind {
            NodeKind::File(fi) => Some(page.root.join(&r.repo.files[fi].rel_path)),
            NodeKind::Dir => None,
        })
    }

    fn format_file(&mut self, rel_path: Option<String>) {
        let path = match (rel_path, self.pages.get(self.active)) {
            (Some(rel), Some(page)) => Some(page.root.join(rel)),
            _ => self.current_file(),
        };
        match path {
            Some(path) => self.ext.format(&path),
            None => self.ext.notice(
                ext::Level::Info,
                "Format",
                "Select a file first: click its card, or its name in the explorer.".to_string(),
            ),
        }
        self.request_redraw();
    }

    fn sync_documents(&mut self) {
        if self.drag.is_some() {
            return;
        }
        let vp = self.viewport();
        let Some(page) = self.pages.get(self.active) else { return };
        let Some(r) = page.ready() else { return };
        if page.target.is_some() || Lod::for_zoom(page.camera.zoom) != Lod::Text {
            return;
        }
        const MAX_VISIBLE: usize = 16;
        let (z, area) = (page.camera.zoom, self.canvas);
        let mut world = page.camera.view(vp).world_rect();
        world.x += area.x / z;
        world.y += area.y / z;
        world.w = area.w / z;
        world.h = area.h / z;
        let mut visible = Vec::new();
        r.layout.visit_visible(&world, |_, n| {
            if let NodeKind::File(fi) = n.kind {
                if r.repo.files[fi].note.is_none() {
                    visible.push(page.root.join(&r.repo.files[fi].rel_path));
                }
            }
            visible.len() < MAX_VISIBLE
        });
        let focus = page.hover_cell.map(|(fi, _, _)| page.root.join(&r.repo.files[fi as usize].rel_path));
        self.ext.show_documents(&visible, focus.as_deref());
    }

    fn place_now(&self) -> Option<Place> {
        let page = self.pages.get(self.active)?;
        let r = page.ready()?;
        let camera = page.target.unwrap_or(page.camera);
        let middle = camera.view(self.viewport()).to_world(self.canvas.center());
        let node = &r.layout.nodes[r.layout.hit(middle).unwrap_or(0)];
        let selected = page.selected.and_then(|n| r.layout.nodes.get(n)).map(|n| n.rel_path.clone());
        Some(Place {
            page: page.id,
            anchor: node.rel_path.clone(),
            offset: [camera.center[0] - node.rect.x, camera.center[1] - node.rect.y],
            zoom: camera.zoom,
            selected,
        })
    }

    fn remember_place(&mut self) {
        let Some(now) = self.place_now() else { return };
        let by_itself = self.moved_by_itself;
        let same = self.back.last().is_some_and(|last| {
            let near = (last.offset[0] - now.offset[0]).abs() + (last.offset[1] - now.offset[1]).abs() < 4.0 / now.zoom;
            last.page == now.page && last.anchor == now.anchor && near && (last.zoom / now.zoom - 1.0).abs() < 0.02
        });
        if same || (by_itself && self.last_move_by_itself) {
            return;
        }
        self.last_move_by_itself = by_itself;
        self.back.push(now);
        self.forward.clear();
        if self.back.len() > PLACES_KEPT {
            self.back.remove(0);
        }
    }

    fn travel(&mut self, back: bool) {
        let gone = if back { self.back.pop() } else { self.forward.pop() };
        let (Some(place), Some(here)) = (gone, self.place_now()) else { return };
        if back {
            self.forward.push(here);
        } else {
            self.back.push(here);
        }
        self.last_move_by_itself = false;
        if !self.pages.iter().any(|p| p.id == place.page) {
            return self.travel(back);
        }
        self.go_place(&place);
    }

    fn go_place(&mut self, place: &Place) {
        let Some(index) = self.pages.iter().position(|p| p.id == place.page) else { return };
        self.switch_to(index);
        let page = &mut self.pages[index];
        let Some(r) = page.ready() else { return };
        let at = |path: &str| r.layout.node_for_path(&r.repo, path);
        let node = &r.layout.nodes[at(&place.anchor).unwrap_or(0)];
        let center = [node.rect.x + place.offset[0], node.rect.y + place.offset[1]];
        let selected = place.selected.as_deref().and_then(at);
        page.target = Some(Camera { center, zoom: place.zoom });
        page.selected = selected;
        self.request_redraw();
    }

    fn frame_node(&mut self, node: usize) {
        self.remember_place();
        let (area, vp, s) = (self.canvas, self.viewport(), self.scale());
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let Some(n) = r.layout.nodes.get(node) else { return };
        let target = if n.is_dir() {
            Camera::fit(n.rect, area, vp, 24.0 * s)
        } else {
            let z = (area.w * 0.7 / n.rect.w).min(2.0 * s).max(Camera::fit(n.rect, area, vp, 24.0 * s).zoom);
            let ac = area.center();
            Camera {
                center: [
                    n.rect.center()[0] - (ac[0] - vp[0] * 0.5) / z,
                    n.rect.y - FILE_LABEL_H - (area.y + 24.0 * s - vp[1] * 0.5) / z,
                ],
                zoom: z,
            }
        };
        page.target = Some(target);
        self.request_redraw();
    }

    fn frame_site(&mut self, site: Site) {
        self.remember_place();
        let (area, vp, s) = (self.canvas, self.viewport(), self.scale());
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let Some((p, exact)) = r.layout.anchor(&site) else { return };
        let z = if exact { page.camera.zoom.max(1.25 * s) } else { page.camera.zoom };
        let (sx, sy) = (area.x + area.w * 0.3, area.y + area.h * 0.35);
        page.target = Some(Camera { center: [p[0] - (sx - vp[0] * 0.5) / z, p[1] - (sy - vp[1] * 0.5) / z], zoom: z });
        self.request_redraw();
    }

    fn frame_all(&mut self) {
        self.frame_node(0);
    }

    fn frame_every_folder(&mut self) {
        if !self.shares_canvas() {
            return self.frame_all();
        }
        self.remember_place();
        let origins = self.origins();
        let own = origins[self.active.min(origins.len() - 1)];
        let everything = self
            .pages
            .iter()
            .zip(&origins)
            .filter_map(|(page, origin)| {
                let extent = page.extent?;
                Some(Rect::new(extent.x + origin[0] - own[0], extent.y + origin[1] - own[1], extent.w, extent.h))
            })
            .reduce(|all, extent| all.union(&extent));
        let (area, vp, s) = (self.canvas, self.viewport(), self.scale());
        if let (Some(everything), Some(page)) = (everything, self.pages.get_mut(self.active)) {
            page.target = Some(Camera::fit(everything, area, vp, 24.0 * s));
            self.request_redraw();
        }
    }

    fn frame_selection(&mut self) {
        self.remember_place();
        let (area, vp, s) = (self.canvas, self.viewport(), self.scale());
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let mut bounds: Option<Rect> = None;
        let filters = &self.ui.filters;
        for sel in &page.selections {
            for rel in sel.rel.items.iter().filter(|x| filters.enabled[x.kind.index()]) {
                for site in [rel.from, rel.to] {
                    if let Some((p, _)) = r.layout.anchor(&site) {
                        let pt = Rect::new(p[0], p[1], 240.0, LINE_H);
                        bounds = Some(bounds.map(|b| b.union(&pt)).unwrap_or(pt));
                    }
                }
            }
        }
        match bounds {
            Some(b) => {
                let mut cam = Camera::fit(b.expand(60.0), area, vp, 90.0 * s);
                cam.zoom = cam.zoom.min(1.6 * s);
                let (c, ac) = (b.center(), area.center());
                cam.center = [c[0] - (ac[0] - vp[0] * 0.5) / cam.zoom, c[1] - (ac[1] - vp[1] * 0.5) / cam.zoom];
                page.target = Some(cam);
                self.request_redraw();
            }
            None => {
                if let Some(n) = page.selected {
                    self.frame_node(n);
                }
            }
        }
    }

    fn zoom_center(&mut self, factor: f32) {
        let (area, vp) = (self.canvas, self.viewport());
        if let Some(p) = self.pages.get_mut(self.active) {
            let mut cam = p.target.unwrap_or(p.camera);
            cam.zoom_at(area.center(), vp, factor);
            p.target = Some(cam);
        }
        self.request_redraw();
    }

    fn pan(&mut self, dx: f32, dy: f32) {
        if let Some(p) = self.pages.get_mut(self.active) {
            p.target = None;
            p.camera.pan_pixels(dx, dy);
        }
        self.update_hover();
        self.request_redraw();
    }

    fn update_hover(&mut self) {
        let vp = self.viewport();
        let cursor = self.cursor;
        let on_canvas = !self.over_ui(cursor) && self.drag.as_ref().is_none_or(|d| !d.moved);
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let (mut hover, mut cell) = (None, None);
        if on_canvas {
            if let PageState::Ready(r) = &page.state {
                let world = page.camera.view(vp).to_world(cursor);
                hover = r.layout.hit(world);
                if let Some(n) = hover.map(|h| &r.layout.nodes[h]) {
                    if let NodeKind::File(fi) = n.kind {
                        if page.camera.zoom * LINE_H >= SYMBOL_HOVER_MIN_PX {
                            cell = n
                                .cell_at(world, r.repo.files[fi].line_count())
                                .map(|(line, col)| (fi as u32, line, col));
                        }
                    }
                }
            }
        }
        if hover != page.hover || cell != page.hover_cell {
            page.hover = hover;
            page.hover_cell = cell;
            self.request_redraw();
        }
    }

    fn select_symbol(&mut self, sym: SymRef, clicked: Option<Site>, toggle: bool) {
        let Some(page) = self.pages.get_mut(self.active) else { return };
        if !toggle && page.selections.iter().any(|s| s.sym == sym) {
            return;
        }
        page.toggle_symbol(sym, clicked);
        if page.only_connected {
            self.relayout();
        }
        self.ui.show(Panel::Symbols);
        self.request_redraw();
    }

    fn exact_uses(&mut self, page: u32, of: (u32, u32), places: Vec<(PathBuf, u32, u32)>) {
        let Some(index) = self.pages.iter().position(|p| p.id == page) else { return };
        let mut root = ext::path_key(&self.pages[index].root.to_string_lossy());
        root.push('/');
        let inside = |path: &PathBuf| {
            let text = path.to_string_lossy().replace('\\', "/");
            let key = ext::path_key(&text);
            let rel = text.get(root.len()..).filter(|_| key.starts_with(&root) && key.len() == text.len())?;
            Some(rel.to_string())
        };
        let places: Vec<(String, u32, u32)> =
            places.iter().filter_map(|(path, line, col)| Some((inside(path)?, *line, *col))).collect();
        if self.pages[index].exact_uses(of, &places) {
            if index == self.active && self.pages[index].only_connected {
                self.relayout();
            }
            self.request_redraw();
        }
    }

    fn clear_symbols(&mut self) {
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let had = !page.selections.is_empty();
        page.selections.clear();
        page.curves_dirty = true;
        if page.only_connected {
            page.only_connected = false;
            self.relayout();
        } else if had {
            self.request_redraw();
        }
    }

    fn on_canvas_click(&mut self, pos: [f32; 2]) {
        self.completion = None;
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, p)| {
            now.duration_since(t) < Duration::from_millis(400)
                && (p[0] - pos[0]).abs() < 6.0
                && (p[1] - pos[1]).abs() < 6.0
        });
        self.last_click = if double { None } else { Some((now, pos)) };

        if let Some(&(_, sym)) = self.chip_hits.iter().find(|(r, _)| r.contains(pos)) {
            self.select_symbol(sym, None, false);
            return;
        }
        if let Some(path) = self.preview_hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, p)| p.clone()) {
            self.apply(Action::Preview(path));
            return;
        }
        if let Some(path) = self.toggle_hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, p)| p.clone()) {
            self.apply(Action::ToggleCollapsed(path));
            return;
        }

        let vp = self.viewport();
        let (ctrl, shift) = (self.mods.control_key() || self.mods.super_key(), self.mods.shift_key());
        let can_edit = self.ui.prefs.edit;
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let world = page.camera.view(vp).to_world(pos);
        let hit = r.layout.hit(world);
        let zoom = page.camera.zoom;
        let readable = Lod::for_zoom(zoom) == Lod::Text;

        let mut caret_at = None;
        let mut picked = None;
        if let Some(n) = hit.map(|h| &r.layout.nodes[h]) {
            if let NodeKind::File(fi) = n.kind {
                let f = &r.repo.files[fi];
                if let Some((line, col)) = n.cell_at(world, f.line_count()) {
                    if readable && !ctrl && can_edit && f.note.is_none() && page.showing.is_none() {
                        let half = n.cell_rect(0, 0, 1).w * 0.5;
                        let near = n.cell_at([world[0] + half, world[1]], f.line_count());
                        let col = near.filter(|_| !double).map_or(col, |(_, col)| col);
                        caret_at = Some((f.rel_path.clone(), line as usize, col as usize));
                    } else if zoom * LINE_H >= SYMBOL_HOVER_MIN_PX {
                        let fi = fi as u32;
                        let named = readable.then(|| r.repo.resolve_at(fi, line, col)).flatten();
                        picked = match named {
                            Some((sym, site)) => Some((sym, (site != r.repo.sym_site(sym)).then_some(site))),
                            None => r.repo.sym_at(fi, line).map(|sym| (sym, None)),
                        };
                    }
                }
            }
        }

        let media = hit.and_then(|h| match r.layout.nodes[h].kind {
            NodeKind::File(fi) => Some(page.root.join(&r.repo.files[fi].rel_path)),
            NodeKind::Dir => None,
        });
        let media = media.filter(|p| !double && crate::media::kind(p).is_some() && caret_at.is_none());

        page.selected = hit;
        if let Some((rel, line, col)) = caret_at {
            let line = page.rows(&rel).and_then(|rows| rows.get(line)).map_or(line, |row| row.line as usize);
            if page.begin_edit(&rel) {
                if let Some(editor) = page.editor_mut() {
                    let at = editor.pos_at_display(line, col);
                    if double {
                        editor.select_word(at);
                    } else {
                        editor.move_to(at, shift);
                    }
                }
                self.request_redraw();
                return;
            }
        }
        if !ctrl {
            page.editing = None;
        }
        match media {
            Some(file) => {
                self.open_media(&file);
            }
            None => self.stop_playing(),
        }
        let Some(node) = hit else {
            self.request_redraw();
            return;
        };
        if double {
            self.frame_node(node);
            return;
        }
        match picked {
            Some((sym, clicked)) => self.select_symbol(sym, clicked, true),
            None => self.request_redraw(),
        }
    }

    fn on_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if self.ui.ask.is_some() {
            return self.request_redraw();
        }
        let cmd = self.mods.control_key() || self.mods.super_key();
        let (shift, alt) = (self.mods.shift_key(), self.mods.alt_key());
        let typing = self.gfx.as_ref().is_some_and(|g| g.egui.egui_wants_keyboard_input());
        let chord = key_name(&event.logical_key).map(|key| Chord::new(&key, cmd, shift, alt));

        if !self.ui.settings.open {
            self.ui.rebinding = None;
        }
        if let Some(command) = self.ui.rebinding {
            match chord {
                Some(chord) if chord.key == "escape" && !cmd && !alt => self.ui.rebinding = None,
                Some(chord) => {
                    if self.ui.key_file.bind(command, chord) {
                        self.ui.rebinding = None;
                        self.keys_changed();
                    }
                }
                None => {}
            }
            self.request_redraw();
            return;
        }

        let bound = chord.as_ref().and_then(|c| self.ui.keys.command(c)).map(Command::info);
        if let Some(info) = bound.filter(|i| i.scope == Scope::Anywhere) {
            self.run_command(info.command);
            return;
        }
        if self.ui.settings.open && self.ui.quick.is_none() {
            let leaves = chord.as_ref().is_some_and(|c| c.key == "escape" && !cmd && !alt);
            if leaves || bound.is_some_and(|i| i.command == Command::Settings) {
                self.ui.settings.open = false;
                self.request_redraw();
            }
            return;
        }
        if typing || self.ui.quick.is_some() {
            return;
        }
        let before_editor = bound.is_some() && alt && !cmd;
        if !before_editor && self.edit_key(event) {
            return;
        }
        if let Some(info) = bound {
            self.run_command(info.command);
            return;
        }
        let Some(chord) = chord else { return };
        if chord.key == "escape" {
            if self.sharing.following.is_some() || self.agents.following.is_some() {
                self.share_let_go();
                self.request_redraw();
            } else if self.playing.is_some() {
                self.stop_playing();
            } else if self.pages.get(self.active).is_some_and(|p| !p.selections.is_empty()) {
                self.clear_symbols();
            } else if let Some(p) = self.pages.get_mut(self.active) {
                p.selected = None;
                self.request_redraw();
            }
        }
    }

    fn run_command(&mut self, command: Command) {
        match command {
            Command::Palette => {
                self.ui.settings.open = false;
                self.ui.quick = Some(QuickOpen { query: ">".into(), selected: 0, focus: true })
            }
            Command::GoToFile => {
                self.ui.settings.open = false;
                self.ui.quick = Some(QuickOpen { query: String::new(), selected: 0, focus: true })
            }
            Command::SearchInFiles => self.ui.show(Panel::Search),
            Command::Find => self.find(ui::FindOp::Open),
            Command::Back => self.travel(true),
            Command::Forward => self.travel(false),
            Command::Settings => self.ui.settings.show(None),
            Command::ToggleLeftPanel => self.ui.toggle_side(Side::Left),
            Command::ToggleSideBySide => self.apply(Action::ToggleSideBySide),
            Command::ToggleUi => self.apply(Action::ToggleUi),
            Command::FormatFile => self.format_file(None),
            Command::Save => self.save_edit(),
            Command::MoveTool | Command::EditTool => {
                let edit = command == Command::EditTool;
                self.apply(Action::SetPrefs(crate::prefs::Prefs { edit, ..self.ui.prefs.clone() }));
            }
            Command::GoToDefinition => self.go_to_definition(),
            Command::OpenFolder => self.pick_folder(),
            Command::CloseFolder => self.close_folder(self.active, false),
            Command::NextFolder => self.cycle(1),
            Command::PreviousFolder => self.cycle(-1),
            Command::Rescan => self.rescan(),
            Command::ShowAll => self.frame_every_folder(),
            Command::GoToSelection => self.frame_selection(),
            Command::ZoomToParent => self.zoom_to_parent(),
            Command::ZoomIn => self.zoom_center(1.25),
            Command::ZoomOut => self.zoom_center(0.8),
            Command::PanLeft => self.pan(160.0, 0.0),
            Command::PanRight => self.pan(-160.0, 0.0),
            Command::PanUp => self.pan(0.0, 160.0),
            Command::PanDown => self.pan(0.0, -160.0),
        }
        self.request_redraw();
    }

    fn page_chord(&mut self, k: &crate::webview::PageKey) {
        let chord = Chord::new(&k.key, k.ctrl, k.shift, k.alt);
        match self.ui.keys.command(&chord).map(Command::info) {
            Some(info) if info.scope != Scope::Canvas => {
                if matches!(info.command, Command::Palette | Command::GoToFile | Command::SearchInFiles) {
                    self.webviews.focus_app();
                }
                self.run_command(info.command);
            }
            Some(_) => {}
            None => {}
        }
    }

    fn keys_changed(&mut self) {
        self.ui.keys = Keymap::new(&self.ui.key_file);
        crate::keys::save(&self.ui.key_file);
        self.request_redraw();
    }

    fn zoom_to_parent(&mut self) {
        let Some(page) = self.pages.get(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let node = page.selected.or(page.hover).unwrap_or(0);
        let parent = r.layout.nodes.get(node).and_then(|n| n.parent).map(|p| p as usize).unwrap_or(0);
        if let Some(p) = self.pages.get_mut(self.active) {
            p.selected = Some(parent);
        }
        self.frame_node(parent);
    }

    fn apply(&mut self, action: Action) {
        if !matches!(action, Action::HoverPath(_) | Action::DragWindow | Action::Minimize | Action::CloseWindow) {
            self.session_dirty = true;
        }
        match action {
            Action::SwitchTab(i) => self.show_folder(i),
            Action::CloseTab(i) => self.close_folder(i, false),
            Action::OpenFolder => self.pick_folder(),
            Action::Rescan => self.rescan(),
            Action::Fit => self.frame_every_folder(),
            Action::ZoomBy(f) => self.zoom_center(f),
            Action::FrameSelection => self.frame_selection(),
            Action::FlyToSite(site) => self.frame_site(site),
            Action::OpenMedia(path) => {
                if crate::media::kind(&path).is_some() {
                    self.reveal_file(&path);
                } else {
                    self.open_preview(&path);
                }
            }
            Action::Preview(rel) => {
                if let Some(file) = self.pages.get(self.active).map(|p| p.root.join(&rel)) {
                    self.toggle_preview(&file);
                }
            }
            Action::PreviewSelected => match self.current_file() {
                Some(file) if crate::markdown::is_markdown(&file) => {
                    self.open_preview(&file);
                }
                _ => self.ext.notice(ext::Level::Info, "markdown", "Select a Markdown file first.".to_string()),
            },
            Action::ShowSource(path) => self.reveal_file(&path),
            Action::OpenIgnored(rel) => {
                if let Some(file) = self.pages.get(self.active).map(|p| p.root.join(&rel)) {
                    if !file.is_dir() {
                        self.reveal_file(&file);
                    } else if let Err(why) = crate::files::reveal(&file) {
                        self.ext.notice(ext::Level::Warn, "files", format!("It could not be shown: {why}."));
                    }
                }
            }
            Action::OpenExternally(path) => crate::update::open_page(&path.to_string_lossy()),
            Action::FlyToPath(path) => {
                if let Some(file) = self.pages.get(self.active).map(|p| p.root.join(&path)) {
                    self.open_media(&file);
                }
                let node = self
                    .pages
                    .get(self.active)
                    .and_then(|p| p.ready())
                    .and_then(|r| r.layout.node_for_path(&r.repo, &path));
                if let Some(n) = node {
                    if let Some(p) = self.pages.get_mut(self.active) {
                        p.selected = Some(n);
                    }
                    self.frame_node(n);
                }
            }
            Action::HoverPath(path) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    let node = page.ready().and_then(|r| r.layout.node_for_path(&r.repo, &path));
                    if node != page.hover {
                        page.hover = node;
                        page.hover_cell = None;
                        self.request_redraw();
                    }
                }
            }
            Action::ToggleTreeOpen(path) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    if !page.tree_open.remove(&path) {
                        page.tree_open.insert(path);
                    }
                    if let Some(r) = page.ready_mut() {
                        r.tree_dirty = true;
                    }
                    self.request_redraw();
                }
            }
            Action::ToggleHidden(path) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    if !page.filter.hidden.remove(&path) {
                        page.filter.hidden.insert(path);
                    }
                    self.relayout();
                }
            }
            Action::ToggleCollapsed(path) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    let chain: Vec<String> = page
                        .ready()
                        .map(|r| {
                            let node = r.layout.dir_node.get(&path).copied();
                            r.layout
                                .dir_node
                                .iter()
                                .filter(|(_, n)| Some(**n) == node)
                                .map(|(p, _)| p.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    let was = chain.iter().any(|p| page.filter.collapsed.contains(p));
                    if was {
                        for p in &chain {
                            page.filter.collapsed.remove(p);
                        }
                    } else {
                        page.filter.collapsed.insert(path);
                    }
                    self.relayout();
                }
            }
            Action::CollapseAll => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    let top: Vec<String> = page
                        .ready()
                        .map(|r| r.repo.tree.dirs.iter().map(|d| d.rel_path.clone()).collect())
                        .unwrap_or_default();
                    page.filter.collapsed = top.into_iter().collect();
                    self.relayout();
                    self.frame_all();
                }
            }
            Action::ExpandAll => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.filter.collapsed.clear();
                    self.relayout();
                    self.frame_all();
                }
            }
            Action::ShowAll => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.filter.hidden.clear();
                    self.relayout();
                }
            }
            Action::RemoveSymbol(i) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    if i < page.selections.len() {
                        page.selections.remove(i);
                        page.curves_dirty = true;
                        if page.selections.is_empty() {
                            page.only_connected = false;
                        }
                        self.relayout();
                    }
                }
            }
            Action::ClearSymbols => self.clear_symbols(),
            Action::SetOnlyConnected(on) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.only_connected = on;
                    self.relayout();
                    if self.pages.get(self.active).is_some_and(|p| p.selections.is_empty()) {
                        self.frame_all();
                    } else {
                        self.frame_selection();
                    }
                }
            }
            Action::Search(query) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.run_search(query);
                }
                self.request_redraw();
            }
            Action::DragWindow => {
                if let Some(g) = &self.gfx {
                    let _ = g.window.drag_window();
                }
            }
            Action::ToggleMaximize => {
                if let Some(g) = &self.gfx {
                    g.window.set_maximized(!g.window.is_maximized());
                }
            }
            Action::Minimize => {
                if let Some(g) = &self.gfx {
                    g.window.set_minimized(true);
                }
            }
            Action::CloseWindow => self.exit_requested = true,
            Action::FormatFile(rel) => self.format_file(rel),
            Action::RailsChanged => {
                self.session_dirty = true;
                self.request_redraw();
            }
            Action::Show(panel) => {
                self.ui.show(panel);
                self.request_redraw();
            }
            Action::Settings(tab) => {
                self.ui.settings.show(Some(tab));
                self.request_redraw();
            }
            Action::SaveFile => self.save_edit(),
            Action::RunCommand(id, title) => {
                self.ext.run_command(&id, &title);
                self.request_redraw();
            }
            Action::Answered(about, answer) => self.answered(about, answer),
            Action::Share(op) => self.share_op(op),
            Action::Workspace(op) => self.workspace_op(op),
            Action::Agent(op) => self.agent_op(op),
            Action::ToggleUi => {
                self.ui.bare = !self.ui.bare;
                if self.ui.bare {
                    let key = self.ui.keys.label(Command::ToggleUi);
                    let text =
                        format!("The interface is hidden. {key}, or a right click on the canvas, brings it back.");
                    self.ext.notice(ext::Level::Info, "Canvas", text);
                }
                self.request_redraw();
            }
            Action::Commands => self.run_command(Command::Palette),
            Action::RevealInFiles(path) => {
                self.ui.bare = false;
                self.ui.show(Panel::Files);
                self.ui.explorer_reveal = Some(path);
                self.request_redraw();
            }
            Action::Git(op) => {
                use crate::git::{GitOp, Part};
                let lost = match &op {
                    GitOp::Discard(path) => Some(("Discard the changes of this file?", vec![path.clone()])),
                    GitOp::Part { path, what: Part::Discard, .. } => Some(("Discard this change?", vec![path.clone()])),
                    GitOp::DiscardAll => {
                        let changed = self.pages.get(self.active).and_then(|page| page.git.as_ref());
                        let tracked = |c: &&crate::git::FileChange| c.change != crate::git::Change::Untracked;
                        let files =
                            changed.map(|git| git.changes.iter().filter(tracked).map(|c| c.path.clone()).collect());
                        Some(("Discard every change?", files.unwrap_or_default()))
                    }
                    _ => None,
                };
                let Some((title, files)) = lost else { return self.run_git(self.active, op) };
                let Some(folder) = self.pages.get(self.active).map(|page| page.root.clone()) else { return };
                let text = "The files go back to how the last commit left them. This cannot be undone.".to_string();
                let mut ask = ui::Ask::new(ui::Asked::Git { folder, op }, title, text)
                    .choice("Discard", ui::ChoiceLook::Danger, ui::Answer::Yes)
                    .choice("Cancel", ui::ChoiceLook::Plain, ui::Answer::Cancel);
                ask.items = files;
                self.ui.ask = Some(ask);
                self.request_redraw();
            }
            Action::WriteCommitMessage => {
                let settings = self.ui.commit_ai.clone();
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.write_message(settings);
                }
                self.request_redraw();
            }
            Action::AcceptCompletion(index) => self.accept_completion(index),
            Action::CheckUpdates => {
                self.updater.check_now();
                self.request_redraw();
            }
            Action::InstallUpdate => {
                if self.updater.install() {
                    self.ext.notice(ext::Level::Info, "Code Canvas", "downloading the new version…");
                } else if let Some(release) = &self.updater.offer {
                    crate::update::open_page(&release.page);
                }
                self.request_redraw();
            }
            Action::SetPrefs(prefs) => {
                crate::prefs::save(&prefs);
                let versions = prefs.check_versions != self.ui.prefs.check_versions;
                self.webviews.set_follow(prefs.follow_chat);
                self.updater.set_enabled(prefs.check_updates);
                if !prefs.edit {
                    for page in &mut self.pages {
                        page.editing = None;
                    }
                    self.completion = None;
                }
                let numbered = prefs.line_numbers != self.ui.prefs.line_numbers;
                let skipped =
                    (&prefs.skip_dirs, &prefs.skip_files) != (&self.ui.prefs.skip_dirs, &self.ui.prefs.skip_files);
                self.ui.prefs = prefs;
                if skipped {
                    crate::page::never_read(self.ui.prefs.skip_dirs.clone(), self.ui.prefs.skip_files.clone());
                    let (s, vp) = (self.scale(), self.viewport());
                    for page in self.pages.iter_mut().filter(|page| page.ready().is_some() && !page.only) {
                        page.restore_camera = page.portable_camera(s, vp);
                        page.framed = false;
                        page.start_scan();
                    }
                }
                if let (true, Some(g)) = (numbered, self.gfx.as_mut()) {
                    canvas_core::layout::set_line_numbers(self.ui.prefs.line_numbers);
                    for page in &mut self.pages {
                        g.renderer.text.drop_page(page.id);
                        page.relayout(&g.renderer, &g.device, &self.ui.filters);
                    }
                }
                if versions {
                    self.versions_due = self.pages.iter().map(|p| p.id).collect();
                    for path in std::mem::take(&mut self.manifests).into_keys() {
                        self.ext.set_own_decorations(&path.to_string_lossy(), "versions", Vec::new());
                    }
                    self.versions_told.clear();
                }
                self.request_redraw();
            }
            Action::SetCommitAi(settings) => {
                crate::commit_ai::save(&settings);
                self.ui.commit_ai = settings;
            }
            Action::RefreshGit => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.refresh_git();
                }
            }
            Action::Find(op) => self.find(op),
            Action::Back => self.travel(true),
            Action::Forward => self.travel(false),
            Action::SetDiff(view) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.set_diff(view);
                }
                self.request_redraw();
            }
            Action::ShowCommit(commit) => {
                let Some(shown) = self.pages.get(self.active).map(|page| page.showing.is_some()) else { return };
                if !shown {
                    self.commit_from = self.place_now();
                }
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.show_commit(commit);
                }
                self.request_redraw();
            }
            Action::SetOnlyChanged(on) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.only_changed = on;
                }
                self.relayout();
                self.frame_all();
            }
            Action::SetTheme(key) => {
                self.ui.theme.from_extension = theme_of_extension(&self.ext, &key);
                self.ui.theme.set_base(&key);
                self.theme_changed();
            }
            Action::SetColor(key, color) => {
                self.ui.theme.set_color(key, color);
                self.theme_changed();
            }
            Action::ResetColors => {
                self.ui.theme.colors.clear();
                self.theme_changed();
            }
            Action::ToggleSideBySide => {
                self.ui.single = !self.ui.single;
                self.request_redraw();
            }
            Action::SetIconTheme(id) => {
                self.ui.theme.icons = id;
                self.theme_dirty = true;
                self.request_redraw();
            }
            Action::File(op) => self.file_op(op),
            Action::ResetKey(command) => {
                self.ui.key_file.reset(command);
                self.keys_changed();
            }
            Action::UnbindKey(command) => {
                self.ui.key_file.unbind(command);
                self.keys_changed();
            }
            Action::InstallVsix => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Install an extension from a .vsix file")
                    .add_filter("VS Code extension", &["vsix"]);
                if let Some(g) = &self.gfx {
                    dialog = dialog.set_parent(&g.window);
                }
                if let Some(file) = dialog.pick_file() {
                    self.ext.install_vsix(file);
                }
                self.request_redraw();
            }
            Action::DepsChanged => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.curves_dirty = true;
                }
                self.request_redraw();
            }
            Action::FiltersChanged => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.curves_dirty = true;
                    if page.only_connected {
                        self.relayout();
                    }
                }
                self.request_redraw();
            }
        }
    }

    fn run_script(&mut self) -> bool {
        let Some(script) = self.script.as_mut() else { return false };
        let busy = self.pages.get(self.active).is_some_and(|p| p.is_loading() || p.target.is_some() || !p.framed);
        if busy || !self.ui_events.is_empty() {
            self.request_redraw();
            return true;
        }
        let Some(step) = script.next(Instant::now()) else {
            let more = !script.is_done();
            if !more {
                self.script = None;
            }
            return more;
        };
        log::info!("script: {step:?}");
        match step {
            Step::Wait(_) => {}
            Step::Fit => self.apply(Action::Fit),
            Step::Locate => self.apply(Action::FrameSelection),
            Step::Goto(path) => self.apply(Action::FlyToPath(path)),
            Step::Search(text) => {
                self.ui.search_query = text.clone();
                self.ui.show(Panel::Search);
                self.apply(Action::Search(text));
            }
            Step::Select(name) => {
                let sym = self
                    .pages
                    .get(self.active)
                    .and_then(|p| p.ready())
                    .and_then(|r| r.repo.symbols.defs(&name).first().copied());
                match sym {
                    Some(sym) => self.select_symbol(sym, None, false),
                    None => log::warn!("script: no symbol named '{name}'"),
                }
            }
            Step::Left(which) => match which.as_str() {
                "files" => self.ui.show(Panel::Files),
                "search" => self.ui.show(Panel::Search),
                "help" => self.ui.show(Panel::Help),
                "extensions" => self.ui.show(Panel::Extensions),
                "problems" => self.ui.show(Panel::Problems),
                "changes" => self.ui.show(Panel::Changes),
                "settings" => self.ui.settings.show(None),
                "settings:close" => self.ui.settings.open = false,
                id if id.starts_with("settings:") => match ui::SettingsTab::from_key(&id[9..]) {
                    Some(tab) => self.ui.settings.show(Some(tab)),
                    None => self.ui.settings.search(&id[9..]),
                },
                id if id.contains('.') => {
                    self.ui.extension = id.to_string();
                    self.ui.show(Panel::Extension);
                }
                _ => self.ui.close(Side::Left),
            },
            Step::Install(id) => match id.split_once('.') {
                Some((namespace, name)) => self.ext.install(namespace, name),
                None => log::warn!("script: install needs publisher.name, got '{id}'"),
            },
            Step::ExtSearch(text) => {
                self.ui.show(Panel::Extensions);
                self.ext.query = text;
                self.ext.search();
            }
            Step::Format(path) => self.format_file(Some(path)),
            Step::Command(id) => self.ext.run_command(&id, &id),
            Step::Answer(choice) => {
                let asked = self.ui.ask.as_ref().and_then(|ask| {
                    let said = ask.choices.iter().find(|(label, ..)| label.eq_ignore_ascii_case(&choice))?;
                    Some((ask.about.clone(), said.2))
                });
                match asked {
                    Some((about, answer)) => {
                        self.ui.ask = None;
                        self.answered(about, answer);
                    }
                    None if self.ext.answer_with(&choice) => {}
                    None => log::warn!("script: nothing offers '{choice}'"),
                }
            }
            Step::OnlyChanged(on) => self.apply(Action::SetOnlyChanged(on)),
            Step::Edit(what) => {
                let mut parts = what.split_whitespace();
                let rel = parts.next().unwrap_or_default().to_string();
                let line = parts.next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);
                let col = parts.next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);
                let started = self.pages.get_mut(self.active).is_some_and(|page| {
                    let ok = page.begin_edit(&rel);
                    if let (true, Some(e)) = (ok, page.editor_mut()) {
                        e.move_to(crate::editor::Pos::new(line, col), false);
                    }
                    ok
                });
                if started {
                    self.keep_caret_visible();
                } else {
                    log::warn!("script: cannot edit '{rel}'");
                }
            }
            Step::Type(text) => {
                if let Some(e) = self.pages.get_mut(self.active).and_then(|p| p.editor_mut()) {
                    e.insert(&text);
                }
                self.after_edit();
                self.update_completion();
                self.ask_completion();
            }
            Step::Key(name)
                if self.completion.is_some() && matches!(name.as_str(), "up" | "down" | "enter" | "tab" | "escape") =>
            {
                let (count, selected) = self.completion.as_ref().map_or((1, 0), |c| (c.items.len(), c.selected));
                match name.as_str() {
                    "down" => self.completion.as_mut().unwrap().selected = (selected + 1) % count,
                    "up" => self.completion.as_mut().unwrap().selected = (selected + count - 1) % count,
                    "escape" => self.completion = None,
                    _ => self.accept_completion(selected),
                }
                self.request_redraw();
            }
            Step::Key(name) => {
                let mut save = false;
                if let Some(page) = self.pages.get_mut(self.active) {
                    if name == "escape" {
                        page.editing = None;
                    } else if name == "save" {
                        save = true;
                    } else if let Some(e) = page.editor_mut() {
                        match name.as_str() {
                            "enter" => e.newline(),
                            "backspace" => e.backspace(),
                            "delete" => e.delete(),
                            "tab" => e.tab(),
                            "left" => e.left(false),
                            "right" => e.right(false),
                            "up" => e.vertical(-1, false),
                            "down" => e.vertical(1, false),
                            "home" => e.home(false),
                            "end" => e.end(false),
                            "undo" => {
                                e.undo();
                            }
                            "redo" => {
                                e.redo();
                            }
                            other => log::warn!("script: unknown key '{other}'"),
                        }
                    }
                }
                if save {
                    self.save_edit();
                } else {
                    self.after_edit();
                }
            }
            Step::Git(what) => {
                use crate::git::GitOp;
                let (verb, arg) = what.split_once(' ').unwrap_or((what.as_str(), ""));
                let op = match verb {
                    "stage" if arg.is_empty() => Some(GitOp::StageAll),
                    "stage" => Some(GitOp::Stage(arg.to_string())),
                    "unstage" if arg.is_empty() => Some(GitOp::UnstageAll),
                    "unstage" => Some(GitOp::Unstage(arg.to_string())),
                    "commit" if !arg.is_empty() => Some(GitOp::commit(arg)),
                    "discard" if arg.is_empty() => Some(GitOp::DiscardAll),
                    "discard" => Some(GitOp::Discard(arg.to_string())),
                    _ => None,
                };
                match op {
                    Some(op) => self.apply(Action::Git(op)),
                    None => log::warn!("script: git needs stage|unstage|discard [path] or commit <message>"),
                }
            }
            Step::Right(which) => match which.as_str() {
                "symbols" => self.ui.show(Panel::Symbols),
                "deps" => self.ui.show(Panel::Deps),
                key if !key.is_empty() && key != "none" => {
                    let items = self.ext.rail_items();
                    let found = items.iter().find(|w| w.view == key || w.extension == key).map(|w| w.view.clone());
                    let open = self.ext.webviews.iter().find(|w| w.key == key || w.extension == key);
                    match found.or(open.map(|w| w.key.clone())) {
                        Some(k) => {
                            self.ui.show(Panel::Webview(k));
                        }
                        None => log::warn!("script: no webview '{key}'"),
                    }
                }
                _ => self.ui.close(Side::Right),
            },
            Step::CollapseAll => self.apply(Action::CollapseAll),
            Step::ExpandAll => self.apply(Action::ExpandAll),
            Step::Connected(on) => self.apply(Action::SetOnlyConnected(on)),
            Step::Tab(n) => self.show_folder(n.saturating_sub(1)),
            Step::Folders => {
                self.ui.show(Panel::Files);
                self.ui.open_folder_menu = true;
            }
            Step::Zoom(f) => self.apply(Action::ZoomBy(f)),
            Step::Quick(text) => self.ui.quick = Some(QuickOpen { query: text, selected: 0, focus: true }),
            Step::Updates => self.apply(Action::CheckUpdates),
            Step::Focus(what) => match what.as_str() {
                "page" => self.webviews.focus_page(),
                _ => self.webviews.focus_app(),
            },
            Step::Open(path) => {
                let given = PathBuf::from(&path);
                let file = match self.pages.get(self.active) {
                    Some(page) if given.is_relative() => page.root.join(given),
                    _ => given,
                };
                self.apply(Action::OpenMedia(file));
            }
            Step::Resize(w, h) => {
                if let Some(g) = &self.gfx {
                    let _ = g.window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
                }
            }
            Step::Palette(text) => {
                self.ui.quick = Some(QuickOpen { query: format!(">{text}"), selected: 0, focus: true })
            }
            Step::Maximize => self.apply(Action::ToggleMaximize),
            Step::Ui(what) => {
                let mut parts = what.split_whitespace();
                let gesture = parts.next().unwrap_or_default().to_string();
                let numbers: Vec<f32> = parts.filter_map(|n| n.parse().ok()).collect();
                let at = |i: usize| egui::pos2(numbers[i], numbers[i + 1]);
                let press = |pos, button, pressed| egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                let (left, right) = (egui::PointerButton::Primary, egui::PointerButton::Secondary);
                let frames: Vec<Vec<egui::Event>> = match (gesture.as_str(), numbers.len()) {
                    ("move", 2) => vec![vec![egui::Event::PointerMoved(at(0))]],
                    ("down", 2) => vec![vec![egui::Event::PointerMoved(at(0))], vec![press(at(0), left, true)]],
                    ("up", 2) => vec![vec![egui::Event::PointerMoved(at(0))], vec![press(at(0), left, false)]],
                    ("click", 2) | ("right", 2) => {
                        let button = if gesture == "click" { left } else { right };
                        vec![
                            vec![egui::Event::PointerMoved(at(0))],
                            vec![press(at(0), button, true)],
                            vec![press(at(0), button, false)],
                        ]
                    }
                    ("drag", 4) => {
                        let (from, to) = (at(0), at(2));
                        let mut frames = vec![vec![egui::Event::PointerMoved(from)], vec![press(from, left, true)]];
                        frames.extend((1..=8).map(|i| vec![egui::Event::PointerMoved(from.lerp(to, i as f32 / 8.0))]));
                        frames.push(vec![press(to, left, false)]);
                        frames
                    }
                    _ => {
                        log::warn!("script: ui needs move|down|up|click|right <x> <y> or drag <x> <y> <x> <y>");
                        Vec::new()
                    }
                };
                self.ui_events.extend(frames);
            }
            Step::Preview(path) => self.apply(Action::Preview(path)),
            Step::Page(script) => self.webviews.run(&script),
            Step::Back => self.travel(true),
            Step::Forward => self.travel(false),
            Step::Menu(x, y) => self.card_menu([x, y]),
            Step::Minimize(on) => {
                if let Some(g) = &self.gfx {
                    g.window.set_minimized(on);
                }
            }
            Step::Interface => {
                self.ui.card_menu = None;
                self.apply(Action::ToggleUi);
            }
            Step::Delete(path) => self.apply(Action::File(FileOp::Delete(path))),
            Step::Rename(path) => {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                self.ui.naming = Some(ui::Naming { kind: ui::NamingKind::Rename, path, text: name, focus: true });
                self.request_redraw();
            }
            Step::Ignored(path) => self.apply(Action::OpenIgnored(path)),
            Step::Part(asked) => {
                let mut words = asked.split_whitespace();
                let what = match words.next() {
                    Some("stage") => Some(crate::git::Part::Stage),
                    Some("unstage") => Some(crate::git::Part::Unstage),
                    _ => None,
                };
                let (path, line) = (words.next(), words.next().and_then(|line| line.parse::<u32>().ok()));
                match (what, path, line) {
                    (Some(what), Some(path), Some(line)) => {
                        let line = line.saturating_sub(1);
                        self.apply(Action::Git(crate::git::GitOp::Part { path: path.to_string(), line, what }))
                    }
                    _ => log::warn!("script: part needs stage|unstage, a path and a line"),
                }
            }
            Step::Share(asked) => {
                let (verb, rest) = asked.split_once(' ').unwrap_or((asked.as_str(), ""));
                let named = |me: &App, name: &str| {
                    let session = me.sharing.session.as_ref()?;
                    session.people.iter().find(|person| person.name.eq_ignore_ascii_case(name.trim())).map(|p| p.id)
                };
                let op = match verb {
                    "start" => {
                        self.ui.share.guests_write = rest.trim() == "writers";
                        Some(ui::ShareOp::Start)
                    }
                    "offer" => Some(ui::ShareOp::Offer),
                    "stop" => Some(ui::ShareOp::Stop),
                    "enter" => {
                        self.ui.share.typed = rest.trim().to_string();
                        Some(ui::ShareOp::Enter)
                    }
                    "join" => {
                        let mut words = rest.split_whitespace();
                        self.ui.share.typed = words.next().unwrap_or_default().to_string();
                        if let Some(name) = words.next() {
                            self.ui.share.name = name.to_string();
                        }
                        Some(ui::ShareOp::Join)
                    }
                    "panel" => {
                        self.ui.share.open = rest.trim() != "close";
                        None
                    }
                    "follow" => named(self, rest).map(|id| ui::ShareOp::Follow(Some(id))),
                    "unfollow" => Some(ui::ShareOp::Follow(None)),
                    "writer" => named(self, rest).map(|id| ui::ShareOp::SetWriter(id, true)),
                    "reader" => named(self, rest).map(|id| ui::ShareOp::SetWriter(id, false)),
                    _ => None,
                };
                match op {
                    Some(op) => self.share_op(op),
                    None if verb == "panel" => {}
                    None => log::warn!("script: share needs start [writers], offer, stop, join <invitation> [name], enter <invitation>, follow|writer|reader <name>, unfollow or panel"),
                }
            }
            Step::Agent(said) => match serde_json::from_str(&said) {
                Ok(event) => {
                    self.agents.hear(&event);
                    self.request_redraw();
                }
                Err(_) => match self.agents.list.iter().find(|agent| agent.name().eq_ignore_ascii_case(said.trim())) {
                    Some(agent) => {
                        let key = agent.key.clone();
                        self.agent_op(ui::AgentOp::Follow(Some(key)));
                    }
                    None => log::warn!("script: agent needs the JSON of an event, or the name of one to follow"),
                },
            },
            Step::Workspace(asked) => {
                let (verb, rest) = asked.split_once(' ').unwrap_or((asked.as_str(), ""));
                let path = PathBuf::from(rest.trim());
                let op = match verb {
                    "put" => Some(ui::WorkspaceOp::Put(path)),
                    "off" => Some(ui::WorkspaceOp::TakeOff(path)),
                    "forget" => Some(ui::WorkspaceOp::Forget(path)),
                    "all" => Some(ui::WorkspaceOp::PutAll),
                    _ => None,
                };
                match op {
                    Some(op) => self.workspace_op(op),
                    None => log::warn!("script: workspace needs put|off|forget <folder>, or all"),
                }
            }
            Step::Theme(named) => {
                let own =
                    canvas_render::theme::PRESETS.iter().map(|(key, name, _)| (key.to_string(), name.to_string()));
                let theirs = self.ext.color_themes().map(|theme| (theme.key.clone(), theme.label.clone()));
                let found = own.chain(theirs).find(|(key, name)| *key == named || name.eq_ignore_ascii_case(&named));
                match found {
                    Some((key, _)) => self.apply(Action::SetTheme(key)),
                    None => log::warn!("script: no theme named '{named}'"),
                }
            }
            Step::Commit(which) => {
                let graph =
                    self.pages.get(self.active).and_then(|page| page.git.as_ref()).map(|git| git.graph.as_slice());
                let found = match (which.trim().parse::<usize>(), graph) {
                    (Ok(nth), Some(graph)) => graph.get(nth),
                    (Err(_), Some(graph)) => graph.iter().find(|node| node.commit.hash.starts_with(which.trim())),
                    _ => None,
                };
                let commit = found.map(|node| node.commit.clone());
                if commit.is_none() && which.trim() != "off" {
                    log::warn!("script: commit needs off, the place of a commit in the graph or the start of its hash");
                }
                self.apply(Action::ShowCommit(commit));
            }
            Step::Diff(what) => {
                let mut view = self.pages.get(self.active).map(|page| page.diff).unwrap_or_default();
                match what.split_once(' ').unwrap_or((what.as_str(), "")) {
                    ("on", _) => view.on = true,
                    ("off", _) => view.on = false,
                    ("fold", _) => view.fold = true,
                    ("unfold", _) => view.fold = false,
                    ("split", _) => view.split = true,
                    ("together", _) => view.split = false,
                    ("around", lines) => view.around = lines.trim().parse().unwrap_or(view.around),
                    _ => log::warn!("script: diff needs on|off|split|together|fold|unfold|around <lines>"),
                }
                self.apply(Action::SetDiff(view));
            }
            Step::Tool(which) => {
                let edit = which == "edit";
                self.apply(Action::SetPrefs(crate::prefs::Prefs { edit, ..self.ui.prefs.clone() }));
            }
            Step::Touch(what) => {
                let (rel, text) = what.split_once(' ').unwrap_or((what.as_str(), ""));
                let file = self.pages.get(self.active).map(|page| page.root.join(rel));
                let written = file.as_ref().map(|file| {
                    if let Some(parent) = file.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let mut all = std::fs::read_to_string(file).unwrap_or_default();
                    all.push_str(&text.replace("\\n", "\n"));
                    all.push('\n');
                    std::fs::write(file, all)
                });
                if !matches!(written, Some(Ok(()))) {
                    log::warn!("script: cannot append to '{rel}'");
                }
                self.last_input = Instant::now().checked_sub(Duration::from_secs(10)).unwrap_or(self.last_input);
            }
            Step::Drop(path) => self.drop_file(PathBuf::from(path)),
            Step::Mark(name) => {
                let name: String =
                    name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
                let folder = session::data_dir().join("marks");
                if name.is_empty()
                    || std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(folder.join(&name), "")).is_err()
                {
                    log::warn!("script: the mark '{name}' could not be left");
                }
            }
            Step::Move(what) => {
                let mut parts = what.split_whitespace();
                let panel = parts.next().and_then(|name| {
                    let view = self.ext.rail_items().into_iter().find(|w| w.view == name || w.extension == name);
                    view.map(|w| Panel::Webview(w.view)).or_else(|| Panel::from_key(name))
                });
                let side = match parts.next() {
                    Some("left") => Some(Side::Left),
                    Some("right") => Some(Side::Right),
                    _ => None,
                };
                match (panel, side) {
                    (Some(panel), Some(side)) => self.ui.request_move(panel, side, parts.next().map(str::to_string)),
                    _ => log::warn!("script: move needs <panel> left|right [panel to sit before]"),
                }
            }
            Step::Find(what) => {
                let (verb, text) = what.split_once(' ').unwrap_or((what.as_str(), ""));
                match verb {
                    "next" => self.find(ui::FindOp::Next),
                    "previous" => self.find(ui::FindOp::Previous),
                    "replace" | "replace-all" => {
                        if let Some(find) = self.ui.find.as_mut() {
                            find.with = text.to_string();
                        }
                        self.find(if verb == "replace" { ui::FindOp::Replace } else { ui::FindOp::ReplaceAll });
                    }
                    "close" => self.find(ui::FindOp::Close),
                    _ => {
                        self.find(ui::FindOp::Open);
                        if let Some(find) = self.ui.find.as_mut() {
                            find.query = what.clone();
                            find.focus = false;
                        }
                        self.find(ui::FindOp::Changed);
                    }
                }
            }
            Step::Drag(from, to) => {
                self.cursor = from;
                let select = self.code_under(from).filter(|_| self.ui.prefs.edit);
                match select {
                    Some(rel) => {
                        self.cursor = to;
                        self.drag_select(&rel, from, true);
                    }
                    None => self.pan(to[0] - from[0], to[1] - from[1]),
                }
            }
            Step::Click(x, y) => {
                self.cursor = [x, y];
                self.focus_folder_at(self.cursor);
                if !self.over_ui(self.cursor) {
                    self.on_canvas_click(self.cursor);
                }
                self.update_hover();
            }
            Step::Quit => self.exit_requested = true,
        }
        self.request_redraw();
        true
    }

    fn poll_background(&mut self) -> bool {
        let mut busy = false;
        let mut changed = false;
        if self.symbols_stale.is_some_and(|t| t.elapsed() > Duration::from_millis(700)) {
            self.symbols_stale = None;
            for page in &mut self.pages {
                if let PageState::Ready(r) = &mut page.state {
                    if !page.editors.is_empty() {
                        r.repo.reindex_symbols();
                    }
                }
            }
        }
        busy |= self.symbols_stale.is_some();
        busy |= self.poll_copies();
        if self.unsent_edit.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(250)) {
            if let Some((path, _)) = self.unsent_edit.take() {
                let text =
                    self.locate_file(&path).and_then(|(i, rel)| self.pages[i].editors.get(&rel).map(|e| e.text()));
                if let Some(text) = text {
                    self.ext.file_edited(&path, &text);
                }
                if let Some((i, _)) = self.locate_file(&path) {
                    self.pages[i].preview_unsaved();
                }
                self.refresh_preview();
            }
        }
        busy |= self.unsent_edit.is_some();
        busy |= self.poll_share();
        if !self.rescan_later.is_empty() {
            self.rescan_what_waited();
        }
        if !self.changed_under.is_empty() {
            self.settle_changed_under();
        }
        busy |= self.poll_agents();
        if self.claude_offer_due.is_some_and(|due| due <= Instant::now()) {
            self.claude_offer_due = None;
            self.offer_claude();
        }
        let mut git_relayout = false;
        for (i, page) in self.pages.iter_mut().enumerate() {
            changed |= page.poll_git_op();
            for sym in std::mem::take(&mut page.want_exact) {
                if let Some((path, line, col)) = page.use_site(sym) {
                    self.ext.request_uses(&path, line, col, page.id, (sym.file, sym.index));
                }
            }
            if let Some((fine, said)) = page.git_said.take() {
                let level = if fine { ext::Level::Info } else { ext::Level::Warn };
                self.ext.notice(level, "Source Control", said);
            }
            changed |= page.poll_message();
            changed |= page.poll_commit();
            if page.poll_heads() {
                page.preview_unsaved();
                changed = true;
            }
            let shown = page.touched.len();
            page.touched.retain(|t| t.strength() > 0.0);
            changed |= shown != page.touched.len();
            busy |= !page.touched.is_empty();
            if page.poll_git() {
                changed = true;
                git_relayout |= i == self.active && page.only_changed;
            }
        }
        if git_relayout {
            self.relayout();
        }
        if let Some(page) = self.pages.get_mut(self.active) {
            let selected = page.ready().and_then(|r| {
                let n = r.layout.nodes.get(page.selected?)?;
                matches!(n.kind, NodeKind::File(_)).then(|| n.rel_path.clone())
            });
            if page.ready().is_some() {
                page.want_timeline(selected);
            }
            changed |= page.poll_timeline();
        }
        let cell = self
            .pages
            .get(self.active)
            .and_then(|p| p.hover_cell.filter(|_| Lod::for_zoom(p.camera.zoom) == Lod::Text));
        match (cell, &mut self.hover_probe) {
            (None, probe) => *probe = None,
            (Some(c), Some((at, since, asked))) if *at == c => {
                if !*asked && since.elapsed() > Duration::from_millis(350) {
                    *asked = true;
                    let place = self.pages.get(self.active).filter(|p| p.showing.is_none()).and_then(|p| {
                        let rel = &p.ready()?.repo.files.get(c.0 as usize)?.rel_path;
                        Some((p.root.join(rel), p.line_of(rel, c.1)?))
                    });
                    if let Some((path, line)) = place {
                        self.ext.request_hover(&path, line, c.2);
                    }
                }
                busy |= !*asked;
            }
            (Some(c), probe) => {
                *probe = Some((c, Instant::now(), false));
                busy = true;
            }
        }
        let folders: Vec<(PathBuf, bool)> =
            self.pages.iter().filter(|p| !p.is_waiting()).map(|p| (p.root.clone(), !p.only)).collect();
        self.watcher.set_folders(&folders);
        if let Some(batch) = self.watcher.poll() {
            self.on_disk_changes(batch);
            changed = true;
        }
        busy |= self.watcher.busy();
        let worked_in = self.pages.iter().filter(|p| !p.is_waiting() && !p.only);
        self.ext.set_folders(worked_in.map(|p| p.root.clone()).collect());
        let (events, ext_changed) = self.ext.poll();
        changed |= ext_changed;
        for event in events {
            match event {
                ext::Event::Write { path, text } => self.write_file(&path, &text),
                ext::Event::Reveal(path) => self.reveal_file(&path),
                ext::Event::GoTo { path, line, col, len } => self.show_path(&path, Some((line, col, len))),
                ext::Event::Suggestions(found) => self.offer_code(found),
                ext::Event::Uses { page, of, places } => self.exact_uses(page, of, places),
                ext::Event::ExtraEdits { path, edits } => self.extra_edits(&path, edits),
                ext::Event::WebviewHtml { handle, html } => self.webviews.set_html(&handle, html),
                ext::Event::WebviewPost { handle, message } => self.webviews.post(&handle, &message),
                ext::Event::WebviewClosed(handle) => {
                    self.webviews.close(&handle);
                    for side in Side::BOTH {
                        let Some(Panel::Webview(view)) = self.ui.open(side).cloned() else { continue };
                        if view.starts_with(ext::TABS) && self.ext.shown(&view).is_none() {
                            let extension = &view[ext::TABS.len()..];
                            let other = self.ext.rail_items().into_iter().find(|item| item.extension == extension);
                            match other {
                                Some(item) if self.ui.side_of(&Panel::Webview(item.view.clone())) == side => {
                                    self.ui.show(Panel::Webview(item.view))
                                }
                                _ => self.ui.close(side),
                            }
                        }
                    }
                }
                ext::Event::ShowWebview(key) => {
                    self.ui.show(Panel::Webview(key));
                }
            }
        }
        let mut roots: Vec<PathBuf> = self.pages.iter().map(|p| p.root.clone()).collect();
        roots.push(self.ext.extensions_dir().to_path_buf());
        if roots != self.webview_roots {
            self.webviews.set_roots(roots.clone());
            self.webview_roots = roots;
        }
        for (handle, message) in self.webviews.poll() {
            self.ext.webview_message(&handle, message);
        }
        for key in self.webviews.take_keys() {
            self.page_chord(&key);
        }
        for wheel in self.webviews.take_wheels() {
            let Some(place) = self.player_place().filter(|_| wheel.page == crate::media::PLAYER) else { continue };
            let turned = self.scale() * WHEEL_NOTCH / 100.0;
            let moved = [-wheel.moved[0] * turned, -wheel.moved[1] * turned];
            let at = [place.x + wheel.at[0] * self.scale(), place.y + wheel.at[1] * self.scale()];
            self.wheel(moved, at, wheel.ctrl, wheel.shift);
        }
        for url in self.webviews.take_links() {
            self.follow_link(&url);
        }
        self.poll_updates();
        for id in std::mem::take(&mut self.versions_due) {
            self.read_dependencies(id);
            changed = true;
        }
        if self.versions.poll() {
            self.show_versions();
            changed = true;
        }
        if self.sync_diffs() {
            self.find_refresh(false);
            changed = true;
        }
        let budget = crate::page::text_budget();
        if self.text_checked.elapsed() > Duration::from_secs(2) {
            self.text_checked = Instant::now();
            if canvas_core::scan::loaded_text_bytes() > budget {
                let share = budget / self.pages.len().max(1);
                let freed: usize =
                    self.pages.iter_mut().filter_map(Page::ready_mut).map(|r| r.repo.trim_text(share)).sum();
                log::debug!("text over its budget: {} MB let go", freed >> 20);
            }
        }
        let idle = self.drag.is_none() && self.last_input.elapsed() > Duration::from_millis(400);
        let share = (budget / self.pages.len().max(1)) as u64;
        for page in self.pages.iter_mut().filter(|page| idle && page.target.is_none()) {
            let (at, batch) = (page.warm_at, page.warm_batch);
            let Some(r) = page.ready().filter(|r| at < r.repo.files.len() && r.repo.text_bytes / 3 * 4 < share) else {
                continue;
            };
            let started = Instant::now();
            page.warm_at = r.repo.warm(at, batch);
            let took = started.elapsed();
            page.warm_batch = match took.as_millis() {
                0..=5 => (batch * 2).min(8192),
                14.. => (batch / 2).max(64),
                _ => batch,
            };
            busy = true;
            break;
        }
        busy |= self.follow_now();
        busy |= !self.ext.notices.is_empty();
        if changed {
            self.request_redraw();
        }
        busy
    }

    fn poll_pages(&mut self) {
        let Some(g) = &mut self.gfx else { return };
        let mut changed = false;
        let mut made: Vec<(usize, String)> = Vec::new();
        for (index, page) in self.pages.iter_mut().enumerate() {
            if let Some(loaded) = page.poll() {
                let gpu = g.renderer.create_scene(&g.device, &loaded.repo.name, loaded.scene);
                g.renderer.text.drop_page(page.id);
                page.state = PageState::Ready(Box::new(Ready {
                    repo: loaded.repo,
                    deps: loaded.deps,
                    layout: loaded.layout,
                    gpu,
                    tokens: Default::default(),
                    load_ms: loaded.total_ms,
                    tree: Vec::new(),
                    tree_dirty: true,
                }));
                let kept: Vec<String> = page.editors.keys().cloned().collect();
                for rel in &kept {
                    page.show_edit(rel, &g.renderer, &g.device, &self.ui.filters);
                }
                let there = |rel: &String| page.ready().is_some_and(|r| r.repo.file_index(rel).is_some());
                if page.editing.as_ref().is_some_and(|rel| !there(rel)) {
                    page.editing = None;
                }
                page.curves_dirty = true;
                changed = true;
                self.versions_due.push(page.id);
                made.extend(page.reveal.take().map(|rel| (index, rel)));
            }
        }
        if changed {
            g.window.request_redraw();
        }
        if self.frame_all_when_read && self.pages.iter().all(|page| !page.is_loading() && !page.is_waiting()) {
            self.frame_all_when_read = false;
            self.frame_every_folder();
        }
        let read = self
            .show_when_read
            .as_ref()
            .is_some_and(|(path, _)| self.locate_file(path).is_none_or(|(i, _)| self.pages[i].ready().is_some()));
        if let Some((path, at)) = self.show_when_read.take_if(|_| read) {
            self.open_media(&path);
            self.show_path(&path, at);
        }
        for (index, rel) in made {
            let lines = self.pages[index]
                .ready()
                .and_then(|r| Some(r.repo.files[r.repo.file_index(&rel)?].line_count() as u32))
                .unwrap_or(0);
            if lines > 0 {
                self.pages[index].touch(&rel, vec![(0, lines)]);
                self.follow_change(index, &rel, 0);
            }
        }
    }

    fn poll_updates(&mut self) {
        use crate::update::News;
        const UPDATE: &str = "Update and restart";
        const PAGE: &str = "Open the download page";
        match self.updater.poll() {
            Some(News::Available(release)) => {
                let text = format!("Version {} is available. This is {}.", release.version, crate::update::current());
                let first = if release.download.is_some() { UPDATE } else { PAGE };
                self.ext.ask("Code Canvas", text, &[first, "Later"], "update");
                self.request_redraw();
            }
            Some(News::Installed(program)) => {
                self.restart_as = Some(program);
                self.exit_requested = true;
            }
            Some(News::Failed(why)) => {
                self.ext.notice(ext::Level::Error, "Code Canvas", format!("the update was not installed: {why}"));
                self.request_redraw();
            }
            Some(News::UpToDate) => {
                let text = format!("version {} is the latest", crate::update::current());
                self.ext.notice(ext::Level::Info, "Code Canvas", text);
                self.request_redraw();
            }
            Some(News::CheckFailed(why)) => {
                self.ext.notice(ext::Level::Warn, "Code Canvas", format!("could not look for a new version: {why}"));
                self.request_redraw();
            }
            None => {}
        }
        for (about, choice) in self.ext.take_answers() {
            if let Some(id) = about.strip_prefix("checker:") {
                match (choice.as_str(), id.split_once('.')) {
                    (INSTALL_IT, Some((namespace, name))) => self.ext.install(namespace, name),
                    (NEVER, _) => {
                        let mut prefs = self.ui.prefs.clone();
                        prefs.unwanted.push(id.to_string());
                        self.apply(Action::SetPrefs(prefs));
                    }
                    _ => {}
                }
                self.request_redraw();
                continue;
            }
            if about == assistants::CLAUDE_OFFER {
                self.claude_answered(&choice);
                continue;
            }
            if about != "update" {
                continue;
            }
            match choice.as_str() {
                UPDATE if self.updater.install() => {
                    self.ext.notice(ext::Level::Info, "Code Canvas", "downloading the new version…");
                }
                PAGE => {
                    if let Some(release) = &self.updater.offer {
                        crate::update::open_page(&release.page);
                    }
                }
                _ => {}
            }
            self.request_redraw();
        }
    }

    fn read_dependencies(&mut self, page_id: u32) {
        const MAX_MANIFESTS: usize = 40;
        if !self.ui.prefs.check_versions {
            return;
        }
        let Some(page) = self.pages.iter().find(|p| p.id == page_id) else { return };
        let Some(r) = page.ready() else { return };
        let root = page.root.clone();
        for gone in self.manifests.keys().filter(|path| path.starts_with(&root)).cloned().collect::<Vec<_>>() {
            self.manifests.remove(&gone);
            self.ext.set_own_decorations(&gone.to_string_lossy(), "versions", Vec::new());
        }
        let mut all = Vec::new();
        let manifests = r.repo.files.iter().filter(|f| matches!(f.name.as_str(), "package.json" | "Cargo.toml"));
        for f in manifests.take(MAX_MANIFESTS) {
            let found = crate::versions::dependencies(&f.name, f.text());
            if !found.is_empty() {
                all.extend(found.iter().cloned());
                self.manifests.insert(root.join(&f.rel_path), found);
            }
        }
        self.versions.want(&all);
        self.show_versions();
    }

    fn show_versions(&mut self) {
        use crate::versions::Standing;
        let t = canvas_render::theme::get();
        let mut counts: HashMap<PathBuf, (usize, usize)> = HashMap::new();
        for (path, wanted) in &self.manifests {
            let notes = self.versions.notes(wanted);
            if let Some(page) = self.pages.iter().find(|p| path.starts_with(&p.root)) {
                let count = counts.entry(page.root.clone()).or_default();
                count.0 += notes.iter().filter(|n| n.standing != Standing::Latest).count();
                count.1 += notes.iter().filter(|n| n.standing == Standing::Major).count();
            }
            let items = notes
                .into_iter()
                .map(|n| ext::Decoration {
                    line: n.line,
                    col: 0,
                    end_line: n.line,
                    end_col: 0,
                    whole_line: false,
                    text: n.text,
                    color: match n.standing {
                        Standing::Latest => t.git_added,
                        Standing::Newer => t.problem_info,
                        Standing::Major => t.problem_warning,
                    },
                    background: 0,
                })
                .collect();
            self.ext.set_own_decorations(&path.to_string_lossy(), "versions", items);
        }
        if self.versions.busy() {
            return;
        }
        for (root, (newer, major)) in counts {
            if newer > 0 && self.versions_told.insert(root.clone()) {
                let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let upgrades = match major {
                    0 => String::new(),
                    n => format!(", {n} of them a new major version"),
                };
                let text =
                    format!("{newer} dependencies have a newer version{upgrades}. They are marked in the manifests.");
                self.ext.notice(ext::Level::Info, &name, text);
            }
        }
    }

    fn describe(&self) -> (String, Option<(String, String)>) {
        let Some(page) = self.pages.get(self.active) else {
            return (
                format!(
                    "{}  ·  drop folders on the window  ·  each folder gets its own canvas",
                    self.ui.keys.hint("Open a folder", Command::OpenFolder)
                ),
                None,
            );
        };
        match &page.state {
            PageState::Waiting => (format!("{} has not been read yet", page.root.display()), None),
            PageState::Loading { .. } => (format!("Scanning {} …", page.root.display()), None),
            PageState::Failed(e) => (e.clone(), None),
            PageState::Ready(r) => {
                let overview = || format!("{}  ·  {} files", page.title, r.repo.files.len());
                if let Some(editor) = page.editor() {
                    let place = format!("Ln {}, Col {}", editor.caret.line + 1, editor.display_col(editor.caret) + 1);
                    return (format!("{}  ·  {place}", editor.rel_path), None);
                }
                let Some(n) = page.hover.or(page.selected).and_then(|i| r.layout.nodes.get(i)) else {
                    return (overview(), None);
                };
                let on_canvas = page.hover.is_some() && !self.over_ui(self.cursor) && self.drag.is_none();
                match n.kind {
                    NodeKind::File(fi) => {
                        let f = &r.repo.files[fi];
                        let mut status = format!("{}  ·  {} lines  ·  {}", f.rel_path, f.line_count(), f.lang.name());
                        if let Some(note) = &f.note {
                            status.push_str(&format!("  ·  {note}"));
                        }
                        if let Some(what) = canvas_core::scan::media_kind(&f.rel_path) {
                            let how = if what == "picture" { "" } else { "  ·  click it to play it here" };
                            status = format!("{}  ·  {what}{how}", f.rel_path);
                        }
                        let sym = page.hover_cell.and_then(|(file, line, _)| r.repo.sym_at(file, line));
                        let in_file = |cell: (u32, u32, u32)| Some((page.line_of(&f.rel_path, cell.1)?, cell.2));
                        let problem =
                            page.hover_cell.filter(|_| on_canvas).and_then(in_file).and_then(|(line, col)| {
                                let key = ext::path_key(&page.root.join(&f.rel_path).to_string_lossy());
                                let found = self.ext.diagnostics.get(&key)?;
                                found.items.iter().filter(|d| d.covers(line, col)).min_by_key(|d| d.severity)
                            });
                        if let Some(d) = problem {
                            return (status, Some((d.title(), d.message.clone())));
                        }
                        let known = page.hover_cell.filter(|_| on_canvas).and_then(in_file).and_then(|(line, col)| {
                            let h = self.ext.hover.as_ref()?;
                            let here = h.line == line && h.col == col && !h.text.is_empty();
                            (here && h.path == page.root.join(&f.rel_path)).then(|| crate::ui::hover_text(&h.text))
                        });
                        if let Some(tip) = known {
                            return (status, Some(tip));
                        }
                        let tip = on_canvas.then(|| match (sym, page.hover_cell) {
                            (Some(sym), Some((_, line, _))) => (
                                format!("{} {}", r.repo.sym(sym).kind.badge(), r.repo.sym_name(sym)),
                                format!("{}:{}", f.rel_path, line + 1),
                            ),
                            _ => (f.name.clone(), f.rel_path.clone()),
                        });
                        (status, tip)
                    }
                    NodeKind::Dir => {
                        let path = if n.rel_path.is_empty() { r.repo.name.clone() } else { n.rel_path.clone() };
                        let status = format!("{path}/  ·  {} files  ·  {} lines", n.files, n.lines);
                        let detail = if n.collapsed {
                            format!("{} files · collapsed, click + to expand", n.files)
                        } else {
                            format!("{} files · {} lines", n.files, n.lines)
                        };
                        (status, on_canvas.then(|| (format!("{}/", n.name), detail)))
                    }
                }
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let Some(g) = self.gfx.as_mut() else { return };
        if g.window.is_minimized() == Some(true) || !can_show(g.window.inner_size()) {
            return;
        }
        if std::mem::take(&mut g.resized) {
            let t = Instant::now();
            g.surface.configure(&g.device, &g.config);
            let ms = t.elapsed().as_secs_f32() * 1000.0;
            if ms > 40.0 {
                log::debug!("configuring the surface took {ms:.1} ms");
            }
        }
        let s = self.scale();
        let vp = self.viewport();

        self.ui.maximized = self.gfx.as_ref().is_some_and(|g| g.window.is_maximized());

        self.ui.completion = self.completion.as_ref().and_then(|offer| {
            let (tl, br) = self.caret_cell()?;
            Some(ui::CompletionView {
                at: egui::pos2(tl[0] / s, br[1] / s),
                items: match &offer.items {
                    Offered::Paths(items) => items
                        .iter()
                        .map(|c| ui::CompletionRow {
                            label: c.name.clone(),
                            detail: String::new(),
                            icon: ui::CompletionIcon::Path { is_dir: c.is_dir },
                        })
                        .collect(),
                    Offered::Code { items, .. } => items
                        .iter()
                        .map(|(_, item)| ui::CompletionRow {
                            label: item.label.clone(),
                            detail: item.detail.clone(),
                            icon: ui::CompletionIcon::Kind(item.kind.clone()),
                        })
                        .collect(),
                },
                selected: offer.selected,
            })
        });
        self.share_view();
        self.agents_view();
        self.know_folders();
        self.ui.workspace = self.workspace_rows();
        let mut travelling = false;
        for page in &mut self.pages {
            if let Some(t) = page.target {
                if page.camera.approach(&t, dt) {
                    page.target = None;
                } else {
                    travelling = true;
                }
            }
        }
        self.follow_active_camera();
        if let Some(g) = self.gfx.as_ref() {
            self.pictures.begin_frame(&g.egui);
        }
        self.place_pictures();
        let (status, tooltip) = self.describe();
        let zoom = self.pages.get(self.active).map(|p| p.camera.zoom / s).unwrap_or(1.0);
        let mut actions = Vec::new();
        let (mut full, canvas_pts) = {
            let g = self.gfx.as_mut().unwrap();
            let mut raw = g.egui_state.take_egui_input(&g.window);
            raw.events.extend(self.ui_events.pop_front().unwrap_or_default());
            self.ui.update = self.updater.state();
            self.ui.can_go = (!self.back.is_empty(), !self.forward.is_empty());
            let info = FrameInfo { stats: &self.stats, frame_ms: self.frame_ms, zoom, tooltip, status };
            let mut canvas = egui::Rect::NOTHING;
            let (ui_state, pages, active, ext) = (&mut self.ui, &mut self.pages, self.active, &mut self.ext);
            let full = g.egui.run_ui(raw, |ui| {
                canvas = ui::build(ui, ui_state, pages, active, ext, &info, &mut actions);
            });
            (full, canvas)
        };
        {
            let g = self.gfx.as_mut().unwrap();
            g.egui_state.handle_platform_output(&g.window, full.platform_output);
        }
        if canvas_pts.is_positive() {
            self.canvas_full =
                Rect::new(canvas_pts.left() * s, canvas_pts.top() * s, canvas_pts.width() * s, canvas_pts.height() * s);
        }
        for a in actions {
            self.apply(a);
        }
        let shared = self.shares_canvas();
        if shared != self.was_shared {
            self.was_shared = shared;
            if !shared {
                let active = self.active;
                for page in self.pages.iter_mut().enumerate().filter(|(i, _)| *i != active).map(|(_, page)| page) {
                    page.restore_camera = None;
                    page.framed = false;
                    page.target = None;
                    page.pane_center = None;
                }
            }
        }
        self.canvas = self.canvas_full;
        let shown = self.shown();
        {
            let places: Vec<(String, Rect)> = self
                .ui
                .page_places
                .iter()
                .filter_map(|(panel, r)| {
                    let handle = match panel {
                        Panel::Media => self.ui.media.as_ref().map(|_| crate::media::HANDLE.to_string())?,
                        Panel::Webview(view) => self.ext.shown(view)?.handle.clone()?,
                        _ => return None,
                    };
                    Some((handle, Rect::new(r.left() * s, r.top() * s, r.width() * s, r.height() * s)))
                })
                .collect();
            let mut places = places;
            let player = self.player_place();
            if std::mem::replace(&mut self.player_shown, player.is_some()) && player.is_none() {
                let quiet = "document.querySelectorAll('video,audio').forEach((m) => m.pause())";
                self.webviews.run_in(crate::media::PLAYER, quiet);
            }
            places.extend(player.map(|at| (crate::media::PLAYER.to_string(), at)));
            let targets: Vec<(&str, Rect)> = places.iter().map(|(handle, at)| (handle.as_str(), *at)).collect();
            let g = self.gfx.as_ref().unwrap();
            let t = Instant::now();
            self.webviews.show(&g.window, &targets);
            let ms = t.elapsed().as_secs_f32() * 1000.0;
            if ms > 8.0 {
                log::debug!("placing the webview took {ms:.1} ms");
            }
        }
        let area = self.canvas;

        let mut animating = travelling;
        for i in shown.iter().copied() {
            let active = i == self.active;
            let Some(page) = self.pages.get_mut(i) else { continue };
            page.ensure_scanned();
            if let PageState::Ready(r) = &page.state {
                page.extent = Some(r.layout.bounds);
                if !page.framed {
                    let restored = page.restore_camera.take();
                    if active || !shared {
                        page.camera = match restored {
                            Some(c) => Page::place_camera(c, s, vp, area.center()),
                            None => Camera::fit(r.layout.bounds, area, vp, 24.0 * s),
                        };
                    }
                    page.framed = true;
                    page.pane_center = Some(area.center());
                }
                let middle = area.center();
                if let Some(old) = page.pane_center.replace(middle) {
                    let z = page.camera.zoom;
                    let (dx, dy) = ((old[0] - middle[0]) / z, (old[1] - middle[1]) / z);
                    page.camera.center = [page.camera.center[0] + dx, page.camera.center[1] + dy];
                    if let Some(t) = page.target.as_mut() {
                        t.center =
                            [t.center[0] + (old[0] - middle[0]) / t.zoom, t.center[1] + (old[1] - middle[1]) / t.zoom];
                    }
                }
                if let Some(bench) = self.bench.as_mut().filter(|_| active) {
                    let fit = Camera::fit(r.layout.bounds, area, vp, 24.0 * s);
                    bench.drive(&mut page.camera, fit, 1.6 * s, vp);
                    page.target = None;
                    animating = true;
                }
            }
            animating |= page.touched.iter().any(|t| t.strength() < 1.0);
        }
        self.follow_active_camera();
        if animating {
            self.update_hover();
        }
        self.sync_documents();

        let filters = self.ui.filters;
        let mut canvas_overlay = CanvasOverlay::default();
        {
            let g = self.gfx.as_mut().unwrap();
            match self.pages.get_mut(self.active) {
                Some(page) => {
                    if page.curves_node != page.selected {
                        page.curves_node = page.selected;
                        page.curves_dirty = true;
                    }
                    if page.curves_dirty || self.curves_for != Some(page.id) {
                        let curves = overlay::build_curves(page, &filters, &self.ui.deps, s);
                        g.renderer.set_curves(&g.device, &g.queue, &curves);
                        page.curves_dirty = false;
                        self.curves_for = Some(page.id);
                    }
                    let view = page.camera.view(vp);
                    canvas_overlay = overlay::build(page, view, area, s, &filters, &self.ext, &mut g.renderer.text);
                }
                None => {
                    if self.curves_for.take().is_some() {
                        g.renderer.set_curves(&g.device, &g.queue, &[]);
                    }
                }
            }
        }
        self.chip_hits = std::mem::take(&mut canvas_overlay.chip_hits);
        self.toggle_hits = std::mem::take(&mut canvas_overlay.toggle_hits);
        self.preview_hits = std::mem::take(&mut canvas_overlay.preview_hits);
        let labels: Vec<Label> = canvas_overlay.labels.iter().map(|l| l.as_label()).collect();
        let chip_labels: Vec<Label> = canvas_overlay.chip_labels.iter().map(|l| l.as_label()).collect();

        let build_ms = now.elapsed().as_secs_f32() * 1000.0;
        let any_loading = self.pages.iter().any(|p| p.is_loading());
        let g = self.gfx.as_mut().unwrap();
        let textures = std::mem::take(&mut full.textures_delta);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                g.egui_renderer.update_texture(&g.device, &g.queue, *id, delta);
            }
        }
        let unused_textures = textures.free.clone();
        {
            let mut done = textures;
            done.clear();
        }
        let t_acquire = Instant::now();
        let acquired = g.surface.get_current_texture();
        let acquire_ms = t_acquire.elapsed().as_secs_f32() * 1000.0;
        if acquire_ms > 40.0 {
            log::debug!("acquiring the frame took {acquire_ms:.1} ms");
        }
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            skipped => {
                for id in &unused_textures {
                    g.egui_renderer.free_texture(id);
                }
                match skipped {
                    wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                        g.window.request_redraw();
                    }
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        g.surface.configure(&g.device, &g.config);
                        g.window.request_redraw();
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        match g.instance.create_surface(g.window.clone()) {
                            Ok(surface) => g.surface = surface,
                            Err(e) => log::error!("recreate surface: {e}"),
                        }
                        g.surface.configure(&g.device, &g.config);
                        g.window.request_redraw();
                    }
                    _ => log::error!("surface validation error"),
                }
                return;
            }
        };
        let target = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let side_by_side = shown.len() > 1;
        let edges = |r: Rect| [r.x, r.y, r.right(), r.bottom()];
        let mut first = true;
        let mut others_pending = false;
        let pane = area;
        for i in shown.iter().copied().filter(|i| *i != self.active) {
            let Some(page) = self.pages.get_mut(i) else { continue };
            let view = page.camera.view(vp);
            let over = overlay::build(page, view, pane, s, &filters, &self.ext, &mut g.renderer.text);
            let labels: Vec<Label> = over.labels.iter().map(|l| l.as_label()).collect();
            let chip_labels: Vec<Label> = over.chip_labels.iter().map(|l| l.as_label()).collect();
            let id = page.id;
            let scene = match &mut page.state {
                PageState::Ready(r) => {
                    let Ready { repo, layout, gpu, tokens, .. } = &mut **r;
                    Some(ScenePass { page: id, gpu, repo, layout, tokens, clip: edges(pane) })
                }
                _ => None,
            };
            let mut encoder = g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pane") });
            let stats = g.renderer.render(
                &g.device,
                &g.queue,
                &mut encoder,
                &target,
                view,
                Pane { area: Some(edges(pane)), clear: first, curves: false },
                scene,
                &Overlay {
                    under: &over.under,
                    labels: &labels,
                    over: &over.over,
                    chips: &over.chips,
                    chip_labels: &chip_labels,
                },
            );
            g.queue.submit(Some(encoder.finish()));
            first = false;
            others_pending |= stats.text_pending;
        }

        let (view, scene) = match self.pages.get_mut(self.active) {
            Some(page) => {
                let view = page.camera.view(vp);
                let id = page.id;
                let scene = match &mut page.state {
                    PageState::Ready(r) => {
                        let Ready { repo, layout, gpu, tokens, .. } = &mut **r;
                        Some(ScenePass {
                            page: id,
                            gpu,
                            repo,
                            layout,
                            tokens,
                            clip: [area.x, area.y, area.right(), area.bottom()],
                        })
                    }
                    _ => None,
                };
                (view, scene)
            }
            None => (Camera::default().view(vp), None),
        };

        let t0 = Instant::now();
        let prev = self.stats;
        let mut encoder = g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        self.stats = g.renderer.render(
            &g.device,
            &g.queue,
            &mut encoder,
            &target,
            view,
            Pane { area: side_by_side.then(|| edges(area)), clear: first, curves: true },
            scene,
            &Overlay {
                under: &canvas_overlay.under,
                labels: &labels,
                over: &canvas_overlay.over,
                chips: &canvas_overlay.chips,
                chip_labels: &chip_labels,
            },
        );

        self.stats.text_pending |= others_pending;

        let ppp = full.pixels_per_point;
        let prims = g.egui.tessellate(full.shapes, ppp);
        let screen =
            egui_wgpu::ScreenDescriptor { size_in_pixels: [g.config.width, g.config.height], pixels_per_point: ppp };
        let extra = g.egui_renderer.update_buffers(&g.device, &g.queue, &mut encoder, &prims, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            g.egui_renderer.render(&mut pass.forget_lifetime(), &prims, &screen);
        }
        for id in &unused_textures {
            g.egui_renderer.free_texture(id);
        }
        g.queue.submit(extra.into_iter().chain(Some(encoder.finish())));
        g.renderer.end_frame();

        let render_ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.frame_ms = self.frame_ms * 0.8 + (build_ms + render_ms) * 0.2;
        g.window.pre_present_notify();
        let t_present = Instant::now();
        g.queue.present(frame);
        let present_ms = t_present.elapsed().as_secs_f32() * 1000.0;
        if present_ms > 40.0 {
            log::debug!("presenting took {present_ms:.1} ms (build {build_ms:.1}, render {render_ms:.1})");
        }

        if let Some(b) = self.bench.as_mut() {
            b.record(dt * 1000.0, build_ms + render_ms);
        }

        let repaint = full.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
        self.next_repaint = None;
        let mut again = false;
        match repaint {
            Some(d) if d.is_zero() => again = true,
            Some(d) if d < Duration::from_secs(3600) => self.next_repaint = Some(Instant::now() + d),
            _ => {}
        }
        let st = &self.stats;
        let stats_changed = (prev.lod, prev.code_instances, prev.frame_instances, prev.draw_calls)
            != (st.lod, st.code_instances, st.frame_instances, st.draw_calls);
        if again || animating || st.text_pending || any_loading || stats_changed {
            g.window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {}

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(g) = &self.gfx {
            g.window.set_visible(false);
        }
        self.share_stop(None);
        self.agents.close();
        self.ext.shutdown();
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let screens: Vec<crate::window_place::Screen> = event_loop
            .available_monitors()
            .map(|m| crate::window_place::Screen {
                origin: [m.position().x, m.position().y],
                size: [m.size().width, m.size().height],
                scale: m.scale_factor() as f32,
            })
            .collect();
        let first = event_loop.primary_monitor().map(|m| [m.position().x, m.position().y]);
        let place = crate::window_place::place(&screens, first, self.window_size, self.window_pos);
        let size = place.size;
        let attrs = Window::default_attributes()
            .with_title("Code Canvas")
            .with_inner_size(LogicalSize::new(size[0] as f64, size[1] as f64))
            .with_position(winit::dpi::PhysicalPosition::new(place.position[0], place.position[1]))
            .with_maximized(self.start_maximized)
            .with_min_inner_size(LogicalSize::new(720.0, 420.0))
            .with_decorations(false)
            .with_window_icon(crate::icon::window_icon());
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
            attrs
                .with_undecorated_shadow(true)
                .with_corner_preference(CornerPreference::Round)
                .with_taskbar_icon(crate::icon::taskbar_icon())
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let connect = |backends: wgpu::Backends| {
            let mut descriptor =
                wgpu::InstanceDescriptor::new_with_display_handle(Box::new(event_loop.owned_display_handle()));
            descriptor.backends = backends;
            let instance = wgpu::Instance::new(descriptor);
            let surface = instance.create_surface(window.clone()).ok()?;
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            }))
            .ok()?;
            Some((instance, surface, adapter))
        };
        let wanted = match std::env::var("CODE_CANVAS_BACKEND").unwrap_or_default().to_lowercase().as_str() {
            "vulkan" => wgpu::Backends::VULKAN,
            "dx12" => wgpu::Backends::DX12,
            "metal" => wgpu::Backends::METAL,
            "gl" => wgpu::Backends::GL,
            _ if cfg!(windows) => wgpu::Backends::DX12,
            _ => wgpu::Backends::all(),
        };
        let (instance, surface, adapter) =
            connect(wanted).or_else(|| connect(wgpu::Backends::all())).expect("no suitable GPU adapter");
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?})", info.name, info.backend);
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor { label: Some("code-canvas"), ..Default::default() }),
        )
        .expect("request device");
        device.on_uncaptured_error(Arc::new(|e| log::error!("GPU error: {e}")));

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = caps.formats.iter().find(|f| !f.is_srgb()) {
            config.format = *f;
        }
        config.present_mode = if std::env::var_os("CODE_CANVAS_NO_VSYNC").is_some() {
            wgpu::PresentMode::AutoNoVsync
        } else {
            wgpu::PresentMode::AutoVsync
        };
        surface.configure(&device, &config);

        let renderer = Renderer::new(&device, &queue, config.format);
        let egui = egui::Context::default();
        ui::style(&egui);
        let egui_state = egui_winit::State::new(
            egui.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, egui_wgpu::RendererOptions::default());

        if let Some(b) = self.bench.as_mut() {
            let hz = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).map(|mhz| mhz as f32 / 1000.0);
            b.set_refresh_hz(hz);
            log::info!("benchmark mode: present {:?}, display {:?} Hz", config.present_mode, hz);
        }
        self.canvas = Rect::new(0.0, 0.0, config.width as f32, config.height as f32);
        self.canvas_full = self.canvas;
        self.gfx = Some(Gfx {
            renderer,
            egui_renderer,
            egui_state,
            egui,
            queue,
            device,
            config,
            surface,
            resized: false,
            instance,
            window,
        });

        for (path, saved) in std::mem::take(&mut self.initial) {
            let path = canvas_core::scan::clean_path(path.canonicalize().unwrap_or(path));
            if !self.pages.iter().any(|p| p.root == path) {
                self.pages.push(Page::open(self.next_id, path, saved.as_ref(), false));
                self.next_id += 1;
            }
        }
        if !self.pages.is_empty() {
            self.switch_to(self.initial_active.min(self.pages.len() - 1));
        }
        self.save_session();
        self.request_redraw();
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::CursorMoved { position, .. } => {
                let edge = self.edge_at([position.x as f32, position.y as f32]);
                if edge != self.resize_edge {
                    self.resize_edge = edge;
                    if let Some(g) = &self.gfx {
                        g.window.set_cursor(match edge {
                            Some(dir) => CursorIcon::from(dir),
                            None => CursorIcon::Default,
                        });
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let (Some(dir), Some(g)) = (self.resize_edge, &self.gfx) {
                    let _ = g.window.drag_resize_window(dir);
                    return;
                }
            }
            _ => {}
        }
        if let Some(g) = &mut self.gfx {
            if !matches!(event, WindowEvent::RedrawRequested) {
                let resp = g.egui_state.on_window_event(&g.window, &event);
                if resp.repaint {
                    g.window.request_redraw();
                }
            }
        }
        match event {
            WindowEvent::CloseRequested => self.exit_requested = true,
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut().filter(|_| can_show(size)) {
                    g.config.width = size.width;
                    g.config.height = size.height;
                    g.resized = true;
                    g.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let t = Instant::now();
                self.redraw();
                let ms = t.elapsed().as_secs_f32() * 1000.0;
                if ms > 25.0 {
                    log::debug!("slow frame: {ms:.1} ms");
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                self.last_input = Instant::now();
                self.on_key(&event)
            }
            WindowEvent::DroppedFile(path) => {
                if path.is_dir() {
                    self.open_folder(path, None);
                } else if self.pages.get(self.active).is_some() {
                    self.drop_file(path);
                } else if let Some(parent) = path.parent() {
                    self.open_folder(parent.to_path_buf(), None);
                }
                self.save_session();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = [position.x as f32, position.y as f32];
                let mut selecting = None;
                if let Some(d) = &mut self.drag {
                    let (dx, dy) = (p[0] - d.last[0], p[1] - d.last[1]);
                    d.last = p;
                    if (p[0] - d.start[0]).abs() + (p[1] - d.start[1]).abs() > 4.0 {
                        d.moved = true;
                        self.ui.card_menu = None;
                        self.sharing.following = None;
                        self.agents.following = None;
                    }
                    if let (true, Some(rel)) = (d.moved, &d.select) {
                        selecting = Some((rel.clone(), d.start, !std::mem::replace(&mut d.anchored, true)));
                    } else if d.moved {
                        if let Some(page) = self.pages.get_mut(self.active) {
                            page.target = None;
                            page.camera.pan_pixels(dx, dy);
                        }
                        self.request_redraw();
                    }
                }
                self.cursor = p;
                if let Some((rel, from, first)) = selecting {
                    self.drag_select(&rel, from, first);
                }
                self.update_hover();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = [-1.0, -1.0];
                self.update_hover();
            }
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    self.last_input = Instant::now();
                    self.webviews.focus_app();
                    self.focus_folder_at(self.cursor);
                    if self.drag.is_none() && !self.over_ui(self.cursor) {
                        let plain = button == MouseButton::Left && !self.mods.control_key() && !self.mods.shift_key();
                        let select = if plain && self.ui.prefs.edit { self.code_under(self.cursor) } else { None };
                        self.drag = Some(Drag {
                            last: self.cursor,
                            start: self.cursor,
                            moved: false,
                            button,
                            select,
                            anchored: false,
                        });
                    }
                }
                ElementState::Released => {
                    if let Some(d) = self.drag.take() {
                        if d.button != button {
                            self.drag = Some(d);
                        } else if !d.moved && button == MouseButton::Left {
                            self.on_canvas_click(self.cursor);
                        } else if !d.moved && button == MouseButton::Right {
                            self.card_menu(self.cursor);
                        } else {
                            self.session_dirty = true;
                        }
                    }
                    self.update_hover();
                }
            },
            WindowEvent::MouseWheel { delta, .. } => {
                self.focus_folder_at(self.cursor);
                let had_menu = self.ui.card_menu.take().is_some();
                if had_menu {
                    self.request_redraw();
                }
                if self.over_ui(self.cursor) && !(had_menu && self.canvas.contains(self.cursor)) {
                    return;
                }
                self.last_input = Instant::now();
                let notch = WHEEL_NOTCH * self.scale();
                let moved = match delta {
                    MouseScrollDelta::LineDelta(x, y) => [x * notch, y * notch],
                    MouseScrollDelta::PixelDelta(p) => [p.x as f32, p.y as f32],
                };
                let held = self.drag.as_mut().filter(|drag| drag.button == MouseButton::Right);
                let with_button = held.map(|drag| drag.moved = true).is_some();
                let zooming = with_button || self.mods.control_key() || self.mods.super_key();
                self.wheel(moved, self.cursor, zooming, self.mods.shift_key());
            }
            WindowEvent::PinchGesture { delta, .. } => {
                self.focus_folder_at(self.cursor);
                if self.over_ui(self.cursor) {
                    return;
                }
                self.ui.card_menu = None;
                self.share_let_go();
                let vp = self.viewport();
                let cursor = self.cursor;
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.target = None;
                    page.camera.zoom_at(cursor, vp, (1.0 + delta as f32).max(0.1));
                }
                self.update_hover();
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_requested && !self.confirm_exit() {
            self.exit_requested = false;
        }
        if self.exit_requested {
            self.save_session();
            if let Some(program) = self.restart_as.take() {
                if let Err(e) = std::process::Command::new(&program).spawn() {
                    log::error!("the new version did not start ({}): {e}", program.display());
                    self.exit_requested = false;
                    self.exit_confirmed = false;
                    let old = program.with_extension("old.exe");
                    let text = format!(
                        "The new version was installed but did not start ({e}). This one keeps running; the one before it is kept as {}.",
                        old.display()
                    );
                    self.ext.notice(ext::Level::Error, "Code Canvas", text);
                    return;
                }
            }
            event_loop.exit();
            return;
        }
        self.poll_pages();
        self.follow_theme();
        let now = Instant::now();
        if self.next_repaint.is_some_and(|t| t <= now) {
            self.next_repaint = None;
            self.request_redraw();
        }
        if self.session_dirty && self.last_save.elapsed() > Duration::from_secs(3) {
            self.save_session();
        }
        let working = self.poll_background();
        let scripting = self.run_script();
        let loading = self.pages.iter().any(|p| p.is_loading()) || working || scripting;
        let tick = now + Duration::from_millis(100);
        let wake = match (loading, self.next_repaint) {
            (true, t) => Some(t.map_or(tick, |t| t.min(tick))),
            (false, t) => t,
        };
        let shown =
            self.gfx.as_ref().is_some_and(|g| g.window.is_minimized() != Some(true) && can_show(g.window.inner_size()));
        if loading && shown {
            self.request_redraw();
        }
        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_keeps_the_names_that_have_its_letters_in_order() {
        assert!(typed_in_order("", "title"));
        assert!(typed_in_order("ti", "title"));
        assert!(typed_in_order("TL", "title"));
        assert!(typed_in_order("hm", "HashMap"));
        assert!(!typed_in_order("tx", "title"));
        assert!(!typed_in_order("titles", "title"));
        assert!(word_char('_') && word_char('ñ') && !word_char('.'));
    }
}
