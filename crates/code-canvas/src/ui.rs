use std::sync::atomic::Ordering;

use canvas_core::{RelKind, Site};
use canvas_render::{theme, FrameStats};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use crate::ext::Extensions;
use crate::icons;
use crate::page::{build_tree, Page, PageState, RelFilters};

mod deps_panel;
mod ext_panels;
mod git_panel;
mod settings;

use crate::keys::{Command, Keymap};

pub use crate::rails::Side;
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
    Webview(String),
}

impl Panel {
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
            Panel::Webview(view) => format!("view:{view}"),
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
            view => Panel::Webview(view.strip_prefix("view:")?.to_string()),
        })
    }

    fn home(&self) -> Side {
        match self {
            Panel::Symbols | Panel::Deps | Panel::Media | Panel::Webview(_) => Side::Right,
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

pub struct CompletionView {
    pub at: Pos2,
    pub items: Vec<(String, bool)>,
    pub selected: usize,
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

pub struct UiState {
    pub left: Option<Panel>,
    pub right: Option<Panel>,
    pub last_left: Panel,
    pub last_right: Panel,
    pub explorer_filter: String,
    pub search_query: String,
    pub quick: Option<QuickOpen>,
    pub filters: RelFilters,
    pub floating: Vec<Rect>,
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
    pub commit_ai: crate::commit_ai::Settings,
    file_icons: FileIcons,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            left: Some(Panel::Files),
            right: Some(Panel::Symbols),
            last_left: Panel::Files,
            last_right: Panel::Symbols,
            explorer_filter: String::new(),
            search_query: String::new(),
            quick: None,
            filters: RelFilters::default(),
            floating: Vec::new(),
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
            commit_ai: Default::default(),
            file_icons: FileIcons::default(),
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

    pub fn open(&self, side: Side) -> Option<&Panel> {
        match side {
            Side::Left => self.left.as_ref(),
            Side::Right => self.right.as_ref(),
        }
    }

    pub fn is_open(&self, panel: &Panel) -> bool {
        Side::BOTH.into_iter().any(|side| self.open(side) == Some(panel))
    }

    pub fn close(&mut self, side: Side) {
        match side {
            Side::Left => self.left = None,
            Side::Right => self.right = None,
        }
    }

    pub fn close_panel(&mut self, panel: &Panel) {
        for side in Side::BOTH {
            if self.open(side) == Some(panel) {
                self.close(side);
            }
        }
    }

    fn remember(&mut self, side: Side, panel: Panel) {
        match side {
            Side::Left => self.last_left = panel,
            Side::Right => self.last_right = panel,
        }
    }

    pub fn show(&mut self, panel: Panel) {
        let side = self.side_of(&panel);
        self.remember(side, panel.clone());
        match side {
            Side::Left => self.left = Some(panel),
            Side::Right => self.right = Some(panel),
        }
    }

    pub fn toggle(&mut self, panel: Panel) {
        if self.is_open(&panel) {
            self.close_panel(&panel);
        } else {
            self.show(panel);
        }
    }

    pub fn toggle_side(&mut self, side: Side) {
        if self.open(side).is_some() {
            return self.close(side);
        }
        let last = match side {
            Side::Left => self.last_left.clone(),
            Side::Right => self.last_right.clone(),
        };
        let wanted =
            if self.side_of(&last) == side { Some(last) } else { self.rail_order[side.index()].first().cloned() };
        if let Some(panel) = wanted {
            self.show(panel);
        }
    }

    pub fn request_move(&mut self, panel: Panel, side: Side, before: Option<String>) {
        self.rail_move = Some((panel, side, before));
    }

    fn move_panel(&mut self, available: &[(String, Side)], panel: &Panel, side: Side, before: Option<&str>) {
        let key = panel.key();
        let open = Side::BOTH.into_iter().find_map(|from| self.open(from).filter(|shown| shown.key() == key).cloned());
        if let Some(open) = &open {
            self.close_panel(open);
        }
        self.rails.place(available, &key, side, before);
        if let Some(open) = open {
            self.show(open);
        }
    }

    pub fn restore(&mut self, side: Side, open: bool, last: &str) {
        let own = [
            Panel::Files,
            Panel::Search,
            Panel::Changes,
            Panel::Extensions,
            Panel::Problems,
            Panel::Symbols,
            Panel::Deps,
        ];
        let available: Vec<(String, Side)> = own.iter().map(|panel| (panel.key(), panel.home())).collect();
        let first = self.rails.arrange(&available)[side.index()].first().and_then(|key| Panel::from_key(key));
        let saved = Panel::from_key(last).filter(|panel| !panel.is_page() && self.side_of(panel) == side);
        match saved.or(first) {
            Some(panel) if open => self.show(panel),
            Some(panel) => {
                self.close(side);
                self.remember(side, panel);
            }
            None => self.close(side),
        }
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
    FrameSelection,
    CollapseAll,
    ExpandAll,
    ShowAll,
    Search(String),
    FormatFile(Option<String>),
    Show(Panel),
    RailsChanged,
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
    SetDiff(crate::page::DiffView),
    InstallVsix,
    SetTheme(&'static str),
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
const GAP: i8 = 6;

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

fn card_panel(ui: &mut egui::Ui, panel: egui::Panel, add: impl FnOnce(&mut egui::Ui)) {
    panel.frame(Frame::NONE.fill(pal().chrome).inner_margin(Margin { left: 0, right: 0, top: 0, bottom: GAP })).show(
        ui,
        |ui| {
            Frame::NONE.fill(pal().card).corner_radius(CornerRadius::same(10)).show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.set_width(ui.available_width());
                add(ui);
            });
        },
    );
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
    st.page_places.clear();
    st.file_icons.set(ext.icon_theme(&st.theme.icons).map(|t| t.file.as_path()));
    header(ui, st, actions);
    status_bar(ui, st, ext, pages.get(active), info, actions);
    let entries = rail_entries(st, ext);
    let available: Vec<(String, Side)> = entries.iter().map(|entry| (entry.panel.key(), entry.panel.home())).collect();
    let order = st.rails.arrange(&available);
    st.rail_order = order.clone().map(|keys| keys.iter().filter_map(|key| Panel::from_key(key)).collect());
    if let Some(drag) = st.rail_drag.as_mut() {
        drag.target = None;
    }
    for side in Side::BOTH {
        rail(ui, st, ext, pages.get(active), side, &entries, &order[side.index()]);
    }
    finish_rail_drag(ui, st, ext, &entries);
    if let Some((panel, side, before)) = st.rail_move.take() {
        st.move_panel(&available, &panel, side, before.as_deref());
        actions.push(Action::RailsChanged);
    }

    for side in Side::BOTH {
        side_panel(ui, st, pages, active, ext, actions, side);
    }

    let canvas = ui.available_rect_before_wrap();
    let ctx = ui.ctx().clone();
    center_message(ui, canvas, pages.get(active), &st.keys);
    toolbar(&ctx, st, canvas, info, actions);
    find_bar(&ctx, st, canvas, actions);
    completion_list(&ctx, st, actions);
    ext_panels::notices(&ctx, st, canvas, ext);
    settings::settings_modal(&ctx, st, ext, actions);
    quick_open(&ctx, st, canvas, pages.get(active), ext, actions);
    tooltip(&ctx, canvas, info);
    canvas
}

fn header(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    egui::Panel::top("header").frame(Frame::NONE.fill(pal().chrome)).exact_size(HEADER_H).show(ui, |ui| {
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
            let (slot, _) = ui.allocate_exact_size(Vec2::new(RAIL_W, HEADER_H), Sense::hover());
            let at = Rect::from_center_size(slot.center(), Vec2::splat(24.0));
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(logo.id(), at, uv, Color32::WHITE);
            ui.add_space(2.0);
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

fn status_bar(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &Extensions,
    page: Option<&Page>,
    info: &FrameInfo,
    actions: &mut Vec<Action>,
) {
    let mut panel_to_show = None;
    let (mut update_now, mut settings, mut save) = (false, false, false);
    egui::Panel::bottom("status").frame(Frame::NONE.fill(pal().chrome).inner_margin(Margin::symmetric(12, 0))).exact_size(24.0).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                if let Some(git) = page.and_then(|p| p.git.as_ref()) {
                    let icon = crate::codicons::glyph("source-control").unwrap_or(icons::GIT_BRANCH);
                    let mut text = format!("{icon} {}{}", git.branch, if git.changes.is_empty() { "" } else { "*" });
                    if git.ahead > 0 {
                        text.push_str(&format!(" ↑{}", git.ahead));
                    }
                    if git.behind > 0 {
                        text.push_str(&format!(" ↓{}", git.behind));
                    }
                    let label = ui.add(egui::Label::new(RichText::new(text).size(12.0).color(pal().text)).sense(Sense::click()));
                    if label.on_hover_text("Source control").clicked() {
                        panel_to_show = Some(Panel::Changes);
                    }
                    ui.add_space(10.0);
                }
                let unsaved = page.map_or(0, |p| p.unsaved().count());
                if unsaved > 0 {
                    let (dot, _) = ui.allocate_exact_size(Vec2::new(8.0, 12.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.2, pal().text);
                    let text = format!("{unsaved} not saved");
                    let label =
                        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(pal().text)).sense(Sense::click()));
                    let tip = st.keys.hint("Typed but not written to disk yet: click to save", Command::Save);
                    save = label.on_hover_text(tip).clicked();
                    ui.add_space(10.0);
                }
                ui.label(RichText::new(&info.status).size(12.0).color(pal().text_dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let version = RichText::new(format!("v{}", crate::update::current())).size(12.0).color(pal().text_faint);
                    let label = ui.add(egui::Label::new(version).sense(Sense::click()));
                    if label.on_hover_text("Code Canvas version: click for Settings, where updates are").clicked() {
                        settings = true;
                    }
                    if let crate::update::State::Available(new) = &st.update {
                        let glyph = crate::codicons::glyph("cloud-download").unwrap_or(icons::DOWNLOAD);
                        let offer = RichText::new(format!("{glyph} Update to {new}")).size(12.0).color(pal().accent);
                        let label = ui.add(egui::Label::new(offer).sense(Sense::click()));
                        if label.on_hover_text("Download the new version and restart into it").clicked() {
                            update_now = true;
                        }
                        ui.add_space(6.0);
                    } else if let crate::update::State::Installing(new) = &st.update {
                        ui.label(RichText::new(format!("Downloading {new}…")).size(12.0).color(pal().accent));
                        ui.add_space(6.0);
                    }
                    ui.add_space(6.0);
                    let st = info.stats;
                    let perf = format!(
                        "{}  ·  {} draws  ·  {:.2} ms  ·  {:.3}x",
                        st.lod.map(|l| l.name()).unwrap_or("-"),
                        st.draw_calls,
                        info.frame_ms,
                        info.zoom
                    );
                    ui.label(RichText::new(perf).size(12.0).color(pal().text_faint)).on_hover_text(format!(
                        "level of detail: {}\ncode instances: {}\nboxes: {}\ntoken chunks cached: {}\ntext chunks cached: {}\nCPU time per frame: {:.2} ms",
                        st.lod.map(|l| l.name()).unwrap_or("-"),
                        st.code_instances,
                        st.frame_instances,
                        st.token_chunks,
                        st.cached_chunks,
                        info.frame_ms
                    ));
                    let (errors, warnings) = ext.totals();
                    if errors + warnings > 0 {
                        ui.add_space(8.0);
                        let text = format!("{}  {errors}   {}  {warnings}", icons::CIRCLE_ALERT, icons::TRIANGLE_ALERT);
                        let color = if errors > 0 { c(theme::get().problem_error) } else { c(theme::get().problem_warning) };
                        let label = ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).sense(Sense::click()));
                        if label.on_hover_text("Problems reported by extensions: click to list them").clicked() {
                            panel_to_show = Some(Panel::Problems);
                        }
                    }
                    for (_, text) in &ext.status_items {
                        ui.add_space(8.0);
                        ui.label(RichText::new(text).size(12.0).color(pal().text_dim));
                    }
                });
            });
        },
    );
    if let Some(panel) = panel_to_show {
        st.show(panel);
    }
    if settings {
        st.settings.show(Some(SettingsTab::General));
    }
    if update_now {
        actions.push(Action::InstallUpdate);
    }
    if save {
        actions.push(Action::SaveFile);
    }
}

