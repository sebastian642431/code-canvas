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
use crate::overlay::{self, CanvasOverlay, SYMBOL_HOVER_MIN_PX};
use crate::page::{Page, PageState, Ready};
use crate::script::{Script, Step};
use crate::session::{self, PageSession, Session};
use crate::ui::{self, Action, FrameInfo, LeftPanel, QuickOpen, RightPanel, UiState};

struct Gfx {
    renderer: Renderer,
    egui_renderer: egui_wgpu::Renderer,
    egui_state: egui_winit::State,
    egui: egui::Context,
    queue: wgpu::Queue,
    device: wgpu::Device,
    config: wgpu::SurfaceConfiguration,
    surface: wgpu::Surface<'static>,
    instance: wgpu::Instance,
    // Dropped last: the surface must go before the window.
    window: Arc<Window>,
}

struct Drag {
    last: [f32; 2],
    start: [f32; 2],
    moved: bool,
    button: MouseButton,
}

/// Left panels in the order their index is saved in the session.
const LEFT_PANELS: [LeftPanel; 6] = [
    LeftPanel::Files,
    LeftPanel::Search,
    LeftPanel::Help,
    LeftPanel::Extensions,
    LeftPanel::Problems,
    LeftPanel::Changes,
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
    /// Browser views showing extensions' webviews.
    webviews: crate::webview::WebViews,
    /// Folders webview pages may load files from (to notice changes).
    webview_roots: Vec<PathBuf>,
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
        let mut ui = UiState::default();
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
            ext: Extensions::new(session::data_dir(), waker),
            webviews: crate::webview::WebViews::new(),
            webview_roots: Vec::new(),
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

    fn save_session(&mut self) {
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

    fn open_folder(&mut self, path: PathBuf, saved: Option<PageSession>) {
        let path = canvas_core::scan::clean_path(path.canonicalize().unwrap_or(path));
        if let Some(i) = self.pages.iter().position(|p| p.root == path) {
            self.switch_to(i);
            return;
        }
        log::info!("opening {}", path.display());
        let page = Page::open(self.next_id, path, saved.as_ref());
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
        if let Err(e) = std::fs::write(path, text) {
            log::error!("cannot write {}: {e}", path.display());
            return;
        }
        log::info!("wrote {} ({} bytes)", path.display(), text.len());
        self.ext.file_changed(path);
        if let Some((i, rel)) = self.locate_file(path) {
            if let (Some(g), Some(page)) = (self.gfx.as_mut(), self.pages.get_mut(i)) {
                g.renderer.text.drop_page(page.id);
                page.reload_file(&rel, &g.renderer, &g.device, &self.ui.filters);
                page.refresh_git();
            }
        }
        self.request_redraw();
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
        let Some(page) = self.pages.get_mut(self.active) else { return };
        let PageState::Ready(r) = &page.state else { return };
        let world = page.camera.view(vp).to_world(pos);
        let hit = r.layout.hit(world);
        page.selected = hit;
        let Some(node) = hit else {
            self.request_redraw();
            return;
        };
        if double {
            self.frame_node(node);
            return;
        }

        // On code: a click on a name selects that symbol; elsewhere inside a
        // function it selects the function.
        let n = &r.layout.nodes[node];
        let zoom = page.camera.zoom;
        let mut picked = None;
        if let NodeKind::File(fi) = n.kind {
            if zoom * LINE_H >= SYMBOL_HOVER_MIN_PX {
                if let Some((line, col)) = n.cell_at(world, r.repo.files[fi].line_count()) {
                    let fi = fi as u32;
                    let named = (Lod::for_zoom(zoom) == Lod::Text).then(|| r.repo.resolve_at(fi, line, col)).flatten();
                    picked = match named {
                        Some((sym, site)) => Some((sym, (site != r.repo.sym_site(sym)).then_some(site))),
                        None => r.repo.sym_at(fi, line).map(|sym| (sym, None)),
                    };
                }
            }
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
        let shift = self.mods.shift_key();
        let typing = self.gfx.as_ref().is_some_and(|g| g.egui.egui_wants_keyboard_input());

        // Shortcuts that work even while typing in a text field.
        if let Key::Character(c) = &event.logical_key {
            if cmd && c.as_str().eq_ignore_ascii_case("p") {
                self.ui.quick = Some(QuickOpen { query: String::new(), selected: 0, focus: true });
                self.request_redraw();
                return;
            }
        }
        if let Key::Character(c) = &event.logical_key {
            if cmd && shift && c.as_str().eq_ignore_ascii_case("f") {
                self.ui.show_left(LeftPanel::Search);
                self.request_redraw();
                return;
            }
        }
        if typing || self.ui.quick.is_some() {
            return;
        }
        if let Key::Character(c) = &event.logical_key {
            if shift && self.mods.alt_key() && c.as_str().eq_ignore_ascii_case("f") {
                self.format_file(None);
                return;
            }
        }

        match &event.logical_key {
            Key::Named(NamedKey::Tab) if cmd => self.cycle(if shift { -1 } else { 1 }),
            Key::Named(NamedKey::Home) => self.frame_all(),
            Key::Named(NamedKey::F5) => self.rescan(),
            Key::Named(NamedKey::Escape) => {
                if self.pages.get(self.active).is_some_and(|p| !p.selections.is_empty()) {
                    self.clear_symbols();
                } else if let Some(p) = self.pages.get_mut(self.active) {
                    p.selected = None;
                    self.request_redraw();
                }
            }
            Key::Named(NamedKey::Backspace) => self.zoom_to_parent(),
            Key::Named(NamedKey::ArrowLeft) => self.pan(160.0, 0.0),
            Key::Named(NamedKey::ArrowRight) => self.pan(-160.0, 0.0),
            Key::Named(NamedKey::ArrowUp) => self.pan(0.0, 160.0),
            Key::Named(NamedKey::ArrowDown) => self.pan(0.0, -160.0),
            Key::Character(c) => match (cmd, c.to_lowercase().as_str()) {
                (true, "o") | (true, "t") => self.pick_folder(),
                (true, "w") => self.close_page(self.active),
                (true, "r") => self.rescan(),
                (true, "b") => {
                    self.ui.toggle_left();
                    self.request_redraw();
                }
                (true, d) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => {
                    self.switch_to((d.as_bytes()[0] - b'1') as usize)
                }
                (false, "f") => self.frame_all(),
                (false, "l") => self.frame_selection(),
                (_, "+") | (_, "=") => self.zoom_center(1.25),
                (_, "-") => self.zoom_center(0.8),
                _ => {}
            },
            _ => {}
        }
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
            Action::Git(op) => {
                if let Some(page) = self.pages.get_mut(self.active) {
                    page.run_git(op);
                }
                self.request_redraw();
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
            Step::Git(what) => {
                use crate::git::GitOp;
                let (verb, arg) = what.split_once(' ').unwrap_or((what.as_str(), ""));
                let op = match verb {
                    "stage" if arg.is_empty() => Some(GitOp::StageAll),
                    "stage" => Some(GitOp::Stage(arg.to_string())),
                    "unstage" if arg.is_empty() => Some(GitOp::UnstageAll),
                    "unstage" => Some(GitOp::Unstage(arg.to_string())),
                    "commit" if !arg.is_empty() => Some(GitOp::Commit(arg.to_string())),
                    _ => None,
                };
                match op {
                    Some(op) => self.apply(Action::Git(op)),
                    None => log::warn!("script: git needs stage|unstage [path] or commit <message>"),
                }
            }
            Step::Right(which) => match which.as_str() {
                "symbols" => self.ui.show_right(RightPanel::Symbols),
                key if !key.is_empty() && key != "none" => {
                    // `right <view id or extension id>`: that extension's webview.
                    let found = self.ext.webviews.iter().find(|w| w.key == key || w.extension == key);
                    match found.map(|w| w.key.clone()) {
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
            Step::Folders => self.ui.show_left(LeftPanel::Files),
            Step::Zoom(f) => self.apply(Action::ZoomBy(f)),
            Step::Quick(text) => self.ui.quick = Some(QuickOpen { query: text, selected: 0, focus: true }),
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
        // Source control: results of `git status` runs.
        let mut git_relayout = false;
        for (i, page) in self.pages.iter_mut().enumerate() {
            changed |= page.poll_git_op();
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
        // Extensions: their workspace is the set of open folders.
        self.ext.set_folders(self.pages.iter().map(|p| p.root.clone()).collect());
        let (events, ext_changed) = self.ext.poll();
        changed |= ext_changed;
        for event in events {
            match event {
                ext::Event::Write { path, text } => self.write_file(&path, &text),
                ext::Event::Reveal(path) => self.reveal_file(&path),
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
                "Ctrl+O: open a folder  ·  drop folders on the window  ·  each folder gets its own canvas".into(),
                None,
            );
        };
        match &page.state {
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
        if self.gfx.is_none() {
            return;
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
            self.webviews.show(&g.window, shown.as_ref().map(|(h, r)| (h.as_str(), *r)));
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
                    if page.curves_dirty || self.curves_for != Some(page.id) {
                        let curves = overlay::build_curves(page, &filters, s);
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
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                g.surface.configure(&g.device, &g.config);
                f
            }
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
        g.queue.present(frame);

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

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        )));
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no suitable GPU adapter");
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?})", info.name, info.backend);
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor { label: Some("code-canvas"), ..Default::default() }),
        )
        .expect("request device");

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
        self.gfx =
            Some(Gfx { renderer, egui_renderer, egui_state, egui, queue, device, config, surface, instance, window });

        for (path, saved) in std::mem::take(&mut self.initial) {
            self.open_folder(path, saved);
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
                self.save_session();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gfx {
                    g.config.width = size.width.max(1);
                    g.config.height = size.height.max(1);
                    g.surface.configure(&g.device, &g.config);
                    g.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::DroppedFile(path) => {
                if path.is_dir() {
                    self.open_folder(path, None);
                } else if let Some(parent) = path.parent() {
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
            self.save_session();
            event_loop.exit();
            return;
        }
        self.poll_pages();
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
