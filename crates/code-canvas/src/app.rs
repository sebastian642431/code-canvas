//! Window, input and the per-frame glue between pages, the UI and the renderer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_core::layout::{NodeKind, FILE_LABEL_H};
use canvas_core::{Rect, Site, SymRef, LINE_H};
use canvas_render::{FrameStats, Label, Lod, Overlay, Renderer, ScenePass};
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
use crate::ui::{self, Action, FileOp, FrameInfo, LeftPanel, QuickOpen, RightPanel, UiState};

struct Gfx {
    renderer: Renderer,
    egui_renderer: egui_wgpu::Renderer,
    egui_state: egui_winit::State,
    egui: egui::Context,
    queue: wgpu::Queue,
    device: wgpu::Device,
    config: wgpu::SurfaceConfiguration,
    surface: wgpu::Surface<'static>,
    /// The window changed size and the surface has not been told yet. It is
    /// told once per frame: while an edge is dragged many sizes arrive
    /// between two frames, and each reconfiguration costs 15-30 ms.
    resized: bool,
    instance: wgpu::Instance,
    // Dropped last: the surface must go before the window.
    window: Arc<Window>,
}

/// The name shortcuts know a key by (`keys::Chord`). `None` for keys that
/// are not a shortcut on their own: modifiers, dead keys, media keys.
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

struct Drag {
    last: [f32; 2],
    start: [f32; 2],
    moved: bool,
    button: MouseButton,
}

/// Left panels in the order their index is saved in the session.
const LEFT_PANELS: [LeftPanel; 7] = [
    LeftPanel::Files,
    LeftPanel::Search,
    LeftPanel::Help,
    LeftPanel::Extensions,
    LeftPanel::Problems,
    LeftPanel::Changes,
    LeftPanel::Settings,
];

pub struct App {
    gfx: Option<Gfx>,
    pages: Vec<Page>,
    active: usize,
    next_id: u32,
    initial: Vec<(PathBuf, Option<PageSession>)>,
    initial_active: usize,
    window_size: Option<[f32; 2]>,
    cursor: [f32; 2],
    drag: Option<Drag>,
    mods: ModifiersState,
    last_click: Option<(Instant, [f32; 2])>,
    ui: UiState,
    /// Canvas area in physical pixels (the window minus the panels).
    canvas: Rect,
    chip_hits: Vec<(Rect, SymRef)>,
    toggle_hits: Vec<(Rect, String)>,
    /// Page whose relation curves are on the GPU.
    curves_for: Option<u32>,
    last_frame: Instant,
    frame_ms: f32,
    stats: FrameStats,
    bench: Option<Bench>,
    /// When egui asked to be repainted (animations, tooltips).
    next_repaint: Option<Instant>,
    /// Scripted steps to run (`CODE_CANVAS_SCRIPT`), for tests and demos.
    script: Option<Script>,
    /// The close button of our own title bar was pressed.
    exit_requested: bool,
    /// Edge under the pointer where a press resizes the window.
    resize_edge: Option<ResizeDirection>,
    /// Something worth persisting changed since the last save.
    session_dirty: bool,
    last_save: Instant,
    /// Installed VS Code extensions and the process that runs them.
    ext: Extensions,
    /// When an edit last changed a file: the symbol index is rebuilt once
    /// the typing pauses.
    symbols_stale: Option<Instant>,
    /// An edit the extensions have not been told about yet, and when it
    /// happened: they get the text after a short pause, not per keystroke.
    unsent_edit: Option<(PathBuf, Instant)>,
    /// The code cell the pointer rests on and since when: the language
    /// extensions are asked about it once it has stayed there a moment.
    hover_probe: Option<((u32, u32, u32), Instant, bool)>,
    /// Reports files that change on disk.
    watcher: crate::watch::Watcher,
    /// When each page was last rescanned because of such changes.
    auto_rescan: Option<Instant>,
    /// Browser views showing extensions' webviews.
    webviews: crate::webview::WebViews,
    /// Folders webview pages may load files from (to notice changes).
    webview_roots: Vec<PathBuf>,
    /// The theme (`theme::revision`) the interface and the scenes were last
    /// built with.
    theme_revision: u64,
    /// The choice of theme changed since it was last written to disk.
    theme_dirty: bool,
}

