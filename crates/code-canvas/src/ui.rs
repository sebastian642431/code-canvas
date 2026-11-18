use std::sync::atomic::Ordering;

use canvas_core::{RelKind, Relations, Site};
use canvas_render::{theme, FrameStats};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use crate::ext::Extensions;
use crate::icons;
use crate::page::{build_tree, Page, PageState, RelFilters};

mod console;
mod deps_panel;
mod ext_panels;
mod git_panel;
mod settings;

use crate::keys::{Command, Keymap};

pub use crate::rails::Side;
pub use console::ConsoleOp;
pub use settings::{SettingsView, Tab as SettingsTab};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Panel {
    Files,
    Search,
    Extensions,
    Problems,
    Changes,
    Extension,
    Help,
    Symbols,
    Deps,
    Media,
    Console,
    Webview(String),
    /// A container of an extension's tree views, by its id.
    Tree(String),
}

impl Panel {
    pub fn docks(&self) -> bool {
        matches!(self, Panel::Files | Panel::Changes | Panel::Search | Panel::Extensions)
    }

    pub fn key(&self) -> String {
        match self {
            Panel::Files => "files".to_string(),
            Panel::Search => "search".to_string(),
            Panel::Extensions | Panel::Extension => "extensions".to_string(),
            Panel::Problems => "problems".to_string(),
            Panel::Changes => "changes".to_string(),
            Panel::Help => "help".to_string(),
            Panel::Symbols => "symbols".to_string(),
            Panel::Deps => "deps".to_string(),
            Panel::Media => "media".to_string(),
            Panel::Console => "console".to_string(),
            Panel::Webview(view) => format!("view:{view}"),
            Panel::Tree(id) => format!("{}{id}", crate::ext::TREE),
        }
    }

    pub fn from_key(key: &str) -> Option<Panel> {
        Some(match key {
            "files" => Panel::Files,
            "search" => Panel::Search,
            "extensions" => Panel::Extensions,
            "problems" => Panel::Problems,
            "changes" => Panel::Changes,
            "help" => Panel::Help,
            "symbols" => Panel::Symbols,
            "deps" => Panel::Deps,
            "media" => Panel::Media,
            "console" => Panel::Console,
            tree if tree.starts_with(crate::ext::TREE) => Panel::Tree(tree[crate::ext::TREE.len()..].to_string()),
            view => Panel::Webview(view.strip_prefix("view:")?.to_string()),
        })
    }

    /// What an icon of an extension in the rail opens: a webview, or a container of trees.
    pub fn of_rail(view: String) -> Panel {
        match view.strip_prefix(crate::ext::TREE) {
            Some(id) => Panel::Tree(id.to_string()),
            None => Panel::Webview(view),
        }
    }

    fn home(&self) -> Side {
        match self {
            Panel::Webview(_) | Panel::Tree(_) => Side::Right,
            _ => Side::Left,
        }
    }

    fn stays_put(&self) -> bool {
        matches!(self, Panel::Help)
    }

    fn is_page(&self) -> bool {
        matches!(self, Panel::Media | Panel::Webview(_))
    }
}

#[derive(Clone)]
struct RailDrag {
    key: String,
    target: Option<(Side, Option<String>)>,
}

enum RailLook {
    Glyph(&'static str),
    Extension { id: String, codicon: Option<String> },
}

struct RailEntry {
    panel: Panel,
    tip: String,
    look: RailLook,
}

#[derive(Default)]
struct FileIcons {
    file: Option<std::path::PathBuf>,
    theme: Option<crate::icon_theme::IconTheme>,
    textures: std::collections::HashMap<std::path::PathBuf, Option<egui::TextureHandle>>,
}

impl FileIcons {
    fn set(&mut self, file: Option<&std::path::Path>) {
        if self.file.as_deref() == file {
            return;
        }
        self.theme = file.and_then(|f| match crate::icon_theme::IconTheme::load(f) {
            Ok(theme) => Some(theme),
            Err(e) => {
                log::warn!("icon theme {} could not be read: {e}", f.display());
                None
            }
        });
        self.file = file.map(std::path::Path::to_path_buf);
        self.textures.clear();
    }

    fn paint(&mut self, ui: &egui::Ui, name: &str, is_dir: bool, open: bool, at: Rect, dim: bool) -> bool {
        let Some(theme) = &self.theme else { return false };
        let Some(svg) = (if is_dir { theme.folder(name, open) } else { theme.file(name) }) else { return false };
        const PIXELS: u32 = 32;
        let texture = self.textures.entry(svg.to_path_buf()).or_insert_with(|| {
            let pixels = crate::icon_theme::rasterize(svg, PIXELS)?;
            let image = egui::ColorImage::from_rgba_premultiplied([PIXELS as usize; 2], &pixels);
            Some(ui.ctx().load_texture(format!("file-icon-{}", svg.display()), image, egui::TextureOptions::LINEAR))
        });
        let Some(texture) = texture else { return false };
        let tint = if dim { Color32::from_white_alpha(110) } else { Color32::WHITE };
        egui::Image::new(&*texture).tint(tint).paint_at(ui, at);
        true
    }
}

pub enum FileOp {
    Reveal(String),
    Terminal(String),
    CopyPath { path: String, relative: bool },
    NewFile { dir: String, name: String },
    NewFolder { dir: String, name: String },
    Rename { path: String, name: String },
    Delete(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NamingKind {
    NewFile,
    NewFolder,
    Rename,
}

pub struct Naming {
    pub kind: NamingKind,
    pub path: String,
    pub text: String,
    pub focus: bool,
}

pub enum CompletionIcon {
    Path { is_dir: bool },
    Kind(String),
}

pub struct CompletionRow {
    pub label: String,
    pub detail: String,
    pub icon: CompletionIcon,
}

pub struct CompletionView {
    pub at: Pos2,
    pub items: Vec<CompletionRow>,
    pub selected: usize,
}

fn kind_glyph(kind: &str) -> (&'static str, Color32) {
    let (name, color) = match kind {
        "Method" | "Function" | "Constructor" => ("symbol-method", theme::get().rel_impl),
        "Field" | "Property" => ("symbol-field", theme::get().accent),
        "Variable" | "Value" | "Reference" => ("symbol-variable", theme::get().accent),
        "Class" | "Struct" => ("symbol-class", theme::get().rel_ref),
        "Interface" | "TypeParameter" => ("symbol-interface", theme::get().rel_ref),
        "Module" | "Folder" | "File" => ("symbol-namespace", theme::get().ui_text_dim),
        "Enum" | "EnumMember" => ("symbol-enum", theme::get().rel_ref),
        "Constant" | "Unit" => ("symbol-constant", theme::get().accent),
        "Keyword" | "Operator" => ("symbol-keyword", theme::get().ui_text_dim),
        "Snippet" => ("symbol-snippet", theme::get().ui_text_dim),
        _ => ("symbol-misc", theme::get().ui_text_dim),
    };
    (crate::codicons::glyph(name).unwrap_or(icons::CODE), c(color))
}

pub struct QuickOpen {
    pub query: String,
    pub selected: usize,
    pub focus: bool,
}

#[derive(Default)]
pub struct Find {
    pub query: String,
    pub with: String,
    pub focus: bool,
    pub at: usize,
    pub count: usize,
    pub file: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FindOp {
    Open,
    Changed,
    Next,
    Previous,
    Replace,
    ReplaceAll,
    Close,
}

pub struct CardPlace {
    pub over: Rect,
    pub pages: std::ops::Range<usize>,
    floating: usize,
    layer: egui::LayerId,
}

pub struct UiState {
    pub files: bool,
    pub dock: Panel,
    pub layout_open: bool,
    faces_stack: Option<Rect>,
    pub new_build: bool,
    layout_button: Option<Rect>,
    pub full_screen: bool,
    pub account: crate::account::Account,
    pub cards: Vec<Panel>,
    pub cards_reach: f32,
    pub card_places: Vec<CardPlace>,
    pub console: crate::term::Console,
    card_widths: std::collections::HashMap<String, f32>,
    pub explorer_filter: String,
    pub search_query: String,
    pub quick: Option<QuickOpen>,
    pub filters: RelFilters,
    pub floating: Vec<Rect>,
    pub covers: Vec<Rect>,
    pub maximized: bool,
    logo: Option<egui::TextureHandle>,
    pub deps: crate::page::DepFilters,
    pub open_folder_menu: bool,
    pub page_places: Vec<(Panel, Rect)>,
    pub rails: crate::rails::Rails,
    rail_order: [Vec<Panel>; 2],
    rail_drag: Option<RailDrag>,
    rail_move: Option<(Panel, Side, Option<String>)>,
    pub extension: String,
    ext_icons: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    pub theme: crate::themes::ThemeFile,
    pub keys: Keymap,
    pub key_file: crate::keys::KeyFile,
    pub rebinding: Option<Command>,
    pub settings: SettingsView,
    pub naming: Option<Naming>,
    pub single: bool,
    pub prefs: crate::prefs::Prefs,
    pub media: Option<std::path::PathBuf>,
    pub update: crate::update::State,
    pub completion: Option<CompletionView>,
    pub find: Option<Find>,
    pub card_menu: Option<CardMenu>,
    pub ask: Option<Ask>,
    pub share: ShareView,
    pub workspace: Vec<WorkspaceRow>,
    pub agents: Vec<AgentView>,
    pub agent_marks: Vec<AgentMark>,
    pub agent_trails: Vec<AgentTrail>,
    pub claude_connected: bool,
    faces: Vec<egui::TextureHandle>,
    pub workspace_shut: bool,
    workspace_drag: Option<(std::path::PathBuf, String)>,
    canvas_was: Rect,
    pub bare: bool,
    pub explorer_reveal: Option<String>,
    explorer_shown: Option<(u32, String)>,
    explorer_seen: (f32, f32),
    pub can_go: (bool, bool),
    pub pictures: Vec<(egui::TextureId, Rect)>,
    pub ask_picture: Option<(egui::TextureId, [u32; 2])>,
    pub ask_page: Option<Rect>,
    pub pen: crate::doodle::Pen,
    pub doodles: Vec<(Vec<Pos2>, f32, Color32)>,
    pub rubber: Option<(Pos2, f32)>,
    pub commit_ai: crate::commit_ai::Settings,
    file_icons: FileIcons,
    /// The views of a container's card that are folded away, by view id.
    tree_shut: std::collections::HashSet<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            files: true,
            dock: Panel::Files,
            layout_open: false,
            faces_stack: None,
            new_build: false,
            layout_button: None,
            full_screen: false,
            account: Default::default(),
            cards: Vec::new(),
            cards_reach: 0.0,
            card_places: Vec::new(),
            console: Default::default(),
            card_widths: Default::default(),
            explorer_filter: String::new(),
            search_query: String::new(),
            quick: None,
            filters: RelFilters::default(),
            floating: Vec::new(),
            covers: Vec::new(),
            maximized: false,
            logo: None,
            extension: String::new(),
            deps: Default::default(),
            open_folder_menu: false,
            page_places: Vec::new(),
            rails: Default::default(),
            rail_order: Default::default(),
            rail_drag: None,
            rail_move: None,
            ext_icons: Default::default(),
            theme: Default::default(),
            keys: Keymap::new(&Default::default()),
            key_file: Default::default(),
            rebinding: None,
            settings: Default::default(),
            naming: None,
            single: false,
            prefs: Default::default(),
            media: None,
            update: Default::default(),
            completion: None,
            find: None,
            card_menu: None,
            ask: None,
            share: ShareView::default(),
            workspace: Vec::new(),
            agents: Vec::new(),
            agent_marks: Vec::new(),
            agent_trails: Vec::new(),
            claude_connected: false,
            faces: Vec::new(),
            workspace_shut: false,
            workspace_drag: None,
            canvas_was: Rect::NOTHING,
            bare: false,
            explorer_reveal: None,
            explorer_shown: None,
            explorer_seen: (0.0, 0.0),
            can_go: (false, false),
            pictures: Vec::new(),
            ask_picture: None,
            ask_page: None,
            pen: Default::default(),
            doodles: Vec::new(),
            rubber: None,
            commit_ai: Default::default(),
            file_icons: FileIcons::default(),
            tree_shut: Default::default(),
        }
    }
}

impl UiState {
    pub fn side_of(&self, panel: &Panel) -> Side {
        if panel.stays_put() {
            return Side::Left;
        }
        self.rails.side_of(&panel.key()).unwrap_or(panel.home())
    }

    pub fn rails_at_home(&mut self) {
        self.rails.at_home(|key| Panel::from_key(key).map_or(Side::Left, |panel| panel.home()));
    }

    pub fn is_open(&self, panel: &Panel) -> bool {
        match panel {
            panel if panel.docks() => self.files && self.dock == *panel,
            card => self.cards.contains(card),
        }
    }

    pub fn close(&mut self, side: Side) {
        if side == Side::Left {
            self.files = false;
        }
        let kept: Vec<Panel> = self.cards.iter().filter(|card| self.side_of(card) != side).cloned().collect();
        self.cards = kept;
    }

    pub fn close_panel(&mut self, panel: &Panel) {
        match panel {
            panel if panel.docks() => {
                if self.dock == *panel {
                    self.files = false;
                }
            }
            card => self.cards.retain(|open| open != card),
        }
    }

    pub fn show(&mut self, panel: Panel) {
        if panel.docks() {
            (self.files, self.dock) = (true, panel);
            return;
        }
        let key = panel.key();
        match self.cards.iter().position(|card| card.key() == key) {
            Some(at) => self.cards[at] = panel,
            None => self.cards.push(panel),
        }
    }

    pub fn toggle(&mut self, panel: Panel) {
        if self.is_open(&panel) {
            self.close_panel(&panel);
        } else {
            self.show(panel);
        }
    }

    pub fn request_move(&mut self, panel: Panel, side: Side, before: Option<String>) {
        self.rail_move = Some((panel, side, before));
    }

    fn move_panel(&mut self, available: &[(String, Side)], panel: &Panel, side: Side, before: Option<&str>) {
        self.rails.place(available, &panel.key(), side, before);
    }

    pub fn restore(&mut self, files: bool, cards: &[String]) {
        self.files = files;
        let open: Vec<Panel> = cards.iter().filter_map(|key| Panel::from_key(key)).collect();
        self.dock = open.iter().find(|panel| panel.docks()).cloned().unwrap_or(Panel::Files);
        self.cards = open.into_iter().filter(|panel| !panel.docks() && !panel.is_page()).collect();
    }

    pub fn open_keys(&self) -> Vec<String> {
        let docked = (self.dock != Panel::Files).then(|| self.dock.key());
        docked.into_iter().chain(self.cards.iter().map(Panel::key)).collect()
    }
}

pub enum Action {
    SwitchTab(usize),
    CloseTab(usize),
    OpenFolder,
    Rescan,
    FlyToPath(String),
    FlyToSite(Site),
    ToggleHidden(String),
    ToggleCollapsed(String),
    ToggleTreeOpen(String),
    HoverPath(String),
    RemoveSymbol(usize),
    ClearSymbols,
    SetOnlyConnected(bool),
    FiltersChanged,
    Fit,
    ZoomBy(f32),
    ToggleFullScreen,
    RestartIntoNewBuild,
    Account(AccountOp),
    ClearDoodles,
    FrameSelection,
    CollapseAll,
    ExpandAll,
    ShowAll,
    Search(String),
    FormatFile(Option<String>),
    Show(Panel),
    RailsChanged,
    Console(ConsoleOp),
    SaveFile,
    RunCommand(String, String),
    DepsChanged,
    Git(crate::git::GitOp),
    RefreshGit,
    WriteCommitMessage,
    SetCommitAi(crate::commit_ai::Settings),
    Settings(SettingsTab),
    SetPrefs(crate::prefs::Prefs),
    OpenMedia(std::path::PathBuf),
    Preview(String),
    PreviewSelected,
    OpenIgnored(String),
    ShowSource(std::path::PathBuf),
    OpenExternally(std::path::PathBuf),
    CheckUpdates,
    InstallUpdate,
    AcceptCompletion(usize),
    SetOnlyChanged(bool),
    Find(FindOp),
    Back,
    Forward,
    SetDiff(crate::page::DiffView),
    ShowCommit(Option<crate::git::Commit>),
    Answered(Asked, Answer),
    Share(ShareOp),
    Workspace(WorkspaceOp),
    Agent(AgentOp),
    ToggleUi,
    Commands,
    RevealInFiles(String),
    InstallVsix,
    SetTheme(String),
    SetColor(&'static str, u32),
    ResetColors,
    SetIconTheme(String),
    ToggleSideBySide,
    ResetKey(Command),
    UnbindKey(Command),
    File(FileOp),
    DragWindow,
    ToggleMaximize,
    Minimize,
    CloseWindow,
}

pub struct FrameInfo<'a> {
    pub stats: &'a FrameStats,
    pub frame_ms: f32,
    pub zoom: f32,
    pub tooltip: Option<(String, String)>,
    pub status: String,
}

fn c(rgba: u32) -> Color32 {
    let [r, g, b, a] = theme::unpack(rgba);
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

#[derive(Clone, Copy)]
struct Pal {
    chrome: Color32,
    card: Color32,
    well: Color32,
    row: Color32,
    raised: Color32,
    hover: Color32,
    active: Color32,
    selection: Color32,
    border: Color32,
    border_strong: Color32,
    text: Color32,
    text_dim: Color32,
    text_faint: Color32,
    accent: Color32,
    on_accent: Color32,
}

fn pal() -> Pal {
    let t = theme::get();
    Pal {
        chrome: c(t.chrome),
        card: c(t.card),
        well: c(t.well),
        row: c(t.row),
        raised: c(t.raised),
        hover: c(t.hover),
        active: c(t.active),
        selection: c(t.selection),
        border: c(t.border),
        border_strong: c(t.border_strong),
        text: c(t.text),
        text_dim: c(t.text_dim),
        text_faint: c(t.text_faint),
        accent: c(t.accent),
        on_accent: c(t.on_accent),
    }
}

const ROW_H: f32 = 24.0;
const HEADER_H: f32 = 40.0;
const RAIL_W: f32 = 44.0;
const EDGE: f32 = 4.0;
const ROUND: f32 = 10.0;
const CARD_X: &str = "the card's cross";

pub fn style(ctx: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    ctx.add_font(FontInsert {
        name: "lucide".into(),
        data: egui::FontData::from_static(include_bytes!("../assets/lucide.ttf")),
        families: vec![
            InsertFontFamily { family: egui::FontFamily::Proportional, priority: FontPriority::Lowest },
            InsertFontFamily { family: egui::FontFamily::Monospace, priority: FontPriority::Lowest },
        ],
    });

    ctx.add_font(FontInsert {
        name: "codicon".into(),
        data: egui::FontData::from_static(include_bytes!("../assets/codicon.ttf")),
        families: vec![
            InsertFontFamily { family: egui::FontFamily::Proportional, priority: FontPriority::Lowest },
            InsertFontFamily { family: egui::FontFamily::Monospace, priority: FontPriority::Lowest },
        ],
    });

    apply_theme(ctx);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(6.0, 5.0);
        s.spacing.button_padding = Vec2::new(8.0, 4.0);
        s.spacing.slider_rail_height = 5.0;
        s.interaction.tooltip_delay = 0.4;
    });
}

pub fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = pal().chrome;
    v.window_fill = pal().card;
    v.extreme_bg_color = pal().well;
    v.faint_bg_color = pal().raised;
    v.window_stroke = Stroke::new(1.0, pal().border);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal().border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal().text);
    v.widgets.inactive.bg_fill = pal().raised;
    v.widgets.inactive.weak_bg_fill = pal().raised;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, pal().text);
    v.widgets.inactive.corner_radius = CornerRadius::same(6);
    v.widgets.hovered.bg_fill = pal().hover;
    v.widgets.hovered.weak_bg_fill = pal().hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, pal().border_strong);
    v.widgets.hovered.corner_radius = CornerRadius::same(6);
    v.widgets.active.bg_fill = pal().active;
    v.widgets.active.weak_bg_fill = pal().active;
    v.widgets.active.corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = pal().selection;
    v.selection.stroke = Stroke::new(1.0, pal().text);
    v.hyperlink_color = pal().accent;
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
}

fn icon(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph).size(size).color(color)
}

fn icon_button(ui: &mut egui::Ui, glyph: &str, size: Vec2, active: bool, hover_fill: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let inner = rect.shrink2(Vec2::new(4.0, 4.0).min(size * 0.15));
        if active {
            ui.painter().rect_filled(inner, 8.0, pal().raised);
        } else if resp.hovered() {
            ui.painter().rect_filled(inner, 8.0, hover_fill);
        }
        let color = if active || resp.hovered() { pal().text } else { pal().text_dim };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), color);
    }
    resp
}

fn check(ui: &mut egui::Ui, on: &mut bool, label: &str, enabled: bool, dim: bool) -> egui::Response {
    let font = FontId::proportional(if dim { 12.0 } else { 13.0 });
    let text_color = if !enabled {
        pal().text_faint
    } else if dim {
        pal().text_dim
    } else {
        pal().text
    };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text_color);
    let size = Vec2::new(16.0 + 8.0 + galley.size().x, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let boxr = Rect::from_center_size(Pos2::new(rect.left() + 8.0, rect.center().y), Vec2::splat(16.0));
        let fill = if resp.hovered() && enabled { pal().hover } else { pal().raised };
        ui.painter().rect_filled(boxr, 4.0, fill);
        if *on {
            let color = if enabled { pal().text } else { pal().text_faint };
            ui.painter().text(boxr.center(), Align2::CENTER_CENTER, icons::CHECK, FontId::proportional(12.0), color);
        }
        ui.painter().galley(Pos2::new(rect.left() + 24.0, rect.center().y - galley.size().y * 0.5), galley, text_color);
    }
    resp
}

fn section(ui: &mut egui::Ui, glyph: &str, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(icon(glyph, 14.0, pal().text_dim));
        ui.label(RichText::new(title).size(13.5).color(pal().text));
    });
    ui.add_space(2.0);
}

