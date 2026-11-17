//! The application chrome, built with egui around the canvas.
//!
//! The window has no system title bar: the header is ours (panel toggles,
//! rescan, folder tabs, the "go to file" box, window buttons). An icon rail
//! on each side opens one panel at a time, like an activity bar:
//!
//! * left: files, search in files, extensions, problems, shortcuts
//! * right: symbols (selection, filters, names), and extensions' webviews
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

mod deps_panel;
mod ext_panels;
mod git_panel;
mod settings_panel;

use crate::keys::{Command, Keymap};

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
    /// The theme and its colours.
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightPanel {
    Symbols,
    /// Dependencies between files: what imports what.
    Deps,
    /// An extension's webview (`UiState::webview`).
    Webview,
}

/// The file icon theme in use (`icon_theme.rs`) and the images of it drawn
/// so far. Without one, callers draw the app's own glyphs.
#[derive(Default)]
struct FileIcons {
    /// The theme file `theme` was read from, or failed to be.
    file: Option<std::path::PathBuf>,
    theme: Option<crate::icon_theme::IconTheme>,
    textures: std::collections::HashMap<std::path::PathBuf, Option<egui::TextureHandle>>,
}

impl FileIcons {
    /// Use the theme in `file`; `None` for the app's own glyphs.
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

