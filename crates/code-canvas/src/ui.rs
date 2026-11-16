//! The application chrome, built with egui around the canvas.
//!
//! The window has no system title bar: the header is ours (panel toggles,
//! rescan, folder tabs, the "go to file" box, window buttons). An icon rail
//! on each side opens one panel at a time, like an activity bar:
//!
//! * left: files, search in files, extensions, problems, shortcuts
//! * right: symbols (selection, filters, names), Claude Code chat
//!
//! The UI never mutates pages directly; it emits [`Action`]s that the app
//! applies after the egui pass.

use std::sync::atomic::Ordering;

use canvas_core::{RelKind, Site};
use canvas_render::{theme, FrameStats};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use crate::ext::Extensions;
use crate::icons;
use crate::page::{build_tree, Page, PageState, RelFilters};

mod ext_panels;
mod git_panel;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeftPanel {
    Files,
    Search,
    /// Find, install and manage VS Code extensions.
    Extensions,
    /// What the extensions report: errors and warnings.
    Problems,
    /// Source control: what git says changed.
    Changes,
    /// One installed extension (`UiState::extension`): what it offers.
    Extension,
    Help,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightPanel {
    Symbols,
    Claude,
    /// An extension's webview (`UiState::webview`).
    Webview,
}

/// Is the `claude` command-line tool usable?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeStatus {
    Checking,
    /// Its `--version` output.
    Ready(String),
    Missing(String),
}

/// The two views of the files panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilesTab {
    /// The tree of the active folder.
    Files,
    /// Every opened folder (each one is its own canvas).
    Folders,
}

pub struct QuickOpen {
    pub query: String,
    pub selected: usize,
    pub focus: bool,
}

pub struct UiState {
    /// The open left panel, if any (the rail icon toggles it).
    pub left: Option<LeftPanel>,
    pub right: Option<RightPanel>,
    /// What the rail reopens when a closed side is toggled from the header.
    pub last_left: LeftPanel,
    pub last_right: RightPanel,
    pub files_tab: FilesTab,
    pub explorer_filter: String,
    pub search_query: String,
    pub quick: Option<QuickOpen>,
    pub filters: RelFilters,
    /// Interactive floating areas over the canvas, in points.
    pub floating: Vec<Rect>,
    pub claude: ClaudeStatus,
    pub maximized: bool,
    /// The app icon, shown in the top-left corner of the header.
    logo: Option<egui::TextureHandle>,
    /// Key of the webview shown by [`RightPanel::Webview`].
    pub webview: String,
    /// Where that webview's page goes this frame, in points.
    pub webview_rect: Option<Rect>,
    /// Id of the extension shown by [`LeftPanel::Extension`].
    pub extension: String,
    /// Icons of installed extensions, by extension id (`None`: it has none).
    ext_icons: std::collections::HashMap<String, Option<egui::TextureHandle>>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            left: Some(LeftPanel::Files),
            right: Some(RightPanel::Symbols),
            last_left: LeftPanel::Files,
            last_right: RightPanel::Symbols,
            files_tab: FilesTab::Files,
            explorer_filter: String::new(),
            search_query: String::new(),
            quick: None,
            filters: RelFilters::default(),
            floating: Vec::new(),
            claude: ClaudeStatus::Checking,
            maximized: false,
            logo: None,
            extension: String::new(),
            webview: String::new(),
            webview_rect: None,
            ext_icons: Default::default(),
        }
    }
}

impl UiState {
    pub fn toggle_left(&mut self) {
        self.left = match self.left {
            Some(_) => None,
            None => Some(self.last_left),
        };
    }

    pub fn toggle_right(&mut self) {
        self.right = match self.right {
            Some(_) => None,
            None => Some(self.last_right),
        };
    }

    pub fn show_left(&mut self, p: LeftPanel) {
        self.left = Some(p);
        self.last_left = p;
    }

    pub fn show_right(&mut self, p: RightPanel) {
        self.right = Some(p);
        self.last_right = p;
    }
}

pub enum Action {
    SwitchTab(usize),
    CloseTab(usize),
    OpenFolder,
    Rescan,
    /// Select a file or folder and fly the camera to it.
    FlyToPath(String),
    FlyToSite(Site),
    ToggleHidden(String),
    ToggleCollapsed(String),
    ToggleTreeOpen(String),
    /// The explorer row under the pointer.
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
    /// Search the text of every file of the active page.
    Search(String),
    /// Format a file of the active page with the installed formatter:
    /// the given path, else the file under the pointer or the selected one.
    FormatFile(Option<String>),
    /// Ask git again what changed in the active page.
    RefreshGit,
    /// Show only the files git reports as changed.
    SetOnlyChanged(bool),
    /// Pick a `.vsix` file and install it.
    InstallVsix,
    /// Send the chat input of the active page to Claude Code.
    ChatSend,
    ChatStop,
    ChatNew,
    // The window itself (it has no system title bar).
    DragWindow,
    ToggleMaximize,
    Minimize,
    CloseWindow,
}

/// Per-frame facts the UI displays but does not own.
pub struct FrameInfo<'a> {
    pub stats: &'a FrameStats,
    pub frame_ms: f32,
    pub zoom: f32,
    /// Title and detail for the tooltip next to the pointer, when the
    /// pointer is over something on the canvas.
    pub tooltip: Option<(String, String)>,
    pub status: String,
}