fn well(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    Frame::NONE.fill(pal().well).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

fn corner(painter: &egui::Painter, at: Pos2, into: Vec2, color: Color32) {
    const STEPS: u32 = 10;
    let centre = at + into * ROUND;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(at, color);
    for step in 0..=STEPS {
        let turn = step as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
        let point = Pos2::new(centre.x - into.x * ROUND * turn.sin(), centre.y - into.y * ROUND * turn.cos());
        mesh.colored_vertex(point, color);
    }
    for step in 0..STEPS {
        mesh.add_triangle(0, step + 1, step + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn enclose(painter: &egui::Painter, free: Rect, canvas: Rect) {
    let chrome = pal().chrome;
    for strip in [
        Rect::from_min_max(free.min, Pos2::new(canvas.left(), free.bottom())),
        Rect::from_min_max(Pos2::new(canvas.right(), free.top()), free.max),
        Rect::from_min_max(Pos2::new(free.left(), canvas.bottom()), free.max),
    ] {
        painter.rect_filled(strip, 0.0, chrome);
    }
    corner(painter, canvas.left_top(), Vec2::new(1.0, 1.0), chrome);
    corner(painter, canvas.right_top(), Vec2::new(-1.0, 1.0), chrome);
    corner(painter, canvas.left_bottom(), Vec2::new(1.0, -1.0), chrome);
    corner(painter, canvas.right_bottom(), Vec2::new(-1.0, -1.0), chrome);
    painter.rect_stroke(canvas, ROUND, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
}

pub fn build(
    ui: &mut egui::Ui,
    st: &mut UiState,
    pages: &mut [Page],
    active: usize,
    ext: &mut Extensions,
    info: &FrameInfo,
    actions: &mut Vec<Action>,
) -> Rect {
    st.floating.clear();
    st.card_places.clear();
    st.page_places.clear();
    st.file_icons.set(ext.icon_theme(&st.theme.icons).map(|t| t.file.as_path()));
    header(ui, st, actions);
    if !st.bare && st.prefs.status {
        status_bar(ui, st, ext, pages.get(active), info, actions);
    }
    let entries = rail_entries(st, ext);
    let available: Vec<(String, Side)> = entries.iter().map(|entry| (entry.panel.key(), entry.panel.home())).collect();
    let order = st.rails.arrange(&available);
    st.rail_order = order.clone().map(|keys| keys.iter().filter_map(|key| Panel::from_key(key)).collect());
    if let Some(drag) = st.rail_drag.as_mut() {
        drag.target = None;
    }
    let bare = st.bare;
    let rails = !bare && st.prefs.rail;
    for side in Side::BOTH.into_iter().filter(|_| rails) {
        rail(ui, st, ext, pages.get(active), side, &entries, &order[side.index()], actions);
    }
    finish_rail_drag(ui, st, ext, &entries);
    if let Some((panel, side, before)) = st.rail_move.take() {
        st.move_panel(&available, &panel, side, before.as_deref());
        actions.push(Action::RailsChanged);
    }
    if st.files && !bare {
        files_dock(ui, st, pages, active, ext, actions);
    }

    let free = ui.available_rect_before_wrap();
    let canvas = match bare {
        true => free,
        false => Rect::from_min_max(free.min + Vec2::new(EDGE, 0.0), free.max - Vec2::splat(EDGE)),
    };
    let ctx = ui.ctx().clone();
    let on_canvas = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(canvas);
    let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    for (picture, at) in st.pictures.drain(..) {
        on_canvas.image(picture, at, whole, Color32::WHITE);
    }
    for (points, width, colour) in st.doodles.drain(..) {
        let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else { continue };
        on_canvas.circle_filled(first, width * 0.5, colour);
        if first != last || points.len() > 2 {
            on_canvas.circle_filled(last, width * 0.5, colour);
            on_canvas.add(egui::Shape::line(points, Stroke::new(width, colour)));
        }
    }
    if let Some((at, reach)) = st.rubber.take() {
        on_canvas.circle_stroke(at, reach, Stroke::new(1.0, pal().text));
    }
    center_message(ui, canvas, pages.get(active), &st.keys);
    agent_marks(&ctx, &on_canvas, st, canvas);
    peer_marks(&on_canvas, st, canvas);
    if !bare {
        enclose(&ctx.layer_painter(egui::LayerId::background()), free, canvas);
        cards(&ctx, st, canvas, pages, active, ext, actions);
        if st.prefs.toolbar {
            toolbar(&ctx, st, canvas, info, actions);
        }
    }
    layout_menu(&ctx, st, actions);
    account_menu(&ctx, st, actions);
    sign_in_modal(&ctx, st, actions);
    find_bar(&ctx, st, canvas, actions);
    commit_bar(&ctx, st, pages.get(active), canvas, actions);
    workspace_drop(&ctx, st, canvas);
    following_bar(&ctx, st, canvas, actions);
    share_panel(&ctx, st, actions);
    card_menu(&ctx, st, canvas, actions);
    completion_list(&ctx, st, actions);
    ext_panels::notices(&ctx, st, canvas, ext);
    settings::settings_modal(&ctx, st, ext, actions);
    ask_modal(&ctx, st, actions);
    let over_canvas = |at: Pos2| canvas.contains(at) && !st.floating.iter().any(|taken| taken.contains(at));
    if st.pen.on && ctx.pointer_hover_pos().is_some_and(over_canvas) {
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    quick_open(&ctx, st, canvas, pages.get(active), ext, actions);
    tooltip(&ctx, canvas, info);
    canvas
}

fn header(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    let top = egui::Panel::top("header").frame(Frame::NONE.fill(pal().chrome)).exact_size(HEADER_H);
    top.show_separator_line(false).show(ui, |ui| {
        let bar = ui.max_rect();
        let drag = ui.interact(bar, egui::Id::unique("titlebar"), Sense::click_and_drag());
        if drag.drag_started_by(egui::PointerButton::Primary) {
            actions.push(Action::DragWindow);
        }
        if drag.double_clicked() {
            actions.push(Action::ToggleMaximize);
        }

        let mut left_end = bar.left();
        let mut right_start = bar.right();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let logo = st.logo.get_or_insert_with(|| {
                let pixels = include_bytes!("../assets/icon_128.rgba");
                let image = egui::ColorImage::from_rgba_unmultiplied([128, 128], pixels);
                ui.ctx().load_texture("logo", image, egui::TextureOptions::LINEAR)
            });
            let (slot, _) = ui.allocate_exact_size(Vec2::new(RAIL_W + EDGE, HEADER_H), Sense::hover());
            let at = Rect::from_center_size(slot.center() + Vec2::new(EDGE * 0.5, 0.0), Vec2::splat(24.0));
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(logo.id(), at, uv, Color32::WHITE);
            let tip = st.keys.hint("Show or hide the files", Command::ToggleLeftPanel);
            if icon_button(ui, icons::PANEL_LEFT, Vec2::splat(30.0), st.files, pal().hover).on_hover_text(tip).clicked()
            {
                st.files = !st.files;
            }
            ui.add_space(4.0);
            let rescan = ui.add(
                egui::Button::new(
                    RichText::new(format!("{}  Rescan", icons::REFRESH_CW)).size(13.0).color(pal().text_dim),
                )
                .frame(false),
            );
            if rescan.on_hover_text(st.keys.hint("Reload the folder from disk", Command::Rescan)).clicked() {
                actions.push(Action::Rescan);
            }
            ui.add_space(8.0);

            left_end = ui.min_rect().right();

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let wb = Vec2::new(46.0, HEADER_H);
                let (rect, resp) = ui.allocate_exact_size(wb, Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(196, 43, 28));
                }
                let col = if resp.hovered() { Color32::WHITE } else { pal().text_dim };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, icons::X, FontId::proportional(16.0), col);
                if resp.on_hover_text("Close").clicked() {
                    actions.push(Action::CloseWindow);
                }
                for (glyph, tip, action) in [
                    (if st.maximized { icons::COPY } else { icons::SQUARE }, "Maximize", 1),
                    (icons::MINUS, "Minimize", 0),
                ] {
                    let (rect, resp) = ui.allocate_exact_size(wb, Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, pal().hover);
                    }
                    let col = if resp.hovered() { pal().text } else { pal().text_dim };
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), col);
                    if resp.on_hover_text(tip).clicked() {
                        actions.push(if action == 1 { Action::ToggleMaximize } else { Action::Minimize });
                    }
                }
                ui.add_space(6.0);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(34.0, 28.0), Sense::click());
                if resp.hovered() || st.layout_open {
                    ui.painter().rect_filled(rect, 7.0, pal().hover);
                }
                let col = if resp.hovered() || st.layout_open { pal().text } else { pal().text_dim };
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    icons::PANELS_TOP_LEFT,
                    FontId::proportional(15.0),
                    col,
                );
                if resp.on_hover_text("Customize the layout").clicked() {
                    st.layout_open = !st.layout_open;
                }
                st.layout_button = Some(rect);
                right_start = ui.min_rect().left();
            });
        });

        let room = (right_start - left_end - 24.0).max(0.0);
        let w = room.min(440.0);
        if w >= 150.0 {
            let center = bar.center().x.clamp(left_end + 12.0 + w * 0.5, right_start - 12.0 - w * 0.5);
            let rect = Rect::from_center_size(Pos2::new(center, bar.center().y), Vec2::new(w, 28.0));
            let resp = ui.interact(rect, egui::Id::unique("goto"), Sense::click());
            let fill = if resp.hovered() { pal().raised } else { pal().row };
            ui.painter().rect(rect, 7.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
            ui.painter().text(
                Pos2::new(rect.left() + 12.0, rect.center().y),
                Align2::LEFT_CENTER,
                icons::SEARCH,
                FontId::proportional(14.0),
                pal().text_dim,
            );
            let hint = if w > 330.0 { "Go to a file or folder by name" } else { "Go to file" };
            let hint = st.keys.hint(hint, Command::GoToFile);
            ui.painter().text(
                Pos2::new(rect.left() + 34.0, rect.center().y),
                Align2::LEFT_CENTER,
                hint,
                FontId::proportional(13.0),
                pal().text_dim,
            );
            if resp.clicked() {
                st.quick = Some(QuickOpen { query: String::new(), selected: 0, focus: true });
            }
        }
    });
}

fn layout_menu(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    const WIDTH: f32 = 330.0;
    let Some(button) = st.layout_button.filter(|_| st.layout_open) else { return };
    let at = Pos2::new(button.right() + 4.0, button.bottom() + 6.0);
    let area = egui::Area::new(egui::Id::unique("layout"))
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(at)
        .order(egui::Order::Foreground);
    let mut prefs = st.prefs.clone();
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border_strong))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(6, 6));
        frame.show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 1.0;
            let head = |ui: &mut egui::Ui, text: &str, reset: bool| {
                let mut again = false;
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new(text).size(11.5).color(pal().text_faint));
                    if reset {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            again = small_icon(ui, icons::ROTATE_CCW, "Put every part back");
                        });
                    }
                });
                again
            };
            if head(ui, "Visible", true) {
                (prefs.rail, prefs.status, prefs.toolbar, prefs.dock_right) = (true, true, true, false);
                st.files = true;
            }
            let row = |ui: &mut egui::Ui, glyph: &str, label: &str, key: String, on: bool| {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, pal().hover);
                }
                let mid = rect.center().y;
                ui.painter().text(
                    Pos2::new(rect.left() + 18.0, mid),
                    Align2::CENTER_CENTER,
                    glyph,
                    FontId::proportional(14.0),
                    pal().text_dim,
                );
                ui.painter().text(
                    Pos2::new(rect.left() + 36.0, mid),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(13.0),
                    pal().text,
                );
                let mut right = rect.right() - 10.0;
                if on {
                    ui.painter().text(
                        Pos2::new(right, mid),
                        Align2::RIGHT_CENTER,
                        icons::CHECK,
                        FontId::proportional(14.0),
                        pal().accent,
                    );
                }
                right -= 24.0;
                if !key.is_empty() {
                    let font = FontId::proportional(11.5);
                    ui.painter().text(Pos2::new(right, mid), Align2::RIGHT_CENTER, key, font, pal().text_faint);
                }
                resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
            };
            let keys = &st.keys;
            if row(ui, icons::LAYOUT_GRID, "Bar of icons", String::new(), prefs.rail) {
                prefs.rail = !prefs.rail;
            }
            if row(ui, icons::PANEL_LEFT, "Files and source control", keys.label(Command::ToggleLeftPanel), st.files) {
                st.files = !st.files;
            }
            if row(ui, icons::PANEL_BOTTOM, "Status bar", String::new(), prefs.status) {
                prefs.status = !prefs.status;
            }
            if row(ui, icons::MOUSE_POINTER_2, "Toolbar of the canvas", String::new(), prefs.toolbar) {
                prefs.toolbar = !prefs.toolbar;
            }
            ui.add_space(4.0);
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top() - 2.0, Stroke::new(1.0, pal().border));
            head(ui, "Files and the bar of icons", false);
            if row(ui, icons::PANEL_LEFT, "On the left", String::new(), !prefs.dock_right) {
                prefs.dock_right = false;
            }
            if row(ui, icons::PANEL_RIGHT, "On the right", String::new(), prefs.dock_right) {
                prefs.dock_right = true;
            }
            ui.add_space(4.0);
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top() - 2.0, Stroke::new(1.0, pal().border));
            head(ui, "Modes", false);
            if row(ui, icons::FULLSCREEN, "Full screen", keys.label(Command::FullScreen), st.full_screen) {
                actions.push(Action::ToggleFullScreen);
            }
            if row(ui, icons::APP_WINDOW, "Only the canvas", keys.label(Command::ToggleUi), st.bare) {
                actions.push(Action::ToggleUi);
                close = true;
            }
        });
    });
    let clicked_away = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|at| !resp.response.rect.contains(at) && !button.contains(at));
    st.floating.push(resp.response.rect);
    if prefs != st.prefs {
        actions.push(Action::SetPrefs(prefs));
    }
    if close || clicked_away {
        st.layout_open = false;
    }
}

const STATUS_H: f32 = 22.0;

fn status_bar(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &Extensions,
    page: Option<&Page>,
    info: &FrameInfo,
    actions: &mut Vec<Action>,
) {
    let mut panel_to_show = None;
    let (mut update_now, mut save) = (false, false);
    let frame = Frame::NONE.fill(pal().chrome).inner_margin(Margin::symmetric(12, 0));
    egui::Panel::bottom("status").frame(frame).exact_size(STATUS_H).show_separator_line(false).show(ui, |ui| {
        let most = (ui.available_width() * 0.5).max(120.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let version = RichText::new(format!("v{}", crate::update::current())).size(11.5).color(pal().text_faint);
            let version = ui.add(egui::Label::new(version).sense(Sense::click()));
            if version.on_hover_text("The version of Code Canvas: click for Settings and updates").clicked() {
                st.settings.show(Some(settings::Tab::General));
            }
            if st.new_build {
                let offer = RichText::new(format!("{}  Restart into the new build", icons::REFRESH_CW)).size(12.0).color(pal().accent);
                let label = ui.add(egui::Label::new(offer).sense(Sense::click()));
                let tip = "A new build of Code Canvas was put in place of this one. Click to restart into it when it suits you (an assistant's chat open here restarts too). Open folders and files come back as they are.";
                if label.on_hover_text(tip).clicked() {
                    actions.push(Action::RestartIntoNewBuild);
                }
            }
            if let crate::update::State::Available(new) = &st.update {
                let glyph = crate::codicons::glyph("cloud-download").unwrap_or(icons::DOWNLOAD);
                let offer = RichText::new(format!("{glyph} Update to {new}")).size(12.0).color(pal().accent);
                let label = ui.add(egui::Label::new(offer).sense(Sense::click()));
                if label.on_hover_text("Download the new version and restart into it").clicked() {
                    update_now = true;
                }
            } else if let crate::update::State::Installing(new) = &st.update {
                ui.label(RichText::new(format!("Downloading {new}…")).size(12.0).color(pal().accent));
            }
            if let Some(git) = page.and_then(|p| p.git.as_ref()) {
                let icon = crate::codicons::glyph("source-control").unwrap_or(icons::GIT_BRANCH);
                let mut text = format!("{icon} {}{}", git.branch, if git.changes.is_empty() { "" } else { "*" });
                if git.ahead > 0 {
                    text.push_str(&format!(" ↑{}", git.ahead));
                }
                if git.behind > 0 {
                    text.push_str(&format!(" ↓{}", git.behind));
                }
                let label =
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(pal().text)).sense(Sense::click()));
                if label.on_hover_text("Source control").clicked() {
                    panel_to_show = Some(Panel::Changes);
                }
            }
            let unsaved = page.map_or(0, |p| p.unsaved().count());
            if unsaved > 0 {
                let text = format!("●  {unsaved} not saved");
                let label =
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(pal().text)).sense(Sense::click()));
                let tip = st.keys.hint("Typed but not written to disk yet: click to save", Command::Save);
                save = label.on_hover_text(tip).clicked();
            }
            let (errors, warnings) = ext.totals();
            {
                let text = format!("{}  {errors}   {}  {warnings}", icons::CIRCLE_ALERT, icons::TRIANGLE_ALERT);
                let color = match (errors, warnings) {
                    (0, 0) if st.is_open(&Panel::Problems) => pal().text,
                    (0, 0) => pal().text_dim,
                    (0, _) => c(theme::get().problem_warning),
                    _ => c(theme::get().problem_error),
                };
                let label = ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).sense(Sense::click()));
                if label.on_hover_text("Problems reported by extensions: click to list them").clicked() {
                    panel_to_show = Some(Panel::Problems);
                }
            }
            for (_, text) in ext.status_items.iter().rev() {
                ui.label(RichText::new(text).size(12.0).color(pal().text_faint));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let said = RichText::new(&info.status).size(12.0).color(pal().text_dim);
                let more = page.and_then(|page| page.ready().map(|r| (page, r))).map(|(page, r)| {
                    format!(
                        "{}\n{} files  ·  {} lines  ·  {} symbols\nread in {} ms",
                        page.root.display(),
                        r.repo.files.len(),
                        r.repo.total_lines,
                        r.repo.symbols.count,
                        r.load_ms
                    )
                });
                ui.set_max_width(most.min(ui.available_width()));
                let said = ui.add(egui::Label::new(said).truncate());
                if let Some(more) = more {
                    said.on_hover_text(more);
                }
            });
        });
    });
    if let Some(panel) = panel_to_show {
        st.toggle(panel);
    }
    if update_now {
        actions.push(Action::InstallUpdate);
    }
    if save {
        actions.push(Action::SaveFile);
    }
}

fn rail_frame(on_right: bool) -> Frame {
    let edge = EDGE as i8;
    let margin = match on_right {
        false => Margin { left: edge, right: 0, top: 0, bottom: edge },
        true => Margin { left: 0, right: edge, top: 0, bottom: edge },
    };
    Frame::NONE.fill(pal().chrome).inner_margin(margin)
}

fn rail_tab(painter: &egui::Painter, icon: Rect, bar: Rect, toward_right: bool) {
    // The flares reach 2 * FLARE past the tab, more than the gap above the first icon; keep the
    // edge by the dock unclipped so the tab still covers the seam into the dock.
    let painter = painter.with_clip_rect(bar.shrink2(Vec2::new(0.0, 1.0)));
    const FLARE: f32 = 7.0;
    let (tab, round, inner) = match toward_right {
        true => (
            Rect::from_min_max(icon.left_top() + Vec2::new(3.0, 0.0), Pos2::new(bar.right(), icon.bottom())),
            CornerRadius { nw: 9, sw: 9, ne: 0, se: 0 },
            bar.right() - FLARE,
        ),
        false => (
            Rect::from_min_max(Pos2::new(bar.left(), icon.top()), icon.right_bottom() - Vec2::new(3.0, 0.0)),
            CornerRadius { nw: 0, sw: 0, ne: 9, se: 9 },
            bar.left() + FLARE,
        ),
    };
    painter.rect_filled(tab, round, pal().row);
    let edge_x = if toward_right { tab.right() } else { tab.left() };
    for (edge, away) in [(tab.top(), -FLARE), (tab.bottom(), FLARE)] {
        let square = Rect::from_two_pos(Pos2::new(inner, edge), Pos2::new(edge_x, edge + away));
        painter.rect_filled(square, 0.0, pal().row);
        painter.circle_filled(Pos2::new(inner, edge + away), FLARE, pal().well);
    }
}

const RAIL_ICON: Vec2 = Vec2::splat(34.0);
const RAIL_REACH: f32 = 56.0;

fn files_dock(
    ui: &mut egui::Ui,
    st: &mut UiState,
    pages: &mut [Page],
    active: usize,
    ext: &mut Extensions,
    actions: &mut Vec<Action>,
) {
    let right = st.prefs.dock_right;
    let dock = match right {
        false => egui::Panel::left("left"),
        true => egui::Panel::right("left on the right"),
    };
    let dock = dock.resizable(true).default_size(290.0).size_range(230.0..=560.0);
    let frame = Frame::NONE.fill(pal().chrome).inner_margin(Margin { left: 0, right: 0, top: 0, bottom: EDGE as i8 });
    let was = ui.style().visuals.widgets.clone();
    let widgets = &mut ui.style_mut().visuals.widgets;
    (widgets.hovered.fg_stroke, widgets.active.fg_stroke) = (Stroke::NONE, Stroke::NONE);
    dock.frame(frame).show_separator_line(false).show(ui, |ui| {
        ui.style_mut().visuals.widgets = was.clone();
        let all = ui.max_rect();
        let round = match right {
            false => CornerRadius { nw: 0, sw: 0, ne: ROUND as u8, se: ROUND as u8 },
            true => CornerRadius { nw: ROUND as u8, sw: ROUND as u8, ne: 0, se: 0 },
        };
        ui.painter().rect(all, round, pal().row, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
        let by_bar = if right { all.right() - 1.0 } else { all.left() };
        let seam = Rect::from_min_max(Pos2::new(by_bar, all.top() + 1.0), Pos2::new(by_bar + 1.0, all.bottom() - 1.0));
        ui.painter().rect_filled(seam, 0.0, pal().row);
        ui.set_min_height(ui.available_height());
        ui.set_width(ui.available_width());
        match st.dock {
            Panel::Changes => {
                folder_row(ui, st, pages, active, actions, false);
                workspace_list(ui, st, actions);
                git_panel::changes_panel(ui, &mut st.file_icons, pages.get_mut(active), actions);
            }
            Panel::Search => {
                folder_row(ui, st, pages, active, actions, false);
                search_panel(ui, st, pages.get(active), actions);
            }
            Panel::Extensions => ext_panels::extensions_panel(ui, st, ext, actions),
            _ => files_panel(ui, st, pages, active, actions),
        }
    });
    ui.style_mut().visuals.widgets = was;
}

fn card_of(
    ui: &mut egui::Ui,
    st: &mut UiState,
    pages: &mut [Page],
    active: usize,
    ext: &mut Extensions,
    actions: &mut Vec<Action>,
    panel: &Panel,
) {
    match panel {
        Panel::Files => {}
        Panel::Search | Panel::Extensions => {}
        Panel::Problems => ext_panels::problems_panel(ui, pages.get(active), ext, actions),
        Panel::Changes => {}
        Panel::Extension => ext_panels::extension_view(ui, st, ext),
        Panel::Help => help_panel(ui, &st.keys),
        Panel::Symbols => symbols_panel(ui, st, pages.get(active), actions),
        Panel::Deps => deps_panel::deps_panel(ui, st, pages.get(active), actions),
        Panel::Webview(view) => {
            let title = ext.shown(view).map(|shown| shown.title.clone()).unwrap_or_default();
            Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
                panel_title(ui, &title, |_| {});
            });
            ext_panels::webview_panel(ui, st, ext, view)
        }
        Panel::Media => ext_panels::media_panel(ui, st, actions),
        Panel::Console => console::console_panel(ui, st, actions),
        Panel::Tree(id) => ext_panels::tree_panel(ui, st, ext, id),
    }
}