impl App {
    /// Restores the previous session and adds `paths` (from the command
    /// line) to it.
    pub fn new(paths: Vec<PathBuf>, waker: ext::Waker) -> Self {
        let saved: Session = session::load();
        // The previous session always comes back; folders given on the
        // command line are added to it (and the first one is shown).
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
        // The colours first: everything built from here on takes them.
        let chosen = crate::themes::load();
        canvas_render::theme::set(chosen.resolve());
        let mut ext = Extensions::new(session::data_dir(), waker);
        // Not in scripted or benchmark runs: those must not reach for the network.
        if std::env::var_os("CODE_CANVAS_SCRIPT").is_none() && std::env::var_os("CODE_CANVAS_BENCH").is_none() {
            ext.install_defaults();
        }
        let mut ui = UiState::default();
        ui.theme = chosen;
        ui.commit_ai = crate::commit_ai::load();
        ui.key_file = crate::keys::load();
        ui.keys = Keymap::new(&ui.key_file);
        ui.last_left = LEFT_PANELS[(saved.left_panel as usize).min(LEFT_PANELS.len() - 1)];
        ui.last_right = RightPanel::Symbols;
        ui.left = saved.left_open.then_some(ui.last_left);
        ui.right = saved.right_open.then_some(ui.last_right);
        Self {
            gfx: None,
            pages: Vec::new(),
            active: 0,
            next_id: 1,
            initial,
            initial_active,
            window_size: saved.window,
            cursor: [-1.0, -1.0],
            drag: None,
            mods: ModifiersState::empty(),
            last_click: None,
            ui,
            canvas: Rect::new(0.0, 0.0, 1.0, 1.0),
            chip_hits: Vec::new(),
            toggle_hits: Vec::new(),
            curves_for: None,
            last_frame: Instant::now(),
            frame_ms: 0.0,
            stats: FrameStats::default(),
            bench: Bench::from_env(),
            next_repaint: None,
            script: Script::from_env(),
            exit_requested: false,
            resize_edge: None,
            session_dirty: false,
            last_save: Instant::now(),
            ext,
            symbols_stale: None,
            unsent_edit: None,
            hover_probe: None,
            watcher: crate::watch::Watcher::new(),
            auto_rescan: None,
            webviews: crate::webview::WebViews::new(),
            webview_roots: Vec::new(),
            theme_revision: canvas_render::theme::revision(),
            theme_dirty: false,
        }
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

    /// Is this physical-pixel position over the UI rather than the canvas?
    fn over_ui(&self, p: [f32; 2]) -> bool {
        if !self.canvas.contains(p) {
            return true;
        }
        let s = self.scale();
        let pt = egui::Pos2::new(p[0] / s, p[1] / s);
        self.ui.floating.iter().any(|r| r.contains(pt))
    }

    /// The user chose a theme or changed a colour (`self.ui.theme`): use it.
    /// What is on screen follows in [`App::follow_theme`].
    fn theme_changed(&mut self) {
        canvas_render::theme::set(self.ui.theme.resolve());
        self.theme_dirty = true;
        self.request_redraw();
    }

    /// Bring everything that keeps colours of its own up to date with the
    /// theme in use: egui's look, the web pages and the pages' GPU scenes
    /// (the renderer sees to its own buffers).
    fn follow_theme(&mut self) {
        // Pages that finished loading since the theme changed get it now.
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
        let window = self.gfx.as_ref().map(|g| {
            let size = g.window.inner_size().to_logical::<f32>(g.window.scale_factor());
            [size.width, size.height]
        });
        session::save(&Session {
            pages: self.pages.iter().map(|p| p.to_session(s)).collect(),
            active: self.active,
            left_open: self.ui.left.is_some(),
            right_open: self.ui.right.is_some(),
            left_panel: LEFT_PANELS.iter().position(|p| *p == self.ui.last_left).unwrap_or(0) as u8,
            right_panel: 0,
            window,
        });
    }

    /// The window edge or corner under `p`, where dragging resizes.
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

    // ----- pages -------------------------------------------------------

    /// A file was dropped on the window while a folder is open: put a copy
    /// of it there. It goes into the folder selected on the canvas or in the
    /// explorer (the one a selected file is in), else the open folder itself.
    /// Nothing is ever overwritten; the file watcher shows it once it is in.
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
        match crate::files::copy_into(file, &dir) {
            Ok(_) => {
                let text = format!("Copying \"{shown}\" into {}", place.display());
                self.ext.notice(ext::Level::Info, "files", text)
            }
            Err(why) if why.is_empty() => {}
            Err(why) => self.ext.notice(ext::Level::Warn, "files", format!("\"{shown}\" was not copied: {why}.")),
        }
        self.request_redraw();
    }