fn c(rgba: u32) -> Color32 {
    let [r, g, b, a] = theme::unpack(rgba);
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

// Design tokens.
const CHROME: Color32 = Color32::from_rgb(10, 11, 14);
const CARD: Color32 = Color32::from_rgb(16, 17, 21);
const WELL: Color32 = Color32::from_rgb(11, 12, 15);
const RAISED: Color32 = Color32::from_rgb(27, 29, 36);
const HOVER: Color32 = Color32::from_rgb(34, 37, 46);
const BORDER: Color32 = Color32::from_rgb(34, 37, 46);
const TEXT: Color32 = Color32::from_rgb(230, 231, 236);
const TEXT_DIM: Color32 = Color32::from_rgb(138, 142, 153);
const TEXT_FAINT: Color32 = Color32::from_rgb(92, 96, 108);
const ACCENT: Color32 = Color32::from_rgb(90, 150, 255);
const ROW_H: f32 = 24.0;
const HEADER_H: f32 = 40.0;
const RAIL_W: f32 = 44.0;
const GAP: i8 = 6;

/// Fonts and the app's dark look (call once).
pub fn style(ctx: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    // Lucide icons as a fallback font: icon code points render anywhere text does.
    ctx.add_font(FontInsert {
        name: "lucide".into(),
        data: egui::FontData::from_static(include_bytes!("../assets/lucide.ttf")),
        families: vec![
            InsertFontFamily { family: egui::FontFamily::Proportional, priority: FontPriority::Lowest },
            InsertFontFamily { family: egui::FontFamily::Monospace, priority: FontPriority::Lowest },
        ],
    });

    let mut v = egui::Visuals::dark();
    v.panel_fill = CHROME;
    v.window_fill = CARD;
    v.extreme_bg_color = WELL;
    v.faint_bg_color = RAISED;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = CornerRadius::same(6);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(58, 63, 78));
    v.widgets.hovered.corner_radius = CornerRadius::same(6);
    v.widgets.active.bg_fill = Color32::from_rgb(44, 49, 62);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(44, 49, 62);
    v.widgets.active.corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = Color32::from_rgb(52, 64, 88);
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.hyperlink_color = ACCENT;
    // Always dark, whatever the system theme is.
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(6.0, 5.0);
        s.spacing.button_padding = Vec2::new(8.0, 4.0);
        s.spacing.slider_rail_height = 5.0;
        s.interaction.tooltip_delay = 0.4;
    });
}

fn icon(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph).size(size).color(color)
}

/// A square icon button drawn by hand (rail, header, window buttons).
fn icon_button(ui: &mut egui::Ui, glyph: &str, size: Vec2, active: bool, hover_fill: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let inner = rect.shrink2(Vec2::new(4.0, 4.0).min(size * 0.15));
        if active {
            ui.painter().rect_filled(inner, 8.0, RAISED);
        } else if resp.hovered() {
            ui.painter().rect_filled(inner, 8.0, hover_fill);
        }
        let color = if active || resp.hovered() { TEXT } else { TEXT_DIM };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), color);
    }
    resp
}

/// A rounded checkbox with a check mark, like the concept's.
fn check(ui: &mut egui::Ui, on: &mut bool, label: &str, enabled: bool, dim: bool) -> egui::Response {
    let font = FontId::proportional(if dim { 12.0 } else { 13.0 });
    let text_color = if !enabled {
        TEXT_FAINT
    } else if dim {
        TEXT_DIM
    } else {
        TEXT
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
        let fill = if resp.hovered() && enabled { HOVER } else { RAISED };
        ui.painter().rect_filled(boxr, 4.0, fill);
        if *on {
            let color = if enabled { TEXT } else { TEXT_FAINT };
            ui.painter().text(boxr.center(), Align2::CENTER_CENTER, icons::CHECK, FontId::proportional(12.0), color);
        }
        ui.painter().galley(Pos2::new(rect.left() + 24.0, rect.center().y - galley.size().y * 0.5), galley, text_color);
    }
    resp
}

/// A section title with an icon, e.g. "Filters".
fn section(ui: &mut egui::Ui, glyph: &str, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(icon(glyph, 14.0, TEXT_DIM));
        ui.label(RichText::new(title).size(13.5).color(TEXT));
    });
    ui.add_space(2.0);
}

/// A recessed rounded area inside a panel.
fn well(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    Frame::NONE.fill(WELL).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// A side panel: a rounded card with a gap around it.
fn card_panel(ui: &mut egui::Ui, panel: egui::Panel, add: impl FnOnce(&mut egui::Ui)) {
    panel.frame(Frame::NONE.fill(CHROME).inner_margin(Margin { left: 0, right: 0, top: 0, bottom: GAP })).show(
        ui,
        |ui| {
            Frame::NONE.fill(CARD).corner_radius(CornerRadius::same(10)).show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.set_width(ui.available_width());
                add(ui);
            });
        },
    );
}

/// Build the whole chrome. Returns the canvas rectangle in points.
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
    st.webview_rect = None;
    header(ui, st, pages, active, actions);
    status_bar(ui, st, ext, info);
    left_rail(ui, st, ext, pages.get(active));
    right_rail(ui, st, ext);

    if let Some(panel) = st.left {
        let p = egui::Panel::left("left").resizable(true).default_size(300.0).size_range(220.0..=560.0);
        card_panel(ui, p, |ui| match panel {
            LeftPanel::Files => files_panel(ui, st, pages, active, actions),
            LeftPanel::Search => search_panel(ui, st, pages.get(active), actions),
            LeftPanel::Extensions => ext_panels::extensions_panel(ui, st, ext, actions),
            LeftPanel::Problems => ext_panels::problems_panel(ui, pages.get(active), ext, actions),
            LeftPanel::Changes => git_panel::changes_panel(ui, pages.get(active), actions),
            LeftPanel::Extension => ext_panels::extension_view(ui, st, ext),
            LeftPanel::Help => help_panel(ui),
        });
    }
    if let Some(panel) = st.right {
        let p = egui::Panel::right("right").resizable(true).default_size(340.0).size_range(260.0..=600.0);
        card_panel(ui, p, |ui| match panel {
            RightPanel::Symbols => symbols_panel(ui, st, pages.get(active), actions),
            RightPanel::Claude => claude_panel(ui, st, pages.get_mut(active), actions),
            RightPanel::Webview => ext_panels::webview_panel(ui, st, ext),
        });
    }

    let canvas = ui.available_rect_before_wrap();
    let ctx = ui.ctx().clone();
    center_message(ui, canvas, pages.get(active));
    toolbar(&ctx, st, canvas, info, actions);
    ext_panels::notices(&ctx, st, canvas, ext);
    quick_open(&ctx, st, canvas, pages.get(active), actions);
    tooltip(&ctx, canvas, info);
    canvas
}