fn cards(
    ctx: &egui::Context,
    st: &mut UiState,
    canvas: Rect,
    pages: &mut [Page],
    active: usize,
    ext: &mut Extensions,
    actions: &mut Vec<Action>,
) {
    const AWAY: f32 = 8.0;
    const BETWEEN: f32 = 6.0;
    const NARROWEST: f32 = 250.0;
    const LOWEST: f32 = 150.0;
    const UNDER: f32 = 50.0;
    let mut next = canvas.right() - AWAY;
    const TOOLBAR_REACH: f32 = 420.0;
    let height = (canvas.height() - AWAY - UNDER).max(120.0);
    st.cards_reach = 0.0;
    for panel in st.cards.clone().iter().rev() {
        let key = panel.key();
        let room = next - canvas.left() - AWAY;
        let free = st.rails.cards.get(&key).copied();
        if free.is_none() && room < NARROWEST {
            continue;
        }
        let usual = match panel {
            Panel::Console => 600.0,
            page if page.is_page() => 420.0,
            _ => 330.0,
        };
        let wanted = st.card_widths.get(&key).copied().unwrap_or(usual);
        let width = wanted.clamp(NARROWEST, room.min(760.0));
        let rect = match free {
            Some([x, y, w, h]) => {
                let w = w.clamp(NARROWEST, (canvas.width() - 2.0 * AWAY).max(NARROWEST));
                let h = h.clamp(LOWEST, (canvas.height() - 2.0 * AWAY).max(LOWEST));
                let x = x.clamp(AWAY, (canvas.width() - w - AWAY).max(AWAY));
                let y = y.clamp(AWAY, (canvas.height() - h - AWAY).max(AWAY));
                Rect::from_min_size(canvas.min + Vec2::new(x, y), Vec2::new(w, h)).round()
            }
            None => {
                let over_toolbar = st.prefs.toolbar && next - width < canvas.left() + TOOLBAR_REACH;
                let tall = if over_toolbar { height } else { (canvas.height() - 2.0 * AWAY).max(120.0) };
                Rect::from_min_size(Pos2::new(next - width, canvas.top() + AWAY), Vec2::new(width, tall))
            }
        };
        let mut placed = None;
        let mut settled = false;
        let id = egui::Id::unique(("card", &key));
        let own_cross = *panel == Panel::Media;
        let mut shut = false;
        let first_page = st.page_places.len();
        egui::Area::new(id).fixed_pos(rect.min).order(egui::Order::Middle).show(ctx, |ui| {
            ui.painter().rect(rect, 12.0, pal().card, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
            let title = Rect::from_min_max(rect.left_top(), Pos2::new(rect.right() - 34.0, rect.top() + 40.0));
            let hold = ui.interact(title, id.with("hold"), Sense::click_and_drag());
            if hold.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                placed = Some(rect.translate(hold.drag_delta()));
            }
            settled |= hold.drag_stopped();
            if hold.double_clicked() && free.is_some() {
                st.rails.cards.remove(&key);
                settled = true;
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(1.0)), |ui| {
                ui.set_clip_rect(rect.shrink(1.0));
                ui.set_min_size(rect.shrink(1.0).size());
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique(CARD_X), !own_cross));
                card_of(ui, st, pages, active, ext, actions, panel);
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique(CARD_X), false));
            });
            if !own_cross {
                let cross = Rect::from_min_size(Pos2::new(rect.right() - 33.0, rect.top() + 11.0), Vec2::splat(24.0));
                let resp = ui.interact(cross, id.with("cross"), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(cross, 6.0, pal().hover);
                }
                let color = if resp.hovered() { pal().text } else { pal().text_dim };
                ui.painter().text(cross.center(), Align2::CENTER_CENTER, icons::X, FontId::proportional(14.0), color);
                shut = resp.on_hover_text("Close").clicked();
            }
            let grip = Rect::from_min_max(
                Pos2::new(rect.left() - 5.0, rect.top() + 14.0),
                Pos2::new(rect.left() + 3.0, rect.bottom() - 14.0),
            );
            let pull = ui.interact(grip, id.with("width"), Sense::drag());
            if pull.hovered() || pull.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                let line =
                    Rect::from_center_size(Pos2::new(rect.left(), grip.center().y), Vec2::new(2.0, grip.height()));
                ui.painter().rect_filled(line, 1.0, pal().border_strong);
            }
            if pull.dragged() && free.is_some() {
                let left = (rect.left() + pull.drag_delta().x).min(rect.right() - NARROWEST);
                placed = Some(Rect::from_min_max(Pos2::new(left, rect.top()), rect.max));
            } else if pull.dragged() {
                st.card_widths.insert(key.clone(), (width - pull.drag_delta().x).clamp(NARROWEST, 760.0));
            }
            settled |= pull.drag_stopped() && free.is_some();
            let (r, b) = (rect.right(), rect.bottom());
            let sides = [
                (
                    "right",
                    Rect::from_min_max(Pos2::new(r - 3.0, rect.top() + 14.0), Pos2::new(r + 5.0, b - 14.0)),
                    [1.0, 0.0],
                ),
                (
                    "bottom",
                    Rect::from_min_max(Pos2::new(rect.left() + 14.0, b - 3.0), Pos2::new(r - 14.0, b + 5.0)),
                    [0.0, 1.0],
                ),
                ("corner", Rect::from_min_max(Pos2::new(r - 14.0, b - 14.0), Pos2::new(r + 5.0, b + 5.0)), [1.0, 1.0]),
            ];
            for (name, grip, [wider, taller]) in sides {
                let pull = ui.interact(grip, id.with(name), Sense::drag());
                if pull.hovered() || pull.dragged() {
                    ui.ctx().set_cursor_icon(match (wider > 0.0, taller > 0.0) {
                        (true, true) => egui::CursorIcon::ResizeNwSe,
                        (true, false) => egui::CursorIcon::ResizeHorizontal,
                        _ => egui::CursorIcon::ResizeVertical,
                    });
                    let mark = Stroke::new(2.0, pal().border_strong);
                    if wider > 0.0 {
                        ui.painter().vline(
                            r - 1.0,
                            (b - 26.0 * taller - (1.0 - taller) * (rect.height() - 28.0))..=(b - 14.0),
                            mark,
                        );
                    }
                    if taller > 0.0 {
                        ui.painter().hline(
                            (r - 26.0 * wider - (1.0 - wider) * (rect.width() - 28.0))..=(r - 14.0),
                            b - 1.0,
                            mark,
                        );
                    }
                }
                if pull.dragged() {
                    let grown = rect.size() + Vec2::new(pull.drag_delta().x * wider, pull.drag_delta().y * taller);
                    placed = Some(Rect::from_min_size(rect.min, grown.max(Vec2::new(NARROWEST, LOWEST))));
                }
                settled |= pull.drag_stopped();
            }
        });
        if let Some(at) = placed {
            let from = at.min - canvas.min;
            st.rails.cards.insert(key.clone(), [from.x, from.y, at.width(), at.height()]);
        }
        if settled {
            actions.push(Action::RailsChanged);
        }
        st.card_places.push(CardPlace {
            over: rect.expand(5.0),
            pages: first_page..st.page_places.len(),
            floating: st.floating.len(),
            layer: egui::LayerId::new(egui::Order::Middle, id),
        });
        st.floating.push(rect.expand(5.0));
        if shut {
            st.close_panel(panel);
        }
        if free.is_none() {
            next = rect.left() - BETWEEN;
            st.cards_reach = canvas.right() - rect.left() + AWAY;
        }
    }
}

fn rail_entries(st: &UiState, ext: &Extensions) -> Vec<RailEntry> {
    let own = |panel: Panel, glyph: &'static str, tip: String| RailEntry { panel, tip, look: RailLook::Glyph(glyph) };
    let mut entries = vec![
        own(Panel::Files, icons::FOLDER, st.keys.hint("Files", Command::ToggleLeftPanel)),
        own(Panel::Search, icons::TEXT_SEARCH, st.keys.hint("Search in files", Command::SearchInFiles)),
        own(
            Panel::Changes,
            crate::codicons::glyph("source-control").unwrap_or(icons::GIT_GRAPH),
            "Source control".to_string(),
        ),
        own(Panel::Extensions, crate::codicons::glyph("extensions").unwrap_or(icons::PUZZLE), "Extensions".to_string()),
        own(Panel::Symbols, icons::LIST_TREE, "Symbols: selection, filters, names".to_string()),
        own(Panel::Deps, icons::WAYPOINTS, "Dependencies: which file imports which".to_string()),
        own(
            Panel::Console,
            crate::codicons::glyph("terminal").unwrap_or(icons::CHEVRON_RIGHT),
            st.keys.hint("Console: bash in the folder", Command::ToggleConsole),
        ),
    ];
    if st.media.is_some() {
        entries.push(own(Panel::Media, icons::FILE_TEXT, "The Markdown file being read".to_string()));
    }
    for item in ext.rail_items() {
        entries.push(RailEntry {
            panel: Panel::of_rail(item.view),
            tip: item.title,
            look: RailLook::Extension { id: item.extension, codicon: item.codicon },
        });
    }
    entries
}

fn paint_rail_icon(
    ui: &egui::Ui,
    st: &mut UiState,
    ext: &Extensions,
    entry: &RailEntry,
    rect: Rect,
    active: Option<Rect>,
    hovered: bool,
) {
    match active {
        Some(bar) if entry.panel == st.dock => rail_tab(ui.painter(), rect, bar, !st.prefs.dock_right),
        Some(_) => {
            ui.painter().rect_filled(rect.shrink(3.0), 8.0, pal().selection.gamma_multiply(0.7));
        }
        None if hovered => {
            ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().hover);
        }
        None => {}
    }
    let active = active.is_some();
    let color = if active || hovered { pal().text } else { pal().text_dim };
    let glyph = |shape: &str| {
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, shape, FontId::proportional(17.0), color);
    };
    match &entry.look {
        RailLook::Glyph(shape) => glyph(shape),
        RailLook::Extension { id, codicon } => match codicon.as_deref().and_then(crate::codicons::glyph) {
            Some(codicon) => glyph(codicon),
            None => {
                let installed = ext.installed.iter().find(|e| e.id == *id);
                match installed.and_then(|e| ext_panels::extension_icon(ui.ctx(), &mut st.ext_icons, e)) {
                    Some(texture) => {
                        let at = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
                        egui::Image::new(&texture).corner_radius(4).paint_at(ui, at);
                    }
                    None => glyph(icons::PUZZLE),
                }
            }
        },
    }
}

fn rail_badge(ui: &egui::Ui, ext: &Extensions, page: Option<&Page>, panel: &Panel, icon: Rect) {
    match panel {
        Panel::Problems => {
            let (errors, warnings) = ext.totals();
            if errors + warnings > 0 {
                let color = if errors > 0 { c(theme::get().problem_error) } else { c(theme::get().problem_warning) };
                ui.painter().circle_filled(icon.right_top() + Vec2::new(-9.0, 9.0), 3.5, color);
            }
        }
        Panel::Changes => {
            let changed = page.and_then(|p| p.git.as_ref()).map_or(0, |g| g.changes.len());
            if changed > 0 {
                count_badge(ui, icon, changed);
            }
        }
        Panel::Webview(view) if view.starts_with(crate::ext::TABS) => {
            count_badge(ui, icon, ext.tabs_of(&view[crate::ext::TABS.len()..]).count());
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn rail(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &Extensions,
    page: Option<&Page>,
    side: Side,
    entries: &[RailEntry],
    order: &[String],
    actions: &mut Vec<Action>,
) {
    if side == Side::Right && order.is_empty() && st.rail_drag.is_none() {
        return;
    }
    let on_right = (side == Side::Right) != st.prefs.dock_right;
    let bar = match (side, on_right) {
        (Side::Left, false) => egui::Panel::left("rail_left"),
        (Side::Left, true) => egui::Panel::right("rail_left"),
        (Side::Right, true) => egui::Panel::right("rail_right"),
        (Side::Right, false) => egui::Panel::left("rail_right"),
    };
    let joined = side == Side::Left && st.files;
    bar.frame(rail_frame(on_right)).exact_size(RAIL_W + EDGE).resizable(false).show_separator_line(false).show(
        ui,
        |ui| {
            let zone = ui.max_rect();
            let round = match (joined, on_right) {
                (true, false) => CornerRadius { nw: ROUND as u8, sw: ROUND as u8, ne: 0, se: 0 },
                (true, true) => CornerRadius { nw: 0, sw: 0, ne: ROUND as u8, se: ROUND as u8 },
                (false, _) => CornerRadius::same(ROUND as u8),
            };
            ui.painter().rect(zone, round, pal().well, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
            if joined {
                let by_dock = if on_right { zone.left() } else { zone.right() - 1.0 };
                let seam = Rect::from_min_max(
                    Pos2::new(by_dock, zone.top() + 1.0),
                    Pos2::new(by_dock + 1.0, zone.bottom() - 1.0),
                );
                ui.painter().rect_filled(seam, 0.0, pal().well);
            }
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(5.0);
            let mut slots: Vec<(String, Rect)> = Vec::new();
            for entry in order.iter().filter_map(|key| entries.iter().find(|entry| entry.panel.key() == *key)) {
                let key = entry.panel.key();
                let (slot, resp) =
                    ui.allocate_exact_size(Vec2::new(zone.width(), RAIL_ICON.y), Sense::click_and_drag());
                let rect = Rect::from_center_size(slot.center(), RAIL_ICON);
                slots.push((key.clone(), rect));
                let lifted = st.rail_drag.as_ref().is_some_and(|drag| drag.key == key);
                if !lifted {
                    let active = st.is_open(&entry.panel).then_some(zone);
                    paint_rail_icon(ui, st, ext, entry, rect, active, resp.hovered());
                    rail_badge(ui, ext, page, &entry.panel, rect);
                }
                if resp.drag_started_by(egui::PointerButton::Primary) {
                    st.rail_drag = Some(RailDrag { key, target: None });
                }
                let other = side.other();
                let offer = match other {
                    Side::Left => "Move to the left bar",
                    Side::Right => "Move to the right bar",
                };
                resp.context_menu(|ui| {
                    ui.set_width(200.0);
                    if menu_row(ui, icons::PANEL_LEFT, offer) {
                        st.rail_move = Some((entry.panel.clone(), other, None));
                        ui.close();
                    }
                });
                if resp.on_hover_text(format!("{}\nDrag it to reorder it or to the other bar", entry.tip)).clicked() {
                    st.toggle(entry.panel.clone());
                }
            }
            rail_drop_target(ui, st, side, zone, &slots);
            if side == Side::Right {
                return;
            }
            ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                ui.add_space(5.0);
                account_button(ui, st, actions);
                let tip = st.keys.hint("Settings: version, theme, colours, shortcuts", Command::Settings);
                if icon_button(ui, icons::SETTINGS, RAIL_ICON, st.settings.open, pal().hover)
                    .on_hover_text(tip)
                    .clicked()
                {
                    st.settings.toggle();
                }
                let help = st.is_open(&Panel::Help);
                let tip = "Keyboard and mouse shortcuts";
                if icon_button(ui, icons::CIRCLE_HELP, RAIL_ICON, help, pal().hover).on_hover_text(tip).clicked() {
                    st.toggle(Panel::Help);
                }
                share_button(ui, st, actions);
            });
        },
    );
}

fn account_button(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    st.account.poll(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(RAIL_ICON, Sense::click());
    st.account.button = Some(rect);
    if resp.hovered() || st.account.menu {
        ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().hover);
    }
    let account = &st.account;
    match (&account.user, &account.face) {
        (Some(_), Some(face)) => {
            let at = Rect::from_center_size(rect.center(), Vec2::splat(22.0));
            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            let shape = egui::epaint::RectShape::filled(at, CornerRadius::same(11), Color32::WHITE);
            ui.painter().add(shape.with_texture(face.id(), whole));
        }
        (user, _) => {
            let glyph = if user.is_some() { icons::GITHUB } else { icons::CIRCLE_USER };
            let color = if resp.hovered() { pal().text } else { pal().text_dim };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), color);
        }
    }
    let tip = match &account.user {
        Some(user) => format!("GitHub: {user}\nYour settings and extensions are kept on your GitHub"),
        None => "Sign in with GitHub to keep your settings and extensions, and have them on any computer".to_string(),
    };
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
        match st.account.user.is_some() {
            true => st.account.menu = !st.account.menu,
            false => actions.push(Action::Account(AccountOp::SignIn)),
        }
    }
}

fn account_menu(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    const WIDTH: f32 = 280.0;
    let (Some(button), Some(user)) = (st.account.button.filter(|_| st.account.menu), st.account.user.clone()) else {
        return;
    };
    let area = egui::Area::new(egui::Id::unique("account"))
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(Pos2::new(button.right() + 10.0, button.bottom()))
        .order(egui::Order::Foreground);
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border_strong))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(12));
        frame.show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.label(icon(icons::GITHUB, 15.0, pal().text_dim));
                ui.label(RichText::new(&user).size(14.0).strong().color(pal().text));
            });
            let note = "Settings, colours, shortcuts and the list of extensions are kept in a secret gist of your GitHub, so another computer where you sign in gets them too.";
            ui.add(egui::Label::new(RichText::new(note).size(11.5).color(pal().text_dim)).wrap());
            if !st.account.synced.is_empty() {
                ui.label(RichText::new(&st.account.synced).size(11.5).color(pal().text_faint));
            }
            ui.horizontal(|ui| {
                if ext_panels::pill(ui, "Sync now", false) {
                    actions.push(Action::Account(AccountOp::SyncNow));
                }
                if ext_panels::pill(ui, "Sign out", false) {
                    actions.push(Action::Account(AccountOp::SignOut));
                }
            });
        });
    });
    st.floating.push(resp.response.rect);
    let away = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|at| !resp.response.rect.contains(at) && !button.contains(at));
    if away || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.account.menu = false;
    }
}

fn sign_in_modal(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    if !st.account.signing {
        return;
    }
    st.floating.push(ctx.content_rect());
    st.page_places.clear();
    let frame = Frame::NONE
        .fill(pal().card)
        .stroke(Stroke::new(1.0, pal().border_strong))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(20));
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Modal::new(egui::Id::unique("sign in")).backdrop_color(Color32::from_black_alpha(150)).frame(frame).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            ui.label(icon(icons::GITHUB, 18.0, pal().text));
            ui.label(RichText::new("Sign in with GitHub").size(15.5).strong().color(pal().text));
        });
        let about = "Your settings and extensions will be kept in a secret gist of your own GitHub. Nothing else of your account is used.";
        ui.add(egui::Label::new(RichText::new(about).size(12.5).color(pal().text_dim)).wrap());
        match &st.account.code {
            Some((code, page)) => {
                let step = format!("Enter this code at {page}:");
                ui.label(RichText::new(step).size(12.5).color(pal().text));
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(code).font(FontId::monospace(28.0)).color(pal().text));
                });
                ui.vertical_centered(|ui| {
                    if ext_panels::pill(ui, "Copy the code and open GitHub", true) {
                        actions.push(Action::Account(AccountOp::CopyAndOpen));
                    }
                });
                ui.label(RichText::new("Waiting for you on GitHub…").size(11.5).color(pal().text_faint));
            }
            None if st.account.said.is_empty() => {
                ui.label(RichText::new("Asking GitHub for a code…").size(12.0).color(pal().text_faint));
            }
            None => {}
        }
        if !st.account.said.is_empty() {
            let said = RichText::new(&st.account.said).size(12.0).color(c(theme::get().problem_error));
            ui.add(egui::Label::new(said).wrap());
        }
        ui.add_space(4.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ask_button(ui, "Cancel", ChoiceLook::Plain) {
                cancel = true;
            }
        });
    });
    if cancel {
        actions.push(Action::Account(AccountOp::Cancel));
    }
}

fn rail_drop_target(ui: &egui::Ui, st: &mut UiState, side: Side, zone: Rect, slots: &[(String, Rect)]) {
    let on_right = (side == Side::Right) != st.prefs.dock_right;
    let Some(drag) = st.rail_drag.as_mut() else { return };
    let reach = match on_right {
        false => Rect::from_min_max(zone.min, zone.max + Vec2::new(RAIL_REACH, 0.0)),
        true => Rect::from_min_max(zone.min - Vec2::new(RAIL_REACH, 0.0), zone.max),
    };
    let pointer = ui.input(|input| input.pointer.latest_pos()).filter(|at| reach.contains(*at));
    let Some(pointer) = pointer else { return };
    let others: Vec<&(String, Rect)> = slots.iter().filter(|(key, _)| *key != drag.key).collect();
    let next = others.iter().find(|(_, rect)| pointer.y < rect.center().y);
    let line = match (next, others.last()) {
        (Some((_, rect)), _) => rect.top() - 1.0,
        (None, Some((_, rect))) => rect.bottom() + 1.0,
        (None, None) => zone.top() + 2.0,
    };
    ui.painter().hline(zone.x_range(), line, Stroke::new(2.0, pal().accent));
    drag.target = Some((side, next.map(|(key, _)| key.clone())));
}

fn finish_rail_drag(ui: &egui::Ui, st: &mut UiState, ext: &Extensions, entries: &[RailEntry]) {
    let Some(drag) = st.rail_drag.clone() else { return };
    let Some(entry) = entries.iter().find(|entry| entry.panel.key() == drag.key) else {
        st.rail_drag = None;
        return;
    };
    let (held, pointer, cancelled) = ui.input(|input| {
        (input.pointer.primary_down(), input.pointer.latest_pos(), input.key_pressed(egui::Key::Escape))
    });
    if cancelled || !held {
        st.rail_drag = None;
        if let Some((side, before)) = drag.target.filter(|_| !cancelled) {
            st.rail_move = Some((entry.panel.clone(), side, before));
        }
        return;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let Some(pointer) = pointer else { return };
    egui::Area::new(egui::Id::unique("rail drag"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(pointer - RAIL_ICON * 0.5)
        .show(ui.ctx(), |ui| {
            let (rect, _) = ui.allocate_exact_size(RAIL_ICON, Sense::hover());
            ui.painter().rect_filled(rect.shrink(2.0), 9.0, pal().raised);
            paint_rail_icon(ui, st, ext, entry, rect, None, false);
        });
}

fn count_badge(ui: &egui::Ui, icon: Rect, count: usize) {
    let text = if count > 99 { "99+".to_string() } else { count.to_string() };
    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(9.5), pal().on_accent);
    let size = Vec2::new((galley.size().x + 7.0).max(14.0), 14.0);
    let pill = Rect::from_min_size(Pos2::new(icon.right() - size.x, icon.bottom() - size.y - 2.0), size);
    ui.painter().rect_filled(pill, 7.0, pal().accent);
    ui.painter().galley(pill.center() - galley.size() * 0.5, galley, pal().on_accent);
}

fn panel_title(ui: &mut egui::Ui, title: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(14.0).color(pal().text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if ui.ctx().data(|data| data.get_temp::<bool>(egui::Id::unique(CARD_X))).unwrap_or(false) {
                ui.add_space(26.0);
            }
            right(ui);
        });
    });
}

fn small_icon(ui: &mut egui::Ui, glyph: &str, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal().hover);
    }
    let color = if resp.hovered() { pal().text } else { pal().text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), color);
    resp.on_hover_text(tip).clicked()
}

fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str, glyph: &str) -> egui::Response {
    Frame::NONE
        .fill(pal().well)
        .stroke(Stroke::new(1.0, pal().border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icon(glyph, 14.0, pal().text_dim));
                ui.add(egui::TextEdit::singleline(text).hint_text(hint).frame(Frame::NONE).desired_width(f32::INFINITY))
            })
            .inner
        })
        .inner
}

fn file_glyph(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "json" | "jsonc" | "yaml" | "yml" => icons::BRACES,
        "toml" | "ini" | "cfg" | "lock" | "env" => icons::SETTINGS,
        "md" | "txt" | "rst" | "" => icons::FILE_TEXT,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" => icons::IMAGE,
        _ => icons::FILE_CODE,
    }
}