fn rail_frame() -> Frame {
    Frame::NONE.fill(pal().chrome).inner_margin(Margin { left: 5, right: 5, top: 0, bottom: GAP })
}

const RAIL_ICON: Vec2 = Vec2::splat(34.0);
const RAIL_REACH: f32 = 56.0;

fn side_panel(
    ui: &mut egui::Ui,
    st: &mut UiState,
    pages: &mut [Page],
    active: usize,
    ext: &mut Extensions,
    actions: &mut Vec<Action>,
    side: Side,
) {
    let Some(panel) = st.open(side).cloned() else { return };
    let frame = match side {
        Side::Left => egui::Panel::left("left").resizable(true).default_size(300.0).size_range(240.0..=560.0),
        Side::Right => egui::Panel::right("right").resizable(true).default_size(340.0).size_range(260.0..=600.0),
    };
    card_panel(ui, frame, |ui| match &panel {
        Panel::Files => files_panel(ui, st, pages, active, actions),
        Panel::Search => search_panel(ui, st, pages.get(active), actions),
        Panel::Extensions => ext_panels::extensions_panel(ui, st, ext, actions),
        Panel::Problems => ext_panels::problems_panel(ui, pages.get(active), ext, actions),
        Panel::Changes => git_panel::changes_panel(ui, &mut st.file_icons, pages.get_mut(active), actions),
        Panel::Extension => ext_panels::extension_view(ui, st, ext),
        Panel::Help => help_panel(ui, &st.keys),
        Panel::Symbols => symbols_panel(ui, st, pages.get(active), actions),
        Panel::Deps => deps_panel::deps_panel(ui, st, pages.get(active), actions),
        Panel::Webview(view) => ext_panels::webview_panel(ui, st, ext, view),
        Panel::Media => ext_panels::media_panel(ui, st, actions),
    });
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
        own(Panel::Problems, icons::CIRCLE_ALERT, "Problems reported by extensions".to_string()),
        own(Panel::Symbols, icons::LIST_TREE, "Symbols: selection, filters, names".to_string()),
        own(Panel::Deps, icons::WAYPOINTS, "Dependencies: which file imports which".to_string()),
    ];
    if st.media.is_some() {
        entries.push(own(Panel::Media, icons::IMAGE, "The image, video or PDF you last looked at".to_string()));
    }
    for item in ext.rail_items() {
        entries.push(RailEntry {
            panel: Panel::Webview(item.view),
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
    active: bool,
    hovered: bool,
) {
    if active {
        ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().raised);
    } else if hovered {
        ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().hover);
    }
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

fn rail(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &Extensions,
    page: Option<&Page>,
    side: Side,
    entries: &[RailEntry],
    order: &[String],
) {
    let bar = match side {
        Side::Left => egui::Panel::left("rail_left"),
        Side::Right => egui::Panel::right("rail_right"),
    };
    bar.frame(rail_frame()).exact_size(RAIL_W).resizable(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let zone = ui.max_rect();
        let mut slots: Vec<(String, Rect)> = Vec::new();
        for entry in order.iter().filter_map(|key| entries.iter().find(|entry| entry.panel.key() == *key)) {
            let key = entry.panel.key();
            let (rect, resp) = ui.allocate_exact_size(RAIL_ICON, Sense::click_and_drag());
            slots.push((key.clone(), rect));
            let lifted = st.rail_drag.as_ref().is_some_and(|drag| drag.key == key);
            if !lifted {
                let active = st.is_open(&entry.panel);
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
                if ui.button(offer).clicked() {
                    st.rail_move = Some((entry.panel.clone(), other, None));
                    ui.close();
                }
            });
            if resp.on_hover_text(&entry.tip).clicked() {
                st.toggle(entry.panel.clone());
            }
        }
        rail_drop_target(ui, st, side, zone, &slots);
        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            let (glyph, tip) = match side {
                Side::Left => {
                    (icons::PANEL_LEFT, st.keys.hint("Show or hide the left panel", Command::ToggleLeftPanel))
                }
                Side::Right => (icons::PANEL_RIGHT, "Show or hide the right panel".to_string()),
            };
            if icon_button(ui, glyph, RAIL_ICON, false, pal().hover).on_hover_text(tip).clicked() {
                st.toggle_side(side);
            }
            if side == Side::Right {
                return;
            }
            let tip = st.keys.hint("Settings: version, theme, colours, shortcuts", Command::Settings);
            if icon_button(ui, icons::SETTINGS, RAIL_ICON, st.settings.open, pal().hover).on_hover_text(tip).clicked() {
                st.settings.toggle();
            }
            let help = st.is_open(&Panel::Help);
            let tip = "Keyboard and mouse shortcuts";
            if icon_button(ui, icons::CIRCLE_HELP, RAIL_ICON, help, pal().hover).on_hover_text(tip).clicked() {
                st.toggle(Panel::Help);
            }
        });
    });
}