fn header(ui: &mut egui::Ui, st: &mut UiState, pages: &[Page], active: usize, actions: &mut Vec<Action>) {
    egui::Panel::top("header").frame(Frame::NONE.fill(CHROME)).exact_size(HEADER_H).show(ui, |ui| {
        let bar = ui.max_rect();
        // The empty parts of the header move the window. Added first, so
        // every button placed afterwards wins over it.
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
            // The app icon sits in the corner, above the left rail.
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
                egui::Button::new(RichText::new(format!("{}  Rescan", icons::REFRESH_CW)).size(13.0).color(TEXT_DIM))
                    .frame(false),
            );
            if rescan.on_hover_text("Reload the folder from disk (F5)").clicked() {
                actions.push(Action::Rescan);
            }
            ui.add_space(8.0);

            // The active folder, as a switcher. Its width does not depend on
            // how many folders are open; the full list lives in the files panel.
            let title = match pages.get(active) {
                Some(p) if p.is_loading() => format!("{} …", p.title),
                Some(p) => p.title.clone(),
                None => "No folder".into(),
            };
            let count = if pages.len() > 1 { format!("  {}/{}", active + 1, pages.len()) } else { String::new() };
            let label = format!("{}  {title}{count}  {}", icons::FOLDER, icons::CHEVRON_DOWN);
            ui.menu_button(RichText::new(label).size(13.0).color(TEXT), |ui| {
                ui.set_min_width(260.0);
                for (i, p) in pages.iter().enumerate() {
                    let text = RichText::new(format!("{}  {}", icons::FOLDER, p.title)).size(13.0);
                    let row = ui.selectable_label(i == active, text).on_hover_text(p.root.display().to_string());
                    if row.clicked() {
                        actions.push(Action::SwitchTab(i));
                        ui.close();
                    }
                }
                if !pages.is_empty() {
                    ui.separator();
                }
                if ui.button(format!("{}  Open folder…   Ctrl+O", icons::PLUS)).clicked() {
                    actions.push(Action::OpenFolder);
                    ui.close();
                }
                if pages.len() > 1 && ui.button(format!("{}  All folders…", icons::FOLDER_TREE)).clicked() {
                    st.files_tab = FilesTab::Folders;
                    st.show_left(LeftPanel::Files);
                    ui.close();
                }
            })
            .response
            .on_hover_text("Switch folder (Ctrl+Tab)");
            left_end = ui.min_rect().right();

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let wb = Vec2::new(46.0, HEADER_H);
                let (rect, resp) = ui.allocate_exact_size(wb, Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(196, 43, 28));
                }
                let col = if resp.hovered() { Color32::WHITE } else { TEXT_DIM };
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
                        ui.painter().rect_filled(rect, 0.0, HOVER);
                    }
                    let col = if resp.hovered() { TEXT } else { TEXT_DIM };
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), col);
                    if resp.on_hover_text(tip).clicked() {
                        actions.push(if action == 1 { Action::ToggleMaximize } else { Action::Minimize });
                    }
                }
                ui.add_space(6.0);
                right_start = ui.min_rect().left();
            });
        });

        // The "go to file" box, centred in the window when there is room.
        let room = (right_start - left_end - 24.0).max(0.0);
        let w = room.min(440.0);
        if w >= 150.0 {
            let center = bar.center().x.clamp(left_end + 12.0 + w * 0.5, right_start - 12.0 - w * 0.5);
            let rect = Rect::from_center_size(Pos2::new(center, bar.center().y), Vec2::new(w, 28.0));
            let resp = ui.interact(rect, egui::Id::unique("goto"), Sense::click());
            let fill = if resp.hovered() { RAISED } else { Color32::from_rgb(18, 19, 24) };
            ui.painter().rect(rect, 7.0, fill, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
            ui.painter().text(
                Pos2::new(rect.left() + 12.0, rect.center().y),
                Align2::LEFT_CENTER,
                icons::SEARCH,
                FontId::proportional(14.0),
                TEXT_DIM,
            );
            let hint = if w > 330.0 { "Go to a file or folder by name (Ctrl+P)" } else { "Go to file (Ctrl+P)" };
            ui.painter().text(
                Pos2::new(rect.left() + 34.0, rect.center().y),
                Align2::LEFT_CENTER,
                hint,
                FontId::proportional(13.0),
                TEXT_DIM,
            );
            if resp.clicked() {
                st.quick = Some(QuickOpen { query: String::new(), selected: 0, focus: true });
            }
        }
    });
}

fn status_bar(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions, info: &FrameInfo) {
    let mut panel_to_show = None;
    egui::Panel::bottom("status").frame(Frame::NONE.fill(CHROME).inner_margin(Margin::symmetric(12, 0))).exact_size(24.0).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new(&info.status).size(12.0).color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let st = info.stats;
                    let perf = format!(
                        "{}  ·  {} draws  ·  {:.2} ms  ·  {:.3}x",
                        st.lod.map(|l| l.name()).unwrap_or("-"),
                        st.draw_calls,
                        info.frame_ms,
                        info.zoom
                    );
                    ui.label(RichText::new(perf).size(12.0).color(TEXT_FAINT)).on_hover_text(format!(
                        "level of detail: {}\ncode instances: {}\nboxes: {}\ntoken chunks cached: {}\ntext chunks cached: {}\nCPU time per frame: {:.2} ms",
                        st.lod.map(|l| l.name()).unwrap_or("-"),
                        st.code_instances,
                        st.frame_instances,
                        st.token_chunks,
                        st.cached_chunks,
                        info.frame_ms
                    ));
                    // What extensions report: problems, and their status items.
                    let (errors, warnings) = ext.totals();
                    if errors + warnings > 0 {
                        ui.add_space(8.0);
                        let text = format!("{}  {errors}   {}  {warnings}", icons::CIRCLE_ALERT, icons::TRIANGLE_ALERT);
                        let color = if errors > 0 { c(theme::PROBLEM_ERROR) } else { c(theme::PROBLEM_WARNING) };
                        let label = ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).sense(Sense::click()));
                        if label.on_hover_text("Problems reported by extensions: click to list them").clicked() {
                            panel_to_show = Some(LeftPanel::Problems);
                        }
                    }
                    for (_, text) in &ext.status_items {
                        ui.add_space(8.0);
                        ui.label(RichText::new(text).size(12.0).color(TEXT_DIM));
                    }
                });
            });
        },
    );
    if let Some(panel) = panel_to_show {
        st.show_left(panel);
    }
}

fn rail_frame() -> Frame {
    Frame::NONE.fill(CHROME).inner_margin(Margin { left: 5, right: 5, top: 0, bottom: GAP })
}