fn folder_switcher(
    ui: &mut egui::Ui,
    open: &mut bool,
    keys: &Keymap,
    pages: &[Page],
    active: usize,
    actions: &mut Vec<Action>,
) {
    let title = match pages.get(active) {
        Some(p) if p.is_loading() => format!("{} …", p.title),
        Some(p) => p.title.clone(),
        None => "No folder".into(),
    };
    let count = if pages.len() > 1 { format!("  {}/{}", active + 1, pages.len()) } else { String::new() };
    let label = format!("{}  {title}{count}  {}", icons::FOLDER, icons::CHEVRON_DOWN);
    let button = ui.menu_button(RichText::new(label).size(13.5).color(pal().text), |ui| {
        let font = FontId::proportional(13.0);
        let longest = pages
            .iter()
            .map(|p| ui.painter().layout_no_wrap(p.title.clone(), font.clone(), pal().text).size().x)
            .fold(0.0f32, f32::max);
        let width = (longest + 86.0).clamp(210.0, 420.0);
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 1.0;
        let row = |ui: &mut egui::Ui| ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
        for (i, p) in pages.iter().enumerate() {
            let (rect, resp) = row(ui);
            let close_at = Rect::from_center_size(Pos2::new(rect.right() - 14.0, rect.center().y), Vec2::splat(22.0));
            let close = ui.interact(close_at, ui.scope_id().with(("close folder", i)), Sense::click());
            if i == active {
                ui.painter().rect_filled(rect, 6.0, pal().raised);
            } else if ui.rect_contains_pointer(rect) {
                ui.painter().rect_filled(rect, 6.0, pal().hover);
            }
            let mid = rect.center().y;
            let color = if i == active { pal().text } else { pal().text_dim };
            ui.painter().text(
                Pos2::new(rect.left() + 10.0, mid),
                Align2::LEFT_CENTER,
                icons::FOLDER,
                font.clone(),
                color,
            );
            let clip = Rect::from_min_max(
                Pos2::new(rect.left() + 32.0, rect.top()),
                Pos2::new(close_at.left() - 4.0, rect.bottom()),
            );
            ui.painter().with_clip_rect(clip).text(
                clip.left_center(),
                Align2::LEFT_CENTER,
                &p.title,
                font.clone(),
                pal().text,
            );
            if ui.rect_contains_pointer(rect) {
                if close.hovered() {
                    ui.painter().rect_filled(close_at, 5.0, pal().border_strong);
                }
                ui.painter().text(
                    close_at.center(),
                    Align2::CENTER_CENTER,
                    icons::X,
                    FontId::proportional(13.0),
                    pal().text_dim,
                );
            }
            let files = match &p.state {
                PageState::Ready(r) => format!("{} files · ", r.repo.files.len()),
                PageState::Waiting => "not read yet · ".to_string(),
                _ => String::new(),
            };
            if close.on_hover_text(keys.hint("Close this folder", Command::CloseFolder)).clicked() {
                actions.push(Action::CloseTab(i));
                ui.close();
            } else if resp.on_hover_text(format!("{files}{}", p.root.display())).clicked() {
                actions.push(Action::SwitchTab(i));
                ui.close();
            }
        }
        if !pages.is_empty() {
            ui.add_space(3.0);
            let (line, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
            ui.painter().rect_filled(line, 0.0, pal().border);
            ui.add_space(3.0);
        }
        let (rect, resp) = row(ui);
        if resp.hovered() {
            ui.painter().rect_filled(rect, 6.0, pal().hover);
        }
        let mid = rect.center().y;
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, mid),
            Align2::LEFT_CENTER,
            icons::PLUS,
            font.clone(),
            pal().text_dim,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 32.0, mid),
            Align2::LEFT_CENTER,
            "Open folder…",
            font.clone(),
            pal().text,
        );
        ui.painter().text(
            Pos2::new(rect.right() - 10.0, mid),
            Align2::RIGHT_CENTER,
            keys.label(Command::OpenFolder),
            FontId::proportional(11.5),
            pal().text_faint,
        );
        if resp.clicked() {
            actions.push(Action::OpenFolder);
            ui.close();
        }
    });
    if std::mem::take(open) {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&button.response));
    }
    button.response.on_hover_text(keys.hint("Switch folder", Command::NextFolder));
}

fn files_panel(ui: &mut egui::Ui, st: &mut UiState, pages: &mut [Page], active: usize, actions: &mut Vec<Action>) {
    folder_row(ui, st, pages, active, actions, true);
    workspace_list(ui, st, actions);
    git_panel::timeline(ui, pages.get(active), &st.prefs, actions);
    explorer(ui, st, pages.get_mut(active), actions);
}

fn folder_row(
    ui: &mut egui::Ui,
    st: &mut UiState,
    pages: &mut [Page],
    active: usize,
    actions: &mut Vec<Action>,
    tools: bool,
) {
    Frame::NONE.inner_margin(Margin { left: 6, right: 8, top: 8, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if !tools {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        folder_switcher(ui, &mut st.open_folder_menu, &st.keys, pages, active, actions)
                    });
                    return;
                }
                if small_icon(ui, icons::PLUS, &st.keys.hint("Open another folder", Command::OpenFolder)) {
                    actions.push(Action::OpenFolder);
                }
                let split =
                    if st.single { "Show every open folder on the canvas" } else { "Show one folder at a time" };
                if pages.len() > 1
                    && small_icon(ui, icons::LAYOUT_GRID, &st.keys.hint(split, Command::ToggleSideBySide))
                {
                    actions.push(Action::ToggleSideBySide);
                }
                if small_icon(ui, icons::EYE, "Put every hidden file back on the canvas") {
                    actions.push(Action::ShowAll);
                }
                if small_icon(ui, icons::FOLD_VERTICAL, "Collapse every folder on the canvas") {
                    actions.push(Action::CollapseAll);
                }
                if small_icon(ui, icons::UNFOLD_VERTICAL, "Expand every folder on the canvas") {
                    actions.push(Action::ExpandAll);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    folder_switcher(ui, &mut st.open_folder_menu, &st.keys, pages, active, actions)
                });
            });
        });
    });
}

fn short_branch(branch: &str) -> String {
    const MOST: usize = 18;
    match branch.chars().count() > MOST {
        true => format!("{}…", branch.chars().take(MOST - 1).collect::<String>()),
        false => branch.to_string(),
    }
}

pub struct WorkspaceRow {
    pub path: std::path::PathBuf,
    pub title: String,
    pub files: Option<u32>,
    pub page: Option<usize>,
    pub active: bool,
    pub loading: bool,
    pub branch: Option<String>,
    pub branches: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceOp {
    Put(std::path::PathBuf),
    TakeOff(std::path::PathBuf),
    Forget(std::path::PathBuf),
    PutAll,
}

fn workspace_list(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    if st.workspace.len() < 2 {
        return;
    }
    let off = st.workspace.iter().filter(|row| row.page.is_none()).count();
    Frame::NONE.inner_margin(Margin { left: 8, right: 8, top: 6, bottom: 2 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.horizontal(|ui| {
            let glyph = if st.workspace_shut { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN };
            let (head, resp) = ui.allocate_exact_size(Vec2::new(110.0, 22.0), Sense::click());
            let mid = head.center().y;
            ui.painter().text(
                Pos2::new(head.left() + 2.0, mid),
                Align2::LEFT_CENTER,
                glyph,
                FontId::proportional(12.0),
                pal().text_faint,
            );
            let label = FontId::proportional(11.5);
            ui.painter().text(
                Pos2::new(head.left() + 20.0, mid),
                Align2::LEFT_CENTER,
                "WORKSPACE",
                label,
                pal().text_faint,
            );
            if resp.clicked() {
                st.workspace_shut = !st.workspace_shut;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if off > 0 && ext_panels::pill(ui, &format!("Put all {} on the canvas", st.workspace.len()), false) {
                    actions.push(Action::Workspace(WorkspaceOp::PutAll));
                }
            });
        });
        if st.workspace_shut {
            return;
        }
        let pointer = ui.ctx().pointer_latest_pos();
        for row in &st.workspace {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click_and_drag());
            let on = row.page.is_some();
            let hot = ui.rect_contains_pointer(rect);
            if row.active {
                ui.painter().rect_filled(rect, 6.0, pal().raised);
            } else if hot {
                ui.painter().rect_filled(rect, 6.0, pal().hover);
            }
            let mid = rect.center().y;
            let font = FontId::proportional(13.0);
            let tint = if on { pal().text } else { pal().text_faint };
            if on {
                ui.painter().circle_filled(Pos2::new(rect.left() + 6.0, mid), 2.5, pal().accent);
            }
            let folder = if on { icons::FOLDER_OPEN } else { icons::FOLDER };
            ui.painter().text(Pos2::new(rect.left() + 22.0, mid), Align2::CENTER_CENTER, folder, font.clone(), tint);
            let button = Rect::from_center_size(Pos2::new(rect.right() - 13.0, mid), Vec2::splat(22.0));
            let pressed = ui.interact(button, ui.scope_id().with(("workspace", &row.path)), Sense::click());
            let count = match (row.loading, &row.branch, row.files) {
                (true, ..) => "reading…".to_string(),
                (false, Some(branch), _) => format!("{} {}", icons::GIT_BRANCH, short_branch(branch)),
                (false, None, Some(files)) => files.to_string(),
                (false, None, None) => String::new(),
            };
            let right = if hot { button.left() - 4.0 } else { rect.right() - 8.0 };
            let shade = if row.branch.is_some() && on { pal().text_dim } else { pal().text_faint };
            let counted = ui.painter().layout_no_wrap(count, FontId::proportional(11.0), shade);
            let name_end = right - counted.size().x - 8.0;
            let chip = Rect::from_min_size(
                Pos2::new(right - counted.size().x - 4.0, mid - 10.0),
                Vec2::new(counted.size().x + 8.0, 20.0),
            );
            let switchable = row.active && !row.branches.is_empty();
            let mut chip_hot = false;
            if switchable {
                let place = ui.interact(chip, ui.scope_id().with(("branch", &row.path)), Sense::click());
                chip_hot = place.hovered();
                if chip_hot {
                    ui.painter().rect_filled(chip, 5.0, pal().border_strong);
                }
                let current = row.branch.as_deref().unwrap_or_default().trim_end_matches('*').to_string();
                egui::Popup::menu(&place).show(|ui| {
                    ui.set_min_width(200.0);
                    ui.label(RichText::new("Switch to a branch").size(11.0).color(pal().text_faint));
                    for name in &row.branches {
                        if ui.selectable_label(*name == current, name).clicked() {
                            if *name != current {
                                actions.push(Action::Git(crate::git::GitOp::Switch(name.clone())));
                            }
                            ui.close();
                        }
                    }
                });
                place.on_hover_text("Branch: click to switch to another");
            }
            let shade = if chip_hot { pal().text } else { shade };
            ui.painter().galley(Pos2::new(right - counted.size().x, mid - counted.size().y * 0.5), counted, shade);
            text_fit(
                ui.painter(),
                Pos2::new(rect.left() + 36.0, mid),
                &row.title,
                font,
                tint,
                name_end - rect.left() - 36.0,
            );
            if hot {
                if pressed.hovered() {
                    ui.painter().rect_filled(button, 5.0, pal().border_strong);
                }
                let glyph = if on { icons::X } else { icons::PLUS };
                ui.painter().text(
                    button.center(),
                    Align2::CENTER_CENTER,
                    glyph,
                    FontId::proportional(13.0),
                    pal().text_dim,
                );
            }
            let mut about = row.path.display().to_string();
            if let Some(branch) = &row.branch {
                about.push_str(&format!("\nBranch {branch}"));
            }
            if let Some(files) = row.files {
                about.push_str(&format!(" · {files} files"));
            }
            let tip = match on {
                true => format!("{about}\nOn the canvas. Click to go to it; the cross takes it off."),
                false => format!("{about}\nNot on the canvas. Click, or drag it onto the canvas, to put it there."),
            };
            if pressed.on_hover_text(if on { "Take it off the canvas" } else { "Put it on the canvas" }).clicked() {
                let op = if on { WorkspaceOp::TakeOff(row.path.clone()) } else { WorkspaceOp::Put(row.path.clone()) };
                actions.push(Action::Workspace(op));
            } else if resp.clicked() {
                actions.push(match row.page {
                    Some(page) => Action::SwitchTab(page),
                    None => Action::Workspace(WorkspaceOp::Put(row.path.clone())),
                });
            }
            if resp.drag_started() {
                st.workspace_drag = Some((row.path.clone(), row.title.clone()));
            }
            if resp.drag_stopped() {
                let dropped = pointer.is_some_and(|at| st.canvas_was.contains(at));
                if let (true, Some((path, _))) = (dropped, st.workspace_drag.take()) {
                    actions.push(Action::Workspace(WorkspaceOp::Put(path)));
                }
                st.workspace_drag = None;
            }
            resp.on_hover_text(tip).context_menu(|ui| {
                ui.set_width(236.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                let mut op = None;
                if on && menu_row(ui, icons::X, "Take off the canvas") {
                    op = Some(WorkspaceOp::TakeOff(row.path.clone()));
                }
                if !on && menu_row(ui, icons::PLUS, "Put on the canvas") {
                    op = Some(WorkspaceOp::Put(row.path.clone()));
                }
                if menu_row(ui, icons::TRASH_2, "Forget this folder") {
                    op = Some(WorkspaceOp::Forget(row.path.clone()));
                }
                if let Some(op) = op {
                    actions.push(Action::Workspace(op));
                    ui.close();
                }
            });
        }
        ui.add_space(4.0);
        let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, pal().border);
    });
}

fn workspace_drop(ctx: &egui::Context, st: &mut UiState, canvas: Rect) {
    st.canvas_was = canvas;
    if !ctx.input(|i| i.pointer.any_down()) {
        st.workspace_drag = None;
    }
    let Some((_, title)) = &st.workspace_drag else { return };
    let Some(at) = ctx.pointer_latest_pos() else { return };
    let over = canvas.contains(at);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("workspace drag")));
    if over {
        let edge = canvas.shrink(6.0);
        painter.rect_filled(edge, 10.0, pal().accent.gamma_multiply(0.06));
        painter.rect_stroke(edge, 10.0, Stroke::new(2.0, pal().accent.gamma_multiply(0.8)), egui::StrokeKind::Inside);
    }
    let text = if over {
        format!("{}  Drop to put {title} on the canvas", icons::PLUS)
    } else {
        format!("{}  {title}", icons::FOLDER)
    };
    let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), pal().text);
    let tag = Rect::from_min_size(at + Vec2::new(14.0, 10.0), galley.size() + Vec2::new(20.0, 12.0));
    painter.rect_filled(tag, 8.0, pal().card);
    painter.rect_stroke(
        tag,
        8.0,
        Stroke::new(1.0, if over { pal().accent } else { pal().border_strong }),
        egui::StrokeKind::Inside,
    );
    painter.galley(tag.min + Vec2::new(10.0, 6.0), galley, pal().text);
    ctx.request_repaint();
}

fn explorer(ui: &mut egui::Ui, st: &mut UiState, page: Option<&mut Page>, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 6 }).show(ui, |ui| {
        search_field(ui, &mut st.explorer_filter, "Filter by name, e.g. api types", icons::SEARCH);
    });
    naming_box(ui, st, actions);

    let Some(page) = page else {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("No folder open").color(pal().text_dim)));
        return;
    };
    let selected_path = page
        .selected
        .and_then(|n| page.ready().and_then(|r| r.layout.nodes.get(n)))
        .map(|n| n.rel_path.clone())
        .unwrap_or_default();
    let has_selection = page.selected.is_some();
    let now_shown = has_selection.then(|| (page.id, selected_path.clone()));
    if now_shown.is_some() && now_shown != st.explorer_shown {
        st.explorer_reveal = Some(selected_path.clone());
    }
    st.explorer_shown = now_shown;
    let reveal = st.explorer_reveal.take();
    let mut opened = false;
    if let Some(path) = &reveal {
        let mut at = path.as_str();
        while let Some((parent, _)) = at.rsplit_once('/') {
            opened |= page.tree_open.insert(parent.to_string());
            at = parent;
        }
    }
    let PageState::Ready(r) = &mut page.state else {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Scanning…").color(pal().text_dim)));
        return;
    };
    r.tree_dirty |= opened;
    if r.tree_dirty {
        let ignored = page.git.as_ref().map_or(&[][..], |git| git.ignored.as_slice());
        r.tree = build_tree(&r.repo, &page.tree_open, &page.filter.hidden, ignored);
        r.tree_dirty = false;
    }

    let terms: Vec<String> = st.explorer_filter.split_whitespace().map(|t| t.to_lowercase()).collect();
    let matches: Option<Vec<usize>> = (!terms.is_empty()).then(|| {
        r.repo
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                let path = f.rel_path.to_lowercase();
                terms.iter().all(|t| path.contains(t.as_str()))
            })
            .map(|(i, _)| i)
            .take(5000)
            .collect()
    });
    let rows = matches.as_ref().map(|m| m.len()).unwrap_or(r.tree.len());
    if let Some(m) = &matches {
        let note = if m.len() >= 5000 { "5000+ files".to_string() } else { format!("{} files", m.len()) };
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(note).size(11.0).color(pal().text_dim));
        });
    }

    ui.spacing_mut().item_spacing.y = 0.0;
    let deepest = if matches.is_some() { 0 } else { r.tree.iter().map(|row| row.depth).max().unwrap_or(0) };
    let indent_room = (ui.available_width() - 200.0).max(0.0);
    let indent_step = (indent_room / deepest.max(1) as f32).clamp(6.0, 16.0);
    let indent = |depth: u16| (depth as f32 * indent_step).min(indent_room);
    let mut list = egui::ScrollArea::vertical().auto_shrink([false, false]);
    let row_of =
        reveal.as_ref().filter(|_| matches.is_none()).and_then(|path| r.tree.iter().position(|row| row.path == *path));
    if let Some(row) = row_of {
        let (top, (from, tall)) = (row as f32 * ROW_H, st.explorer_seen);
        if top < from || top + ROW_H > from + tall {
            list = list.vertical_scroll_offset((top - tall * 0.35).max(0.0));
        }
    }
    let seen = list.show_rows(ui, ROW_H, rows, |ui, range| {
        for i in range {
            let (depth, open, name, path, count, state) = match &matches {
                Some(m) => {
                    let f = &r.repo.files[m[i]];
                    let hidden = page.filter.hidden.contains(&f.rel_path);
                    let state = RowState { is_dir: false, hidden, locked: false, ignored: false };
                    (0u16, false, f.rel_path.as_str(), f.rel_path.as_str(), 0, state)
                }
                None => {
                    let row = &r.tree[i];
                    let state =
                        RowState { is_dir: row.is_dir, hidden: row.hidden, locked: row.locked, ignored: row.ignored };
                    (row.depth, row.open, row.name.as_str(), row.path.as_str(), row.count, state)
                }
            };
            let RowState { is_dir, hidden, locked, ignored } = state;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            let row_bg = rect.shrink2(Vec2::new(6.0, 0.0));
            if has_selection && path == selected_path {
                painter.rect_filled(row_bg, 6.0, pal().raised);
            } else if resp.hovered() {
                painter.rect_filled(row_bg, 6.0, pal().row);
            }
            let mid = rect.center().y;
            for d in 0..depth {
                let gx = rect.left() + 22.0 + indent(d);
                painter.vline(gx, rect.y_range(), Stroke::new(1.0, pal().raised));
            }
            let x0 = rect.left() + 14.0 + indent(depth);
            if is_dir && !ignored {
                let chevron = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
                painter.text(
                    Pos2::new(x0 + 8.0, mid),
                    Align2::CENTER_CENTER,
                    chevron,
                    FontId::proportional(13.0),
                    pal().text_dim,
                );
            }
            let on = !hidden && !locked;
            let cb = Rect::from_center_size(Pos2::new(x0 + 27.0, mid), Vec2::splat(15.0));
            if !ignored {
                painter.rect_filled(cb, 4.0, pal().raised);
            }
            if on {
                painter.text(cb.center(), Align2::CENTER_CENTER, icons::CHECK, FontId::proportional(11.0), pal().text);
            }
            let mut text_x = x0 + 42.0;
            let change = page.git.as_ref().filter(|_| !is_dir && !ignored).and_then(|git| git.change(path));
            let unsaved = !is_dir && page.editors.get(path).is_some_and(|e| e.dirty);
            let inside = is_dir && !ignored && page.changed_dirs.contains(path);
            let tint = match (change, inside) {
                (Some(change), _) => Some(git_panel::change_color(change.change)),
                (None, true) => Some(c(theme::get().git_modified)),
                (None, false) => None,
            };
            let color = match (on, tint) {
                (false, _) => pal().text_faint,
                (true, Some(tint)) => tint.lerp_to_gamma(pal().text, if is_dir { 0.5 } else { 0.25 }),
                (true, None) if is_dir => pal().text,
                (true, None) => c(theme::mix(theme::get().text, theme::get().text_dim, 0.35)),
            };
            let base = name.rsplit_once('/').map_or(name, |(_, base)| base);
            let icon_at = Rect::from_center_size(Pos2::new(text_x + 7.0, mid), Vec2::splat(16.0));
            if st.file_icons.paint(ui, base, is_dir, open, icon_at, !on) {
                text_x += 20.0;
            } else if !is_dir || ignored {
                let glyph = if is_dir { icons::FOLDER } else { file_glyph(name) };
                let ic = if on { pal().text_dim } else { pal().text_faint };
                painter.text(
                    Pos2::new(text_x + 7.0, mid),
                    Align2::CENTER_CENTER,
                    glyph,
                    FontId::proportional(13.0),
                    ic,
                );
                text_x += 20.0;
            }
            let mut right_pad = if is_dir && !ignored { 44.0 } else { 10.0 };
            if let (Some(change), Some(tint)) = (change, tint) {
                let at = Pos2::new(rect.right() - 14.0, mid);
                painter.text(at, Align2::RIGHT_CENTER, change.change.letter(), FontId::monospace(11.5), tint);
                right_pad += 16.0;
            } else if let (true, Some(tint)) = (inside, tint) {
                painter.circle_filled(Pos2::new(rect.right() - right_pad - 2.0, mid), 2.5, tint);
                right_pad += 10.0;
            }
            if unsaved {
                painter.circle_filled(Pos2::new(rect.right() - right_pad - 4.0, mid), 3.2, pal().text);
                right_pad += 14.0;
            }
            let room = rect.right() - right_pad - text_x;
            let renamed = st.naming.as_ref().is_some_and(|n| n.kind == NamingKind::Rename && n.path == path);
            if renamed {
                let field = Rect::from_min_max(
                    Pos2::new(text_x - 5.0, rect.top() + 1.0),
                    Pos2::new(rect.right() - 8.0, rect.bottom() - 1.0),
                );
                rename_in_place(ui, st, field, actions);
                continue;
            }
            text_fit(painter, Pos2::new(text_x, mid), name, FontId::proportional(13.0), color, room);
            if is_dir && !ignored {
                let at = Pos2::new(rect.right() - 14.0, mid);
                painter.text(at, Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(11.0), pal().text_faint);
            }

            if resp.hovered() && on {
                actions.push(Action::HoverPath(path.to_string()));
            }
            if resp.clicked() && ignored {
                actions.push(Action::OpenIgnored(path.to_string()));
            } else if resp.clicked() {
                let x = resp.interact_pointer_pos().map(|p| p.x).unwrap_or(text_x);
                if is_dir && x < x0 + 17.0 {
                    actions.push(Action::ToggleTreeOpen(path.to_string()));
                } else if x < x0 + 37.0 {
                    if !locked {
                        actions.push(Action::ToggleHidden(path.to_string()));
                    }
                } else {
                    if is_dir && !open {
                        actions.push(Action::ToggleTreeOpen(path.to_string()));
                    }
                    if on {
                        actions.push(Action::FlyToPath(path.to_string()));
                    }
                }
            }
            if resp.double_clicked() && is_dir && on {
                actions.push(Action::ToggleCollapsed(path.to_string()));
            }
            resp.context_menu(|ui| file_menu(ui, &mut st.naming, path, state, actions));
            if ignored {
                resp.on_hover_text("Git ignores it, so it is not on the canvas");
            } else if let Some(change) = change {
                let state = if unsaved { ", not saved yet" } else { "" };
                resp.on_hover_text(format!("{path} · {}{state}", change.change.label()));
            } else if unsaved {
                resp.on_hover_text(format!("{path} · not saved yet"));
            } else if name != path {
                resp.on_hover_text(path);
            }
        }
    });
    st.explorer_seen = (seen.state.clamped_offset().y, seen.inner_rect.height());
}