fn rail_drop_target(ui: &egui::Ui, st: &mut UiState, side: Side, zone: Rect, slots: &[(String, Rect)]) {
    let Some(drag) = st.rail_drag.as_mut() else { return };
    let reach = match side {
        Side::Left => Rect::from_min_max(zone.min, zone.max + Vec2::new(RAIL_REACH, 0.0)),
        Side::Right => Rect::from_min_max(zone.min - Vec2::new(RAIL_REACH, 0.0), zone.max),
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
            paint_rail_icon(ui, st, ext, entry, rect, true, false);
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
    Frame::NONE.inner_margin(Margin { left: 6, right: 8, top: 8, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
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
    git_panel::timeline(ui, pages.get(active));
    explorer(ui, st, pages.get_mut(active), actions);
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
    let PageState::Ready(r) = &mut page.state else {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Scanning…").color(pal().text_dim)));
        return;
    };
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
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, rows, |ui, range| {
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
                let gx = rect.left() + 22.0 + d as f32 * 16.0;
                painter.vline(gx, rect.y_range(), Stroke::new(1.0, pal().raised));
            }
            let x0 = rect.left() + 14.0 + depth as f32 * 16.0;
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
            }
        }
    });
}

fn completion_list(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>) {
    const ROWS: usize = 9;
    let Some(view) = &st.completion else { return };
    let first = view.selected.saturating_sub(ROWS - 1).min(view.items.len().saturating_sub(ROWS));
    let shown = &view.items[first..view.items.len().min(first + ROWS)];
    let file_icons = &mut st.file_icons;
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
                    for (i, (name, is_dir)) in shown.iter().enumerate() {
                        let index = first + i;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(260.0, 24.0), Sense::click());
                        if index == view.selected {
                            ui.painter().rect_filled(rect, 6.0, pal().raised);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, pal().row);
                        }
                        let mid = rect.center().y;
                        let icon_at = Rect::from_center_size(Pos2::new(rect.left() + 14.0, mid), Vec2::splat(16.0));
                        if !file_icons.paint(ui, name, *is_dir, false, icon_at, false) {
                            let glyph = if *is_dir { icons::FOLDER } else { file_glyph(name) };
                            let font = FontId::proportional(13.0);
                            ui.painter().text(icon_at.center(), Align2::CENTER_CENTER, glyph, font, pal().text_dim);
                        }
                        let label = if *is_dir { format!("{name}/") } else { name.clone() };
                        let (from, font) = (rect.left() + 30.0, FontId::proportional(13.0));
                        text_fit(
                            ui.painter(),
                            Pos2::new(from, mid),
                            &label,
                            font,
                            pal().text,
                            rect.right() - 8.0 - from,
                        );
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