fn left_rail(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions, page: Option<&Page>) {
    egui::Panel::left("rail_left").frame(rail_frame()).exact_size(RAIL_W).resizable(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let b = Vec2::splat(34.0);
        for (panel, glyph, tip) in [
            (LeftPanel::Files, icons::FOLDER, "Files (Ctrl+B)"),
            (LeftPanel::Search, icons::TEXT_SEARCH, "Search in files (Ctrl+Shift+F)"),
            (LeftPanel::Changes, icons::GIT_BRANCH, "Source control: what changed"),
            (LeftPanel::Extensions, icons::PUZZLE, "Extensions"),
            (LeftPanel::Problems, icons::CIRCLE_ALERT, "Problems reported by extensions"),
        ] {
            let resp = icon_button(ui, glyph, b, st.left == Some(panel), HOVER);
            if panel == LeftPanel::Problems {
                // A dot on the icon while there is something to look at.
                let (errors, warnings) = ext.totals();
                if errors + warnings > 0 {
                    let color = if errors > 0 { c(theme::PROBLEM_ERROR) } else { c(theme::PROBLEM_WARNING) };
                    ui.painter().circle_filled(resp.rect.right_top() + Vec2::new(-9.0, 9.0), 3.5, color);
                }
            }
            if panel == LeftPanel::Changes && page.and_then(|p| p.git.as_ref()).is_some_and(|g| !g.changes.is_empty()) {
                ui.painter().circle_filled(resp.rect.right_top() + Vec2::new(-9.0, 9.0), 3.5, c(theme::GIT_MODIFIED));
            }
            if resp.on_hover_text(tip).clicked() {
                if st.left == Some(panel) {
                    st.left = None;
                } else {
                    st.show_left(panel);
                }
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            if icon_button(ui, icons::PANEL_LEFT, b, false, HOVER)
                .on_hover_text("Show or hide the left panel (Ctrl+B)")
                .clicked()
            {
                st.toggle_left();
            }
            if icon_button(ui, icons::CIRCLE_HELP, b, st.left == Some(LeftPanel::Help), HOVER)
                .on_hover_text("Keyboard and mouse shortcuts")
                .clicked()
            {
                if st.left == Some(LeftPanel::Help) {
                    st.left = None;
                } else {
                    st.show_left(LeftPanel::Help);
                }
            }
        });
    });
}

fn right_rail(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions) {
    egui::Panel::right("rail_right").frame(rail_frame()).exact_size(RAIL_W).resizable(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let b = Vec2::splat(34.0);
        for (panel, glyph, tip) in [
            (RightPanel::Symbols, icons::LIST_TREE, "Symbols: selection, filters, names"),
            (RightPanel::Claude, icons::MESSAGE_CIRCLE, "Ask Claude Code about this repository"),
        ] {
            if icon_button(ui, glyph, b, st.right == Some(panel), HOVER).on_hover_text(tip).clicked() {
                if st.right == Some(panel) {
                    st.right = None;
                } else {
                    st.show_right(panel);
                }
            }
        }
        // Extensions' own interfaces (webviews): one icon per extension.
        let mut seen: Vec<&str> = Vec::new();
        for w in &ext.webviews {
            if seen.contains(&w.extension.as_str()) {
                continue;
            }
            seen.push(&w.extension);
            let active = st.right == Some(RightPanel::Webview) && st.webview == w.key;
            let (rect, resp) = ui.allocate_exact_size(b, Sense::click());
            if active {
                ui.painter().rect_filled(rect.shrink(4.0), 8.0, RAISED);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect.shrink(4.0), 8.0, HOVER);
            }
            let installed = ext.installed.iter().find(|e| e.id == w.extension);
            match installed.and_then(|e| ext_panels::extension_icon(ui.ctx(), &mut st.ext_icons, e)) {
                Some(texture) => {
                    let at = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
                    egui::Image::new(&texture).corner_radius(4).paint_at(ui, at);
                }
                None => {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        icons::PUZZLE,
                        FontId::proportional(17.0),
                        TEXT_DIM,
                    );
                }
            }
            if resp.on_hover_text(&w.title).clicked() {
                if active {
                    st.right = None;
                } else {
                    st.webview = w.key.clone();
                    st.right = Some(RightPanel::Webview);
                }
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            if icon_button(ui, icons::PANEL_RIGHT, b, false, HOVER)
                .on_hover_text("Show or hide the right panel")
                .clicked()
            {
                st.toggle_right();
            }
        });
    });
}

fn panel_title(ui: &mut egui::Ui, title: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(14.0).color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            right(ui);
        });
    });
}

fn small_icon(ui: &mut egui::Ui, glyph: &str, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, HOVER);
    }
    let color = if resp.hovered() { TEXT } else { TEXT_DIM };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), color);
    resp.on_hover_text(tip).clicked()
}

/// A text field with a leading icon, in a rounded well.
fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str, glyph: &str) -> egui::Response {
    Frame::NONE
        .fill(WELL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icon(glyph, 14.0, TEXT_DIM));
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

/// A text tab with an accent underline when active.
fn text_tab(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let color = if active { TEXT } else { TEXT_DIM };
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(14.0), color);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x, 26.0), Sense::click());
    let color = if resp.hovered() { TEXT } else { color };
    ui.painter().galley(Pos2::new(rect.left(), rect.top() + 2.0), galley, color);
    if active {
        ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, ACCENT));
    }
    resp.clicked()
}

fn files_panel(ui: &mut egui::Ui, st: &mut UiState, pages: &mut [Page], active: usize, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if text_tab(ui, "Files", st.files_tab == FilesTab::Files) {
                st.files_tab = FilesTab::Files;
            }
            if text_tab(ui, &format!("Folders ({})", pages.len()), st.files_tab == FilesTab::Folders) {
                st.files_tab = FilesTab::Folders;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if small_icon(ui, icons::PLUS, "Open a folder in a new canvas (Ctrl+O)") {
                    actions.push(Action::OpenFolder);
                }
                if st.files_tab == FilesTab::Files {
                    if small_icon(ui, icons::EYE, "Put every hidden file back on the canvas") {
                        actions.push(Action::ShowAll);
                    }
                    if small_icon(ui, icons::FOLD_VERTICAL, "Collapse every folder on the canvas") {
                        actions.push(Action::CollapseAll);
                    }
                    if small_icon(ui, icons::UNFOLD_VERTICAL, "Expand every folder on the canvas") {
                        actions.push(Action::ExpandAll);
                    }
                }
            });
        });
    });
    if st.files_tab == FilesTab::Files {
        git_panel::timeline(ui, pages.get(active));
    }
    match st.files_tab {
        FilesTab::Files => explorer(ui, st, pages.get_mut(active), actions),
        FilesTab::Folders => folders_list(ui, st, pages, active, actions),
    }
}