fn completion_list(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    const ROWS: usize = 9;
    let Some(view) = &st.completion else { return };
    let first = view.selected.saturating_sub(ROWS - 1).min(view.items.len().saturating_sub(ROWS));
    let shown = &view.items[first..view.items.len().min(first + ROWS)];
    let file_icons = &mut st.file_icons;
    let wide = shown.iter().any(|row| matches!(row.icon, CompletionIcon::Kind(_)));
    let width = if wide { 440.0 } else { 260.0 };
    let area = egui::Area::new(egui::Id::unique("path completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(view.at + Vec2::new(-28.0, 4.0))
        .show(ctx, |ui| {
            Frame::NONE
                .fill(pal().card)
                .stroke(Stroke::new(1.0, pal().border_strong))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(4))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (i, row) in shown.iter().enumerate() {
                        let index = first + i;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
                        if index == view.selected {
                            ui.painter().rect_filled(rect, 6.0, pal().raised);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, pal().row);
                        }
                        let mid = rect.center().y;
                        let icon_at = Rect::from_center_size(Pos2::new(rect.left() + 14.0, mid), Vec2::splat(16.0));
                        let (name, from) = (&row.label, rect.left() + 30.0);
                        let label = match &row.icon {
                            CompletionIcon::Path { is_dir } => {
                                if !file_icons.paint(ui, name, *is_dir, false, icon_at, false) {
                                    let glyph = if *is_dir { icons::FOLDER } else { file_glyph(name) };
                                    let font = FontId::proportional(13.0);
                                    ui.painter().text(
                                        icon_at.center(),
                                        Align2::CENTER_CENTER,
                                        glyph,
                                        font,
                                        pal().text_dim,
                                    );
                                }
                                if *is_dir {
                                    format!("{name}/")
                                } else {
                                    name.clone()
                                }
                            }
                            CompletionIcon::Kind(kind) => {
                                let (glyph, color) = kind_glyph(kind);
                                let font = FontId::proportional(14.0);
                                ui.painter().text(icon_at.center(), Align2::CENTER_CENTER, glyph, font, color);
                                name.clone()
                            }
                        };
                        let font = if wide { FontId::monospace(12.5) } else { FontId::proportional(13.0) };
                        let room = rect.right() - 8.0 - from;
                        let used = text_fit(ui.painter(), Pos2::new(from, mid), &label, font, pal().text, room);
                        if !row.detail.is_empty() && room - used > 60.0 {
                            let font = FontId::proportional(11.5);
                            let at = Pos2::new(from + used + 12.0, mid);
                            text_fit(ui.painter(), at, &row.detail, font, pal().text_faint, room - used - 12.0);
                        }
                        if resp.clicked() {
                            actions.push(Action::AcceptCompletion(index));
                        }
                    }
                });
        });
    st.floating.push(area.response.rect);
}

fn text_fit(painter: &egui::Painter, at: Pos2, text: &str, font: FontId, color: Color32, width: f32) -> f32 {
    if width <= 1.0 {
        return 0.0;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap =
        egui::text::TextWrapping {
            max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…')
        };
    let galley = painter.layout_job(job);
    let size = galley.size();
    painter.galley(Pos2::new(at.x, at.y - size.y * 0.5), galley, color);
    size.x
}

fn menu_row(ui: &mut egui::Ui, glyph: &str, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal().hover);
    }
    let (mid, font) = (rect.center().y, FontId::proportional(13.0));
    ui.painter().text(Pos2::new(rect.left() + 10.0, mid), Align2::LEFT_CENTER, glyph, font.clone(), pal().text_dim);
    ui.painter().text(Pos2::new(rect.left() + 32.0, mid), Align2::LEFT_CENTER, label, font, pal().text);
    resp.clicked()
}

fn menu_gap(ui: &mut egui::Ui) {
    ui.add_space(3.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, pal().border);
    ui.add_space(3.0);
}

#[derive(Clone, Copy)]
struct RowState {
    is_dir: bool,
    hidden: bool,
    locked: bool,
    ignored: bool,
}

fn file_menu(ui: &mut egui::Ui, naming: &mut Option<Naming>, path: &str, row: RowState, actions: &mut Vec<Action>) {
    let RowState { is_dir, hidden, locked, ignored } = row;
    ui.set_width(236.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let dir = if is_dir { path } else { path.rsplit_once('/').map_or("", |(dir, _)| dir) };
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    let mut op = None;
    let mut ask = None;

    let reveal = if cfg!(windows) {
        "Reveal in File Explorer"
    } else if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else {
        "Open the containing folder"
    };
    if menu_row(ui, icons::FOLDER_OPEN, reveal) {
        op = Some(FileOp::Reveal(path.to_string()));
    }
    if menu_row(ui, icons::CODE, "Open in terminal") {
        op = Some(FileOp::Terminal(dir.to_string()));
    }
    menu_gap(ui);
    if menu_row(ui, icons::COPY, "Copy path") {
        op = Some(FileOp::CopyPath { path: path.to_string(), relative: false });
    }
    if menu_row(ui, icons::COPY, "Copy relative path") {
        op = Some(FileOp::CopyPath { path: path.to_string(), relative: true });
    }
    menu_gap(ui);
    if menu_row(ui, icons::FILE, "New file…") {
        ask = Some((NamingKind::NewFile, dir.to_string(), String::new()));
    }
    if menu_row(ui, icons::FOLDER, "New folder…") {
        ask = Some((NamingKind::NewFolder, dir.to_string(), String::new()));
    }
    if menu_row(ui, icons::FILE_TEXT, "Rename…") {
        ask = Some((NamingKind::Rename, path.to_string(), name.to_string()));
    }
    if menu_row(ui, icons::TRASH_2, "Delete") {
        op = Some(FileOp::Delete(path.to_string()));
    }
    menu_gap(ui);
    let mut canvas = None;
    let markdown = !is_dir && crate::markdown::is_markdown(std::path::Path::new(name));
    if markdown && menu_row(ui, crate::codicons::glyph("open-preview").unwrap_or(icons::EYE), "Open preview") {
        canvas = Some(Action::Preview(path.to_string()));
    }
    if !is_dir && !ignored && menu_row(ui, icons::WAND_SPARKLES, "Format") {
        canvas = Some(Action::FormatFile(Some(path.to_string())));
    }
    if !locked && !ignored {
        let (glyph, label) =
            if hidden { (icons::EYE, "Show on the canvas") } else { (icons::EYE_OFF, "Hide from the canvas") };
        if menu_row(ui, glyph, label) {
            canvas = Some(Action::ToggleHidden(path.to_string()));
        }
    }
    if is_dir && !hidden && !ignored && menu_row(ui, icons::FOLD_VERTICAL, "Collapse or expand on the canvas") {
        canvas = Some(Action::ToggleCollapsed(path.to_string()));
    }

    if let Some((kind, path, text)) = ask {
        *naming = Some(Naming { kind, path, text, focus: true });
        ui.close();
    }
    if let Some(action) = op.map(Action::File).or(canvas) {
        actions.push(action);
        ui.close();
    }
}

fn rename_in_place(ui: &mut egui::Ui, st: &mut UiState, field: Rect, actions: &mut Vec<Action>) {
    let Some(naming) = &mut st.naming else { return };
    ui.painter().rect_filled(field, 5.0, pal().well);
    ui.painter().rect_stroke(field, 5.0, Stroke::new(1.0, pal().accent), egui::StrokeKind::Inside);
    let inside = Rect::from_center_size(field.center(), Vec2::new(field.width() - 10.0, 18.0));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(inside).layout(Layout::left_to_right(Align::Center)));
    let edit = egui::TextEdit::singleline(&mut naming.text)
        .frame(Frame::NONE)
        .font(FontId::proportional(13.0))
        .desired_width(inside.width())
        .margin(Margin::ZERO);
    let mut shown = edit.show(&mut row);
    if std::mem::take(&mut naming.focus) {
        shown.response.request_focus();
        let stem = naming
            .text
            .rfind('.')
            .filter(|at| *at > 0)
            .map_or(naming.text.chars().count(), |at| naming.text[..at].chars().count());
        let chosen = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(stem));
        shown.state.cursor.set_char_range(Some(chosen));
        shown.state.store(ui.ctx(), shown.response.id);
        return;
    }
    if !shown.response.lost_focus() {
        return;
    }
    let accepted = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if let Some(n) = st.naming.take().filter(|n| accepted && !n.text.trim().is_empty()) {
        let same = n.path.rsplit('/').next() == Some(n.text.trim());
        if !same {
            actions.push(Action::File(FileOp::Rename { path: n.path, name: n.text }));
        }
    }
}

fn naming_box(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    if st.naming.as_ref().is_some_and(|naming| naming.kind == NamingKind::Rename) {
        return;
    }
    let Some(naming) = &mut st.naming else { return };
    let place = |path: &str| if path.is_empty() { "this folder".to_string() } else { format!("{path}/") };
    let (title, hint, glyph) = match naming.kind {
        NamingKind::NewFile => {
            (format!("New file in {}", place(&naming.path)), "name.ext, or folder/name.ext", icons::FILE)
        }
        NamingKind::NewFolder => (format!("New folder in {}", place(&naming.path)), "name", icons::FOLDER),
        NamingKind::Rename => (format!("New name for {}", naming.path), "name", icons::FILE_TEXT),
    };
    let mut done = None;
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 0, bottom: 6 }).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(title).size(11.5).color(pal().text_dim)).truncate());
        let field = search_field(ui, &mut naming.text, hint, glyph);
        if std::mem::take(&mut naming.focus) {
            field.request_focus();
        } else if field.lost_focus() {
            done = Some(ui.input(|i| i.key_pressed(egui::Key::Enter)));
        }
    });
    let Some(accepted) = done else { return };
    if let Some(n) = st.naming.take().filter(|n| accepted && !n.text.trim().is_empty()) {
        actions.push(Action::File(match n.kind {
            NamingKind::NewFile => FileOp::NewFile { dir: n.path, name: n.text },
            NamingKind::NewFolder => FileOp::NewFolder { dir: n.path, name: n.text },
            NamingKind::Rename => FileOp::Rename { path: n.path, name: n.text },
        }));
    }
}

fn search_panel(ui: &mut egui::Ui, st: &mut UiState, page: Option<&Page>, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Search", |_| {});
        ui.add_space(6.0);
        let resp = search_field(ui, &mut st.search_query, "Text to find in every file", icons::TEXT_SEARCH);
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            actions.push(Action::Search(st.search_query.clone()));
        }
        ui.label(RichText::new("Enter to search · lowercase ignores case").size(11.0).color(pal().text_faint));
    });
    let Some(page) = page else { return };
    let Some(r) = page.ready() else { return };
    let Some(search) = &page.search else { return };
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let more = if search.truncated { "+" } else { "" };
        ui.label(
            RichText::new(format!("{}{more} results for “{}” · {:.0} ms", search.hits.len(), search.query, search.ms))
                .size(11.0)
                .color(pal().text_dim),
        );
    });
    ui.spacing_mut().item_spacing.y = 0.0;
    let found_lines = page.found_lines();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, search.hits.len(), |ui, range| {
        for i in range {
            let hit = search.hits[i];
            let f = &r.repo.files[hit.file as usize];
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            if resp.hovered() {
                painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
            }
            let mid = rect.center().y;
            let real = found_lines.and_then(|lines| lines.get(&i));
            let place = format!("{}:{}", f.name, real.map_or(hit.line, |(line, _)| *line) + 1);
            let galley = painter.layout_no_wrap(place, FontId::proportional(12.0), pal().text_dim);
            let pw = galley.size().x;
            painter.galley(Pos2::new(rect.left() + 14.0, mid - galley.size().y * 0.5), galley, pal().text_dim);
            let x = rect.left() + 22.0 + pw;
            let clip = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(rect.right() - 8.0, rect.bottom()));
            let line = real.map_or_else(|| f.line(hit.line as usize).trim_start(), |(_, text)| text.as_str());
            let preview: String = line.chars().take(160).collect();
            painter.with_clip_rect(clip).text(
                Pos2::new(x, mid),
                Align2::LEFT_CENTER,
                preview,
                FontId::monospace(12.0),
                pal().text,
            );
            if resp.clicked() {
                actions.push(Action::FlyToSite(hit));
            }
            resp.on_hover_text(&f.rel_path);
        }
    });
}

fn help_panel(ui: &mut egui::Ui, keys: &Keymap) {
    let key = |command: Command| -> String {
        let all: Vec<String> = keys.chords(command).map(|c| c.label()).collect();
        if all.is_empty() {
            "no shortcut".to_string()
        } else {
            all.join(" or ")
        }
    };
    let fixed = |text: &str| text.to_string();
    let groups: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "Canvas",
            vec![
                (
                    "Zoom",
                    format!(
                        "Ctrl + wheel, or the wheel with the right button held, {} / {}",
                        keys.label(Command::ZoomIn),
                        keys.label(Command::ZoomOut)
                    ),
                ),
                ("Up and down", fixed("Wheel")),
                ("Left and right", fixed("Shift + wheel")),
                ("Pan", fixed("Drag, arrow keys")),
                ("Fly to a file or folder", fixed("Double-click")),
                ("Show everything", key(Command::ShowAll)),
                ("Go to the selection", key(Command::GoToSelection)),
                ("Zoom out to the parent folder", key(Command::ZoomToParent)),
                ("Back to where the canvas was", key(Command::Back)),
                ("Forward again", key(Command::Forward)),
                ("Collapse / expand a folder", fixed("Click − / + next to its name")),
            ],
        ),
        (
            "Symbols",
            vec![
                ("See where a name is used", fixed("Click it (zoomed in to text; Ctrl+click while editing)")),
                ("Select a function", fixed("Click inside it")),
                ("Deselect", fixed("Click it again, or Esc for all")),
                ("Go to the definition", key(Command::GoToDefinition)),
            ],
        ),
        (
            "Editing",
            vec![
                ("Move around without editing (the hand)", key(Command::MoveTool)),
                ("Edit: a click puts the caret (the pointer)", key(Command::EditTool)),
                ("Select code", fixed("Drag over it with the pointer")),
                ("Add or take away a caret", fixed("Alt+click")),
                ("Select the word, then its next occurrence", key(Command::NextOccurrence)),
                ("A caret at every occurrence", key(Command::AllOccurrences)),
                (
                    "A caret on the line above / below",
                    format!("{} / {}", keys.label(Command::CaretAbove), keys.label(Command::CaretBelow)),
                ),
                ("Draw over the canvas (the pencil)", key(Command::DrawTool)),
                ("Save", key(Command::Save)),
                ("Back to one caret, then stop typing in the file", fixed("Esc")),
            ],
        ),
        (
            "Find",
            vec![
                ("Go to a file or folder", key(Command::GoToFile)),
                ("Search in files", key(Command::SearchInFiles)),
                ("Find and replace in a file", key(Command::Find)),
                ("Run a command", key(Command::Palette)),
            ],
        ),
        (
            "Extensions",
            vec![
                ("Format the file under the pointer", key(Command::FormatFile)),
                ("See a problem's message", fixed("Point at the underlined code")),
            ],
        ),
        (
            "Folders and panels",
            vec![
                ("Open another folder", format!("{}, or drop it", key(Command::OpenFolder))),
                (
                    "Next / previous folder",
                    format!("{} / {}", keys.label(Command::NextFolder), keys.label(Command::PreviousFolder)),
                ),
                ("Close the folder", key(Command::CloseFolder)),
                ("Reload from disk", key(Command::Rescan)),
                ("Toggle the left panel", key(Command::ToggleLeftPanel)),
                ("Every folder on the canvas, or one at a time", key(Command::ToggleSideBySide)),
                ("Settings", key(Command::Settings)),
            ],
        ),
    ];
    Frame::NONE.inner_margin(Margin { left: 12, right: 10, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Shortcuts", |_| {});
        ui.add_space(2.0);
        ui.label(RichText::new("Change them in Settings.").size(11.5).color(pal().text_faint));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (title, rows) in &groups {
                section(ui, icons::CHEVRON_DOWN, title);
                well(ui, |ui| {
                    for (what, keys) in rows {
                        let width = |text: &str, size: f32| {
                            let font = FontId::proportional(size);
                            ui.painter().layout_no_wrap(text.to_string(), font, pal().text).size().x
                        };
                        let side_by_side = width(what, 12.5) + width(keys, 12.0) + 16.0 <= ui.available_width();
                        let what = RichText::new(*what).size(12.5).color(pal().text);
                        let keys = RichText::new(keys).size(12.0).color(pal().text_dim);
                        if !side_by_side {
                            ui.add(egui::Label::new(what.clone()).wrap());
                        }
                        ui.horizontal(|ui| {
                            if side_by_side {
                                ui.label(what);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add(egui::Label::new(keys).truncate());
                            });
                        });
                    }
                });
            }
        });
    });
}

fn symbols_panel(ui: &mut egui::Ui, st: &mut UiState, page: Option<&Page>, actions: &mut Vec<Action>) {
    let selections = page.map(|p| p.selections.len()).unwrap_or(0);
    Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 10, bottom: 0 }).show(ui, |ui| {
        let title =
            if selections > 0 { format!("Selected symbols · {selections}") } else { "Selected symbols".into() };
        panel_title(ui, &title, |ui| {
            if selections > 0 && small_icon(ui, icons::TRASH_2, "Deselect everything (Esc)") {
                actions.push(Action::ClearSymbols);
            }
        });

        section(ui, icons::EYE, "Show");
        well(ui, |ui| {
            let mut only = page.map(|p| p.only_connected).unwrap_or(false);
            if check(ui, &mut only, "Show only connected files on canvas", selections > 0, false).changed() {
                actions.push(Action::SetOnlyConnected(only));
            }
            if let Some(p) = page.filter(|p| p.only_connected && selections > 0) {
                let n = p.connected_files(&st.filters).len();
                let said =
                    if n == 1 { "1 connected file shown".to_string() } else { format!("{n} connected files shown") };
                ui.label(RichText::new(said).size(11.0).color(c(theme::get().ui_warn)));
            }
        });

        section(ui, icons::FILTER, "Filters");
        let before = st.filters;
        well(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            let room = ui.available_width();
            let percentages = room >= 244.0;
            ui.spacing_mut().slider_width = (room - if percentages { 214.0 } else { 179.0 }).clamp(24.0, 92.0);
            let pct = |ui: &mut egui::Ui, v: f32| {
                if percentages {
                    let text = format!("{:>3.0}%", v * 100.0);
                    ui.label(RichText::new(text).monospace().size(11.0).color(pal().text_dim));
                }
            };
            for kind in RelKind::ALL {
                let i = kind.index();
                let count: usize = page.map(|p| p.selections.iter().map(|s| s.rel.count(kind)).sum()).unwrap_or(0);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(11.0, 20.0), Sense::hover());
                    let sw = Rect::from_center_size(rect.center(), Vec2::splat(11.0));
                    ui.painter().rect_filled(sw, 3.0, c(theme::get().rel_colors()[i]));
                    check(ui, &mut st.filters.enabled[i], kind.label(), true, false);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        pct(ui, st.filters.opacity[i]);
                        ui.add(egui::Slider::new(&mut st.filters.opacity[i], 0.05..=1.0).show_value(false));
                        ui.label(RichText::new(count.to_string()).monospace().size(11.0).color(pal().text_dim));
                    });
                });
                ui.horizontal(|ui| {
                    ui.add_space(17.0);
                    check(ui, &mut st.filters.bodies[i], "Bodies", st.filters.enabled[i], true);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        pct(ui, st.filters.body_opacity[i]);
                        ui.add(egui::Slider::new(&mut st.filters.body_opacity[i], 0.05..=1.0).show_value(false));
                    });
                });
            }
        });
        if before != st.filters {
            actions.push(Action::FiltersChanged);
        }
        section(ui, icons::LIST_TREE, "Names");
    });

    let Some(page) = page else { return };
    let Some(r) = page.ready() else { return };
    if page.selections.is_empty() {
        Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 8 }).show(ui, |ui| {
            well(ui, |ui| {
                ui.label(
                    RichText::new("Click a name on a card to see where it is defined and used. Zoomed out, click inside a function to select it.")
                        .size(12.0)
                        .color(pal().text_dim),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{} symbols indexed by name. With the language's extension installed (rust-analyzer, TypeScript…) the uses of a name are the exact ones.",
                        r.repo.symbols.count
                    ))
                    .size(11.0)
                    .color(pal().text_faint),
                );
            });
        });
        return;
    }

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, sel) in page.selections.iter().enumerate() {
            let repo = &r.repo;
            let sym = repo.sym(sel.sym);
            let file = &repo.files[sel.sym.file as usize];
            let def = repo.sym_site(sel.sym);
            Frame::NONE
                .fill(pal().well)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(9))
                .outer_margin(Margin { left: 10, right: 10, top: 0, bottom: 6 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        Frame::NONE
                            .fill(pal().raised)
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(Margin::symmetric(5, 1))
                            .show(ui, |ui| {
                                ui.label(RichText::new(sym.kind.badge()).monospace().size(11.0).color(pal().accent));
                            });
                        ui.label(RichText::new(repo.sym_name(sel.sym)).monospace().size(13.5).color(pal().text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if small_icon(ui, icons::X, "Deselect") {
                                actions.push(Action::RemoveSymbol(i));
                            }
                        });
                    });
                    let place = match repo.sym_container(sel.sym) {
                        Some(cn) => format!("{}:{}  ·  {cn}", file.rel_path, page.symbol_line(sel.sym) + 1),
                        None => format!("{}:{}", file.rel_path, page.symbol_line(sel.sym) + 1),
                    };
                    if ui
                        .link(RichText::new(place).monospace().size(11.0))
                        .on_hover_text("Go to the definition")
                        .clicked()
                    {
                        actions.push(Action::FlyToSite(def));
                    }
                    ui.add_space(3.0);
                    ui.label(RichText::new(page.signature(sel.sym)).monospace().size(12.0).color(pal().text));
                    ui.add_space(4.0);

                    for kind in RelKind::ALL {
                        let items: Vec<_> = sel.rel.items.iter().filter(|x| x.kind == kind).collect();
                        if items.is_empty() {
                            continue;
                        }
                        let title = RichText::new(format!("{}  {}", kind.label(), items.len()))
                            .size(12.5)
                            .color(c(theme::get().rel_colors()[kind.index()]));
                        let id = (page.id, sel.sym.file, sel.sym.index, kind.index());
                        egui::CollapsingHeader::new(title).id_salt(id).show(ui, |ui| {
                            for rel in items.iter().take(200) {
                                let site = if kind == RelKind::OutgoingCall { rel.to } else { rel.from };
                                let f = &repo.files[site.file as usize];
                                let who = rel.other.map(|o| repo.sym_name(o)).unwrap_or("");
                                let text = format!("{}:{}  {who}", f.name, page.site_line(site, rel.other) + 1);
                                if ui
                                    .link(RichText::new(text).monospace().size(11.0))
                                    .on_hover_text(&f.rel_path)
                                    .clicked()
                                {
                                    actions.push(Action::FlyToSite(site));
                                }
                            }
                            if items.len() > 200 {
                                ui.label(
                                    RichText::new(format!("… and {} more", items.len() - 200))
                                        .size(11.0)
                                        .color(pal().text_dim),
                                );
                            }
                        });
                    }
                    if sel.outside > 0 {
                        ui.label(
                            RichText::new(format!("Outside the canvas  {}", sel.outside))
                                .size(12.0)
                                .color(pal().text_dim),
                        );
                    }
                    let mut note = exact_note(&sel.rel);
                    if sel.rel.truncated {
                        note.push_str(" · results truncated");
                    }
                    ui.label(RichText::new(note).size(10.0).color(pal().text_faint));
                });
        }
        ui.add_space(8.0);
    });
}