fn naming_box(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
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
            let place = format!("{}:{}", f.name, hit.line + 1);
            let galley = painter.layout_no_wrap(place, FontId::proportional(12.0), pal().text_dim);
            let pw = galley.size().x;
            painter.galley(Pos2::new(rect.left() + 14.0, mid - galley.size().y * 0.5), galley, pal().text_dim);
            let x = rect.left() + 22.0 + pw;
            let clip = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(rect.right() - 8.0, rect.bottom()));
            let line = f.line(hit.line as usize).trim_start();
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
                ("Zoom", format!("Mouse wheel, {} / {}", keys.label(Command::ZoomIn), keys.label(Command::ZoomOut))),
                ("Pan", fixed("Drag, arrow keys")),
                ("Fly to a file or folder", fixed("Double-click")),
                ("Show everything", key(Command::ShowAll)),
                ("Go to the selection", key(Command::GoToSelection)),
                ("Zoom out to the parent folder", key(Command::ZoomToParent)),
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
                ("Edit: a click puts the caret (the pencil)", key(Command::EditTool)),
                ("Select code", fixed("Drag over it with the pencil")),
                ("Save", key(Command::Save)),
                ("Stop typing in a file", fixed("Esc")),
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
                ui.label(RichText::new(format!("{n} connected files shown")).size(11.0).color(c(theme::get().ui_warn)));
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
                        "{} symbols indexed by name. Results are approximate until a language server is connected.",
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
                        Some(cn) => format!("{}:{}  ·  {cn}", file.rel_path, sym.line + 1),
                        None => format!("{}:{}", file.rel_path, sym.line + 1),
                    };
                    if ui
                        .link(RichText::new(place).monospace().size(11.0))
                        .on_hover_text("Go to the definition")
                        .clicked()
                    {
                        actions.push(Action::FlyToSite(def));
                    }
                    ui.add_space(3.0);
                    ui.label(RichText::new(repo.sym_signature(sel.sym)).monospace().size(12.0).color(pal().text));
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
                                let text = format!("{}:{}  {who}", f.name, site.line + 1);
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
                    let mut note = format!("searched in {:.0} ms", sel.rel.search_ms);
                    if sel.rel.truncated {
                        note.push_str(" · results truncated");
                    }
                    ui.label(RichText::new(note).size(10.0).color(pal().text_faint));
                });
        }
        ui.add_space(8.0);
    });
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
                        let tools = [
                            (icons::HAND, false, Command::MoveTool, "Move: drag to travel, click a name to see where it is used. Nothing gets edited"),
                            (icons::PENCIL, true, Command::EditTool, "Edit: click in the code to put the caret there and type"),
                        ];
                        for (glyph, edits, command, tip) in tools {
                            let button = icon_button(ui, glyph, b, edit == edits, pal().hover);
                            if edit && edits {
                                let at = button.rect.center();
                                ui.painter().text(at, Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), pal().accent);
                            }
                            if button.on_hover_text(st.keys.hint(tip, command)).clicked() && edit != edits {
                                actions.push(Action::SetPrefs(crate::prefs::Prefs { edit: edits, ..st.prefs.clone() }));
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
                        ui.label(
                            RichText::new(format!("{:>4.0}%", info.zoom * 100.0))
                                .monospace()
                                .size(11.0)
                                .color(pal().text_dim),
                        );
                        if icon_button(ui, icons::PLUS, b, false, pal().hover).on_hover_text("Zoom in").clicked() {
                            actions.push(Action::ZoomBy(1.4));
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

fn palette_commands(ext: &Extensions) -> Vec<(String, Action)> {
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
        (
            "Source Control: Show What Changed Inside the Files".into(),
            Action::SetDiff(crate::page::DiffView { on: true, ..Default::default() }),
        ),
        ("Source Control: Show the Files as They Are".into(), Action::SetDiff(Default::default())),
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
                        let mut rows: Vec<(i32, String, Action)> = palette_commands(ext)
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
    fn hover_markdown_becomes_a_tooltip() {
        let md = "```typescript\nfunction add(a: number, b: number): number\n```\n\nAdds two numbers.\n\n*@param* `a`";
        let (title, rest) = super::hover_text(md);
        assert_eq!(title, "function add(a: number, b: number): number");
        assert_eq!(rest, "Adds two numbers.\n*@param* `a`");
        assert_eq!(super::hover_text(""), (String::new(), String::new()));
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