/// Every opened folder. Each one is its own canvas; this list is how you
/// move between them when there are more than a couple.
fn folders_list(ui: &mut egui::Ui, st: &mut UiState, pages: &[Page], active: usize, actions: &mut Vec<Action>) {
    ui.add_space(8.0);
    if pages.is_empty() {
        ui.vertical_centered(|ui| ui.label(RichText::new("No folder open").color(TEXT_DIM)));
    }
    ui.spacing_mut().item_spacing.y = 2.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, p) in pages.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            let bg = rect.shrink2(Vec2::new(6.0, 0.0));
            let is_active = i == active;
            if is_active {
                painter.rect_filled(bg, 8.0, RAISED);
                let bar = Rect::from_min_size(bg.left_top() + Vec2::new(0.0, 10.0), Vec2::new(3.0, bg.height() - 20.0));
                painter.rect_filled(bar, 2.0, ACCENT);
            } else if resp.hovered() {
                painter.rect_filled(bg, 8.0, Color32::from_rgb(20, 22, 27));
            }
            let close = Rect::from_center_size(Pos2::new(rect.right() - 22.0, rect.center().y), Vec2::splat(22.0));
            let over_close = resp.hover_pos().is_some_and(|pos| close.contains(pos));
            let icon_color = if is_active { TEXT } else { TEXT_DIM };
            painter.text(
                Pos2::new(rect.left() + 24.0, rect.center().y),
                Align2::CENTER_CENTER,
                icons::FOLDER,
                FontId::proportional(16.0),
                icon_color,
            );
            let clip = Rect::from_min_max(
                rect.left_top() + Vec2::new(42.0, 0.0),
                Pos2::new(rect.right() - 40.0, rect.bottom()),
            );
            let p2 = painter.with_clip_rect(clip);
            let name_color = if is_active { TEXT } else { Color32::from_rgb(196, 199, 208) };
            p2.text(
                Pos2::new(clip.left(), rect.top() + 14.0),
                Align2::LEFT_CENTER,
                &p.title,
                FontId::proportional(13.5),
                name_color,
            );
            let detail = match &p.state {
                PageState::Ready(r) => format!("{} files · {}", r.repo.files.len(), p.root.display()),
                PageState::Loading { .. } => format!("scanning… · {}", p.root.display()),
                PageState::Failed(e) => e.clone(),
            };
            p2.text(
                Pos2::new(clip.left(), rect.top() + 31.0),
                Align2::LEFT_CENTER,
                detail,
                FontId::proportional(11.0),
                TEXT_FAINT,
            );
            if resp.hovered() {
                if over_close {
                    painter.rect_filled(close, 6.0, HOVER);
                }
                let x = if over_close { TEXT } else { TEXT_DIM };
                painter.text(close.center(), Align2::CENTER_CENTER, icons::X, FontId::proportional(13.0), x);
            }
            if resp.clicked() {
                if over_close {
                    actions.push(Action::CloseTab(i));
                } else {
                    actions.push(Action::SwitchTab(i));
                    st.files_tab = FilesTab::Files;
                }
            }
            if resp.middle_clicked() {
                actions.push(Action::CloseTab(i));
            }
            resp.on_hover_text(p.root.display().to_string());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            if ui
                .button(format!("{}  Open folder…", icons::PLUS))
                .on_hover_text("Ctrl+O, or drop a folder on the window")
                .clicked()
            {
                actions.push(Action::OpenFolder);
            }
        });
    });
}

fn explorer(ui: &mut egui::Ui, st: &mut UiState, page: Option<&mut Page>, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 6 }).show(ui, |ui| {
        search_field(ui, &mut st.explorer_filter, "Filter by name, e.g. api types", icons::SEARCH);
    });

    let Some(page) = page else {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("No folder open").color(TEXT_DIM)));
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
        ui.vertical_centered(|ui| ui.label(RichText::new("Scanning…").color(TEXT_DIM)));
        return;
    };
    if r.tree_dirty {
        r.tree = build_tree(&r.repo, &page.tree_open, &page.filter.hidden);
        r.tree_dirty = false;
    }

    // With a filter: a flat list of matching files.
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
            ui.label(RichText::new(note).size(11.0).color(TEXT_DIM));
        });
    }

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, rows, |ui, range| {
        for i in range {
            let (depth, is_dir, open, hidden, locked, name, path, count) = match &matches {
                Some(m) => {
                    let f = &r.repo.files[m[i]];
                    let hidden = page.filter.hidden.contains(&f.rel_path);
                    (0u16, false, false, hidden, false, f.rel_path.as_str(), f.rel_path.as_str(), 0)
                }
                None => {
                    let row = &r.tree[i];
                    (
                        row.depth,
                        row.is_dir,
                        row.open,
                        row.hidden,
                        row.locked,
                        row.name.as_str(),
                        row.path.as_str(),
                        row.count,
                    )
                }
            };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            let row_bg = rect.shrink2(Vec2::new(6.0, 0.0));
            if has_selection && path == selected_path {
                painter.rect_filled(row_bg, 6.0, RAISED);
            } else if resp.hovered() {
                painter.rect_filled(row_bg, 6.0, Color32::from_rgb(20, 22, 27));
            }
            let mid = rect.center().y;
            // Indent guides.
            for d in 0..depth {
                let gx = rect.left() + 22.0 + d as f32 * 16.0;
                painter.vline(gx, rect.y_range(), Stroke::new(1.0, Color32::from_rgb(26, 28, 34)));
            }
            let x0 = rect.left() + 14.0 + depth as f32 * 16.0;
            if is_dir {
                let chevron = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
                painter.text(
                    Pos2::new(x0 + 8.0, mid),
                    Align2::CENTER_CENTER,
                    chevron,
                    FontId::proportional(13.0),
                    TEXT_DIM,
                );
            }
            // Checkbox: is it on the canvas?
            let on = !hidden && !locked;
            let cb = Rect::from_center_size(Pos2::new(x0 + 27.0, mid), Vec2::splat(15.0));
            painter.rect_filled(cb, 4.0, RAISED);
            if on {
                painter.text(cb.center(), Align2::CENTER_CENTER, icons::CHECK, FontId::proportional(11.0), TEXT);
            }
            let mut text_x = x0 + 42.0;
            let color = if !on {
                TEXT_FAINT
            } else if is_dir {
                TEXT
            } else {
                Color32::from_rgb(196, 199, 208)
            };
            if !is_dir {
                let glyph = file_glyph(name);
                let ic = if on { TEXT_DIM } else { TEXT_FAINT };
                painter.text(
                    Pos2::new(text_x + 7.0, mid),
                    Align2::CENTER_CENTER,
                    glyph,
                    FontId::proportional(13.0),
                    ic,
                );
                text_x += 20.0;
            }
            let right_pad = if is_dir { 44.0 } else { 10.0 };
            let clip =
                Rect::from_min_max(Pos2::new(text_x, rect.top()), Pos2::new(rect.right() - right_pad, rect.bottom()));
            painter.with_clip_rect(clip).text(
                Pos2::new(text_x, mid),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(13.0),
                color,
            );
            if is_dir {
                // How many files are inside.
                let at = Pos2::new(rect.right() - 14.0, mid);
                painter.text(at, Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(11.0), TEXT_FAINT);
            }

            if resp.hovered() && on {
                actions.push(Action::HoverPath(path.to_string()));
            }
            if resp.clicked() {
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
        }
    });
}