/// Which relations of a selected name come from a language server and which are guessed
/// by name, for the foot of the Symbols panel.
fn exact_note(rel: &Relations) -> String {
    let parts = [
        (rel.is_exact(RelKind::Reference), "uses"),
        (rel.is_exact(RelKind::OutgoingCall), "outgoing calls"),
        (rel.is_exact(RelKind::Implementation), "implementations"),
    ];
    let exact: Vec<&str> = parts.iter().filter(|(exact, _)| *exact).map(|(_, name)| *name).collect();
    match exact.as_slice() {
        [] => format!("by name · searched in {:.0} ms", rel.search_ms),
        [_, _, _] => "uses, calls and implementations as the language's extension knows them".to_string(),
        [one] => format!("{one} as the language's extension knows them · the rest by name"),
        [one, two] => format!("{one} and {two} as the language's extension knows them · the rest by name"),
        _ => unreachable!(),
    }
}

fn center_message(ui: &mut egui::Ui, canvas: Rect, page: Option<&Page>, keys: &Keymap) {
    let message = match page {
        None => {
            let key = match keys.label(Command::OpenFolder) {
                key if key.is_empty() => String::new(),
                key => format!("{key}, "),
            };
            Some((format!("Open a folder with {key}the + button, or drop it here"), pal().text))
        }
        Some(p) => match &p.state {
            PageState::Loading { progress, started, .. } => Some((
                format!(
                    "Scanning {} …  {} / {} files  ·  {:.1} s",
                    p.title,
                    progress.loaded.load(Ordering::Relaxed),
                    progress.discovered.load(Ordering::Relaxed),
                    started.elapsed().as_secs_f32()
                ),
                pal().text,
            )),
            PageState::Failed(e) => Some((e.clone(), c(theme::get().ui_warn))),
            PageState::Waiting => Some((format!("{} will be read when it is shown", p.title), pal().text_dim)),
            PageState::Ready(_) => None,
        },
    };
    if let Some((text, color)) = message {
        ui.painter().text(canvas.center(), Align2::CENTER_CENTER, text, FontId::proportional(16.0), color);
    }
}

fn toolbar(ctx: &egui::Context, st: &mut UiState, canvas: Rect, info: &FrameInfo, actions: &mut Vec<Action>) {
    let pos = Pos2::new(canvas.left() + 12.0, canvas.bottom() - 50.0);
    let resp =
        egui::Area::new(egui::Id::unique("toolbar")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            Frame::NONE
                .fill(pal().card)
                .stroke(Stroke::new(1.0, pal().border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(5, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let b = Vec2::splat(30.0);
                        let edit = st.prefs.edit;
                        let ways = [
                            (icons::ARROW_LEFT, st.can_go.0, Command::Back, "Back to where the canvas was", Action::Back),
                            (icons::ARROW_RIGHT, st.can_go.1, Command::Forward, "Forward again", Action::Forward),
                        ];
                        for (glyph, can, command, tip, action) in ways {
                            let button = icon_button(ui, glyph, b, false, if can { pal().hover } else { pal().card });
                            if !can {
                                let at = button.rect.center();
                                let faint = FontId::proportional(17.0);
                                ui.painter().circle_filled(at, 11.0, pal().card);
                                ui.painter().text(at, Align2::CENTER_CENTER, glyph, faint, pal().text_faint);
                            }
                            if button.on_hover_text(st.keys.hint(tip, command)).clicked() && can {
                                actions.push(action);
                            }
                        }
                        ui.add_space(4.0);
                        let (gap, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                        ui.painter().rect_filled(gap, 0.0, pal().border);
                        ui.add_space(4.0);
                        let tools = [
                            (icons::MOUSE_POINTER_2, Some(true), Command::EditTool, "Edit: click in the code to put the caret there and type"),
                            (icons::HAND, Some(false), Command::MoveTool, "Move: drag to travel, click a name to see where it is used. Nothing gets edited"),
                            (icons::PENCIL, None, Command::DrawTool, "Draw: scribble over the canvas to point things out. The right button still moves it"),
                        ];
                        for (glyph, edits, command, tip) in tools {
                            let chosen = match edits {
                                Some(edits) => !st.pen.on && edit == edits,
                                None => st.pen.on,
                            };
                            let button = icon_button(ui, glyph, b, chosen, pal().hover);
                            if chosen && edits != Some(false) {
                                let at = button.rect.center();
                                ui.painter().text(at, Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), pal().accent);
                            }
                            if button.on_hover_text(st.keys.hint(tip, command)).clicked() && !chosen {
                                st.pen.on = edits.is_none();
                                if let Some(edits) = edits.filter(|edits| *edits != edit) {
                                    actions.push(Action::SetPrefs(crate::prefs::Prefs { edit: edits, ..st.prefs.clone() }));
                                }
                            }
                        }
                        ui.add_space(4.0);
                        let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                        ui.painter().rect_filled(sep, 0.0, pal().border);
                        ui.add_space(4.0);
                        if icon_button(ui, icons::SCAN, b, false, pal().hover)
                            .on_hover_text("Show the whole canvas (F)")
                            .clicked()
                        {
                            actions.push(Action::Fit);
                        }
                        if icon_button(ui, icons::LOCATE_FIXED, b, false, pal().hover)
                            .on_hover_text("Go to the selected symbols or file (L)")
                            .clicked()
                        {
                            actions.push(Action::FrameSelection);
                        }
                        if icon_button(ui, icons::WAND_SPARKLES, b, false, pal().hover)
                            .on_hover_text("Format the selected file with the installed formatter (Shift+Alt+F formats the file under the pointer)")
                            .clicked()
                        {
                            actions.push(Action::FormatFile(None));
                        }
                        ui.add_space(4.0);
                        let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                        ui.painter().rect_filled(sep, 0.0, pal().border);
                        ui.add_space(4.0);
                        if icon_button(ui, icons::MINUS, b, false, pal().hover).on_hover_text("Zoom out").clicked() {
                            actions.push(Action::ZoomBy(1.0 / 1.4));
                        }
                        let stats = info.stats;
                        let drawn = format!(
                            "{}  ·  {} draws  ·  {:.2} ms a frame
code instances {}  ·  boxes {}
token chunks {}  ·  text chunks {}",
                            stats.lod.map(|l| l.name()).unwrap_or("-"),
                            stats.draw_calls,
                            info.frame_ms,
                            stats.code_instances,
                            stats.frame_instances,
                            stats.token_chunks,
                            stats.cached_chunks
                        );
                        ui.label(
                            RichText::new(format!("{:>4.0}%", info.zoom * 100.0))
                                .monospace()
                                .size(11.0)
                                .color(pal().text_dim),
                        )
                        .on_hover_text(drawn);
                        if icon_button(ui, icons::PLUS, b, false, pal().hover).on_hover_text("Zoom in").clicked() {
                            actions.push(Action::ZoomBy(1.4));
                        }
                    });
                });
        });
    st.floating.push(resp.response.rect);
    if st.pen.on {
        pen_bar(ctx, st, Pos2::new(pos.x, resp.response.rect.top() - 44.0), actions);
    }
}

fn pen_bar(ctx: &egui::Context, st: &mut UiState, pos: Pos2, actions: &mut Vec<Action>) {
    use crate::doodle::{COLOURS, COLOUR_NAMES, WIDTHS, WIDTH_NAMES};
    let resp =
        egui::Area::new(egui::Id::unique("pen bar")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            Frame::NONE
                .fill(pal().card)
                .stroke(Stroke::new(1.0, pal().border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(5, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let b = Vec2::splat(30.0);
                        let rule = |ui: &mut egui::Ui| {
                            ui.add_space(4.0);
                            let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                            ui.painter().rect_filled(sep, 0.0, pal().border);
                            ui.add_space(4.0);
                        };
                        for (nth, [r, g, b_]) in COLOURS.into_iter().enumerate() {
                            let chosen = !st.pen.rubbing && st.pen.colour == nth;
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(26.0, 30.0), Sense::click());
                            let at = rect.center();
                            if chosen {
                                ui.painter().circle_stroke(at, 10.5, Stroke::new(1.5, pal().text));
                            } else if resp.hovered() {
                                ui.painter().circle_stroke(at, 10.5, Stroke::new(1.0, pal().border_strong));
                            }
                            ui.painter().circle_filled(at, 7.0, Color32::from_rgb(r, g, b_));
                            if resp.on_hover_text(COLOUR_NAMES[nth]).clicked() {
                                (st.pen.colour, st.pen.rubbing) = (nth, false);
                            }
                        }
                        rule(ui);
                        let [r, g, b_] = COLOURS[st.pen.colour.min(COLOURS.len() - 1)];
                        for (nth, width) in WIDTHS.into_iter().enumerate() {
                            let chosen = !st.pen.rubbing && st.pen.width == nth;
                            let (rect, resp) = ui.allocate_exact_size(b, Sense::click());
                            if chosen {
                                ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().raised);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().hover);
                            }
                            ui.painter().circle_filled(rect.center(), 1.0 + width * 0.5, Color32::from_rgb(r, g, b_));
                            if resp.on_hover_text(WIDTH_NAMES[nth]).clicked() {
                                (st.pen.width, st.pen.rubbing) = (nth, false);
                            }
                        }
                        rule(ui);
                        let rubber = icon_button(ui, icons::ERASER, b, st.pen.rubbing, pal().hover);
                        if rubber.on_hover_text("Rub out: drag over what was drawn to take that part away").clicked() {
                            st.pen.rubbing = !st.pen.rubbing;
                        }
                        if icon_button(ui, icons::TRASH_2, b, false, pal().hover)
                            .on_hover_text("Clear everything drawn on this canvas")
                            .clicked()
                        {
                            actions.push(Action::ClearDoodles);
                        }
                    });
                });
        });
    st.floating.push(resp.response.rect);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShareMode {
    #[default]
    Off,
    Hosting,
    Joining,
    Joined,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShareOp {
    Offer,
    Start,
    Stop,
    Enter,
    Join,
    SetWriter(u32, bool),
    SendAway(u32),
    Follow(Option<u32>),
    CopyInvite,
    Changed,
    Panel,
    CallEveryone,
}

pub struct PersonView {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub writer: bool,
    pub host: bool,
    pub me: bool,
}

pub struct PeerMark {
    pub name: String,
    pub color: u32,
    pub looked_at: Option<Rect>,
    pub pointer: Option<Pos2>,
    pub caret: Option<Rect>,
    pub chosen: Option<Rect>,
}

#[derive(Default)]
pub struct ShareView {
    pub open: bool,
    pub mode: ShareMode,
    pub invite: String,
    pub typed: String,
    pub name: String,
    pub address: String,
    pub guests_write: bool,
    pub writer: bool,
    pub people: Vec<PersonView>,
    pub marks: Vec<PeerMark>,
    pub following: Option<u32>,
    button: Option<Rect>,
}

fn initial(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_string())
}

fn readable_on(color: Color32) -> Color32 {
    let light = 0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32;
    if light > 150.0 {
        Color32::from_rgb(16, 18, 22)
    } else {
        Color32::WHITE
    }
}

fn avatar(ui: &mut egui::Ui, person: &PersonView, size: f32, followed: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size + 4.0), Sense::click());
    let color = c(person.color);
    if followed {
        ui.painter().circle_stroke(rect.center(), size * 0.5 + 2.0, Stroke::new(1.5, color));
    }
    let fill = if resp.hovered() { color } else { color.gamma_multiply(0.9) };
    ui.painter().circle_filled(rect.center(), size * 0.5 - if followed { 1.0 } else { 0.0 }, fill);
    let font = FontId::proportional(size * 0.52);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, initial(&person.name), font, readable_on(color));
    resp
}

fn share_button(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    let on = st.share.mode != ShareMode::Off;
    let glyph = crate::codicons::glyph("live-share").unwrap_or(icons::WAYPOINTS);
    let (rect, resp) = ui.allocate_exact_size(RAIL_ICON, Sense::click());
    let lit = on || st.share.open;
    if resp.hovered() || st.share.open {
        ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().hover);
    }
    let color = if lit { pal().accent } else { pal().text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), color);
    if on {
        ui.painter().circle_filled(rect.right_top() + Vec2::new(-8.0, 8.0), 3.5, c(theme::get().git_added));
    }
    let tip = match st.share.mode {
        ShareMode::Off => "Share this session, or join someone's",
        ShareMode::Hosting => "You are sharing this session",
        ShareMode::Joining => "Connecting to a session",
        ShareMode::Joined => "You are in someone's session",
    };
    if resp.on_hover_text(tip).clicked() {
        st.share.open = !st.share.open;
    }
    st.share.button = Some(rect);
    const STEP: f32 = 21.0;
    const APART: f32 = 31.0;
    let space = (ui.available_height() - 16.0).max(0.0);
    let room = (space / STEP) as usize;
    let shown = st.agents.len().min(12) + st.share.people.iter().filter(|person| !person.me).count().min(8);
    let pointer = ui.ctx().pointer_hover_pos();
    let over = st.faces_stack.zip(pointer).is_some_and(|(stack, at)| stack.expand2(Vec2::new(6.0, 4.0)).contains(at));
    let spread = ui.ctx().animate_bool_with_time(egui::Id::unique("the faces spread apart"), over, 0.14);
    let step = (STEP + (APART - STEP) * spread).min(space / shown.clamp(1, room.max(1)) as f32).max(STEP);
    let mut stack: Option<Rect> = None;
    if !st.agents.is_empty() || st.share.people.iter().any(|person| !person.me) {
        ui.add_space(4.0);
        let (line, _) = ui.allocate_exact_size(Vec2::new(18.0, 1.0), Sense::hover());
        ui.painter().rect_filled(line, 0.0, pal().border);
        ui.add_space(8.0);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    load_faces(ui.ctx(), st);
    let cut = |ui: &egui::Ui, at: Pos2, round: f32| {
        let behind = Rect::from_center_size(at, Vec2::splat(28.0));
        ui.painter().rect_filled(behind, CornerRadius::same(round as u8), pal().well);
    };
    for agent in st.agents.iter().take(room.min(12)) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(RAIL_ICON.x, step), Sense::click());
        stack = Some(stack.map_or(rect, |stack| stack.union(rect)));
        let at = rect.center() - Vec2::new(0.0, 2.0);
        cut(ui, at, 12.0);
        let lifted = if resp.hovered() { Vec2::new(0.0, -2.0) } else { Vec2::ZERO };
        face(ui.painter(), &st.faces, agent.avatar, at + lifted, 11.0, c(agent.color), agent.quiet, agent.followed);
        let how = if agent.followed { "You follow it. Click to stop." } else { "Click to follow it on the canvas." };
        if resp
            .on_hover_text(format!(
                "{} · {}
{}
{how}",
                agent.name, agent.role, agent.said
            ))
            .clicked()
        {
            actions.push(Action::Agent(AgentOp::Follow((!agent.followed).then(|| agent.key.clone()))));
        }
    }
    let following = st.share.following;
    let left = room.saturating_sub(st.agents.len().min(12)).min(8);
    for person in st.share.people.iter().filter(|person| !person.me).take(left) {
        let followed = following == Some(person.id);
        let tip = match followed {
            true => format!("{}: you follow what they look at. Click to stop.", person.name),
            false => format!("{}: click to follow what they look at", person.name),
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(RAIL_ICON.x, step), Sense::click());
        stack = Some(stack.map_or(rect, |stack| stack.union(rect)));
        let at = rect.center() - Vec2::new(0.0, 2.0);
        cut(ui, at, 15.0);
        let color = c(person.color);
        if followed {
            ui.painter().circle_stroke(at, 13.0, Stroke::new(1.5, color));
        }
        ui.painter().circle_filled(at, 11.0, if resp.hovered() { color } else { color.gamma_multiply(0.9) });
        let font = FontId::proportional(11.5);
        ui.painter().text(at, Align2::CENTER_CENTER, initial(&person.name), font, readable_on(color));
        if resp.on_hover_text(tip).clicked() {
            actions.push(Action::Share(ShareOp::Follow((!followed).then_some(person.id))));
        }
    }
    st.faces_stack = stack;
}

fn name_tag(painter: &egui::Painter, at: Pos2, name: &str, color: Color32) {
    let galley = painter.layout_no_wrap(name.to_string(), FontId::proportional(11.0), readable_on(color));
    let rect = Rect::from_min_size(at, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(rect, 5.0, color);
    painter.galley(rect.min + Vec2::new(5.0, 2.0), galley, readable_on(color));
}

fn peer_marks(painter: &egui::Painter, st: &UiState, canvas: Rect) {
    for mark in &st.share.marks {
        let color = c(mark.color);
        if let Some(rect) =
            mark.looked_at.filter(|rect| rect.intersects(canvas) && !rect.contains_rect(canvas.shrink(8.0)))
        {
            painter.rect_stroke(rect, 6.0, Stroke::new(1.5, color.gamma_multiply(0.75)), egui::StrokeKind::Inside);
            if mark.pointer.is_none_or(|at| !canvas.contains(at)) {
                let at = Pos2::new(rect.left().max(canvas.left()) + 6.0, rect.top().max(canvas.top()) + 6.0);
                name_tag(painter, at, &mark.name, color);
            }
        }
        if let Some(rect) = mark.chosen {
            painter.rect_filled(rect, 1.0, color.gamma_multiply(0.28));
        }
        if let Some(cell) = mark.caret.filter(|cell| canvas.intersects(*cell)) {
            let bar =
                Rect::from_min_max(cell.left_top() - Vec2::new(1.0, 0.0), cell.left_bottom() + Vec2::new(1.0, 0.0));
            painter.rect_filled(bar, 0.0, color);
            if cell.height() >= 7.0 {
                name_tag(painter, Pos2::new(cell.left() - 1.0, cell.top() - 17.0), &mark.name, color);
            }
        }
        if let Some(at) = mark.pointer.filter(|at| canvas.contains(*at)) {
            let arrow = vec![at, at + Vec2::new(0.0, 15.0), at + Vec2::new(4.2, 11.2), at + Vec2::new(10.5, 10.5)];
            painter.add(egui::Shape::convex_polygon(arrow, color, Stroke::new(1.0, Color32::from_black_alpha(160))));
            name_tag(painter, at + Vec2::new(10.0, 14.0), &mark.name, color);
        }
    }
}

pub struct AgentView {
    pub key: String,
    pub name: String,
    pub role: String,
    pub said: String,
    pub color: u32,
    pub avatar: usize,
    pub quiet: bool,
    pub followed: bool,
}

pub struct AgentMark {
    pub avatar: usize,
    pub color: u32,
    pub name: String,
    pub said: String,
    pub at: Pos2,
    pub lines: Vec<Rect>,
    pub busy: bool,
    pub quiet: bool,
}

pub struct AgentTrail {
    pub color: u32,
    pub name: String,
    pub steps: Vec<TrailStep>,
}

pub struct TrailStep {
    pub at: Pos2,
    pub line_h: f32,
    pub lines: Vec<Rect>,
    pub edited: bool,
    pub did: String,
    pub age: std::time::Duration,
    pub left: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccountOp {
    SignIn,
    Cancel,
    CopyAndOpen,
    SyncNow,
    SignOut,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentOp {
    Follow(Option<String>),
    ConnectClaude(bool),
}

fn load_faces(ctx: &egui::Context, st: &mut UiState) {
    if !st.faces.is_empty() || st.agents.is_empty() {
        return;
    }
    for (i, bytes) in crate::agents::AVATARS.iter().enumerate() {
        let Ok(picture) = image::load_from_memory(bytes) else { continue };
        let picture = picture.to_rgba8();
        let size = [picture.width() as usize, picture.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, picture.as_raw());
        st.faces.push(ctx.load_texture(format!("assistant {i}"), pixels, egui::TextureOptions::LINEAR));
    }
}

#[allow(clippy::too_many_arguments)]
fn face(
    painter: &egui::Painter,
    faces: &[egui::TextureHandle],
    avatar: usize,
    center: Pos2,
    radius: f32,
    color: Color32,
    quiet: bool,
    ringed: bool,
) {
    let rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    if ringed {
        let ring = Rect::from_center_size(center, Vec2::splat(radius * 2.0 + 7.0));
        let round = CornerRadius::same(((radius + 3.5) * 0.46) as u8);
        painter.rect_stroke(ring, round, Stroke::new(1.5, color), egui::StrokeKind::Middle);
    }
    let corner = |r: f32| CornerRadius::same((r * 0.46).min(120.0) as u8);
    let around = Rect::from_center_size(center, Vec2::splat(radius * 2.0 + 3.0));
    painter.rect_filled(around, corner(radius + 1.5), if quiet { color.gamma_multiply(0.45) } else { color });
    let tint = if quiet { Color32::from_gray(150) } else { Color32::WHITE };
    match faces.get(avatar % faces.len().max(1)) {
        Some(picture) => {
            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            let round = corner(radius);
            painter.add(egui::epaint::RectShape::filled(rect, round, tint).with_texture(picture.id(), whole));
        }
        None => {
            painter.circle_filled(center, radius, color);
        }
    }
}

fn ago(age: std::time::Duration) -> String {
    match age.as_secs() {
        0..60 => "just now".to_string(),
        60..120 => "a minute ago".to_string(),
        secs => format!("{} minutes ago", secs / 60),
    }
}

fn trail_curve(from: Pos2, to: Pos2, canvas: Rect) -> Vec<Pos2> {
    let along = to - from;
    let bend = from + along * 0.5 + along.rot90() * 0.18;
    let room = Rect::from_points(&[from, to, bend]);
    if !room.intersects(canvas) || along.length() < 1.0 {
        return Vec::new();
    }
    let parts = ((along.length() / 24.0) as usize).clamp(2, 48);
    (0..=parts)
        .map(|i| {
            let t = i as f32 / parts as f32;
            let u = 1.0 - t;
            Pos2::new(
                u * u * from.x + 2.0 * u * t * bend.x + t * t * to.x,
                u * u * from.y + 2.0 * u * t * bend.y + t * t * to.y,
            )
        })
        .collect()
}

fn agent_trails(ctx: &egui::Context, painter: &egui::Painter, st: &UiState, canvas: Rect) {
    if st.agent_trails.is_empty() {
        return;
    }
    let pointer = ctx
        .pointer_hover_pos()
        .filter(|at| canvas.contains(*at) && !st.floating.iter().any(|taken| taken.contains(*at)));
    let mut hovered: Option<(f32, &AgentTrail, &TrailStep)> = None;
    for trail in &st.agent_trails {
        let color = c(trail.color);
        for pair in trail.steps.windows(2) {
            let points = trail_curve(pair[0].at, pair[1].at, canvas);
            if points.len() < 2 {
                continue;
            }
            let length: f32 = points.windows(2).map(|p| (p[1] - p[0]).length()).sum();
            let spacing = (length / 300.0).max(7.0);
            let faint = color.gamma_multiply(0.5 * pair[0].left.max(0.15));
            painter.extend(egui::Shape::dotted_line(&points, faint, spacing, 1.1));
        }
        for step in &trail.steps {
            let strength = step.left.max(0.0).powf(0.6);
            for line in step.lines.iter().filter(|line| line.intersects(canvas)) {
                painter.rect_filled(*line, 1.0, color.gamma_multiply(0.07 * strength));
            }
            let radius = (step.line_h * 0.36).clamp(2.5, 5.0);
            if !canvas.expand(radius).contains(step.at) {
                continue;
            }
            let mark = color.gamma_multiply(0.25 + 0.65 * strength);
            match step.edited {
                true => painter.circle_filled(step.at, radius, mark),
                false => painter.circle_stroke(step.at, radius - 0.6, Stroke::new(1.3, mark)),
            };
            if let Some(near) = pointer.map(|at| (at - step.at).length()).filter(|near| *near <= radius + 4.0) {
                if hovered.is_none_or(|(best, _, _)| near <= best) {
                    hovered = Some((near, trail, step));
                }
            }
        }
    }
    if let Some((_, trail, step)) = hovered {
        painter.circle_stroke(step.at, (step.line_h * 0.36).clamp(2.5, 5.0) + 3.0, Stroke::new(1.5, c(trail.color)));
        egui::Area::new(egui::Id::unique("trail_tip"))
            .fixed_pos(step.at + Vec2::new(14.0, 10.0))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                Frame::NONE
                    .fill(pal().card)
                    .stroke(Stroke::new(1.0, c(trail.color)))
                    .corner_radius(CornerRadius::same(7))
                    .inner_margin(Margin::symmetric(9, 6))
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.label(RichText::new(format!("{} {}", trail.name, step.did)).size(13.0).color(pal().text));
                        ui.label(RichText::new(ago(step.age)).size(11.0).color(pal().text_dim));
                    });
            });
    }
    ctx.request_repaint_after(std::time::Duration::from_secs(20));
}