    /// Paint the icon of a file or folder called `name` in `at`. False when
    /// the theme has none for it: the caller then draws its own.
    fn paint(&mut self, ui: &egui::Ui, name: &str, is_dir: bool, open: bool, at: Rect, dim: bool) -> bool {
        let Some(theme) = &self.theme else { return false };
        let Some(svg) = (if is_dir { theme.folder(name, open) } else { theme.file(name) }) else { return false };
        // Twice the size it is shown at: sharp on dense screens too.
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

/// Something the explorer's menu asks the app to do with a file or folder.
/// Paths are relative to the open folder, with `/`; `""` is the folder itself.
pub enum FileOp {
    /// Show it in the system's file manager.
    Reveal(String),
    /// Open a terminal in this folder.
    Terminal(String),
    CopyPath {
        path: String,
        relative: bool,
    },
    NewFile {
        dir: String,
        name: String,
    },
    NewFolder {
        dir: String,
        name: String,
    },
    Rename {
        path: String,
        name: String,
    },
    /// Move it to the recycle bin (the app asks first).
    Delete(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NamingKind {
    NewFile,
    NewFolder,
    Rename,
}

/// The explorer is asking for a name.
pub struct Naming {
    pub kind: NamingKind,
    /// The folder the new thing goes in, or what is being renamed.
    pub path: String,
    pub text: String,
    /// Put the caret in the field on the next frame.
    pub focus: bool,
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
    pub explorer_filter: String,
    pub search_query: String,
    pub quick: Option<QuickOpen>,
    pub filters: RelFilters,
    /// Interactive floating areas over the canvas, in points.
    pub floating: Vec<Rect>,
    pub maximized: bool,
    /// The app icon, shown in the top-left corner of the header.
    logo: Option<egui::TextureHandle>,
    /// What to draw of the dependencies between files.
    pub deps: crate::page::DepFilters,
    /// Open the folder menu on the next frame (set by scripts).
    pub open_folder_menu: bool,
    /// Key of the webview shown by [`RightPanel::Webview`].
    pub webview: String,
    /// Where that webview's page goes this frame, in points.
    pub webview_rect: Option<Rect>,
    /// Id of the extension shown by [`LeftPanel::Extension`].
    pub extension: String,
    /// Icons of installed extensions, by extension id (`None`: it has none).
    ext_icons: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    /// The theme chosen and the colours changed on top of it. The app keeps
    /// it in step with what is on screen and on disk.
    pub theme: crate::themes::ThemeFile,
    /// The shortcuts in use, and the user's changes they were built from.
    pub keys: Keymap,
    pub key_file: crate::keys::KeyFile,
    /// The command whose new shortcut the settings panel is waiting for:
    /// the next key pressed becomes it.
    pub rebinding: Option<Command>,
    /// The name the explorer is asking for, if it is asking.
    pub naming: Option<Naming>,
    /// Show one folder at a time, instead of all of them side by side.
    pub single: bool,
    pub prefs: crate::prefs::Prefs,
    /// Who writes commit messages when asked (`commit_ai.rs`).
    pub commit_ai: crate::commit_ai::Settings,
    /// The file icon theme in use.
    file_icons: FileIcons,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            left: Some(LeftPanel::Files),
            right: Some(RightPanel::Symbols),
            last_left: LeftPanel::Files,
            last_right: RightPanel::Symbols,
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
            webview: String::new(),
            webview_rect: None,
            ext_icons: Default::default(),
            theme: Default::default(),
            keys: Keymap::new(&Default::default()),
            key_file: Default::default(),
            rebinding: None,
            naming: None,
            single: false,
            prefs: Default::default(),
            commit_ai: Default::default(),
            file_icons: FileIcons::default(),
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
    /// Open a left panel.
    ShowLeft(LeftPanel),
    /// Save the file being edited.
    SaveFile,
    /// Run an extension's command: `(id, title)`.
    RunCommand(String, String),
    /// What to draw of the dependencies changed.
    DepsChanged,
    /// A source control command on the active page.
    Git(crate::git::GitOp),
    /// Ask git again what changed in the active page.
    RefreshGit,
    /// Have an assistant write the commit message of the active page.
    WriteCommitMessage,
    /// Change who writes commit messages.
    SetCommitAi(crate::commit_ai::Settings),
    SetPrefs(crate::prefs::Prefs),
    /// Show only the files git reports as changed.
    SetOnlyChanged(bool),
    /// Pick a `.vsix` file and install it.
    InstallVsix,
    /// Use a built-in theme (a key of `theme::PRESETS`).
    SetTheme(&'static str),
    /// Change one colour of the theme: `(key, 0xRRGGBBAA)`.
    SetColor(&'static str, u32),
    /// Forget every colour changed by hand.
    ResetColors,
    /// Use this file icon theme (an id, or `"none"` for the app's own).
    SetIconTheme(String),
    /// Folders side by side, or one at a time.
    ToggleSideBySide,
    /// Give a command its own shortcuts back.
    ResetKey(Command),
    /// Leave a command without a shortcut.
    UnbindKey(Command),
    /// The explorer's menu: something to do with a file or folder.
    File(FileOp),
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

/// The interface's colours: the "Interface" group of the theme in use
/// (`canvas_render::theme`), ready for egui. Take them with [`pal`].
#[derive(Clone, Copy)]
struct Pal {
    /// The window behind everything, and the rails.
    chrome: Color32,
    /// Panels.
    card: Color32,
    /// Text fields and other insets.
    well: Color32,
    /// A row of a list that stands out a little.
    row: Color32,
    /// Buttons, selected rows.
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
    /// Text drawn on top of the accent colour.
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

/// Space between two folders shown side by side, in points.
const PANE_GAP: f32 = 8.0;

/// How many folders the canvas shows at once.
pub fn pane_count(single: bool, folders: usize) -> usize {
    if single {
        1
    } else {
        folders.max(1)
    }
}

/// The canvas divided into `count` panes side by side, with a gap between
/// them: where each open folder is drawn. Works in any unit; `gap` is the
/// space between two panes in that unit.
pub fn pane_rects(canvas: egui::Rect, count: usize, gap: f32) -> Vec<egui::Rect> {
    let count = count.max(1);
    let width = ((canvas.width() - gap * (count - 1) as f32) / count as f32).max(1.0);
    (0..count)
        .map(|i| {
            let left = canvas.left() + i as f32 * (width + gap);
            egui::Rect::from_min_size(Pos2::new(left, canvas.top()), Vec2::new(width, canvas.height()))
        })
        .collect()
}

/// Fonts and the app's look (call once).
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

    // Codicons (VS Code's icons) too: extensions name them in their texts,
    // and a few panels that mirror VS Code's use the same icons it does.
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

/// Give egui the colours of the theme in use (again, when it changes).
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
    // Always dark, whatever the system theme is.
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
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
            ui.painter().rect_filled(inner, 8.0, pal().raised);
        } else if resp.hovered() {
            ui.painter().rect_filled(inner, 8.0, hover_fill);
        }
        let color = if active || resp.hovered() { pal().text } else { pal().text_dim };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), color);
    }
    resp
}

/// A rounded checkbox with a check mark, like the concept's.
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

/// A section title with an icon, e.g. "Filters".
fn section(ui: &mut egui::Ui, glyph: &str, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(icon(glyph, 14.0, pal().text_dim));
        ui.label(RichText::new(title).size(13.5).color(pal().text));
    });
    ui.add_space(2.0);
}

/// A recessed rounded area inside a panel.
fn well(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    Frame::NONE.fill(pal().well).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

/// A side panel: a rounded card with a gap around it.
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
    st.file_icons.set(ext.icon_theme(&st.theme.icons).map(|t| t.file.as_path()));
    header(ui, st, actions);
    status_bar(ui, st, ext, pages.get(active), info);
    left_rail(ui, st, ext, pages.get(active));
    right_rail(ui, st, ext);

    if let Some(panel) = st.left {
        let p = egui::Panel::left("left").resizable(true).default_size(300.0).size_range(220.0..=560.0);
        card_panel(ui, p, |ui| match panel {
            LeftPanel::Files => files_panel(ui, st, pages, active, actions),
            LeftPanel::Search => search_panel(ui, st, pages.get(active), actions),
            LeftPanel::Extensions => ext_panels::extensions_panel(ui, st, ext, actions),
            LeftPanel::Problems => ext_panels::problems_panel(ui, pages.get(active), ext, actions),
            LeftPanel::Changes => git_panel::changes_panel(ui, &mut st.file_icons, pages.get_mut(active), actions),
            LeftPanel::Extension => ext_panels::extension_view(ui, st, ext),
            LeftPanel::Help => help_panel(ui, &st.keys),
            LeftPanel::Settings => settings_panel::settings_panel(ui, st, ext, actions),
        });
    }
    if let Some(panel) = st.right {
        let p = egui::Panel::right("right").resizable(true).default_size(340.0).size_range(260.0..=600.0);
        card_panel(ui, p, |ui| match panel {
            RightPanel::Symbols => symbols_panel(ui, st, pages.get(active), actions),
            RightPanel::Deps => deps_panel::deps_panel(ui, st, pages.get(active), actions),
            RightPanel::Webview => ext_panels::webview_panel(ui, st, ext),
        });
    }

    let canvas = ui.available_rect_before_wrap();
    let ctx = ui.ctx().clone();
    // One pane per folder shown: its message while it has no canvas yet,
    // and with several, a frame that says which one the keyboard is on.
    let panes = pane_rects(canvas, pane_count(st.single, pages.len()), PANE_GAP);
    if panes.len() > 1 {
        for (i, pane) in panes.iter().enumerate() {
            center_message(ui, *pane, pages.get(i), &st.keys);
            let color = if i == active { pal().border_strong } else { pal().border };
            let frame = egui::Stroke::new(1.0, color);
            ui.painter().rect_stroke(pane.shrink(0.5), 6.0, frame, egui::StrokeKind::Inside);
        }
    } else {
        center_message(ui, canvas, pages.get(active), &st.keys);
    }
    // The zoom buttons act on the active folder: they sit in its pane.
    let active_pane = if panes.len() > 1 { panes.get(active).copied().unwrap_or(canvas) } else { canvas };
    toolbar(&ctx, st, active_pane, info, actions);
    ext_panels::notices(&ctx, st, canvas, ext);
    quick_open(&ctx, st, canvas, pages.get(active), ext, actions);
    tooltip(&ctx, canvas, info);
    canvas
}

fn header(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    egui::Panel::top("header").frame(Frame::NONE.fill(pal().chrome)).exact_size(HEADER_H).show(ui, |ui| {
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

        // The "go to file" box, centred in the window when there is room.
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

fn status_bar(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions, page: Option<&Page>, info: &FrameInfo) {
    let mut panel_to_show = None;
    egui::Panel::bottom("status").frame(Frame::NONE.fill(pal().chrome).inner_margin(Margin::symmetric(12, 0))).exact_size(24.0).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                // Like VS Code's: the branch first (`*` when there are changes).
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
                        panel_to_show = Some(LeftPanel::Changes);
                    }
                    ui.add_space(10.0);
                }
                ui.label(RichText::new(&info.status).size(12.0).color(pal().text_dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                    // What extensions report: problems, and their status items.
                    let (errors, warnings) = ext.totals();
                    if errors + warnings > 0 {
                        ui.add_space(8.0);
                        let text = format!("{}  {errors}   {}  {warnings}", icons::CIRCLE_ALERT, icons::TRIANGLE_ALERT);
                        let color = if errors > 0 { c(theme::get().problem_error) } else { c(theme::get().problem_warning) };
                        let label = ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).sense(Sense::click()));
                        if label.on_hover_text("Problems reported by extensions: click to list them").clicked() {
                            panel_to_show = Some(LeftPanel::Problems);
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
        st.show_left(panel);
    }
}

fn rail_frame() -> Frame {
    Frame::NONE.fill(pal().chrome).inner_margin(Margin { left: 5, right: 5, top: 0, bottom: GAP })
}

fn left_rail(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions, page: Option<&Page>) {
    egui::Panel::left("rail_left").frame(rail_frame()).exact_size(RAIL_W).resizable(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let b = Vec2::splat(34.0);
        for (panel, glyph, tip) in [
            (LeftPanel::Files, icons::FOLDER, st.keys.hint("Files", Command::ToggleLeftPanel)),
            (LeftPanel::Search, icons::TEXT_SEARCH, st.keys.hint("Search in files", Command::SearchInFiles)),
            (
                LeftPanel::Changes,
                crate::codicons::glyph("source-control").unwrap_or(icons::GIT_GRAPH),
                "Source control".to_string(),
            ),
            (
                LeftPanel::Extensions,
                crate::codicons::glyph("extensions").unwrap_or(icons::PUZZLE),
                "Extensions".to_string(),
            ),
            (LeftPanel::Problems, icons::CIRCLE_ALERT, "Problems reported by extensions".to_string()),
        ] {
            let resp = icon_button(ui, glyph, b, st.left == Some(panel), pal().hover);
            if panel == LeftPanel::Problems {
                // A dot on the icon while there is something to look at.
                let (errors, warnings) = ext.totals();
                if errors + warnings > 0 {
                    let color =
                        if errors > 0 { c(theme::get().problem_error) } else { c(theme::get().problem_warning) };
                    ui.painter().circle_filled(resp.rect.right_top() + Vec2::new(-9.0, 9.0), 3.5, color);
                }
            }
            if panel == LeftPanel::Changes {
                // How many files git reports as changed, as VS Code shows it.
                let changed = page.and_then(|p| p.git.as_ref()).map_or(0, |g| g.changes.len());
                if changed > 0 {
                    count_badge(ui, resp.rect, changed);
                }
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
            if icon_button(ui, icons::PANEL_LEFT, b, false, pal().hover)
                .on_hover_text(st.keys.hint("Show or hide the left panel", Command::ToggleLeftPanel))
                .clicked()
            {
                st.toggle_left();
            }
            for (panel, glyph, tip) in [
                (LeftPanel::Settings, icons::SETTINGS, "Settings: theme, colours, shortcuts"),
                (LeftPanel::Help, icons::CIRCLE_HELP, "Keyboard and mouse shortcuts"),
            ] {
                if icon_button(ui, glyph, b, st.left == Some(panel), pal().hover).on_hover_text(tip).clicked() {
                    if st.left == Some(panel) {
                        st.left = None;
                    } else {
                        st.show_left(panel);
                    }
                }
            }
        });
    });
}

/// A number in the corner of a rail icon: a pill in the accent colour,
/// "99+" beyond two digits.
fn count_badge(ui: &egui::Ui, icon: Rect, count: usize) {
    let text = if count > 99 { "99+".to_string() } else { count.to_string() };
    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(9.5), pal().on_accent);
    let size = Vec2::new((galley.size().x + 7.0).max(14.0), 14.0);
    // Its right edge stays inside the rail however many digits there are.
    let pill = Rect::from_min_size(Pos2::new(icon.right() - size.x, icon.bottom() - size.y - 2.0), size);
    ui.painter().rect_filled(pill, 7.0, pal().accent);
    ui.painter().galley(pill.center() - galley.size() * 0.5, galley, pal().on_accent);
}

fn right_rail(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions) {
    egui::Panel::right("rail_right").frame(rail_frame()).exact_size(RAIL_W).resizable(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let b = Vec2::splat(34.0);
        for (panel, glyph, tip) in [
            (RightPanel::Symbols, icons::LIST_TREE, "Symbols: selection, filters, names"),
            (RightPanel::Deps, icons::WAYPOINTS, "Dependencies: which file imports which"),
        ] {
            if icon_button(ui, glyph, b, st.right == Some(panel), pal().hover).on_hover_text(tip).clicked() {
                if st.right == Some(panel) {
                    st.right = None;
                } else {
                    st.show_right(panel);
                }
            }
        }
        // Extensions' own interfaces: what their manifests put in the bars.
        for w in &ext.rail_items() {
            let active = st.right == Some(RightPanel::Webview) && st.webview == w.view;
            let (rect, resp) = ui.allocate_exact_size(b, Sense::click());
            if active {
                ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().raised);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect.shrink(4.0), 8.0, pal().hover);
            }
            let glyph = |ui: &egui::Ui, glyph: &str| {
                let color = if active || resp.hovered() { pal().text } else { pal().text_dim };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(17.0), color);
            };
            // The container's own icon when it is one of VS Code's; else the
            // extension's picture stands in for its image file.
            let installed = ext.installed.iter().find(|e| e.id == w.extension);
            match w.codicon.as_deref().and_then(crate::codicons::glyph) {
                Some(codicon) => glyph(ui, codicon),
                None => match installed.and_then(|e| ext_panels::extension_icon(ui.ctx(), &mut st.ext_icons, e)) {
                    Some(texture) => {
                        let at = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
                        egui::Image::new(&texture).corner_radius(4).paint_at(ui, at);
                    }
                    None => glyph(ui, icons::PUZZLE),
                },
            }
            if resp.on_hover_text(&w.title).clicked() {
                if active {
                    st.right = None;
                } else {
                    st.webview = w.view.clone();
                    st.right = Some(RightPanel::Webview);
                }
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
            if icon_button(ui, icons::PANEL_RIGHT, b, false, pal().hover)
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

/// A text field with a leading icon, in a rounded well.
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

/// The open folders as a menu: pick one, close one, open another. Each
/// folder is its own canvas.
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
        // As wide as its longest folder name needs, within sensible limits:
        // rows are laid out by hand so the menu does not stretch.
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
            // The close button shows on the row under the pointer.
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
    // A script asked for the menu to be shown (there is no click to open it).
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
                if small_icon(ui, icons::PLUS, &st.keys.hint("Open a folder in a new canvas", Command::OpenFolder)) {
                    actions.push(Action::OpenFolder);
                }
                let split = if st.single { "Show every open folder side by side" } else { "Show one folder at a time" };
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
                // The folder being shown: what is left of the row.
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
            ui.label(RichText::new(note).size(11.0).color(pal().text_dim));
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
                painter.rect_filled(row_bg, 6.0, pal().raised);
            } else if resp.hovered() {
                painter.rect_filled(row_bg, 6.0, pal().row);
            }
            let mid = rect.center().y;
            // Indent guides.
            for d in 0..depth {
                let gx = rect.left() + 22.0 + d as f32 * 16.0;
                painter.vline(gx, rect.y_range(), Stroke::new(1.0, pal().raised));
            }
            let x0 = rect.left() + 14.0 + depth as f32 * 16.0;
            if is_dir {
                let chevron = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
                painter.text(
                    Pos2::new(x0 + 8.0, mid),
                    Align2::CENTER_CENTER,
                    chevron,
                    FontId::proportional(13.0),
                    pal().text_dim,
                );
            }
            // Checkbox: is it on the canvas?
            let on = !hidden && !locked;
            let cb = Rect::from_center_size(Pos2::new(x0 + 27.0, mid), Vec2::splat(15.0));
            painter.rect_filled(cb, 4.0, pal().raised);
            if on {
                painter.text(cb.center(), Align2::CENTER_CENTER, icons::CHECK, FontId::proportional(11.0), pal().text);
            }
            let mut text_x = x0 + 42.0;
            let color = if !on {
                pal().text_faint
            } else if is_dir {
                pal().text
            } else {
                // Files: a step below folders.
                c(theme::mix(theme::get().text, theme::get().text_dim, 0.35))
            };
            // Its icon: from the icon theme, else (for files) the app's own.
            let base = name.rsplit_once('/').map_or(name, |(_, base)| base);
            let icon_at = Rect::from_center_size(Pos2::new(text_x + 7.0, mid), Vec2::splat(16.0));
            if st.file_icons.paint(ui, base, is_dir, open, icon_at, !on) {
                text_x += 20.0;
            } else if !is_dir {
                let glyph = file_glyph(name);
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
            let right_pad = if is_dir { 44.0 } else { 10.0 };
            let room = rect.right() - right_pad - text_x;
            text_fit(painter, Pos2::new(text_x, mid), name, FontId::proportional(13.0), color, room);
            if is_dir {
                // How many files are inside.
                let at = Pos2::new(rect.right() - 14.0, mid);
                painter.text(at, Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(11.0), pal().text_faint);
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
            resp.context_menu(|ui| file_menu(ui, &mut st.naming, path, is_dir, hidden, locked, actions));
        }
    });
}