fn search_panel(ui: &mut egui::Ui, st: &mut UiState, page: Option<&Page>, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Search", |_| {});
        ui.add_space(6.0);
        let resp = search_field(ui, &mut st.search_query, "Text to find in every file", icons::TEXT_SEARCH);
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            actions.push(Action::Search(st.search_query.clone()));
        }
        ui.label(RichText::new("Enter to search · lowercase ignores case").size(11.0).color(TEXT_FAINT));
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
                .color(TEXT_DIM),
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
                painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, Color32::from_rgb(20, 22, 27));
            }
            let mid = rect.center().y;
            let place = format!("{}:{}", f.name, hit.line + 1);
            let galley = painter.layout_no_wrap(place, FontId::proportional(12.0), TEXT_DIM);
            let pw = galley.size().x;
            painter.galley(Pos2::new(rect.left() + 14.0, mid - galley.size().y * 0.5), galley, TEXT_DIM);
            let x = rect.left() + 22.0 + pw;
            let clip = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(rect.right() - 8.0, rect.bottom()));
            let line = f.line(hit.line as usize).trim_start();
            let preview: String = line.chars().take(160).collect();
            painter.with_clip_rect(clip).text(
                Pos2::new(x, mid),
                Align2::LEFT_CENTER,
                preview,
                FontId::monospace(12.0),
                TEXT,
            );
            if resp.clicked() {
                actions.push(Action::FlyToSite(hit));
            }
            resp.on_hover_text(&f.rel_path);
        }
    });
}

fn help_panel(ui: &mut egui::Ui) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 10, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Shortcuts", |_| {});
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (title, rows) in [
                (
                    "Canvas",
                    &[
                        ("Zoom", "Mouse wheel, + / −"),
                        ("Pan", "Drag, arrow keys"),
                        ("Fly to a file or folder", "Double-click"),
                        ("Show everything", "F or Home"),
                        ("Go to the selection", "L"),
                        ("Zoom out to the parent folder", "Backspace"),
                        ("Collapse / expand a folder", "Click − / + next to its name"),
                    ][..],
                ),
                (
                    "Symbols",
                    &[
                        ("See where a name is used", "Click it (zoomed in to text)"),
                        ("Select a function", "Click inside it"),
                        ("Deselect", "Click it again, or Esc for all"),
                    ][..],
                ),
                ("Find", &[("Go to a file or folder", "Ctrl+P"), ("Search in files", "Ctrl+Shift+F")][..]),
                (
                    "Extensions",
                    &[
                        ("Format the file under the pointer", "Shift+Alt+F"),
                        ("See a problem's message", "Point at the underlined code"),
                    ][..],
                ),
                (
                    "Folders and panels",
                    &[
                        ("Open a folder in a new canvas", "Ctrl+O, or drop it"),
                        ("Next / previous canvas", "Ctrl+Tab / Ctrl+Shift+Tab"),
                        ("Go to canvas N", "Ctrl+1 … Ctrl+9"),
                        ("Close the canvas", "Ctrl+W"),
                        ("Reload from disk", "F5"),
                        ("Toggle the left panel", "Ctrl+B"),
                    ][..],
                ),
            ] {
                section(ui, icons::CHEVRON_DOWN, title);
                well(ui, |ui| {
                    for (what, keys) in rows {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(*what).size(12.5).color(TEXT));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(*keys).size(12.0).color(TEXT_DIM));
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
                ui.label(RichText::new(format!("{n} connected files shown")).size(11.0).color(c(theme::UI_WARN)));
            }
        });

        section(ui, icons::FILTER, "Filters");
        let before = st.filters;
        well(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.spacing_mut().slider_width = 92.0;
            let pct = |ui: &mut egui::Ui, v: f32| {
                ui.label(RichText::new(format!("{:>3.0}%", v * 100.0)).monospace().size(11.0).color(TEXT_DIM));
            };
            for kind in RelKind::ALL {
                let i = kind.index();
                let count: usize = page.map(|p| p.selections.iter().map(|s| s.rel.count(kind)).sum()).unwrap_or(0);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(11.0, 20.0), Sense::hover());
                    let sw = Rect::from_center_size(rect.center(), Vec2::splat(11.0));
                    ui.painter().rect_filled(sw, 3.0, c(theme::REL_COLORS[i]));
                    check(ui, &mut st.filters.enabled[i], kind.label(), true, false);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        pct(ui, st.filters.opacity[i]);
                        ui.add(egui::Slider::new(&mut st.filters.opacity[i], 0.05..=1.0).show_value(false));
                        ui.label(RichText::new(count.to_string()).monospace().size(11.0).color(TEXT_DIM));
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
                        .color(TEXT_DIM),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{} symbols indexed by name. Results are approximate until a language server is connected.",
                        r.repo.symbols.count
                    ))
                    .size(11.0)
                    .color(TEXT_FAINT),
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
                .fill(WELL)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(9))
                .outer_margin(Margin { left: 10, right: 10, top: 0, bottom: 6 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        Frame::NONE
                            .fill(RAISED)
                            .corner_radius(CornerRadius::same(4))
                            .inner_margin(Margin::symmetric(5, 1))
                            .show(ui, |ui| {
                                ui.label(RichText::new(sym.kind.badge()).monospace().size(11.0).color(ACCENT));
                            });
                        ui.label(RichText::new(repo.sym_name(sel.sym)).monospace().size(13.5).color(TEXT));
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
                    ui.label(RichText::new(repo.sym_signature(sel.sym)).monospace().size(12.0).color(TEXT));
                    ui.add_space(4.0);

                    for kind in RelKind::ALL {
                        let items: Vec<_> = sel.rel.items.iter().filter(|x| x.kind == kind).collect();
                        if items.is_empty() {
                            continue;
                        }
                        let title = RichText::new(format!("{}  {}", kind.label(), items.len()))
                            .size(12.5)
                            .color(c(theme::REL_COLORS[kind.index()]));
                        let id = (page.id, sel.sym.file, sel.sym.index, kind.index());
                        egui::CollapsingHeader::new(title).id_salt(id).show(ui, |ui| {
                            for rel in items.iter().take(200) {
                                // Outgoing calls list the callee; the rest list where the usage is.
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
                                        .color(TEXT_DIM),
                                );
                            }
                        });
                    }
                    if sel.outside > 0 {
                        ui.label(
                            RichText::new(format!("Outside the canvas  {}", sel.outside)).size(12.0).color(TEXT_DIM),
                        );
                    }
                    let mut note = format!("searched in {:.0} ms", sel.rel.search_ms);
                    if sel.rel.truncated {
                        note.push_str(" · results truncated");
                    }
                    ui.label(RichText::new(note).size(10.0).color(TEXT_FAINT));
                });
        }
        ui.add_space(8.0);
    });
}