fn agent_marks(ctx: &egui::Context, painter: &egui::Painter, st: &mut UiState, canvas: Rect) {
    load_faces(ctx, st);
    agent_trails(ctx, painter, st, canvas);
    let beat = (ctx.input(|i| i.time) * 4.0).sin() as f32 * 0.5 + 0.5;
    for mark in &st.agent_marks {
        let color = c(mark.color);
        for line in mark.lines.iter().filter(|line| line.intersects(canvas)) {
            painter.rect_filled(*line, 1.0, color.gamma_multiply(0.13));
            let edge = Rect::from_min_max(line.left_top(), Pos2::new(line.left() + 2.0, line.bottom()));
            painter.rect_filled(edge, 0.0, color.gamma_multiply(0.8));
        }
        let center = mark.at + Vec2::new(-20.0, 12.0);
        if !canvas.expand(40.0).contains(center) {
            continue;
        }
        if mark.busy {
            painter.circle_stroke(center, 17.0 + beat * 4.0, Stroke::new(2.0, color.gamma_multiply(0.75 - beat * 0.5)));
        }
        face(painter, &st.faces, mark.avatar, center, 14.0, color, mark.quiet, false);
        let tag = if mark.quiet { mark.name.clone() } else { format!("{} · {}", mark.name, mark.said) };
        let tint = if mark.quiet { color.gamma_multiply(0.55) } else { color };
        name_tag(painter, center + Vec2::new(-14.0, 18.0), &tag, tint);
    }
    if st.agent_marks.iter().any(|mark| mark.busy) {
        ctx.request_repaint();
    }
}

fn following_bar(ctx: &egui::Context, st: &mut UiState, canvas: Rect, actions: &mut Vec<Action>) {
    let person = st.share.following.and_then(|id| st.share.people.iter().find(|person| person.id == id));
    let agent = st.agents.iter().find(|agent| agent.followed);
    let (name, color, stop) = match (person, agent) {
        (Some(person), _) => (person.name.clone(), person.color, Action::Share(ShareOp::Follow(None))),
        (None, Some(agent)) => {
            (format!("{} ({})", agent.name, agent.role), agent.color, Action::Agent(AgentOp::Follow(None)))
        }
        (None, None) => return,
    };
    let area = egui::Area::new(egui::Id::unique("following"))
        .pivot(Align2::LEFT_TOP)
        .fixed_pos(canvas.left_top() + Vec2::new(12.0, 10.0))
        .order(egui::Order::Foreground);
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, c(color)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(10, 6));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 5.0, c(color));
                ui.label(RichText::new(format!("Following {name}")).size(12.5).color(pal().text));
                ui.label(RichText::new("Esc, or move the canvas, to stop").size(11.5).color(pal().text_faint));
                if small_icon(ui, icons::X, "Stop following") {
                    actions.push(stop);
                }
            });
        });
    });
    st.floating.push(resp.response.rect);
}

fn share_people(ui: &mut egui::Ui, view: &ShareView, hosting: bool, actions: &mut Vec<Action>) {
    for person in &view.people {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let followed = view.following == Some(person.id);
            avatar(ui, person, 22.0, followed);
            let about = match (person.me, person.host, person.writer) {
                (true, true, _) => "you, sharing",
                (true, false, true) => "you, can edit",
                (true, false, false) => "you, can look",
                (false, true, _) => "shares the session",
                (false, false, true) => "can edit",
                (false, false, false) => "can look",
            };
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(egui::Label::new(RichText::new(&person.name).size(13.0).color(pal().text)).truncate());
                ui.label(RichText::new(about).size(11.0).color(pal().text_faint));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if person.me {
                    return;
                }
                if hosting && small_icon(ui, icons::X, "Take them out of the session") {
                    actions.push(Action::Share(ShareOp::SendAway(person.id)));
                }
                if hosting {
                    let label = if person.writer { "Editing" } else { "Looking" };
                    if ext_panels::pill(ui, label, person.writer) {
                        actions.push(Action::Share(ShareOp::SetWriter(person.id, !person.writer)));
                    }
                }
                if ext_panels::pill(ui, if followed { "Following" } else { "Follow" }, followed) {
                    actions.push(Action::Share(ShareOp::Follow((!followed).then_some(person.id))));
                }
            });
        });
    }
}

fn share_panel(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    const WIDTH: f32 = 380.0;
    let Some(button) = st.share.button.filter(|_| st.share.open) else { return };
    let screen = ctx.content_rect();
    let at = Pos2::new((button.right() + 10.0).min(screen.right() - WIDTH - 8.0), button.bottom());
    let area = egui::Area::new(egui::Id::unique("share"))
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(at)
        .order(egui::Order::Foreground);
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let view = &mut st.share;
    let mut changed = false;
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border_strong))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(16));
        frame.show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            let title = |ui: &mut egui::Ui, text: &str, lit: bool| {
                ui.horizontal(|ui| {
                    let glyph = crate::codicons::glyph("live-share").unwrap_or(icons::WAYPOINTS);
                    ui.label(icon(glyph, 16.0, if lit { pal().accent } else { pal().text_dim }));
                    ui.label(RichText::new(text).size(15.0).strong().color(pal().text));
                });
            };
            let dim = |ui: &mut egui::Ui, text: &str| {
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(pal().text_dim)).wrap());
            };
            let field = |ui: &mut egui::Ui, text: &mut String, hint: &str, mono: bool| {
                let font = if mono { FontId::monospace(12.0) } else { FontId::proportional(13.0) };
                ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY).font(font))
            };
            match view.mode {
                ShareMode::Off => {
                    title(ui, "Share this session", false);
                    dim(ui, "Others open your folders on their own canvas, see what everyone types as it is typed, and where each one is looking.");
                    ui.label(RichText::new("Your name").size(11.5).color(pal().text_faint));
                    changed |= field(ui, &mut view.name, "How the others will see you", false).lost_focus();
                    changed |= check(ui, &mut view.guests_write, "Those who join can edit", true, false).clicked();
                    dim(ui, if view.guests_write { "They type in your files. You can change it for each one." } else { "They only look. You can let each one edit later." });
                    if ask_button(ui, "Start sharing", ChoiceLook::Main) {
                        actions.push(Action::Share(ShareOp::Changed));
                        actions.push(Action::Share(ShareOp::Offer));
                    }
                    menu_gap(ui);
                    ui.label(RichText::new("Join someone's session").size(13.0).color(pal().text));
                    let typed = field(ui, &mut view.typed, "Paste the invitation: cc1:…", true);
                    let entered = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ask_button(ui, "Join", ChoiceLook::Plain) || entered) && !view.typed.trim().is_empty() {
                        actions.push(Action::Share(ShareOp::Changed));
                        actions.push(Action::Share(ShareOp::Enter));
                    }
                }
                ShareMode::Hosting => {
                    title(ui, "You are sharing this session", true);
                    ui.label(RichText::new("Invitation").size(11.5).color(pal().text_faint));
                    Frame::NONE.fill(pal().well).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 8)).show(
                        ui,
                        |ui| {
                            ui.set_width(WIDTH - 20.0);
                            let text = RichText::new(&view.invite).font(FontId::monospace(12.0)).color(pal().text);
                            ui.add(egui::Label::new(text).truncate());
                        },
                    );
                    if ask_button(ui, "Copy the invitation", ChoiceLook::Main) {
                        actions.push(Action::Share(ShareOp::CopyInvite));
                    }
                    dim(ui, "Whoever has it gets in: send it only to who should. It stops working when you stop sharing or close the app.");
                    ui.label(RichText::new("The address the others reach this computer at").size(11.5).color(pal().text_faint));
                    changed |= field(ui, &mut view.address, "This network's, unless you write another", true).changed();
                    dim(ui, "On the same network nothing has to be written. From outside: the address of a VPN both are in (Tailscale, ZeroTier), or your public one with the port of the invitation forwarded.");
                    changed |= check(ui, &mut view.guests_write, "Those who join from now on can edit", true, false).clicked();
                    menu_gap(ui);
                    share_people(ui, view, true, actions);
                    if view.people.len() < 2 {
                        dim(ui, "Nobody has joined yet.");
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ask_button(ui, "Stop sharing", ChoiceLook::Danger) {
                            actions.push(Action::Share(ShareOp::Stop));
                        }
                        if view.people.len() > 1 && ask_button(ui, "Bring everyone to what I see", ChoiceLook::Plain) {
                            actions.push(Action::Share(ShareOp::CallEveryone));
                        }
                    });
                }
                ShareMode::Joining => {
                    title(ui, "Connecting…", true);
                    dim(ui, "Reaching the computer that shares.");
                    if ask_button(ui, "Cancel", ChoiceLook::Plain) {
                        actions.push(Action::Share(ShareOp::Stop));
                    }
                }
                ShareMode::Joined => {
                    title(ui, "You are in a shared session", true);
                    dim(ui, if view.writer { "You can edit: take the pencil and type. Ctrl+S saves the file where it lives." } else { "You can look around. The one sharing decides who edits." });
                    menu_gap(ui);
                    share_people(ui, view, false, actions);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ask_button(ui, "Leave", ChoiceLook::Danger) {
                            actions.push(Action::Share(ShareOp::Stop));
                        }
                        if ask_button(ui, "Bring everyone to what I see", ChoiceLook::Plain) {
                            actions.push(Action::Share(ShareOp::CallEveryone));
                        }
                    });
                }
            }
        });
    });
    let rect = resp.response.rect;
    st.floating.push(rect);
    let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    close |= pressed.is_some_and(|at| !rect.contains(at) && !button.contains(at));
    if changed {
        actions.push(Action::Share(ShareOp::Changed));
    }
    if close {
        st.share.open = false;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Asked {
    Delete { folder: std::path::PathBuf, path: String },
    Git { folder: std::path::PathBuf, op: crate::git::GitOp },
    Close { folder: std::path::PathBuf, forget: bool },
    Overwrite { folder: std::path::PathBuf, path: String },
    Share { join: bool },
    Sync,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    Other,
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChoiceLook {
    Main,
    Danger,
    Plain,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    Picture(std::path::PathBuf),
    Plays(std::path::PathBuf),
    Text { lines: Vec<String>, more: usize },
}

pub struct Ask {
    pub about: Asked,
    pub preview: Option<Preview>,
    pub danger: bool,
    pub title: String,
    pub text: String,
    pub items: Vec<String>,
    pub choices: Vec<(&'static str, ChoiceLook, Answer)>,
    pub fresh: bool,
}

impl Ask {
    pub fn new(about: Asked, title: &str, text: String) -> Ask {
        Ask {
            about,
            preview: None,
            danger: true,
            title: title.to_string(),
            text,
            items: Vec::new(),
            choices: Vec::new(),
            fresh: true,
        }
    }

    pub fn choice(mut self, label: &'static str, look: ChoiceLook, answer: Answer) -> Ask {
        self.choices.push((label, look, answer));
        self
    }
}

fn ask_button(ui: &mut egui::Ui, label: &str, look: ChoiceLook) -> bool {
    let font = FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, pal().text);
    let size = Vec2::new((galley.size().x + 28.0).max(84.0), 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let danger = c(theme::get().git_removed);
    let (fill, text, stroke) = match (look, resp.hovered()) {
        (ChoiceLook::Main, hot) => (pal().accent.gamma_multiply(if hot { 1.0 } else { 0.88 }), pal().on_accent, None),
        (ChoiceLook::Danger, hot) => (danger.gamma_multiply(if hot { 1.0 } else { 0.86 }), Color32::WHITE, None),
        (ChoiceLook::Plain, true) => (pal().hover, pal().text, Some(pal().border_strong)),
        (ChoiceLook::Plain, false) => (pal().raised, pal().text, Some(pal().border)),
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    if let Some(stroke) = stroke {
        ui.painter().rect_stroke(rect, 8.0, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn ask_modal(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    const WIDTH: f32 = 440.0;
    const SHOWN: usize = 8;
    const TALL: f32 = 210.0;
    st.ask_page = None;
    let Some(ask) = st.ask.as_mut() else { return };
    st.floating.push(ctx.content_rect());
    st.page_places.clear();
    let fresh = std::mem::take(&mut ask.fresh);
    let (entered, escaped) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    let mut answer = None;
    if escaped {
        answer = Some(Answer::Cancel);
    } else if entered && !fresh {
        answer = ask.choices.iter().find(|choice| choice.1 != ChoiceLook::Plain).map(|choice| choice.2);
    }
    let frame = Frame::NONE
        .fill(pal().card)
        .stroke(Stroke::new(1.0, pal().border_strong))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(20));
    let tint = if ask.danger { c(theme::get().git_removed) } else { pal().accent };
    egui::Modal::new(egui::Id::unique(("ask", ask.title.as_str(), ask.preview.is_some())))
        .backdrop_color(Color32::from_black_alpha(150))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing = Vec2::new(12.0, 6.0);
            ui.horizontal_top(|ui| {
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                ui.painter().rect_filled(badge, 12.0, tint.gamma_multiply(0.16));
                let glyph = if ask.danger { icons::TRIANGLE_ALERT } else { icons::CIRCLE_ALERT };
                ui.painter().text(badge.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(20.0), tint);
                ui.vertical(|ui| {
                    ui.set_width(WIDTH - 52.0);
                    ui.add_space(1.0);
                    ui.label(RichText::new(&ask.title).size(15.5).strong().color(pal().text));
                    if !ask.text.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&ask.text).size(13.0).color(pal().text_dim)).wrap());
                    }
                });
            });
            match &ask.preview {
                Some(Preview::Picture(_)) | Some(Preview::Plays(_)) => {
                    ui.add_space(6.0);
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH, TALL), Sense::hover());
                    ui.painter().rect_filled(rect, 8.0, pal().well);
                    let room = rect.shrink(6.0);
                    match (&ask.preview, st.ask_picture) {
                        (Some(Preview::Plays(_)), _) => st.ask_page = Some(room),
                        (_, Some((picture, [w, h]))) if w > 0 && h > 0 => {
                            let fit = (room.width() / w as f32).min(room.height() / h as f32).min(1.0);
                            let shown = Rect::from_center_size(room.center(), Vec2::new(w as f32, h as f32) * fit);
                            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                            ui.painter().image(picture, shown, whole, Color32::WHITE);
                        }
                        _ => {}
                    }
                }
                Some(Preview::Text { lines, more }) => {
                    ui.add_space(6.0);
                    Frame::NONE
                        .fill(pal().well)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(WIDTH - 24.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            for line in lines {
                                let text = RichText::new(line).font(FontId::monospace(11.5)).color(pal().text_dim);
                                ui.add(egui::Label::new(text).truncate());
                            }
                            if *more > 0 {
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(format!("… and {more} more")).size(11.5).color(pal().text_faint),
                                );
                            }
                        });
                }
                None => {}
            }
            if !ask.items.is_empty() {
                ui.add_space(6.0);
                Frame::NONE
                    .fill(pal().well)
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 24.0);
                        ui.spacing_mut().item_spacing.y = 3.0;
                        for item in ask.items.iter().take(SHOWN) {
                            let text = RichText::new(item).font(FontId::monospace(12.0)).color(pal().text);
                            ui.add(egui::Label::new(text).truncate());
                        }
                        if ask.items.len() > SHOWN {
                            let more = format!("… and {} more", ask.items.len() - SHOWN);
                            ui.label(RichText::new(more).size(11.5).color(pal().text_faint));
                        }
                    });
            }
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (label, look, choice) in ask.choices.iter() {
                    if ask_button(ui, label, *look) {
                        answer = Some(*choice);
                    }
                }
            });
        });
    if let Some(answer) = answer {
        if let Some(ask) = st.ask.take() {
            actions.push(Action::Answered(ask.about, answer));
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MenuOn {
    Canvas,
    Folder { path: String, collapsed: bool },
    File { path: String },
}

pub struct CardMenu {
    pub at: Pos2,
    pub folder: Option<(std::path::PathBuf, String)>,
    pub on: MenuOn,
    pub line: Option<u32>,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub place: Option<Rect>,
    pub nothing_open: bool,
}

fn menu_row_key(ui: &mut egui::Ui, glyph: &str, label: &str, key: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal().hover);
    }
    let (mid, font) = (rect.center().y, FontId::proportional(13.0));
    ui.painter().text(Pos2::new(rect.left() + 10.0, mid), Align2::LEFT_CENTER, glyph, font.clone(), pal().text_dim);
    ui.painter().text(Pos2::new(rect.left() + 32.0, mid), Align2::LEFT_CENTER, label, font, pal().text);
    let small = FontId::proportional(11.5);
    ui.painter().text(Pos2::new(rect.right() - 10.0, mid), Align2::RIGHT_CENTER, key, small, pal().text_faint);
    resp.clicked()
}

fn card_menu(ctx: &egui::Context, st: &mut UiState, canvas: Rect, actions: &mut Vec<Action>) {
    use crate::git::{GitOp, Part};
    const WIDTH: f32 = 252.0;
    let bare = st.bare;
    let (ui_key, fit_key, commands_key) =
        (st.keys.label(Command::ToggleUi), st.keys.label(Command::ShowAll), st.keys.label(Command::Palette));
    let Some(menu) = st.card_menu.as_mut() else { return };
    let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    let outside = pressed.is_some_and(|at| menu.place.is_some_and(|place| !place.contains(at)));
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.card_menu = None;
        return;
    }
    let tall = menu.place.map_or(330.0, |place| place.height());
    let at = Pos2::new(
        menu.at.x.min(canvas.right() - WIDTH - 20.0).max(canvas.left() + 4.0),
        menu.at.y.min(canvas.bottom() - tall - 8.0).max(canvas.top() + 4.0),
    );
    let area = egui::Area::new(egui::Id::unique("card menu")).fixed_pos(at).order(egui::Order::Foreground);
    let mut chosen: Vec<Action> = Vec::new();
    let mut rename = None;
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(6));
        frame.show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 1.0;
            let on = menu.on.clone();
            let any_folder = !menu.nothing_open;
            match &on {
                MenuOn::File { path } => {
                    let part = |what: Part, line: u32| Action::Git(GitOp::Part { path: path.clone(), line, what });
                    if let Some(line) = menu.line {
                        if menu.unstaged && menu_row(ui, icons::PLUS, "Stage this change") {
                            chosen.push(part(Part::Stage, line));
                        }
                        if menu.staged && menu_row(ui, icons::MINUS, "Unstage this change") {
                            chosen.push(part(Part::Unstage, line));
                        }
                        if menu.unstaged && menu_row(ui, icons::ARROW_LEFT, "Discard this change") {
                            chosen.push(part(Part::Discard, line));
                        }
                        menu_gap(ui);
                    }
                    if (menu.unstaged || menu.untracked) && menu_row(ui, icons::PLUS, "Stage the file") {
                        chosen.push(Action::Git(GitOp::Stage(path.clone())));
                    }
                    if menu.staged && menu_row(ui, icons::MINUS, "Unstage the file") {
                        chosen.push(Action::Git(GitOp::Unstage(path.clone())));
                    }
                    if menu.staged || menu.unstaged || menu.untracked {
                        menu_gap(ui);
                    }
                }
                MenuOn::Folder { path, collapsed } => {
                    let label = if *collapsed { "Expand this folder" } else { "Collapse this folder" };
                    let glyph = if *collapsed { icons::UNFOLD_VERTICAL } else { icons::FOLD_VERTICAL };
                    if menu_row(ui, glyph, label) {
                        chosen.push(Action::ToggleCollapsed(path.clone()));
                    }
                }
                MenuOn::Canvas => {}
            }
            let target = match &on {
                MenuOn::File { path } => Some((path.clone(), false)),
                MenuOn::Folder { path, .. } => Some((path.clone(), true)),
                MenuOn::Canvas => None,
            };
            if let Some((path, is_dir)) = &target {
                let inside = !path.is_empty();
                if inside && menu_row(ui, icons::EYE_OFF, "Hide from the canvas") {
                    chosen.push(Action::ToggleHidden(path.clone()));
                }
                if inside && menu_row(ui, icons::FILE_TEXT, "Rename…") {
                    rename = Some(path.clone());
                }
                if inside && menu_row(ui, icons::TRASH_2, "Delete…") {
                    chosen.push(Action::File(FileOp::Delete(path.clone())));
                }
                if inside {
                    menu_gap(ui);
                }
                if menu_row(ui, icons::COPY, "Copy path") {
                    chosen.push(Action::File(FileOp::CopyPath { path: path.clone(), relative: false }));
                }
                if inside && menu_row(ui, icons::COPY, "Copy relative path") {
                    chosen.push(Action::File(FileOp::CopyPath { path: path.clone(), relative: true }));
                }
                let reveal = if cfg!(windows) { "Reveal in File Explorer" } else { "Open the containing folder" };
                if menu_row(ui, icons::FOLDER_OPEN, reveal) {
                    chosen.push(Action::File(FileOp::Reveal(path.clone())));
                }
                if !*is_dir && menu_row(ui, icons::LIST_TREE, "Show it in the files") {
                    chosen.push(Action::RevealInFiles(path.clone()));
                }
                menu_gap(ui);
            }
            let label = if bare { "Show the interface" } else { "Hide the interface" };
            if menu_row_key(ui, icons::PANEL_LEFT, label, &ui_key) {
                chosen.push(Action::ToggleUi);
            }
            if any_folder && menu_row_key(ui, icons::SCAN, "Show the whole canvas", &fit_key) {
                chosen.push(Action::Fit);
            }
            if on == MenuOn::Canvas {
                if any_folder && menu_row(ui, icons::EYE, "Put hidden files back") {
                    chosen.push(Action::ShowAll);
                }
                if any_folder && menu_row(ui, icons::FOLD_VERTICAL, "Collapse every folder") {
                    chosen.push(Action::CollapseAll);
                }
                if any_folder && menu_row(ui, icons::UNFOLD_VERTICAL, "Expand every folder") {
                    chosen.push(Action::ExpandAll);
                }
                menu_gap(ui);
                if menu_row_key(ui, icons::SEARCH, "Commands…", &commands_key) {
                    chosen.push(Action::Commands);
                }
                if menu_row(ui, icons::PLUS, "Open a folder…") {
                    chosen.push(Action::OpenFolder);
                }
            }
            if let Some((root, title)) = &menu.folder {
                menu_gap(ui);
                if menu_row(ui, icons::X, &format!("Take {title} off the canvas")) {
                    chosen.push(Action::Workspace(WorkspaceOp::TakeOff(root.clone())));
                }
            }
        });
    });
    if let Some(path) = rename {
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        st.naming = Some(Naming { kind: NamingKind::Rename, path: path.clone(), text: name, focus: true });
        st.bare = false;
        st.show(Panel::Files);
        st.explorer_reveal = Some(path);
        st.card_menu = None;
        return;
    }
    let Some(menu) = st.card_menu.as_mut() else { return };
    menu.place = Some(resp.response.rect);
    st.floating.push(resp.response.rect);
    if !chosen.is_empty() {
        actions.extend(chosen);
        st.card_menu = None;
    }
}