    /// Something chosen in the explorer's menu. Paths are relative to the
    /// open folder and cannot leave it (`files::inside`).
    fn file_op(&mut self, op: FileOp) {
        let Some(root) = self.pages.get(self.active).map(|p| p.root.clone()) else { return };
        let at = |rel: &str| crate::files::inside(&root, rel);
        let done: Result<(), String> = match op {
            FileOp::Reveal(path) => at(&path).and_then(|p| crate::files::reveal(&p)),
            FileOp::Terminal(dir) => at(&dir).and_then(|p| crate::files::terminal(&p)),
            FileOp::CopyPath { path, relative } => at(&path).and_then(|full| {
                // With the separators of this system, whatever the tree uses.
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
            FileOp::Delete(path) => at(&path).and_then(|p| {
                let answer = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Warning)
                    .set_title("Code Canvas")
                    .set_description(format!("Move \"{path}\" to the recycle bin?"))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                match answer {
                    rfd::MessageDialogResult::Yes => crate::files::recycle(&p),
                    _ => Ok(()),
                }
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
    }

    fn pick_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new().set_title("Open a folder in a new canvas").pick_folder() {
            self.open_folder(path, None);
            self.save_session();
        }
    }

    fn switch_to(&mut self, i: usize) {
        if i < self.pages.len() {
            self.active = i;
            // A folder restored from the last session is read when first shown.
            self.pages[i].ensure_scanned();
            self.drag = None;
            self.update_title();
            self.request_redraw();
        }
    }

    fn close_page(&mut self, i: usize) {
        if i >= self.pages.len() {
            return;
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
            self.switch_to(((self.active as isize + delta).rem_euclid(n)) as usize);
        }
    }

    fn rescan(&mut self) {
        let s = self.scale();
        if let Some(p) = self.pages.get_mut(self.active) {
            if !p.is_loading() {
                // Keep looking at the same place once the scan finishes.
                p.restore_camera = p.framed.then_some(Camera { center: p.camera.center, zoom: p.camera.zoom / s });
                p.framed = false;
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

    // ----- files and extensions ----------------------------------------

    /// The page that holds `path` and the path relative to its root.
    fn locate_file(&self, path: &Path) -> Option<(usize, String)> {
        let key = ext::path_key(&path.to_string_lossy());
        self.pages.iter().enumerate().find_map(|(i, p)| {
            let mut root = ext::path_key(&p.root.to_string_lossy());
            root.push('/');
            // The key differs from the path in case only, so lengths match.
            key.starts_with(&root).then(|| (i, path.to_string_lossy().replace('\\', "/")[root.len()..].to_string()))
        })
    }

    /// Replace a file's contents on disk (a formatter or a fix produced
    /// them) and show the new contents.
    fn write_file(&mut self, path: &Path, text: &str) {
        self.watcher.expect(path);
        if let Err(e) = std::fs::write(path, text) {
            log::error!("cannot write {}: {e}", path.display());
            return;
        }
        log::info!("wrote {} ({} bytes)", path.display(), text.len());
        self.ext.file_changed(path);
        if let Some((i, rel)) = self.locate_file(path) {
            if let Some(page) = self.pages.get_mut(i) {
                // An open editor without changes of its own takes the new text.
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

    /// F12: where is the name at the caret (or under the pointer) defined?
    /// Language extensions answer exactly; without one, the built-in index does.
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
        let path = page.root.join(&r.repo.files[file as usize].rel_path);
        let by_index = r.repo.resolve_at(file, line, col).map(|(sym, _)| r.repo.sym_site(sym));
        if !self.ext.request_definition(&path, line, col) {
            if let Some(site) = by_index {
                self.frame_site(site);
            }
        }
    }

    // ----- changes on disk ---------------------------------------------------

    /// Files changed outside the app: show them as they are now. A few
    /// files are read again one by one; many, or new and removed ones, mean
    /// the folder is scanned again.
    fn on_disk_changes(&mut self, batch: crate::watch::Batch) {
        const ONE_BY_ONE: usize = 24;
        log::debug!(
            "changed on disk: {} files {:?}, git of {} folders",
            batch.files.len(),
            batch.files.iter().take(4).collect::<Vec<_>>(),
            batch.git.len()
        );
        let skip = canvas_core::ScanOptions::default().skip_dirs;
        for root in &batch.git {
            if let Some(page) = self.pages.iter_mut().find(|p| &p.root == root) {
                page.refresh_git();
            }
        }
        // Per page: files already on the canvas, and paths that are not.
        let mut known: Vec<Vec<String>> = vec![Vec::new(); self.pages.len()];
        let mut other: Vec<Vec<String>> = vec![Vec::new(); self.pages.len()];
        for path in &batch.files {
            let Some((i, rel)) = self.locate_file(path) else { continue };
            // Build output and dependencies are not on the canvas.
            if rel.split('/').any(|part| skip.iter().any(|s| s == part)) {
                continue;
            }
            let on_canvas = self.pages[i].ready().is_some_and(|r| r.repo.file_index(&rel).is_some());
            match (on_canvas, path.is_file()) {
                (true, true) => known[i].push(rel),
                (true, false) => other[i].push(rel), // removed
                (false, true) => other[i].push(rel), // new
                (false, false) => {}                 // a folder, or already gone
            }
        }
        for i in 0..self.pages.len() {
            let root = self.pages[i].root.clone();
            // New paths count only when git would not ignore them.
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
                let (big, recent) = (
                    self.pages[i].ready().is_some_and(|r| r.repo.files.len() > 60_000),
                    self.auto_rescan.is_some_and(|t| t.elapsed() < Duration::from_secs(4)),
                );
                // A huge folder takes seconds to scan: that stays the user's call (F5).
                if !big && !recent && !self.pages[i].is_loading() && self.pages[i].editors.values().all(|e| !e.dirty) {
                    log::info!("{}: files were added or removed, scanning again", root.display());
                    self.auto_rescan = Some(Instant::now());
                    let s = self.scale();
                    let p = &mut self.pages[i];
                    p.restore_camera = p.framed.then_some(Camera { center: p.camera.center, zoom: p.camera.zoom / s });
                    p.framed = false;
                    p.editors.clear();
                    p.editing = None;
                    p.start_scan();
                }
                continue;
            }
            for rel in std::mem::take(&mut known[i]) {
                let page = &mut self.pages[i];
                // What is being typed here wins over what changed there.
                if page.editors.get(&rel).is_some_and(|e| e.dirty) {
                    continue;
                }
                let caret = page.editors.remove(&rel).map(|e| e.caret);
                if let Some(g) = self.gfx.as_mut() {
                    g.renderer.text.drop_page(page.id);
                    page.reload_file(&rel, &g.renderer, &g.device, &self.ui.filters);
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
        self.request_redraw();
    }

    // ----- editing ---------------------------------------------------------

    /// The text of the file being edited changed: show it.
    fn after_edit(&mut self) {
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let Some(rel) = page.editing.clone() else { return };
        if let Some(g) = self.gfx.as_mut() {
            g.renderer.text.drop_page(page.id);
            page.show_edit(&rel, &g.renderer, &g.device, &self.ui.filters);
        }
        self.symbols_stale = Some(Instant::now());
        self.unsent_edit = Some((page.root.join(&rel), Instant::now()));
        self.keep_caret_visible();
        self.request_redraw();
    }

    /// Pan just enough for the caret to be on screen.
    fn keep_caret_visible(&mut self) {
        let (vp, area) = (self.viewport(), self.canvas);
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let Some(editor) = page.editor() else { return };
        let (line, col, rel) =
            (editor.caret.line as u32, editor.display_col(editor.caret) as u32, editor.rel_path.clone());
        let Some(r) = page.ready() else { return };
        let node = r.repo.file_index(&rel).and_then(|fi| r.layout.file_node.get(fi).copied());
        let Some(n) = node.filter(|&n| n != canvas_core::layout::NONE).map(|n| &r.layout.nodes[n as usize]) else {
            return;
        };
        let cell = n.cell_rect(line, col, 1);
        let view = page.camera.view(vp);
        let (tl, br) = (view.to_screen([cell.x, cell.y]), view.to_screen([cell.right(), cell.bottom()]));
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

    /// Write the file being edited, then let the extensions format it if
    /// the settings say so.
    fn save_edit(&mut self) {
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let Some(rel) = page.editing.clone() else { return };
        let path = page.root.join(&rel);
        let Some(editor) = page.editors.get_mut(&rel) else { return };
        self.watcher.expect(&path);
        if let Err(e) = std::fs::write(&path, editor.text()) {
            log::error!("cannot save {}: {e}", path.display());
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Code Canvas")
                .set_description(format!("{} could not be saved:\n{e}", path.display()))
                .show();
            return;
        }
        editor.mark_saved();
        self.unsent_edit = None;
        log::info!("saved {}", path.display());
        self.ext.file_changed(&path);
        if let Some(g) = self.gfx.as_mut() {
            // From disk again: the symbol index and git see the saved file.
            g.renderer.text.drop_page(page.id);
            page.reload_file(&rel, &g.renderer, &g.device, &self.ui.filters);
        }
        page.refresh_git();
        self.symbols_stale = None;
        self.ext.format_on_save(&path);
        self.request_redraw();
    }

    /// Files with unsaved edits, over every folder.
    fn unsaved(&self) -> Vec<(usize, String)> {
        let dirty = |i: usize, p: &Page| {
            p.editors.values().filter(|e| e.dirty).map(|e| (i, e.rel_path.clone())).collect::<Vec<_>>()
        };
        self.pages.iter().enumerate().flat_map(|(i, p)| dirty(i, p)).collect()
    }

    /// Before leaving: offer to save what is not saved. False cancels leaving.
    fn confirm_exit(&mut self) -> bool {
        let unsaved = self.unsaved();
        if unsaved.is_empty() {
            return true;
        }
        let names: Vec<&str> = unsaved.iter().take(8).map(|(_, rel)| rel.as_str()).collect();
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Code Canvas")
            .set_description(format!("Save the changes to {} file(s)?\n\n{}", unsaved.len(), names.join("\n")))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match answer {
            rfd::MessageDialogResult::Yes => {
                for (i, rel) in unsaved {
                    if let Some(page) = self.pages.get_mut(i) {
                        if let Some(editor) = page.editors.get_mut(&rel) {
                            match std::fs::write(page.root.join(&rel), editor.text()) {
                                Ok(()) => editor.mark_saved(),
                                Err(e) => log::error!("cannot save {rel}: {e}"),
                            }
                        }
                    }
                }
                self.unsaved().is_empty()
            }
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    /// Keys while the caret is in a file. True when the key was used.
    fn edit_key(&mut self, event: &KeyEvent) -> bool {
        // AltGr reports Ctrl+Alt: that is typing (`@`, `{`, `[` on many layouts), not a shortcut.
        let cmd = (self.mods.control_key() && !self.mods.alt_key()) || self.mods.super_key();
        let shift = self.mods.shift_key();
        let Some(page) = self.pages.get_mut(self.active) else { return false };
        let Some(editor) = page.editor_mut() else { return false };
        let mut changed = false;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                page.editing = None;
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
                // Other shortcuts (Ctrl+P, Ctrl+O…) keep their meaning.
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
        true
    }

    fn reveal_file(&mut self, path: &Path) {
        if let Some((i, rel)) = self.locate_file(path) {
            self.switch_to(i);
            self.apply(Action::FlyToPath(rel));
        }
    }

    /// The file the user means: under the pointer, else the selected one.
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
            None => log::info!("format: point at a file or select one first"),
        }
        self.request_redraw();
    }

    /// Tell the extensions which files are readable on screen: those are
    /// their open documents (what linters check).
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

    // ----- camera helpers ---------------------------------------------

    fn frame_node(&mut self, node: usize) {
        let (area, vp, s) = (self.canvas, self.viewport(), self.scale());
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let Some(n) = r.layout.nodes.get(node) else { return };
        let target = if n.is_dir() {
            Camera::fit(n.rect, area, vp, 24.0 * s)
        } else {
            // Files: readable width, top of the file at the top of the screen.
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

    /// Fly to a place in the code, zooming in enough to read it.
    fn frame_site(&mut self, site: Site) {
        let (area, vp, s) = (self.canvas, self.viewport(), self.scale());
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let Some((p, exact)) = r.layout.anchor(&site) else { return };
        let z = if exact { page.camera.zoom.max(1.25 * s) } else { page.camera.zoom };
        // Put the site a third of the way into the canvas.
        let (sx, sy) = (area.x + area.w * 0.3, area.y + area.h * 0.35);
        page.target = Some(Camera { center: [p[0] - (sx - vp[0] * 0.5) / z, p[1] - (sy - vp[1] * 0.5) / z], zoom: z });
        self.request_redraw();
    }

    fn frame_all(&mut self) {
        self.frame_node(0);
    }

    /// Frame the selected symbols and everything related to them, or the
    /// selected node when no symbol is selected.
    fn frame_selection(&mut self) {
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
                // Re-centre after clamping the zoom.
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

    // ----- symbols ------------------------------------------------------

    fn select_symbol(&mut self, sym: SymRef, clicked: Option<Site>, toggle: bool) {
        let Some(page) = self.pages.get_mut(self.active) else { return };
        if !toggle && page.selections.iter().any(|s| s.sym == sym) {
            return;
        }
        page.toggle_symbol(sym, clicked);
        if page.only_connected {
            self.relayout();
        }
        self.ui.show_right(RightPanel::Symbols);
        self.request_redraw();
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

    // ----- input --------------------------------------------------------

    fn on_canvas_click(&mut self, pos: [f32; 2]) {
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, p)| {
            now.duration_since(t) < Duration::from_millis(400)
                && (p[0] - pos[0]).abs() < 6.0
                && (p[1] - pos[1]).abs() < 6.0
        });
        self.last_click = if double { None } else { Some((now, pos)) };

        // Name chips select the symbol they show.
        if let Some(&(_, sym)) = self.chip_hits.iter().find(|(r, _)| r.contains(pos)) {
            self.select_symbol(sym, None, false);
            return;
        }
        // Folder collapse markers.
        if let Some(path) = self.toggle_hits.iter().find(|(r, _)| r.contains(pos)).map(|(_, p)| p.clone()) {
            self.apply(Action::ToggleCollapsed(path));
            return;
        }

        let vp = self.viewport();
        let (ctrl, shift) = (self.mods.control_key() || self.mods.super_key(), self.mods.shift_key());
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let world = page.camera.view(vp).to_world(pos);
        let hit = r.layout.hit(world);
        let zoom = page.camera.zoom;
        let readable = Lod::for_zoom(zoom) == Lod::Text;

        // What is under the pointer, worked out before anything changes.
        let mut caret_at = None; // (file path, line, display column)
        let mut picked = None; // a symbol and, when it was a use of it, where
        if let Some(n) = hit.map(|h| &r.layout.nodes[h]) {
            if let NodeKind::File(fi) = n.kind {
                let f = &r.repo.files[fi];
                if let Some((line, col)) = n.cell_at(world, f.line_count()) {
                    if readable && !ctrl && f.note.is_none() {
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

        page.selected = hit;
        // Readable code is an editor: a click puts the caret there (Shift
        // extends the selection, a double click takes the word). Ctrl+click
        // asks where the name is used, as in VS Code.
        if let Some((rel, line, col)) = caret_at {
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
        // A click anywhere else leaves the editor (what was typed is kept).
        if !ctrl {
            page.editing = None;
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
        let cmd = self.mods.control_key() || self.mods.super_key();
        let (shift, alt) = (self.mods.shift_key(), self.mods.alt_key());
        let typing = self.gfx.as_ref().is_some_and(|g| g.egui.egui_wants_keyboard_input());
        let chord = key_name(&event.logical_key).map(|key| Chord::new(&key, cmd, shift, alt));

        // Settings is waiting for the new shortcut of a command: this key is
        // it. Esc gives up; a modifier on its own, or a key that cannot be
        // used for that command, keeps it waiting.
        if self.ui.left != Some(LeftPanel::Settings) {
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
        // Some shortcuts work wherever the keyboard is, a text field included.
        if let Some(info) = bound.filter(|i| i.scope == Scope::Anywhere) {
            self.run_command(info.command);
            return;
        }
        if typing || self.ui.quick.is_some() {
            return;
        }
        // The caret is in a file: keys go to the editor first. A shortcut
        // with Alt is the exception (Shift+Alt+F formats, it does not type);
        // Ctrl+Alt is AltGr, which does type.
        let before_editor = bound.is_some() && alt && !cmd;
        if !before_editor && self.edit_key(event) {
            return;
        }
        if let Some(info) = bound {
            self.run_command(info.command);
            return;
        }
        let Some(chord) = chord else { return };
        if self.fixed_chord(&chord) {
            return;
        }
        if chord.key == "escape" {
            if self.pages.get(self.active).is_some_and(|p| !p.selections.is_empty()) {
                self.clear_symbols();
            } else if let Some(p) = self.pages.get_mut(self.active) {
                p.selected = None;
                self.request_redraw();
            }
        }
    }

    /// What a shortcut does (`keys::COMMANDS` says which keys it has).
    fn run_command(&mut self, command: Command) {
        match command {
            Command::Palette => self.ui.quick = Some(QuickOpen { query: ">".into(), selected: 0, focus: true }),
            Command::GoToFile => self.ui.quick = Some(QuickOpen { query: String::new(), selected: 0, focus: true }),
            Command::SearchInFiles => self.ui.show_left(LeftPanel::Search),
            Command::Settings => self.ui.show_left(LeftPanel::Settings),
            Command::ToggleLeftPanel => self.ui.toggle_left(),
            Command::FormatFile => self.format_file(None),
            Command::GoToDefinition => self.go_to_definition(),
            Command::OpenFolder => self.pick_folder(),
            Command::CloseFolder => self.close_page(self.active),
            Command::NextFolder => self.cycle(1),
            Command::PreviousFolder => self.cycle(-1),
            Command::Rescan => self.rescan(),
            Command::ShowAll => self.frame_all(),
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

    /// Shortcuts that are not in the table: Ctrl+1 … Ctrl+9 go to that canvas.
    fn fixed_chord(&mut self, chord: &Chord) -> bool {
        match chord.key.as_bytes() {
            [digit @ b'1'..=b'9'] if chord.ctrl && !chord.alt => {
                self.switch_to((digit - b'1') as usize);
                true
            }
            _ => false,
        }
    }

    /// A chord pressed while a web page (an extension's panel) had the
    /// keyboard: the same shortcuts as in the window, except the canvas's
    /// own, which are not for a page that is being typed in.
    fn page_chord(&mut self, k: &crate::webview::PageKey) {
        let chord = Chord::new(&k.key, k.ctrl, k.shift, k.alt);
        match self.ui.keys.command(&chord).map(Command::info) {
            Some(info) if info.scope != Scope::Canvas => {
                // These open a text field of the app: typing must reach it.
                if matches!(info.command, Command::Palette | Command::GoToFile | Command::SearchInFiles) {
                    self.webviews.focus_app();
                }
                self.run_command(info.command);
            }
            Some(_) => {}
            None => {
                self.fixed_chord(&chord);
            }
        }
    }

    /// The user changed a shortcut (`self.ui.key_file`): use it and save it.
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

    // ----- UI actions -----------------------------------------------------

    fn apply(&mut self, action: Action) {
        if !matches!(action, Action::HoverPath(_) | Action::DragWindow | Action::Minimize | Action::CloseWindow) {
            self.session_dirty = true;
        }
        match action {
            Action::SwitchTab(i) => self.switch_to(i),
            Action::CloseTab(i) => self.close_page(i),
            Action::OpenFolder => self.pick_folder(),
            Action::Rescan => self.rescan(),
            Action::Fit => self.frame_all(),
            Action::ZoomBy(f) => self.zoom_center(f),
            Action::FrameSelection => self.frame_selection(),
            Action::FlyToSite(site) => self.frame_site(site),
            Action::FlyToPath(path) => {
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
                    // A merged chain `a/b/c` may have been collapsed through any of its folders.
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
            Action::ShowLeft(panel) => {
                self.ui.show_left(panel);
                self.request_redraw();
            }
            Action::SaveFile => self.save_edit(),
            Action::RunCommand(id, title) => {
                self.ext.run_command(&id, &title);
                self.request_redraw();
            }
            Action::Git(op) => {
                // Discarding cannot be undone: ask first.
                let lost = match &op {
                    crate::git::GitOp::Discard(path) => Some(format!("the changes in {path}")),
                    crate::git::GitOp::DiscardAll => Some("ALL the changes in tracked files".to_string()),
                    _ => None,
                };
                if let Some(what) = lost {
                    let answer = rfd::MessageDialog::new()
                        .set_level(rfd::MessageLevel::Warning)
                        .set_title("Code Canvas")
                        .set_description(format!("Discard {what}?\n\nThis cannot be undone."))
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show();
                    if answer != rfd::MessageDialogResult::Yes {
                        return;
                    }
                }
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.run_git(op);
                }
                self.request_redraw();
            }
            Action::WriteCommitMessage => {
                let settings = self.ui.commit_ai.clone();
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.write_message(settings);
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
            Action::SetOnlyChanged(on) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.only_changed = on;
                }
                self.relayout();
                self.frame_all();
            }
            Action::SetTheme(key) => {
                self.ui.theme.set_base(key);
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
                let picked = rfd::FileDialog::new()
                    .set_title("Install an extension from a .vsix file")
                    .add_filter("VS Code extension", &["vsix"])
                    .pick_file();
                if let Some(file) = picked {
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

    // ----- scripted driving -------------------------------------------------

    /// Run due script steps. Returns true while the script still has work.
    fn run_script(&mut self) -> bool {
        let Some(script) = self.script.as_mut() else { return false };
        // Wait for the page to load and the camera to settle between steps.
        let busy = self.pages.get(self.active).is_some_and(|p| p.is_loading() || p.target.is_some() || !p.framed);
        if busy {
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
                self.ui.show_left(LeftPanel::Search);
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
                "files" => self.ui.show_left(LeftPanel::Files),
                "search" => self.ui.show_left(LeftPanel::Search),
                "help" => self.ui.show_left(LeftPanel::Help),
                "extensions" => self.ui.show_left(LeftPanel::Extensions),
                "problems" => self.ui.show_left(LeftPanel::Problems),
                "changes" => self.ui.show_left(LeftPanel::Changes),
                "settings" => self.ui.show_left(LeftPanel::Settings),
                id if id.contains('.') => {
                    // `left publisher.name`: that extension's own panel.
                    self.ui.extension = id.to_string();
                    self.ui.show_left(LeftPanel::Extension);
                }
                _ => self.ui.left = None,
            },
            Step::Install(id) => match id.split_once('.') {
                Some((namespace, name)) => self.ext.install(namespace, name),
                None => log::warn!("script: install needs publisher.name, got '{id}'"),
            },
            Step::ExtSearch(text) => {
                self.ui.show_left(LeftPanel::Extensions);
                self.ext.query = text;
                self.ext.search();
            }
            Step::Format(path) => self.format_file(Some(path)),
            Step::Command(id) => self.ext.run_command(&id, &id),
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
            }
            Step::Key(name) => {
                let mut save = false;
                if let Some(page) = self.pages.get_mut(self.active) {
                    if name == "escape" {
                        page.editing = None;
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
                            "save" => save = true,
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
                    _ => None,
                };
                match op {
                    Some(op) => self.apply(Action::Git(op)),
                    None => log::warn!("script: git needs stage|unstage [path] or commit <message>"),
                }
            }
            Step::Right(which) => match which.as_str() {
                "symbols" => self.ui.show_right(RightPanel::Symbols),
                "deps" => self.ui.show_right(RightPanel::Deps),
                key if !key.is_empty() && key != "none" => {
                    // `right <view id or extension id>`: that extension's webview.
                    let items = self.ext.rail_items();
                    let found = items.iter().find(|w| w.view == key || w.extension == key).map(|w| w.view.clone());
                    let open = self.ext.webviews.iter().find(|w| w.key == key || w.extension == key);
                    match found.or(open.map(|w| w.key.clone())) {
                        Some(k) => {
                            self.ui.webview = k;
                            self.ui.right = Some(RightPanel::Webview);
                        }
                        None => log::warn!("script: no webview '{key}'"),
                    }
                }
                _ => self.ui.right = None,
            },
            Step::CollapseAll => self.apply(Action::CollapseAll),
            Step::ExpandAll => self.apply(Action::ExpandAll),
            Step::Connected(on) => self.apply(Action::SetOnlyConnected(on)),
            Step::Tab(n) => self.switch_to(n.saturating_sub(1)),
            Step::Folders => {
                self.ui.show_left(LeftPanel::Files);
                self.ui.open_folder_menu = true;
            }
            Step::Zoom(f) => self.apply(Action::ZoomBy(f)),
            Step::Quick(text) => self.ui.quick = Some(QuickOpen { query: text, selected: 0, focus: true }),
            Step::Resize(w, h) => {
                if let Some(g) = &self.gfx {
                    let _ = g.window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
                }
            }
            Step::Palette(text) => {
                self.ui.quick = Some(QuickOpen { query: format!(">{text}"), selected: 0, focus: true })
            }
            Step::Maximize => self.apply(Action::ToggleMaximize),
            Step::Quit => self.exit_requested = true,
        }
        self.request_redraw();
        true
    }

    // ----- background work -------------------------------------------------

    /// Pull finished work from background helpers. Returns true while any
    /// of them is still running (so the UI keeps ticking).
    fn poll_background(&mut self) -> bool {
        let mut busy = false;
        let mut changed = false;
        // Editing leaves the symbol index behind; catch up when typing pauses.
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
        // Tell the extensions what was typed, once the typing pauses.
        if self.unsent_edit.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(250)) {
            if let Some((path, _)) = self.unsent_edit.take() {
                let text =
                    self.locate_file(&path).and_then(|(i, rel)| self.pages[i].editors.get(&rel).map(|e| e.text()));
                if let Some(text) = text {
                    self.ext.file_edited(&path, &text);
                }
            }
        }
        busy |= self.unsent_edit.is_some();
        // Source control: results of `git status` runs.
        let mut git_relayout = false;
        for (i, page) in self.pages.iter_mut().enumerate() {
            changed |= page.poll_git_op();
            changed |= page.poll_message();
            if page.poll_git() {
                changed = true;
                git_relayout |= i == self.active && page.only_changed;
            }
        }
        if git_relayout {
            self.relayout();
        }
        // The timeline follows the selected file.
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
        // What the language extensions know about the code under the pointer.
        let cell = self
            .pages
            .get(self.active)
            .and_then(|p| p.hover_cell.filter(|_| Lod::for_zoom(p.camera.zoom) == Lod::Text));
        match (cell, &mut self.hover_probe) {
            (None, probe) => *probe = None,
            (Some(c), Some((at, since, asked))) if *at == c => {
                if !*asked && since.elapsed() > Duration::from_millis(350) {
                    *asked = true;
                    let path = self.pages.get(self.active).and_then(|p| {
                        let r = p.ready()?;
                        Some(p.root.join(&r.repo.files.get(c.0 as usize)?.rel_path))
                    });
                    if let Some(path) = path {
                        self.ext.request_hover(&path, c.1, c.2);
                    }
                }
                busy |= !*asked;
            }
            (Some(c), probe) => {
                *probe = Some((c, Instant::now(), false));
                busy = true;
            }
        }
        // Files changed on disk by someone else.
        let folders: Vec<PathBuf> = self.pages.iter().filter(|p| !p.is_waiting()).map(|p| p.root.clone()).collect();
        self.watcher.set_folders(&folders);
        if let Some(batch) = self.watcher.poll() {
            self.on_disk_changes(batch);
            changed = true;
        }
        busy |= self.watcher.busy();
        // Extensions: their workspace is the set of open folders.
        self.ext.set_folders(self.pages.iter().filter(|p| !p.is_waiting()).map(|p| p.root.clone()).collect());
        let (events, ext_changed) = self.ext.poll();
        changed |= ext_changed;
        for event in events {
            match event {
                ext::Event::Write { path, text } => self.write_file(&path, &text),
                ext::Event::Reveal(path) => self.reveal_file(&path),
                ext::Event::GoTo { path, line, col, len } => {
                    if let Some((i, rel)) = self.locate_file(&path) {
                        self.switch_to(i);
                        let file = self.pages[i].ready().and_then(|r| r.repo.file_index(&rel));
                        match file {
                            Some(file) => {
                                let site = Site {
                                    file: file as u32,
                                    line,
                                    col: col.min(u16::MAX as u32) as u16,
                                    len: len.min(400) as u16,
                                };
                                self.frame_site(site);
                                if let Some(page) = self.pages.get_mut(i) {
                                    page.search = Some(crate::page::SearchResults::single(site));
                                }
                            }
                            None => log::info!("definition in {} is not on the canvas", path.display()),
                        }
                    }
                }
                ext::Event::WebviewHtml { handle, html } => self.webviews.set_html(&handle, html),
                ext::Event::WebviewPost { handle, message } => self.webviews.post(&handle, &message),
                ext::Event::WebviewClosed(handle) => self.webviews.close(&handle),
                ext::Event::ShowWebview(key) => {
                    self.ui.webview = key;
                    self.ui.right = Some(RightPanel::Webview);
                }
            }
        }
        // Webview pages: what they may load, and what they sent.
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
        // Notices fade out by themselves: keep ticking while one is up.
        busy |= !self.ext.notices.is_empty();
        if changed {
            self.request_redraw();
        }
        busy
    }

    // ----- frame ----------------------------------------------------------

    fn poll_pages(&mut self) {
        let Some(g) = &mut self.gfx else { return };
        let mut changed = false;
        for page in &mut self.pages {
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
                page.curves_dirty = true;
                changed = true;
            }
        }
        if changed {
            g.window.request_redraw();
        }
    }

    /// Status text and canvas tooltip for the frame.
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
                let overview = || {
                    format!(
                        "{}  ·  {} files  ·  {} lines  ·  {} symbols  ·  loaded in {} ms",
                        r.repo.root.display(),
                        r.repo.files.len(),
                        r.repo.total_lines,
                        r.repo.symbols.count,
                        r.load_ms
                    )
                };
                if let Some(editor) = page.editor() {
                    let state = if editor.dirty { "  ·  not saved (Ctrl+S)" } else { "" };
                    let place = format!("Ln {}, Col {}", editor.caret.line + 1, editor.display_col(editor.caret) + 1);
                    return (format!("Editing {}  ·  {place}{state}  ·  Esc to leave", editor.rel_path), None);
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
                        let sym = page.hover_cell.and_then(|(file, line, _)| r.repo.sym_at(file, line));
                        let problem = page.hover_cell.filter(|_| on_canvas).and_then(|(_, line, col)| {
                            let key = ext::path_key(&page.root.join(&f.rel_path).to_string_lossy());
                            let found = self.ext.diagnostics.get(&key)?;
                            found.items.iter().filter(|d| d.covers(line, col)).min_by_key(|d| d.severity)
                        });
                        if let Some(d) = problem {
                            return (status, Some((d.title(), d.message.clone())));
                        }
                        let known = page.hover_cell.filter(|_| on_canvas).and_then(|(_, line, col)| {
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
        // The latest size, applied once however many arrived since the last frame.
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

        // 1. The UI. It only produces actions; they are applied right after,
        //    so the canvas drawn below already reflects them.
        let (status, tooltip) = self.describe();
        let zoom = self.pages.get(self.active).map(|p| p.camera.zoom / s).unwrap_or(1.0);
        let mut actions = Vec::new();
        let (full, canvas_pts) = {
            let g = self.gfx.as_mut().unwrap();
            let raw = g.egui_state.take_egui_input(&g.window);
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
            self.canvas =
                Rect::new(canvas_pts.left() * s, canvas_pts.top() * s, canvas_pts.width() * s, canvas_pts.height() * s);
        }
        for a in actions {
            self.apply(a);
        }
        // The webview on screen, if any: a browser view over its panel.
        {
            let shown =
                (self.ui.right == Some(RightPanel::Webview)).then_some(()).and(self.ui.webview_rect).and_then(|r| {
                    let entry = self.ext.webviews.iter().find(|w| w.key == self.ui.webview)?;
                    Some((entry.handle.clone()?, Rect::new(r.left() * s, r.top() * s, r.width() * s, r.height() * s)))
                });
            let g = self.gfx.as_ref().unwrap();
            let t = Instant::now();
            self.webviews.show(&g.window, shown.as_ref().map(|(h, r)| (h.as_str(), *r)));
            let ms = t.elapsed().as_secs_f32() * 1000.0;
            if ms > 8.0 {
                log::debug!("placing the webview took {ms:.1} ms");
            }
        }
        let area = self.canvas;

        // 2. Frame newly loaded pages; advance camera animations.
        let mut animating = false;
        if let Some(page) = self.pages.get_mut(self.active) {
            if let PageState::Ready(r) = &page.state {
                if !page.framed {
                    page.camera = match page.restore_camera.take() {
                        Some(c) => Camera { center: c.center, zoom: c.zoom * s },
                        None => Camera::fit(r.layout.bounds, area, vp, 24.0 * s),
                    };
                    page.framed = true;
                }
                if let Some(bench) = self.bench.as_mut() {
                    let fit = Camera::fit(r.layout.bounds, area, vp, 24.0 * s);
                    bench.drive(&mut page.camera, fit, 1.6 * s, vp);
                    page.target = None;
                    animating = true;
                }
            }
            if let Some(t) = page.target {
                if page.camera.approach(&t, dt) {
                    page.target = None;
                } else {
                    animating = true;
                }
            }
        }
        if animating {
            self.update_hover();
        }
        self.sync_documents();

        // 3. Relation curves (only when they changed) and the overlay.
        let filters = self.ui.filters;
        let mut canvas_overlay = CanvasOverlay::default();
        {
            let g = self.gfx.as_mut().unwrap();
            match self.pages.get_mut(self.active) {
                Some(page) => {
                    // The selected node's dependency edges are curves too.
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
        let labels: Vec<Label> = canvas_overlay.labels.iter().map(|l| l.as_label()).collect();
        let chip_labels: Vec<Label> = canvas_overlay.chip_labels.iter().map(|l| l.as_label()).collect();

        // 4. Render: canvas pass, then egui on top.
        let build_ms = now.elapsed().as_secs_f32() * 1000.0;
        let any_loading = self.pages.iter().any(|p| p.is_loading());
        let g = self.gfx.as_mut().unwrap();
        // egui's texture updates must not be lost when a frame is skipped.
        for (id, deltas) in &full.textures_delta.set {
            for delta in deltas {
                g.egui_renderer.update_texture(&g.device, &g.queue, *id, delta);
            }
        }
        let t_acquire = Instant::now();
        let acquired = g.surface.get_current_texture();
        let acquire_ms = t_acquire.elapsed().as_secs_f32() * 1000.0;
        if acquire_ms > 40.0 {
            log::debug!("acquiring the frame took {acquire_ms:.1} ms");
        }
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            // Still usable (the window is being resized and the size event has
            // not arrived yet). It must NOT be reconfigured here: configuring
            // a surface while one of its textures is held is an error that
            // takes the app down. The resize event reconfigures it.
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                g.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                g.surface.configure(&g.device, &g.config);
                g.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                match g.instance.create_surface(g.window.clone()) {
                    Ok(surface) => g.surface = surface,
                    Err(e) => log::error!("recreate surface: {e}"),
                }
                g.surface.configure(&g.device, &g.config);
                g.window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error");
                return;
            }
        };
        let target = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

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
            scene,
            &Overlay {
                under: &canvas_overlay.under,
                labels: &labels,
                over: &canvas_overlay.over,
                chips: &canvas_overlay.chips,
                chip_labels: &chip_labels,
            },
        );

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
        for id in &full.textures_delta.free {
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

        // 5. Decide whether another frame is needed.
        let repaint = full.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
        self.next_repaint = None;
        let mut again = false;
        match repaint {
            Some(d) if d.is_zero() => again = true,
            Some(d) if d < Duration::from_secs(3600) => self.next_repaint = Some(Instant::now() + d),
            _ => {}
        }
        // The status bar shows the previous frame's stats: redraw once more
        // when they changed so it never stays stale.
        let st = &self.stats;
        let stats_changed = (prev.lod, prev.code_instances, prev.frame_instances, prev.draw_calls)
            != (st.lod, st.code_instances, st.frame_instances, st.draw_calls);
        if again || animating || st.text_pending || any_loading || stats_changed {
            g.window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    /// A background thread has something for us; `about_to_wait` picks it up.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {}

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.ext.shutdown();
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let size = self.window_size.filter(|s| s[0] >= 400.0 && s[1] >= 300.0).unwrap_or([1440.0, 900.0]);
        let attrs = Window::default_attributes()
            .with_title("Code Canvas")
            .with_inner_size(LogicalSize::new(size[0] as f64, size[1] as f64))
            .with_min_inner_size(LogicalSize::new(720.0, 420.0))
            // Our own header replaces the system title bar.
            .with_decorations(false)
            .with_window_icon(crate::icon::window_icon());
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
            // The taskbar and Alt+Tab use the big icon. Setting it here keeps
            // them from falling back to whatever Windows cached for the file.
            attrs
                .with_undecorated_shadow(true)
                .with_corner_preference(CornerPreference::Round)
                .with_taskbar_icon(crate::icon::taskbar_icon())
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        // Which graphics API to draw with. On Windows Direct3D 12 comes first:
        // resizing its swap chain is cheap (Vulkan's is rebuilt, 15-30 ms per
        // size), and with a browser view in the window (extensions' webviews)
        // Vulkan stalls for half a second per frame while the window changes
        // size. `CODE_CANVAS_BACKEND=vulkan|dx12|metal|gl` overrides the choice.
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
        // By default such an error ends the program. Here it costs a frame at
        // most, so it is written to the log and the app goes on.
        device.on_uncaptured_error(Arc::new(|e| log::error!("GPU error: {e}")));

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        let caps = surface.get_capabilities(&adapter);
        // Non-sRGB target: colours are authored in sRGB and blended like most UIs.
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

        // Only the folder that will be on screen is read now; the others
        // wait until they are shown (a huge one would take seconds and
        // every core, for a canvas nobody is looking at).
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
        // Remember what is open even if the app is later killed.
        self.save_session();
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Without a system frame the window edges are ours to handle.
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
            // RedrawRequested is not input; everything else goes to egui first.
            if !matches!(event, WindowEvent::RedrawRequested) {
                let resp = g.egui_state.on_window_event(&g.window, &event);
                if resp.repaint {
                    g.window.request_redraw();
                }
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.confirm_exit() {
                    self.save_session();
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gfx {
                    g.config.width = size.width.max(1);
                    g.config.height = size.height.max(1);
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
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::DroppedFile(path) => {
                if path.is_dir() {
                    self.open_folder(path, None);
                } else if self.pages.get(self.active).is_some() {
                    self.drop_file(path);
                } else if let Some(parent) = path.parent() {
                    // Nothing is open yet: show the folder the file is in.
                    self.open_folder(parent.to_path_buf(), None);
                }
                self.save_session();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = [position.x as f32, position.y as f32];
                if let Some(d) = &mut self.drag {
                    let (dx, dy) = (p[0] - d.last[0], p[1] - d.last[1]);
                    d.last = p;
                    if (p[0] - d.start[0]).abs() + (p[1] - d.start[1]).abs() > 4.0 {
                        d.moved = true;
                    }
                    if d.moved {
                        if let Some(page) = self.pages.get_mut(self.active) {
                            page.target = None;
                            page.camera.pan_pixels(dx, dy);
                        }
                        self.request_redraw();
                    }
                }
                self.cursor = p;
                self.update_hover();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = [-1.0, -1.0];
                self.update_hover();
            }
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    // Only presses that start on the canvas belong to the canvas.
                    if self.drag.is_none() && !self.over_ui(self.cursor) {
                        self.drag = Some(Drag { last: self.cursor, start: self.cursor, moved: false, button });
                    }
                }
                ElementState::Released => {
                    if let Some(d) = self.drag.take() {
                        if d.button != button {
                            self.drag = Some(d);
                        } else if !d.moved && button == MouseButton::Left {
                            self.on_canvas_click(self.cursor);
                        } else {
                            self.session_dirty = true;
                        }
                    }
                    self.update_hover();
                }
            },
            WindowEvent::MouseWheel { delta, .. } => {
                if self.over_ui(self.cursor) {
                    return;
                }
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 60.0,
                };
                let vp = self.viewport();
                let cursor = self.cursor;
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.target = None;
                    page.camera.zoom_at(cursor, vp, 1.2f32.powf(steps));
                }
                self.session_dirty = true;
                self.update_hover();
                self.request_redraw();
            }
            WindowEvent::PinchGesture { delta, .. } => {
                if self.over_ui(self.cursor) {
                    return;
                }
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
        if self.exit_requested {
            if !self.confirm_exit() {
                self.exit_requested = false;
                return;
            }
            self.save_session();
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
        // Refresh scan progress a few times per second; otherwise sleep
        // until input arrives or egui's next scheduled repaint.
        let tick = now + Duration::from_millis(100);
        let wake = match (loading, self.next_repaint) {
            (true, t) => Some(t.map_or(tick, |t| t.min(tick))),
            (false, t) => t,
        };
        if loading {
            self.request_redraw();
        }
        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}