fn claude_panel(ui: &mut egui::Ui, st: &mut UiState, page: Option<&mut Page>, actions: &mut Vec<Action>) {
    use crate::claude::{mentions, Role};

    Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 10, bottom: 4 }).show(ui, |ui| {
        panel_title(ui, "Claude Code", |ui| {
            if small_icon(ui, icons::PLUS, "New chat: forget this conversation") {
                actions.push(Action::ChatNew);
            }
        });
        match &st.claude {
            ClaudeStatus::Checking => {
                ui.label(RichText::new("Looking for the claude command…").size(11.0).color(TEXT_DIM));
            }
            ClaudeStatus::Ready(version) => {
                ui.label(
                    RichText::new(format!("Using your Claude Code login · {version}")).size(11.0).color(TEXT_FAINT),
                )
                .on_hover_text(
                    "Messages are answered by the `claude` command installed on this computer, with the \
                         account it is signed in to. Code Canvas stores no API key.",
                );
            }
            ClaudeStatus::Missing(why) => {
                ui.label(
                    RichText::new(
                        "The `claude` command was not found. Install Claude Code and sign in to use this panel.",
                    )
                    .size(12.0)
                    .color(c(theme::UI_WARN)),
                )
                .on_hover_text(why);
            }
        }
    });
    let Some(page) = page else {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Open a folder to chat about it").color(TEXT_DIM)));
        return;
    };

    // Input pinned to the bottom, transcript above it.
    let running = page.chat.is_running();
    egui::Panel::bottom("chat_input").frame(Frame::NONE.inner_margin(Margin::same(10))).show(ui, |ui| {
        let has_context = page.selected.is_some() || !page.selections.is_empty();
        check(ui, &mut page.chat.include_selection, "Include what is selected on the canvas", has_context, true)
            .on_hover_text("Tell Claude which symbols and file are selected");
        check(ui, &mut page.chat.allow_edits, "Allow Claude to edit files", true, true)
            .on_hover_text("Off: Claude can only read the repository. On: it may change files without asking.");
        ui.add_space(2.0);
        let edit = Frame::NONE
            .fill(WELL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut page.chat.input)
                        .hint_text("Ask about this code…  (Ctrl+Enter to send)")
                        .frame(Frame::NONE)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
            })
            .inner;
        let send_key = edit.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            if running {
                if ui.button(format!("{}  Stop", icons::CIRCLE_STOP)).clicked() {
                    actions.push(Action::ChatStop);
                }
                ui.label(
                    RichText::new(format!("working… {:.0} s", page.chat.elapsed_secs())).size(11.0).color(TEXT_DIM),
                );
            } else {
                let can_send = !page.chat.input.trim().is_empty() && matches!(st.claude, ClaudeStatus::Ready(_));
                let send = ui.add_enabled(can_send, egui::Button::new(format!("{}  Send", icons::SEND)));
                if send.clicked() || (send_key && can_send) {
                    actions.push(Action::ChatSend);
                }
            }
        });
    });

    let repo = page.ready().map(|r| &r.repo);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        Frame::NONE.inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
            if page.chat.messages.is_empty() {
                well(ui, |ui| {
                    ui.label(
                        RichText::new(
                            "Ask anything about this repository: how a feature works, where something is handled, \
                             what a function is for. Files mentioned in the answer become buttons that fly to them.",
                        )
                        .size(12.0)
                        .color(TEXT_DIM),
                    );
                });
            }
            for msg in &page.chat.messages {
                match msg.role {
                    Role::User => {
                        Frame::NONE
                            .fill(RAISED)
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(9, 7))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(&msg.text).color(TEXT));
                            });
                    }
                    Role::Assistant => {
                        ui.add(egui::Label::new(RichText::new(&msg.text).color(TEXT)).wrap().selectable(true));
                        // Files named in the answer: one click flies to them.
                        if let Some(repo) = repo {
                            let found = mentions(&msg.text, |p| repo.files.iter().any(|f| f.rel_path == p));
                            if !found.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                                    for m in found {
                                        let name = m.path.rsplit('/').next().unwrap_or(&m.path);
                                        let label = match m.line {
                                            Some(l) => format!("{}  {name}:{}", icons::LOCATE_FIXED, l + 1),
                                            None => format!("{}  {name}", icons::LOCATE_FIXED),
                                        };
                                        if ui
                                            .small_button(RichText::new(label).size(11.0))
                                            .on_hover_text(&m.path)
                                            .clicked()
                                        {
                                            match (m.line, repo.files.iter().position(|f| f.rel_path == m.path)) {
                                                (Some(line), Some(file)) => actions.push(Action::FlyToSite(Site {
                                                    file: file as u32,
                                                    line,
                                                    col: 0,
                                                    len: 0,
                                                })),
                                                _ => actions.push(Action::FlyToPath(m.path.clone())),
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    }
                    Role::Tool => {
                        ui.label(RichText::new(format!("· {}", msg.text)).monospace().size(11.0).color(TEXT_FAINT));
                    }
                    Role::Info => {
                        ui.label(RichText::new(&msg.text).size(10.0).color(TEXT_FAINT));
                    }
                    Role::Error => {
                        ui.label(RichText::new(&msg.text).color(c(theme::UI_WARN)));
                    }
                }
                ui.add_space(6.0);
            }
        });
    });
}