fn commit_bar(ctx: &egui::Context, st: &mut UiState, page: Option<&Page>, canvas: Rect, actions: &mut Vec<Action>) {
    let Some(showing) = page.and_then(|page| page.showing.as_ref()) else { return };
    let area = egui::Area::new(egui::Id::unique("commit shown"))
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(canvas.center_top() + Vec2::new(0.0, 10.0))
        .order(egui::Order::Foreground);
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(10, 6));
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 4.0);
            ui.horizontal(|ui| {
                let commit = &showing.commit;
                ui.label(icon(icons::GIT_COMMIT_HORIZONTAL, 14.0, pal().accent));
                let short = &commit.hash[..commit.hash.len().min(7)];
                ui.label(RichText::new(short).font(FontId::monospace(12.0)).color(pal().text_dim));
                let room = (canvas.width() - 330.0).clamp(80.0, 460.0);
                let subject = RichText::new(&commit.subject).size(12.5).color(pal().text);
                ui.scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(egui::Label::new(subject).truncate());
                });
                let files = match (showing.loading, showing.here, showing.total) {
                    (true, ..) => "reading …".to_string(),
                    (false, _, 0) => "no files of this folder".to_string(),
                    (false, 0, total) => format!("none of its {total} files is here now"),
                    (false, here, total) if here == total => {
                        format!("{total} {}", if total == 1 { "file" } else { "files" })
                    }
                    (false, here, total) => format!("{here} of {total} files are here now"),
                };
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
                let when = crate::git::ago(now.map(|d| d.as_secs() as i64).unwrap_or(0) - commit.time);
                let about = format!("{} · {when} · {files}", commit.author);
                ui.label(RichText::new(about).size(11.5).color(pal().text_faint));
                if small_icon(ui, icons::X, "Back to the files as they are") {
                    actions.push(Action::ShowCommit(None));
                }
            });
        });
    });
    st.floating.push(resp.response.rect);
}

fn find_bar(ctx: &egui::Context, st: &mut UiState, canvas: Rect, actions: &mut Vec<Action>) {
    let can_replace = st.prefs.edit;
    let Some(find) = st.find.as_mut() else { return };
    let area = egui::Area::new(egui::Id::unique("find in file"))
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(canvas.right_top() + Vec2::new(-12.0, 10.0))
        .order(egui::Order::Foreground);
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(8, 6));
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            let field = |ui: &mut egui::Ui, text: &mut String, hint: &str| {
                let edit = egui::TextEdit::singleline(text).hint_text(hint).desired_width(210.0);
                ui.add(edit.font(FontId::proportional(13.0)))
            };
            let glyph = |name: &str, or: &'static str| crate::codicons::glyph(name).unwrap_or(or);
            ui.horizontal(|ui| {
                let query = field(ui, &mut find.query, "Find in the file");
                if std::mem::take(&mut find.focus) {
                    query.request_focus();
                }
                if query.changed() {
                    actions.push(Action::Find(FindOp::Changed));
                }
                let entered = query.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if entered {
                    let back = ui.input(|i| i.modifiers.shift);
                    actions.push(Action::Find(if back { FindOp::Previous } else { FindOp::Next }));
                    find.focus = true;
                }
                let count = match (find.query.is_empty(), find.count) {
                    (true, _) => String::new(),
                    (false, 0) => "nothing".to_string(),
                    (false, n) => format!("{} of {n}", find.at + 1),
                };
                let (slot, _) = ui.allocate_exact_size(Vec2::new(64.0, 22.0), Sense::hover());
                let tint =
                    if find.count == 0 && !find.query.is_empty() { c(theme::get().ui_warn) } else { pal().text_dim };
                ui.painter().text(slot.left_center(), Align2::LEFT_CENTER, count, FontId::proportional(12.0), tint);
                if small_icon(ui, glyph("arrow-up", icons::ARROW_UP), "The one before (Shift+Enter)") {
                    actions.push(Action::Find(FindOp::Previous));
                }
                if small_icon(ui, glyph("arrow-down", icons::CHEVRON_DOWN), "The next one (Enter)") {
                    actions.push(Action::Find(FindOp::Next));
                }
                if small_icon(ui, icons::X, "Close (Esc)") {
                    actions.push(Action::Find(FindOp::Close));
                }
            });
            ui.horizontal(|ui| {
                if can_replace {
                    let with = field(ui, &mut find.with, "Replace with");
                    if with.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        actions.push(Action::Find(FindOp::Replace));
                    }
                    if small_icon(ui, glyph("replace", icons::CORNER_DOWN_LEFT), "Replace this one") {
                        actions.push(Action::Find(FindOp::Replace));
                    }
                    if small_icon(ui, glyph("replace-all", icons::COPY), "Replace every one") {
                        actions.push(Action::Find(FindOp::ReplaceAll));
                    }
                }
                let name = find.file.rsplit('/').next().unwrap_or(&find.file);
                let about = if can_replace { name.to_string() } else { format!("{name}  ·  the pencil replaces") };
                ui.label(RichText::new(about).size(11.5).color(pal().text_faint));
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                actions.push(Action::Find(FindOp::Close));
            }
        });
    });
    st.floating.push(resp.response.rect);
}

fn quick_score(path: &str, query: &str) -> Option<i32> {
    let lower = path.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if name == query {
        return Some(3000 - lower.len() as i32);
    }
    if name.starts_with(query) {
        return Some(2000 - lower.len() as i32);
    }
    if name.contains(query) {
        return Some(1500 - lower.len() as i32);
    }
    if lower.contains(query) {
        return Some(1000 - lower.len() as i32);
    }
    let mut chars = query.chars();
    let mut want = chars.next()?;
    let mut span = 0;
    let mut started = false;
    for ch in lower.chars() {
        if started {
            span += 1;
        }
        if ch == want {
            started = true;
            match chars.next() {
                Some(next) => want = next,
                None => return Some(500 - span - lower.len() as i32 / 4),
            }
        }
    }
    None
}

fn palette_commands(ext: &Extensions, diff: crate::page::DiffView) -> Vec<(String, Action)> {
    let mut out: Vec<(String, Action)> = vec![
        ("File: Open Folder…".into(), Action::OpenFolder),
        ("File: Save (the file being edited, or every unsaved file)".into(), Action::SaveFile),
        ("Format Document".into(), Action::FormatFile(None)),
        ("Find and Replace in the File".into(), Action::Find(FindOp::Open)),
        ("Markdown: Open Preview".into(), Action::PreviewSelected),
        ("View: Explorer".into(), Action::Show(Panel::Files)),
        ("View: Search".into(), Action::Show(Panel::Search)),
        ("View: Source Control".into(), Action::Show(Panel::Changes)),
        ("View: Extensions".into(), Action::Show(Panel::Extensions)),
        ("View: Problems".into(), Action::Show(Panel::Problems)),
        ("View: Keyboard Shortcuts".into(), Action::Show(Panel::Help)),
        ("Preferences: Settings".into(), Action::Settings(SettingsTab::General)),
        ("Preferences: Keyboard Shortcuts".into(), Action::Settings(SettingsTab::Keyboard)),
        ("Preferences: Colours".into(), Action::Settings(SettingsTab::Colours)),
        ("Preferences: Commit Messages".into(), Action::Settings(SettingsTab::Commits)),
        ("Code Canvas: Check for Updates".into(), Action::CheckUpdates),
        (format!("Code Canvas: About (version {})", crate::update::current()), Action::Settings(SettingsTab::General)),
        ("Preferences: Colour Theme".into(), Action::Settings(SettingsTab::Appearance)),
        ("View: Every Folder on the Canvas, or One at a Time".into(), Action::ToggleSideBySide),
        ("View: Show the Whole Canvas".into(), Action::Fit),
        ("View: Go to the Selection".into(), Action::FrameSelection),
        ("View: Reload the Folder from Disk".into(), Action::Rescan),
        ("Canvas: Collapse All Folders".into(), Action::CollapseAll),
        ("Canvas: Expand All Folders".into(), Action::ExpandAll),
        ("Canvas: Show All Hidden Files".into(), Action::ShowAll),
        ("Source Control: Refresh".into(), Action::RefreshGit),
        ("Source Control: Show Only Changed Files".into(), Action::SetOnlyChanged(true)),
        ("Source Control: Show All Files".into(), Action::SetOnlyChanged(false)),
        ("Source Control: Stop Showing the Commit".into(), Action::ShowCommit(None)),
        ("View: Show or Hide the Interface".into(), Action::ToggleUi),
        ("Sharing: Share This Session or Join One".into(), Action::Share(ShareOp::Panel)),
        ("Sharing: Stop Sharing or Leave".into(), Action::Share(ShareOp::Stop)),
        ("Sharing: Stop Following".into(), Action::Share(ShareOp::Follow(None))),
        ("Sharing: Bring Everyone to What I See".into(), Action::Share(ShareOp::CallEveryone)),
        (
            "Source Control: Show What Changed Inside the Files".into(),
            Action::SetDiff(crate::page::DiffView { on: true, ..diff }),
        ),
        (
            "Source Control: Show the Files as They Are".into(),
            Action::SetDiff(crate::page::DiffView { on: false, ..diff }),
        ),
        ("Extensions: Install from VSIX…".into(), Action::InstallVsix),
    ];
    for e in ext.installed.iter().filter(|e| e.enabled && e.has_code) {
        out.extend(e.commands.iter().map(|(id, title)| (title.clone(), Action::RunCommand(id.clone(), title.clone()))));
    }
    out
}

fn quick_open(
    ctx: &egui::Context,
    st: &mut UiState,
    canvas: Rect,
    page: Option<&Page>,
    ext: &Extensions,
    actions: &mut Vec<Action>,
) {
    let Some(q) = &mut st.quick else { return };
    let file_icons = &mut st.file_icons;
    let mut close = false;
    let screen = ctx.content_rect();
    let width = 560.0f32.min(screen.width() - 40.0).max(240.0);
    let pos = Pos2::new(screen.center().x - width * 0.5, canvas.top().max(HEADER_H) + 6.0);
    let resp =
        egui::Area::new(egui::Id::unique("quick_open")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            Frame::NONE
                .fill(pal().card)
                .stroke(Stroke::new(1.0, pal().border_strong))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(width);
                    let edit = search_field(
                        ui,
                        &mut q.query,
                        "Go to a file or folder by name   ·   > for commands",
                        icons::SEARCH,
                    );
                    if q.focus {
                        edit.request_focus();
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                            let end = egui::text::CCursor::new(q.query.chars().count());
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                            state.store(ui.ctx(), edit.id);
                        }
                        q.focus = false;
                    }
                    if edit.changed() {
                        q.selected = 0;
                    }

                    if let Some(filter) = q.query.strip_prefix('>') {
                        let filter = filter.trim().to_lowercase();
                        let diff = page.map(|page| page.diff).unwrap_or_default();
                        let mut rows: Vec<(i32, String, Action)> = palette_commands(ext, diff)
                            .into_iter()
                            .filter_map(|(label, action)| {
                                let score = if filter.is_empty() {
                                    Some(0)
                                } else {
                                    quick_score(&label.to_lowercase(), &filter)
                                };
                                score.map(|s| (s, label, action))
                            })
                            .collect();
                        if !filter.is_empty() {
                            rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                        }
                        rows.truncate(14);
                        let (up, down, enter, esc) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::ArrowUp),
                                i.key_pressed(egui::Key::ArrowDown),
                                i.key_pressed(egui::Key::Enter),
                                i.key_pressed(egui::Key::Escape),
                            )
                        });
                        if down && !rows.is_empty() {
                            q.selected = (q.selected + 1) % rows.len();
                        }
                        if up && !rows.is_empty() {
                            q.selected = (q.selected + rows.len() - 1) % rows.len();
                        }
                        q.selected = q.selected.min(rows.len().saturating_sub(1));
                        let mut chosen = None;
                        ui.add_space(4.0);
                        for (i, (_, label, _)) in rows.iter().enumerate() {
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                            if i == q.selected || resp.hovered() {
                                let fill = if i == q.selected { pal().raised } else { pal().row };
                                ui.painter().rect_filled(rect, 6.0, fill);
                            }
                            let clip = rect.shrink2(Vec2::new(12.0, 0.0));
                            ui.painter().with_clip_rect(clip).text(
                                clip.left_center(),
                                Align2::LEFT_CENTER,
                                label,
                                FontId::proportional(13.0),
                                pal().text,
                            );
                            if resp.clicked() {
                                chosen = Some(i);
                            }
                        }
                        if rows.is_empty() {
                            ui.label(RichText::new("No matching commands").color(pal().text_dim));
                        }
                        if enter && !rows.is_empty() {
                            chosen = chosen.or(Some(q.selected));
                        }
                        if let Some(i) = chosen {
                            actions.push(rows.swap_remove(i).2);
                            close = true;
                        }
                        if esc || (edit.lost_focus() && !enter && !ui.rect_contains_pointer(ui.min_rect())) {
                            close = true;
                        }
                        return;
                    }

                    let query = q.query.trim().to_lowercase();
                    let mut results: Vec<(i32, &str, bool)> = Vec::new();
                    if let Some(r) = page.and_then(|p| p.ready()) {
                        if !query.is_empty() {
                            for f in &r.repo.files {
                                if let Some(score) = quick_score(&f.rel_path, &query) {
                                    results.push((score, f.rel_path.as_str(), false));
                                }
                            }
                            for path in r.layout.dir_node.keys().filter(|p| !p.is_empty()) {
                                if let Some(score) = quick_score(path, &query) {
                                    results.push((score - 10, path.as_str(), true));
                                }
                            }
                            results.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
                            results.truncate(12);
                        }
                    }

                    let (up, down, enter, esc) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::ArrowDown),
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if down && !results.is_empty() {
                        q.selected = (q.selected + 1) % results.len();
                    }
                    if up && !results.is_empty() {
                        q.selected = (q.selected + results.len() - 1) % results.len();
                    }
                    q.selected = q.selected.min(results.len().saturating_sub(1));

                    let mut chosen: Option<String> = None;
                    if !results.is_empty() {
                        ui.add_space(4.0);
                    }
                    for (i, (_, path, is_dir)) in results.iter().enumerate() {
                        let (name, dir) = match path.rsplit_once('/') {
                            Some((d, n)) => (n, d),
                            None => (*path, ""),
                        };
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                        if i == q.selected || resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, if i == q.selected { pal().raised } else { pal().row });
                        }
                        let icon_at =
                            Rect::from_center_size(Pos2::new(rect.left() + 16.0, rect.center().y), Vec2::splat(16.0));
                        if !file_icons.paint(ui, name, *is_dir, false, icon_at, false) {
                            let glyph = if *is_dir { icons::FOLDER } else { file_glyph(name) };
                            ui.painter().text(
                                icon_at.center(),
                                Align2::CENTER_CENTER,
                                glyph,
                                FontId::proportional(13.0),
                                pal().text_dim,
                            );
                        }
                        let label = if *is_dir { format!("{name}/") } else { name.to_string() };
                        let galley = ui.painter().layout_no_wrap(label, FontId::proportional(13.0), pal().text);
                        let name_w = galley.size().x;
                        ui.painter().galley(
                            Pos2::new(rect.left() + 32.0, rect.center().y - galley.size().y * 0.5),
                            galley,
                            pal().text,
                        );
                        let clip =
                            Rect::from_min_max(Pos2::new(rect.left() + 44.0 + name_w, rect.top()), rect.right_bottom());
                        ui.painter().with_clip_rect(clip).text(
                            Pos2::new(rect.left() + 44.0 + name_w, rect.center().y),
                            Align2::LEFT_CENTER,
                            dir,
                            FontId::proportional(11.0),
                            pal().text_faint,
                        );
                        if resp.clicked() {
                            chosen = Some(path.to_string());
                        }
                    }
                    if !query.is_empty() && results.is_empty() {
                        ui.label(RichText::new("No matches").color(pal().text_dim));
                    }
                    if enter {
                        chosen = chosen.or_else(|| results.get(q.selected).map(|r| r.1.to_string()));
                    }
                    if let Some(path) = chosen {
                        actions.push(Action::FlyToPath(path));
                        close = true;
                    }
                    if esc || (edit.lost_focus() && !enter && !ui.rect_contains_pointer(ui.min_rect())) {
                        close = true;
                    }
                });
        });
    st.floating.push(resp.response.rect);
    if close {
        st.quick = None;
    }
}

pub fn hover_text(markdown: &str) -> (String, String) {
    let mut lines = markdown
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim_start().starts_with("```") && !l.trim().is_empty())
        .map(|l| l.chars().take(140).collect::<String>());
    let title = lines.next().unwrap_or_default();
    let rest: Vec<String> = lines.take(12).collect();
    (title, rest.join("\n"))
}

pub fn note_covers(ctx: &egui::Context, st: &mut UiState) {
    let shown: Vec<(egui::LayerId, Rect)> = ctx.memory(|memory| {
        let layers = memory.areas().visible_layer_ids().into_iter();
        let above = layers.filter(|layer| layer.order != egui::Order::Background);
        above.filter_map(|layer| Some((layer, memory.area_rect(layer.id)?))).collect()
    });
    let drawn = |layer: egui::LayerId| ctx.graphics(|all| all.get(layer).is_some_and(|shapes| !shapes.is_empty()));
    let cards = &st.card_places;
    let a_card = |layer: egui::LayerId| cards.iter().any(|card| card.layer == layer);
    let floating = st.floating.iter().enumerate().filter(|(at, _)| !cards.iter().any(|card| card.floating == *at));
    let mut covers: Vec<Rect> = floating.map(|(_, rect)| *rect).collect();
    covers.extend(shown.into_iter().filter(|(layer, _)| !a_card(*layer) && drawn(*layer)).map(|(_, rect)| rect));
    st.covers = covers;
}

pub fn covers_of(covers: &[Rect], cards: &[CardPlace], page: Option<usize>) -> Vec<Rect> {
    let others = cards.iter().filter(|card| !page.is_some_and(|page| card.pages.contains(&page)));
    covers.iter().copied().chain(others.map(|card| card.over)).collect()
}

fn tooltip(ctx: &egui::Context, canvas: Rect, info: &FrameInfo) {
    let Some((title, detail)) = &info.tooltip else { return };
    let Some(pointer) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    if !canvas.contains(pointer) {
        return;
    }
    egui::Area::new(egui::Id::unique("canvas_tip"))
        .fixed_pos(pointer + Vec2::new(16.0, 18.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            Frame::NONE
                .fill(pal().card)
                .stroke(Stroke::new(1.0, pal().border))
                .corner_radius(CornerRadius::same(7))
                .inner_margin(Margin::symmetric(9, 6))
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(RichText::new(title).size(13.0).color(pal().text));
                    if !detail.is_empty() {
                        ui.label(RichText::new(detail).size(11.0).color(pal().text_dim));
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::quick_score;

    #[test]
    fn the_panel_says_which_relations_are_exact() {
        use canvas_core::{RelKind, Relations, SymRef};
        let mut rel = Relations {
            target: SymRef { file: 0, index: 0 },
            items: Vec::new(),
            truncated: false,
            search_ms: 3.0,
            exact: [false; 5],
        };
        assert_eq!(super::exact_note(&rel), "by name · searched in 3 ms");
        rel.exact[RelKind::OutgoingCall.index()] = true;
        assert_eq!(super::exact_note(&rel), "outgoing calls as the language's extension knows them · the rest by name");
        rel.exact[RelKind::Reference.index()] = true;
        assert_eq!(
            super::exact_note(&rel),
            "uses and outgoing calls as the language's extension knows them · the rest by name"
        );
        rel.exact[RelKind::Implementation.index()] = true;
        assert_eq!(super::exact_note(&rel), "uses, calls and implementations as the language's extension knows them");
    }

    #[test]
    fn hover_markdown_becomes_a_tooltip() {
        let md = "```typescript\nfunction add(a: number, b: number): number\n```\n\nAdds two numbers.\n\n*@param* `a`";
        let (title, rest) = super::hover_text(md);
        assert_eq!(title, "function add(a: number, b: number): number");
        assert_eq!(rest, "Adds two numbers.\n*@param* `a`");
        assert_eq!(super::hover_text(""), (String::new(), String::new()));
    }

    #[test]
    fn a_card_does_not_cover_the_page_it_holds() {
        use super::{covers_of, CardPlace};
        use egui::{pos2, Rect};
        let card = |left: f32, pages: std::ops::Range<usize>| CardPlace {
            over: Rect::from_min_max(pos2(left, 40.0), pos2(left + 420.0, 800.0)),
            pages,
            floating: 0,
            layer: egui::LayerId::background(),
        };
        let menu = Rect::from_min_max(pos2(700.0, 100.0), pos2(900.0, 300.0));
        let (assistant, console) = (card(1000.0, 0..1), card(560.0, 1..1));
        let cards = [assistant, console];
        let (theirs, others) = (cards[0].over, cards[1].over);
        assert_eq!(covers_of(&[menu], &cards, Some(0)), vec![menu, others], "its own card is not over it");
        assert_eq!(covers_of(&[menu], &cards, Some(1)), vec![menu, theirs, others], "a page of no card");
        assert_eq!(covers_of(&[menu], &cards, None), vec![menu, theirs, others], "what plays on the canvas");
    }

    #[test]
    fn quick_open_ranking() {
        let score = |p: &str, q: &str| quick_score(p, q);
        assert!(score("src/app/types.ts", "types.ts") > score("src/app/types.ts.bak", "types.ts"));
        assert!(score("src/api.ts", "api") > score("src/rapid/x.ts", "api"), "name match beats path match");
        assert!(score("src/app/types.ts", "appts").is_some(), "subsequence");
        assert!(score("src/app/types.ts", "zzz").is_none());
    }
}