/// One line of text starting at `at` (its left end, vertically centred)
/// that has `width` to fit in: what does not fit is cut at the end, which
/// then reads "…". Returns the width it took.
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

/// A row of a menu: an icon and what it does. True when it was chosen.
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

/// A line between two groups of a menu.
fn menu_gap(ui: &mut egui::Ui) {
    ui.add_space(3.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, pal().border);
    ui.add_space(3.0);
}

/// The menu of a row of the explorer (right click).
fn file_menu(
    ui: &mut egui::Ui,
    naming: &mut Option<Naming>,
    path: &str,
    is_dir: bool,
    hidden: bool,
    locked: bool,
    actions: &mut Vec<Action>,
) {
    ui.set_width(236.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    // Where new things go: into a folder, next to a file.
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
    if !is_dir && menu_row(ui, icons::WAND_SPARKLES, "Format") {
        canvas = Some(Action::FormatFile(Some(path.to_string())));
    }
    if !locked {
        let (glyph, label) =
            if hidden { (icons::EYE, "Show on the canvas") } else { (icons::EYE_OFF, "Hide from the canvas") };
        if menu_row(ui, glyph, label) {
            canvas = Some(Action::ToggleHidden(path.to_string()));
        }
    }
    if is_dir && !hidden && menu_row(ui, icons::FOLD_VERTICAL, "Collapse or expand on the canvas") {
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

/// The field the explorer asks a name with: Enter takes it, Esc or a click
/// elsewhere gives up.
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
    // What a command answers to right now: every chord it has.
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
                ("See where a name is used", fixed("Click it (zoomed in to text)")),
                ("Select a function", fixed("Click inside it")),
                ("Deselect", fixed("Click it again, or Esc for all")),
                ("Go to the definition", key(Command::GoToDefinition)),
            ],
        ),
        (
            "Find",
            vec![
                ("Go to a file or folder", key(Command::GoToFile)),
                ("Search in files", key(Command::SearchInFiles)),
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
                ("Open a folder in a new canvas", format!("{}, or drop it", key(Command::OpenFolder))),
                (
                    "Next / previous canvas",
                    format!("{} / {}", keys.label(Command::NextFolder), keys.label(Command::PreviousFolder)),
                ),
                ("Go to canvas N", fixed("Ctrl+1 … Ctrl+9")),
                ("Close the canvas", key(Command::CloseFolder)),
                ("Reload from disk", key(Command::Rescan)),
                ("Toggle the left panel", key(Command::ToggleLeftPanel)),
                ("Folders side by side, or one at a time", key(Command::ToggleSideBySide)),
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
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(*what).size(12.5).color(pal().text));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(keys).size(12.0).color(pal().text_dim));
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
            ui.spacing_mut().slider_width = 92.0;
            let pct = |ui: &mut egui::Ui, v: f32| {
                ui.label(RichText::new(format!("{:>3.0}%", v * 100.0)).monospace().size(11.0).color(pal().text_dim));
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

/// Everything the command palette offers: the app's own commands, then
/// those of the extensions that are running.
fn palette_commands(ext: &Extensions) -> Vec<(String, Action)> {
    let mut out: Vec<(String, Action)> = vec![
        ("File: Open Folder…".into(), Action::OpenFolder),
        ("File: Save".into(), Action::SaveFile),
        ("Format Document".into(), Action::FormatFile(None)),
        ("View: Explorer".into(), Action::ShowLeft(LeftPanel::Files)),
        ("View: Search".into(), Action::ShowLeft(LeftPanel::Search)),
        ("View: Source Control".into(), Action::ShowLeft(LeftPanel::Changes)),
        ("View: Extensions".into(), Action::ShowLeft(LeftPanel::Extensions)),
        ("View: Problems".into(), Action::ShowLeft(LeftPanel::Problems)),
        ("View: Keyboard Shortcuts".into(), Action::ShowLeft(LeftPanel::Help)),
        ("Preferences: Settings".into(), Action::ShowLeft(LeftPanel::Settings)),
        ("Preferences: Colour Theme".into(), Action::ShowLeft(LeftPanel::Settings)),
        ("View: Folders Side by Side, or One at a Time".into(), Action::ToggleSideBySide),
        ("View: Show the Whole Canvas".into(), Action::Fit),
        ("View: Go to the Selection".into(), Action::FrameSelection),
        ("View: Reload the Folder from Disk".into(), Action::Rescan),
        ("Canvas: Collapse All Folders".into(), Action::CollapseAll),
        ("Canvas: Expand All Folders".into(), Action::ExpandAll),
        ("Canvas: Show All Hidden Files".into(), Action::ShowAll),
        ("Source Control: Refresh".into(), Action::RefreshGit),
        ("Source Control: Show Only Changed Files".into(), Action::SetOnlyChanged(true)),
        ("Source Control: Show All Files".into(), Action::SetOnlyChanged(false)),
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
                        // Opened as the command palette (`>`): the caret goes after it.
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

                    // `>` turns the box into the command palette, as in VS Code.
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

/// A hover from a language extension (Markdown) as the two parts of the
/// canvas tooltip: the first code line as the title, the rest below it.
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