fn center_message(ui: &mut egui::Ui, canvas: Rect, page: Option<&Page>) {
    let message = match page {
        None => Some(("Open a folder with Ctrl+O, the + button, or drop it here".to_string(), TEXT)),
        Some(p) => match &p.state {
            PageState::Loading { progress, started, .. } => Some((
                format!(
                    "Scanning {} …  {} / {} files  ·  {:.1} s",
                    p.title,
                    progress.loaded.load(Ordering::Relaxed),
                    progress.discovered.load(Ordering::Relaxed),
                    started.elapsed().as_secs_f32()
                ),
                TEXT,
            )),
            PageState::Failed(e) => Some((e.clone(), c(theme::UI_WARN))),
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
                .fill(CARD)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(5, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let b = Vec2::splat(30.0);
                        if icon_button(ui, icons::SCAN, b, false, HOVER)
                            .on_hover_text("Show the whole canvas (F)")
                            .clicked()
                        {
                            actions.push(Action::Fit);
                        }
                        if icon_button(ui, icons::LOCATE_FIXED, b, false, HOVER)
                            .on_hover_text("Go to the selected symbols or file (L)")
                            .clicked()
                        {
                            actions.push(Action::FrameSelection);
                        }
                        if icon_button(ui, icons::WAND_SPARKLES, b, false, HOVER)
                            .on_hover_text("Format the selected file with the installed formatter (Shift+Alt+F formats the file under the pointer)")
                            .clicked()
                        {
                            actions.push(Action::FormatFile(None));
                        }
                        ui.add_space(4.0);
                        let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                        ui.painter().rect_filled(sep, 0.0, BORDER);
                        ui.add_space(4.0);
                        if icon_button(ui, icons::MINUS, b, false, HOVER).on_hover_text("Zoom out").clicked() {
                            actions.push(Action::ZoomBy(1.0 / 1.4));
                        }
                        ui.label(
                            RichText::new(format!("{:>4.0}%", info.zoom * 100.0))
                                .monospace()
                                .size(11.0)
                                .color(TEXT_DIM),
                        );
                        if icon_button(ui, icons::PLUS, b, false, HOVER).on_hover_text("Zoom in").clicked() {
                            actions.push(Action::ZoomBy(1.4));
                        }
                    });
                });
        });
    st.floating.push(resp.response.rect);
}

/// Rank `path` against a lowercase query. Higher is better; `None` is no match.
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
    // Subsequence match, e.g. "appts" → "src/app/types.ts".
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

fn quick_open(ctx: &egui::Context, st: &mut UiState, canvas: Rect, page: Option<&Page>, actions: &mut Vec<Action>) {
    let Some(q) = &mut st.quick else { return };
    let mut close = false;
    let screen = ctx.content_rect();
    let width = 560.0f32.min(screen.width() - 40.0).max(240.0);
    let pos = Pos2::new(screen.center().x - width * 0.5, canvas.top().max(HEADER_H) + 6.0);
    let resp =
        egui::Area::new(egui::Id::unique("quick_open")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            Frame::NONE
                .fill(CARD)
                .stroke(Stroke::new(1.0, Color32::from_rgb(58, 63, 78)))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(width);
                    let edit = search_field(ui, &mut q.query, "Go to a file or folder by name", icons::SEARCH);
                    if q.focus {
                        edit.request_focus();
                        q.focus = false;
                    }
                    if edit.changed() {
                        q.selected = 0;
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
                            ui.painter().rect_filled(
                                rect,
                                6.0,
                                if i == q.selected { RAISED } else { Color32::from_rgb(20, 22, 27) },
                            );
                        }
                        let glyph = if *is_dir { icons::FOLDER } else { file_glyph(name) };
                        ui.painter().text(
                            Pos2::new(rect.left() + 16.0, rect.center().y),
                            Align2::CENTER_CENTER,
                            glyph,
                            FontId::proportional(13.0),
                            TEXT_DIM,
                        );
                        let label = if *is_dir { format!("{name}/") } else { name.to_string() };
                        let galley = ui.painter().layout_no_wrap(label, FontId::proportional(13.0), TEXT);
                        let name_w = galley.size().x;
                        ui.painter().galley(
                            Pos2::new(rect.left() + 32.0, rect.center().y - galley.size().y * 0.5),
                            galley,
                            TEXT,
                        );
                        let clip =
                            Rect::from_min_max(Pos2::new(rect.left() + 44.0 + name_w, rect.top()), rect.right_bottom());
                        ui.painter().with_clip_rect(clip).text(
                            Pos2::new(rect.left() + 44.0 + name_w, rect.center().y),
                            Align2::LEFT_CENTER,
                            dir,
                            FontId::proportional(11.0),
                            TEXT_FAINT,
                        );
                        if resp.clicked() {
                            chosen = Some(path.to_string());
                        }
                    }
                    if !query.is_empty() && results.is_empty() {
                        ui.label(RichText::new("No matches").color(TEXT_DIM));
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
                .fill(CARD)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(7))
                .inner_margin(Margin::symmetric(9, 6))
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(RichText::new(title).size(13.0).color(TEXT));
                    if !detail.is_empty() {
                        ui.label(RichText::new(detail).size(11.0).color(TEXT_DIM));
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::quick_score;

    #[test]
    fn quick_open_ranking() {
        let score = |p: &str, q: &str| quick_score(p, q);
        assert!(score("src/app/types.ts", "types.ts") > score("src/app/types.ts.bak", "types.ts"));
        assert!(score("src/api.ts", "api") > score("src/rapid/x.ts", "api"), "name match beats path match");
        assert!(score("src/app/types.ts", "appts").is_some(), "subsequence");
        assert!(score("src/app/types.ts", "zzz").is_none());
    }
}
