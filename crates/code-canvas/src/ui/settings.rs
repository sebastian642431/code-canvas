use canvas_render::theme;
use egui::{Align, Align2, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::{c, check, icon, pal, search_field, section, small_icon, well, Action, UiState};
use crate::icons;
use crate::keys::{self, Scope};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Tab {
    #[default]
    General,
    Appearance,
    Colours,
    Keyboard,
    Commits,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::General, Tab::Appearance, Tab::Colours, Tab::Keyboard, Tab::Commits];

    pub fn from_key(key: &str) -> Option<Tab> {
        Some(match key {
            "general" => Tab::General,
            "appearance" => Tab::Appearance,
            "colours" => Tab::Colours,
            "keyboard" => Tab::Keyboard,
            "commits" => Tab::Commits,
            _ => return None,
        })
    }

    fn title(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Appearance => "Appearance",
            Tab::Colours => "Colours",
            Tab::Keyboard => "Keyboard",
            Tab::Commits => "Commit messages",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Tab::General => icons::INFO,
            Tab::Appearance => icons::PALETTE,
            Tab::Colours => icons::LAYOUT_GRID,
            Tab::Keyboard => icons::CORNER_DOWN_LEFT,
            Tab::Commits => icons::SPARKLES,
        }
    }
}

#[derive(Default)]
pub struct SettingsView {
    pub open: bool,
    pub tab: Tab,
    filter: String,
    focus: bool,
    skips: Option<[String; 2]>,
    folded: std::collections::HashSet<&'static str>,
    only_changed: bool,
    new_name: String,
    mine: Option<Vec<crate::themes::Mine>>,
    previews: std::collections::HashMap<String, Option<theme::Theme>>,
}

impl SettingsView {
    pub fn show(&mut self, tab: Option<Tab>) {
        self.open = true;
        self.focus = true;
        if let Some(tab) = tab {
            self.tab = tab;
            self.filter.clear();
        }
    }

    pub fn toggle(&mut self) {
        match self.open {
            true => self.open = false,
            false => self.show(None),
        }
    }

    pub fn search(&mut self, text: &str) {
        self.show(None);
        self.filter = text.to_string();
    }

    pub fn forget_mine(&mut self) {
        self.mine = None;
    }

    fn mine(&mut self) -> &[crate::themes::Mine] {
        self.mine.get_or_insert_with(crate::themes::mine)
    }
}

pub fn theme_name(st: &mut UiState, ext: &crate::ext::Extensions, key: &str) -> String {
    if let Some(preset) = theme::preset_named(key) {
        return preset.name.to_string();
    }
    if let Some(found) = ext.color_themes().find(|t| t.key == key) {
        return found.label.clone();
    }
    if let Some(found) = st.settings.mine().iter().find(|m| m.key == key) {
        return found.name.clone();
    }
    theme::PRESETS[0].name.to_string()
}

pub fn ask_reset(st: &mut UiState, ext: &crate::ext::Extensions) {
    let n = st.theme.changed();
    if n == 0 {
        return;
    }
    let key = st.theme.base().0.to_string();
    let name = theme_name(st, ext, &key);
    let what =
        if n == 1 { "The colour you changed goes".to_string() } else { format!("The {n} colours you changed go") };
    let text = format!("{what} back to {name}'s own. Save them as a theme of yours first to keep them.");
    let ask = super::Ask::new(super::Asked::ResetColours, &format!("Reset every colour to {name}?"), text)
        .choice("Reset", super::ChoiceLook::Danger, super::Answer::Yes)
        .choice("Cancel", super::ChoiceLook::Plain, super::Answer::Cancel);
    st.ask = Some(ask);
}

const HEAD: f32 = 46.0;
const NAV: f32 = 190.0;

const EDITING: [(&str, &str); 10] = [
    (
        "Show newer versions of dependencies",
        "In package.json, Cargo.toml, pyproject.toml, requirements.txt, .csproj, go.mod, composer.json and pubspec.yaml. Asks each one's registry; only package names are sent, and a scope a .npmrc gives to another registry goes only there. Click a version to update to it.",
    ),
    ("Offer file names while typing a path", "Inside quotes, after ./ or ../ or a folder name and a slash."),
    (
        "Follow what an assistant is writing",
        "Its panel stays scrolled to the end while it answers. Scroll up to stop following.",
    ),
    (
        "Travel to what is changed from outside",
        "When another program changes a file, the canvas goes to the lines that changed and lights them. Moving the canvas yourself holds it for a moment; it never moves while you edit. Where an assistant works, it goes only while you follow that assistant.",
    ),
    ("Number the lines", "To the left of the code, when it is close enough to read."),
    (
        "Save while typing",
        "Each file is written to disk a moment after you stop typing, so nothing is ever left unsaved. Off, Ctrl+S saves.",
    ),
    (
        "Offer code while typing",
        "What the language's extension knows fits there (rust-analyzer, TypeScript and the like). Enter or Tab takes it.",
    ),
    (
        "Show code lenses",
        "The small links extensions put above a function, such as \"3 references\" or \"Run\". A click on one does what it says.",
    ),
    (
        "Wrap long lines inside their card",
        "A line wider than its card goes on in the rows under it, a little indented, and the card grows taller. Off, it is cut at the edge and the rest shows beside the card.",
    ),
    (
        "Show prereleases of dependencies",
        "Next to the latest version, the newest alpha, beta or release candidate when one is newer, and it is offered when you update. Always on for a dependency that already asks for a prerelease.",
    ),
];

const SYMBOLS_PANEL: (&str, &str) = (
    "Open Selected symbols when a name is clicked",
    "The panel with the filters and the uses of the names you click. Off, a click still draws the curves on the canvas, and the panel opens from its icon.",
);

const MINIMAP: (&str, &str) = (
    "Show a minimap of the selected file",
    "A narrow strip on the right of the canvas with the whole file in small, its changes and problems, and the part on screen. Click or drag on it to move along the file.",
);

const BLAME_LINE: (&str, &str) = (
    "Show who last changed the line being edited",
    "Faint, at the end of the line with the caret: who changed it, when, and the commit's subject. A line changed and not committed says so.",
);

const LAST_CHANGE: (&str, &str) = (
    "Show when each file was last changed",
    "Next to the name of each card, and in the status bar for the selected file: who made the last commit that touched it and when, or that it changed and is not committed.",
);

const TRAILS: (&str, &str) = (
    "Show where each assistant has been",
    "Its last places, faint in its colour and joined by a dotted line: a ring where it read, a dot where it edited. They fade out in ten minutes. Point at one to see what it did there.",
);

const GRAMMARS: (&str, &str) = (
    "Colour every language with its extension's grammar",
    "A language Hawk Canvas does not know is always coloured by the grammar its extension brings. On, so are the ones it knows (Rust, TypeScript, Python…), when an extension brings a grammar for them.",
);

const RELAY: (&str, &str) = (
    "Share through a relay",
    "For sharing with someone on another network, with no VPN and no port forwarded: both apps connect out to the relay, and its invitation starts with cc2:. Hawk Canvas has no relay of its own and no account: run canvas-relay on a server with a public address, or use one you trust. The session crosses it sealed with the key in the invitation, which the relay never has.",
);

const UPDATES: (&str, &str) =
    ("Look for new versions of Hawk Canvas", "Asks GitHub for the latest release when the app starts.");

const PROVIDERS: [(&str, &str); 4] = [
    ("auto", "Claude Code, or Codex without it"),
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("custom", "A command of mine"),
];

fn hit(filter: &str, texts: &[&str]) -> bool {
    filter.is_empty() || texts.iter().any(|text| text.to_lowercase().contains(filter))
}

fn group_icon(group: &str) -> &'static str {
    match group {
        "Interface" => icons::LAYOUT_GRID,
        "Canvas" => icons::SCAN,
        "Code" => icons::CODE,
        "Relations" => icons::WAYPOINTS,
        "Status" => icons::ACTIVITY,
        _ => icons::PALETTE,
    }
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal().text_faint)).wrap());
}

fn option(ui: &mut egui::Ui, filter: &str, on: &mut bool, (label, about): (&str, &str)) {
    if !hit(filter, &[label, about]) {
        return;
    }
    check(ui, on, label, true, false);
    let width = (ui.available_width() - 24.0).max(80.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_max_width(width);
            ui.add(egui::Label::new(RichText::new(about).size(11.5).color(pal().text_faint)).wrap());
        });
    });
    ui.add_space(4.0);
}

pub(super) fn settings_modal(
    ctx: &egui::Context,
    st: &mut UiState,
    ext: &crate::ext::Extensions,
    actions: &mut Vec<Action>,
) {
    if !st.settings.open {
        return;
    }
    let screen = ctx.content_rect();
    st.floating.push(screen);
    st.page_places.clear();
    let size =
        Vec2::new((screen.width() - 80.0).clamp(420.0, 960.0), (screen.height() - 90.0).clamp(320.0, 700.0)).floor();
    let frame = Frame::NONE
        .fill(pal().card)
        .stroke(Stroke::new(1.0, pal().border_strong))
        .corner_radius(CornerRadius::same(12));
    let mut filter = std::mem::take(&mut st.settings.filter);
    let (mut close, mut picked) = (false, None);

    let modal =
        egui::Modal::new(egui::Id::unique("settings")).backdrop_color(super::backdrop()).frame(frame).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let (head, body) = rect.split_top_bottom_at_y(rect.top() + HEAD);
            let (nav, content) = body.split_left_right_at_x(body.left() + NAV);
            ui.painter().hline(rect.x_range(), head.bottom(), Stroke::new(1.0, pal().border));
            ui.painter().vline(nav.right(), body.y_range(), Stroke::new(1.0, pal().border));

            let across = Layout::left_to_right(Align::Center);
            let mut top =
                ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(Vec2::new(14.0, 8.0))).layout(across));
            top.label(icon(icons::SETTINGS, 15.0, pal().text_dim));
            top.label(RichText::new("Settings").size(15.0).color(pal().text));
            top.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if small_icon(ui, icons::X, "Close (Esc)") {
                    close = true;
                }
                ui.add_space(6.0);
                ui.allocate_ui_with_layout(Vec2::new(300.0, 28.0), Layout::top_down(Align::Min), |ui| {
                    let edit = search_field(ui, &mut filter, "Search settings", icons::SEARCH);
                    if st.settings.focus {
                        edit.request_focus();
                        st.settings.focus = false;
                    }
                });
            });

            let wanted = filter.trim().to_lowercase();
            let searching = !wanted.is_empty();
            let down = Layout::top_down(Align::Min);
            let mut side = ui.new_child(egui::UiBuilder::new().max_rect(nav.shrink(8.0)).layout(down));
            side.spacing_mut().item_spacing.y = 2.0;
            for tab in Tab::ALL {
                let (row, resp) = side.allocate_exact_size(Vec2::new(side.available_width(), 30.0), Sense::click());
                let on = !searching && st.settings.tab == tab;
                if on {
                    side.painter().rect_filled(row, 7.0, pal().raised);
                } else if resp.hovered() {
                    side.painter().rect_filled(row, 7.0, pal().hover);
                }
                let color = if on || resp.hovered() { pal().text } else { pal().text_dim };
                let at = |x: f32| Pos2::new(row.left() + x, row.center().y);
                side.painter().text(at(10.0), Align2::LEFT_CENTER, tab.glyph(), FontId::proportional(15.0), color);
                side.painter().text(at(36.0), Align2::LEFT_CENTER, tab.title(), FontId::proportional(13.0), color);
                if resp.clicked() {
                    picked = Some(tab);
                }
            }

            let room = Rect::from_min_max(content.min + Vec2::new(16.0, 6.0), content.max - Vec2::new(4.0, 6.0));
            let mut main = ui.new_child(egui::UiBuilder::new().max_rect(room).layout(down));
            let salt = if searching { None } else { Some(st.settings.tab) };
            egui::ScrollArea::vertical().id_salt(("settings", salt)).auto_shrink([false, false]).show(
                &mut main,
                |ui| {
                    Frame::NONE.inner_margin(Margin { left: 0, right: 14, top: 2, bottom: 10 }).show(ui, |ui| {
                        let f = wanted.as_str();
                        let any = match (searching, st.settings.tab) {
                            (true, _) => {
                                general(ui, st, f, actions)
                                    | appearance(ui, st, ext, f, actions)
                                    | colours(ui, st, ext, f, actions)
                                    | keyboard(ui, st, f, actions)
                                    | commits(ui, st, f, actions)
                            }
                            (false, Tab::General) => general(ui, st, f, actions),
                            (false, Tab::Appearance) => appearance(ui, st, ext, f, actions),
                            (false, Tab::Colours) => colours(ui, st, ext, f, actions),
                            (false, Tab::Keyboard) => keyboard(ui, st, f, actions),
                            (false, Tab::Commits) => commits(ui, st, f, actions),
                        };
                        if !any {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(format!("No setting matches “{}”.", filter.trim())).color(pal().text_dim),
                            );
                        }
                    });
                },
            );
        });

    st.settings.filter = filter;
    if let Some(tab) = picked {
        st.settings.tab = tab;
        st.settings.filter.clear();
    }
    if close || modal.backdrop_response.clicked() {
        st.settings.open = false;
        st.rebinding = None;
    }
}

fn general(ui: &mut egui::Ui, st: &mut UiState, f: &str, actions: &mut Vec<Action>) -> bool {
    let mut next = st.prefs.clone();
    let about = hit(f, &["Hawk Canvas", "version", "update", UPDATES.0, UPDATES.1]);
    if about {
        section(ui, icons::INFO, "Hawk Canvas");
        well(ui, |ui| {
            use crate::update::State;
            ui.label(RichText::new(format!("Version {}", crate::update::current())).size(13.0).color(pal().text));
            let (text, color) = match &st.update {
                State::Unknown if st.prefs.check_updates => {
                    ("It has not looked for a newer version yet.".to_string(), pal().text_faint)
                }
                State::Unknown => ("Looking for new versions is switched off.".to_string(), pal().text_faint),
                State::Checking => ("Looking for a newer version…".to_string(), pal().text_dim),
                State::UpToDate => ("This is the latest version.".to_string(), pal().text_dim),
                State::Available(new) => (format!("Version {new} is available."), pal().accent),
                State::Installing(new) => {
                    (format!("Downloading version {new}; the app restarts when it is ready."), pal().accent)
                }
                State::Failed(why) => (format!("Could not look for a newer version: {why}"), pal().text_dim),
            };
            ui.label(RichText::new(text).size(12.0).color(color));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if matches!(st.update, State::Available(_)) && super::ext_panels::pill(ui, "Update and restart", true) {
                    actions.push(Action::InstallUpdate);
                }
                let busy = matches!(st.update, State::Checking | State::Installing(_));
                if !busy && super::ext_panels::pill(ui, "Check for updates", false) {
                    actions.push(Action::CheckUpdates);
                }
            });
            ui.add_space(6.0);
            option(ui, "", &mut next.check_updates, UPDATES);
        });
    }
    let editing = EDITING
        .iter()
        .chain([&SYMBOLS_PANEL, &MINIMAP, &BLAME_LINE, &LAST_CHANGE])
        .any(|(label, about)| hit(f, &[label, about, "Help while editing"]));
    if editing {
        let f = if hit(f, &["Help while editing"]) { "" } else { f };
        section(ui, icons::PACKAGE, "Help while editing");
        well(ui, |ui| {
            option(ui, f, &mut next.check_versions, EDITING[0]);
            if next.check_versions {
                option(ui, f, &mut next.version_prereleases, EDITING[9]);
            }
            option(ui, f, &mut next.complete_paths, EDITING[1]);
            option(ui, f, &mut next.complete_code, EDITING[5]);
            option(ui, f, &mut next.follow_chat, EDITING[2]);
            option(ui, f, &mut next.follow_changes, EDITING[3]);
            option(ui, f, &mut next.line_numbers, EDITING[4]);
            option(ui, f, &mut next.wrap, EDITING[8]);
            option(ui, f, &mut next.minimap, MINIMAP);
            option(ui, f, &mut next.autosave, EDITING[6]);
            option(ui, f, &mut next.code_lenses, EDITING[7]);
            option(ui, f, &mut next.symbols_panel, SYMBOLS_PANEL);
            option(ui, f, &mut next.blame_line, BLAME_LINE);
            option(ui, f, &mut next.last_change, LAST_CHANGE);
        });
    }
    let assistants =
        ["Assistants on the canvas", "Claude Code", "Codex", "agents", "avatar", "hooks", "trail", TRAILS.0];
    let seen = hit(f, &assistants);
    if seen {
        section(ui, icons::EYE, "Assistants on the canvas");
        well(ui, |ui| {
            note(ui, "Each assistant working on your files shows on the canvas with a face of its own: where it reads, what it searches and what it changes. Click a face at the top to follow it.");
            ui.add_space(4.0);
            option(ui, "", &mut next.trails, TRAILS);
            let connected = st.claude_connected;
            let state =
                if connected { "Claude Code tells the canvas what it does." } else { "Claude Code is not connected." };
            ui.label(RichText::new(state).size(12.5).color(pal().text));
            ui.add_space(2.0);
            if super::ext_panels::pill(
                ui,
                if connected { "Disconnect Claude Code" } else { "Connect Claude Code" },
                !connected,
            ) {
                actions.push(Action::Agent(super::AgentOp::ConnectClaude(!connected)));
            }
            ui.add_space(4.0);
            note(ui, "Connecting adds a hook to Claude Code's own settings (~/.claude/settings.json, a copy of the file is kept beside it) that runs after each tool it uses and says which file it touched. Nothing leaves this computer. Every session started anywhere, and each of its agents, shows here while the app is open.");
            ui.add_space(8.0);
            let codex = next.codex;
            let state = if codex { "Codex tells the canvas what it does." } else { "Codex is not connected." };
            ui.label(RichText::new(state).size(12.5).color(pal().text));
            ui.add_space(2.0);
            if super::ext_panels::pill(ui, if codex { "Disconnect Codex" } else { "Connect Codex" }, !codex) {
                next.codex = !codex;
            }
            ui.add_space(4.0);
            note(ui, "Connecting reads the session files Codex writes as it works (~/.codex/sessions, or CODEX_HOME), in a terminal, with codex exec or in its editor extension, and shows each session where it reads, searches and edits. Codex's own settings are not touched and nothing leaves this computer.");
        });
    }
    let sharing = hit(f, &["Sharing", "relay", "invitation", RELAY.0, RELAY.1]);
    if sharing {
        section(ui, icons::NETWORK, "Sharing");
        well(ui, |ui| {
            option(ui, "", &mut next.share_relay, RELAY);
            ui.label(RichText::new("The relay's address").size(12.5).color(pal().text));
            let field = egui::TextEdit::singleline(&mut next.relay)
                .hint_text("relay.example.org:7878")
                .desired_width(f32::INFINITY)
                .font(FontId::monospace(12.0));
            ui.add(field);
            ui.add_space(2.0);
            note(ui, "A name or an address, and the port (7878 when none is written). Sharing directly, on the same network or through a VPN, keeps working as before when this is off.");
        });
    }
    let never = ["Folders never read", "Files never read", "skip", "ignore", "node_modules"];
    let reading = hit(f, &never);
    if reading {
        section(ui, icons::EYE_OFF, "Never read");
        let defaults = canvas_core::ScanOptions::default();
        let listed = |own: &Option<Vec<String>>, usual: &[String]| own.as_deref().unwrap_or(usual).join("  ");
        let typed = st.settings.skips.get_or_insert_with(|| {
            [listed(&next.skip_dirs, &defaults.skip_dirs), listed(&next.skip_files, &defaults.skip_files)]
        });
        let (mut done, mut usual) = (false, false);
        well(ui, |ui| {
            note(ui, "Besides what git ignores. Names separated by spaces; a folder with one of these names is not entered anywhere in a folder. Leaving a box takes it.");
            for (label, text) in ["Folders never read", "Files never read"].into_iter().zip(typed.iter_mut()) {
                ui.add_space(4.0);
                ui.label(RichText::new(label).size(12.5).color(pal().text));
                let edit = egui::TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY);
                done |= ui.add(edit.font(FontId::monospace(12.0))).lost_focus();
            }
            ui.add_space(4.0);
            usual = super::ext_panels::pill(ui, "The usual ones", false);
        });
        if usual {
            (next.skip_dirs, next.skip_files) = (None, None);
            st.settings.skips = None;
        } else if let (true, Some(typed)) = (done, &st.settings.skips) {
            let names = |text: &str| text.split_whitespace().map(str::to_string).collect::<Vec<_>>();
            let own = |text: &str, usual: &[String]| Some(names(text)).filter(|own| own.as_slice() != usual);
            next.skip_dirs = own(&typed[0], &defaults.skip_dirs);
            next.skip_files = own(&typed[1], &defaults.skip_files);
        }
    }
    if next != st.prefs {
        actions.push(Action::SetPrefs(next));
    }
    about || editing || reading || seen || sharing
}

fn appearance(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &crate::ext::Extensions,
    f: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let shown_themes = theme_picker(ui, st, ext, f, actions);
    let highlighting = hit(f, &[GRAMMARS.0, GRAMMARS.1, "grammar", "highlighting"]);
    if highlighting {
        section(ui, icons::CODE, "Highlighting");
        well(ui, |ui| {
            let mut next = st.prefs.clone();
            option(ui, "", &mut next.grammars_everywhere, GRAMMARS);
            if next != st.prefs {
                actions.push(Action::SetPrefs(next));
            }
        });
    }
    let icons_shown = hit(f, &["File icons", "icon theme", "Built in"])
        || ext.icon_themes().any(|theme| hit(f, &[theme.label.as_str()]));
    if icons_shown {
        section(ui, icons::FILE, "File icons");
        well(ui, |ui| {
            let current = ext.icon_theme(&st.theme.icons).map(|t| t.id.clone());
            for theme in ext.icon_themes() {
                let mut on = current.as_deref() == Some(theme.id.as_str());
                if check(ui, &mut on, &theme.label, true, false).clicked() && on {
                    actions.push(Action::SetIconTheme(theme.id.clone()));
                }
            }
            let mut own = current.is_none();
            if check(ui, &mut own, "Built in", true, false).clicked() && own {
                actions.push(Action::SetIconTheme("none".to_string()));
            }
            if ext.icon_themes().next().is_none() {
                note(ui, "Install an icon theme extension (Symbols, Material Icon Theme) for more.");
            }
        });
    }
    shown_themes || icons_shown || highlighting
}

const TILE: Vec2 = Vec2::new(172.0, 116.0);
const PREVIEW_H: f32 = 90.0;

struct Choice {
    key: String,
    name: String,
    about: String,
    theme: Option<theme::Theme>,
    mine: bool,
}

enum Picked {
    Nothing,
    Theme,
    Delete,
}

fn theme_picker(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &crate::ext::Extensions,
    f: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let base_key = st.theme.base().0.to_string();
    let all = hit(f, &["Theme", "Colour theme", "Color theme"]);
    let mut groups: Vec<(&str, Vec<Choice>)> = Vec::new();
    for kind in theme::Kind::ALL {
        let title = kind.title();
        let list = theme::PRESETS
            .iter()
            .filter(|p| p.kind == kind && (all || hit(f, &[p.name, p.about, title])))
            .map(|p| Choice {
                key: p.key.to_string(),
                name: p.name.to_string(),
                about: p.about.to_string(),
                theme: Some(p.theme),
                mine: false,
            })
            .collect();
        groups.push((title, list));
    }
    let mut theirs = Vec::new();
    for decl in ext.color_themes().filter(|t| all || hit(f, &[t.label.as_str(), "From extensions"])) {
        let made = *st
            .settings
            .previews
            .entry(decl.key.clone())
            .or_insert_with(|| crate::themes::from_vscode(&decl.file, decl.light));
        let about = String::from("From an installed extension.");
        theirs.push(Choice { key: decl.key.clone(), name: decl.label.clone(), about, theme: made, mine: false });
    }
    groups.push(("From extensions", theirs));
    let mine = st
        .settings
        .mine()
        .iter()
        .filter(|m| all || hit(f, &[m.name.as_str(), "Mine", "my theme"]))
        .map(|m| Choice {
            key: m.key.clone(),
            name: m.name.clone(),
            about: String::from("Yours, saved from Colours."),
            theme: Some(m.theme),
            mine: true,
        })
        .collect();
    groups.push(("Mine", mine));
    if groups.iter().all(|(_, list)| list.is_empty()) {
        return false;
    }
    let mut delete = None;
    section(ui, icons::PALETTE, "Theme");
    well(ui, |ui| {
        for (title, list) in groups.iter().filter(|(_, list)| !list.is_empty()) {
            ui.add_space(2.0);
            ui.label(RichText::new(*title).size(11.5).color(pal().text_faint));
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                for choice in list {
                    match tile(ui, choice, choice.key == base_key) {
                        Picked::Theme if choice.key != base_key => actions.push(Action::SetTheme(choice.key.clone())),
                        Picked::Delete => delete = Some((choice.key.clone(), choice.name.clone())),
                        _ => {}
                    }
                }
            });
            ui.add_space(6.0);
        }
        if ext.color_themes().next().is_none() {
            note(ui, "A colour theme installed as an extension is listed here too.");
        }
        note(ui, "Every colour of a theme can be changed under Colours, and saved there as a theme of yours.");
    });
    if let Some((key, name)) = delete {
        let text = format!("{name} is deleted from this computer. Themes that come with Hawk Canvas stay.");
        let ask = super::Ask::new(super::Asked::DeleteTheme { key }, &format!("Delete the theme {name}?"), text)
            .choice("Delete", super::ChoiceLook::Danger, super::Answer::Yes)
            .choice("Cancel", super::ChoiceLook::Plain, super::Answer::Cancel);
        st.ask = Some(ask);
    }
    true
}

fn tile(ui: &mut egui::Ui, choice: &Choice, on: bool) -> Picked {
    let (rect, resp) = ui.allocate_exact_size(TILE, Sense::click());
    if !ui.is_rect_visible(rect) {
        return Picked::Nothing;
    }
    let preview = Rect::from_min_size(rect.min, Vec2::new(TILE.x, PREVIEW_H));
    let painter = ui.painter().with_clip_rect(rect.expand(3.0));
    match &choice.theme {
        Some(t) => draw_preview(&painter, preview, t),
        None => {
            painter.rect_filled(preview, 8.0, pal().raised);
            let font = FontId::proportional(11.5);
            painter.text(preview.center(), Align2::CENTER_CENTER, "Could not be read", font, pal().text_faint);
        }
    }
    let ring = match (on, resp.hovered()) {
        (true, _) => Stroke::new(2.0, pal().accent),
        (false, true) => Stroke::new(1.5, pal().border_strong),
        (false, false) => Stroke::new(1.0, pal().border),
    };
    painter.rect_stroke(preview, 8.0, ring, egui::StrokeKind::Outside);
    let mut x = rect.left() + 2.0;
    let y = preview.bottom() + 13.0;
    if on {
        painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, icons::CHECK, FontId::proportional(13.0), pal().accent);
        x += 18.0;
    }
    let color = if on || resp.hovered() { pal().text } else { pal().text_dim };
    painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, &choice.name, FontId::proportional(12.5), color);
    let bin = Rect::from_min_size(Pos2::new(preview.right() - 28.0, preview.top() + 4.0), Vec2::splat(24.0));
    let over_bin = resp.hover_pos().is_some_and(|at| bin.contains(at));
    if choice.mine && resp.hovered() {
        painter.rect_filled(bin, 6.0, if over_bin { pal().hover } else { pal().card });
        painter.rect_stroke(bin, 6.0, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
        let tint = if over_bin { c(theme::get().problem_error) } else { pal().text_dim };
        painter.text(bin.center(), Align2::CENTER_CENTER, icons::TRASH_2, FontId::proportional(13.0), tint);
    }
    let resp = match (choice.mine && over_bin, choice.about.is_empty()) {
        (true, _) => resp.on_hover_text("Delete this theme"),
        (false, false) => resp.on_hover_text(&choice.about),
        (false, true) => resp,
    };
    match (resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked(), choice.mine && over_bin) {
        (true, true) => Picked::Delete,
        (true, false) => Picked::Theme,
        _ => Picked::Nothing,
    }
}

fn draw_preview(p: &egui::Painter, r: Rect, t: &theme::Theme) {
    let bar = |x: f32, y: f32, w: f32, color: u32| {
        p.rect_filled(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w.max(1.0), 3.0)), 1.5, c(color));
    };
    p.rect_filled(r, 8.0, c(t.chrome | 0xff));
    for (i, color) in [t.accent, t.text_dim, t.text_dim, t.text_faint].into_iter().enumerate() {
        p.circle_filled(Pos2::new(r.left() + 9.0, r.top() + 14.0 + i as f32 * 12.0), 2.6, c(color));
    }
    let canvas = Rect::from_min_max(r.min + Vec2::new(18.0, 6.0), Pos2::new(r.right() - 52.0, r.bottom() - 6.0));
    p.rect_filled(canvas, 6.0, c(t.background | 0xff));
    let folder = canvas.shrink(6.0);
    p.rect_filled(folder, 4.0, c(t.dir_fill));
    p.rect_stroke(folder, 4.0, Stroke::new(1.0, c(t.dir_border_0)), egui::StrokeKind::Inside);
    bar(folder.left() + 5.0, folder.top() + 4.0, 24.0, t.label_dir);

    let card = Rect::from_min_size(
        Pos2::new(folder.left() + 5.0, folder.top() + 11.0),
        Vec2::new(folder.width() * 0.64, folder.height() - 15.0),
    );
    p.rect_filled(card.expand(2.0), 4.0, c(t.select_fill));
    p.rect_filled(card, 3.0, c(t.card_fill));
    p.rect_stroke(card, 3.0, Stroke::new(1.0, c(t.card_border)), egui::StrokeKind::Inside);
    let rows: [&[(u32, f32)]; 7] = [
        &[(t.tok_comment, 30.0)],
        &[(t.tok_keyword, 7.0), (t.tok_function, 14.0), (t.tok_punct, 4.0)],
        &[(t.tok_keyword, 6.0), (t.tok_ident, 9.0), (t.tok_punct, 3.0), (t.tok_string, 16.0)],
        &[(t.tok_type, 11.0), (t.tok_punct, 3.0), (t.tok_number, 6.0)],
        &[(t.tok_macro, 12.0), (t.tok_string, 12.0)],
        &[(t.tok_punct, 4.0)],
        &[(t.tok_heading, 10.0), (t.tok_text, 14.0)],
    ];
    let room = card.right() - 3.0;
    for (i, row) in rows.iter().enumerate() {
        let y = card.top() + 4.0 + i as f32 * 6.0;
        if y + 3.0 > card.bottom() - 2.0 {
            break;
        }
        let mut x = card.left() + if (2..5).contains(&i) { 9.0 } else { 4.0 };
        if i == 2 {
            p.rect_filled(
                Rect::from_min_max(Pos2::new(card.left() + 1.0, y - 1.5), Pos2::new(room, y + 4.5)),
                0.0,
                c(t.current_line),
            );
        }
        if i == 3 {
            p.rect_filled(
                Rect::from_min_size(Pos2::new(x - 1.0, y - 1.5), Vec2::new(16.0, 6.0)),
                1.0,
                c(t.code_selection),
            );
        }
        for &(color, w) in row.iter() {
            let w = w.min(room - x);
            if w <= 0.0 {
                break;
            }
            bar(x, y, w, color);
            x += w + 2.5;
        }
        if i == 2 {
            p.rect_filled(Rect::from_min_size(Pos2::new(x.min(room), y - 1.5), Vec2::new(1.2, 6.0)), 0.0, c(t.caret));
        }
    }
    p.rect_stroke(card.expand(2.0), 4.0, Stroke::new(1.2, c(t.select)), egui::StrokeKind::Inside);

    let other = Rect::from_min_max(
        Pos2::new(card.right() + 6.0, card.top() + 2.0),
        Pos2::new(folder.right() - 5.0, card.top() + card.height() * 0.62),
    );
    if other.width() > 8.0 {
        p.rect_filled(other, 3.0, c(t.card_fill));
        p.rect_stroke(other, 3.0, Stroke::new(1.0, c(t.card_border)), egui::StrokeKind::Inside);
        p.rect_filled(Rect::from_min_size(other.left_top(), Vec2::new(2.0, other.height())), 0.0, c(t.git_modified));
        for (i, color) in [t.tok_string, t.tok_ident, t.tok_function].into_iter().enumerate() {
            bar(other.left() + 5.0, other.top() + 4.0 + i as f32 * 6.0, other.width() - 9.0 - i as f32 * 3.0, color);
        }
        let y = other.bottom() + 7.0;
        p.line_segment(
            [Pos2::new(other.center().x, other.bottom()), Pos2::new(other.center().x, y + 4.0)],
            Stroke::new(1.0, c(t.dep_out)),
        );
        p.circle_filled(Pos2::new(other.center().x, y + 5.0), 2.0, c(t.rel_def));
    }

    let panel =
        Rect::from_min_max(Pos2::new(r.right() - 47.0, r.top() + 6.0), Pos2::new(r.right() - 6.0, r.bottom() - 6.0));
    p.rect_filled(panel, 5.0, c(t.card | 0xff));
    p.rect_stroke(panel, 5.0, Stroke::new(1.0, c(t.border)), egui::StrokeKind::Inside);
    let left = panel.left() + 5.0;
    bar(left, panel.top() + 6.0, 24.0, t.text);
    let chosen =
        Rect::from_min_size(Pos2::new(panel.left() + 2.0, panel.top() + 13.0), Vec2::new(panel.width() - 4.0, 8.0));
    p.rect_filled(chosen, 2.0, c(t.raised));
    bar(left, panel.top() + 15.5, 20.0, t.text);
    bar(left, panel.top() + 25.0, 26.0, t.text_dim);
    bar(left, panel.top() + 32.0, 16.0, t.text_faint);
    for (i, color) in [t.git_added, t.git_modified, t.problem_error].into_iter().enumerate() {
        p.circle_filled(Pos2::new(left + 2.0 + i as f32 * 8.0, panel.top() + 42.0), 2.2, c(color));
    }
    let button = Rect::from_min_size(Pos2::new(left, panel.bottom() - 15.0), Vec2::new(panel.width() - 10.0, 10.0));
    p.rect_filled(button, 3.0, c(t.accent));
    bar(button.left() + 6.0, button.center().y - 1.5, button.width() - 12.0, t.on_accent);
}

fn group_header(ui: &mut egui::Ui, group: &str, folded: bool, count: usize, changed: usize) -> bool {
    ui.add_space(4.0);
    let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(row, 6.0, pal().row);
    }
    let at = |x: f32| Pos2::new(row.left() + x, row.center().y);
    let p = ui.painter();
    let chevron = if folded { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN };
    p.text(at(6.0), Align2::LEFT_CENTER, chevron, FontId::proportional(13.0), pal().text_dim);
    p.text(at(24.0), Align2::LEFT_CENTER, group_icon(group), FontId::proportional(14.0), pal().text_dim);
    let title = p.text(at(44.0), Align2::LEFT_CENTER, group, FontId::proportional(13.5), pal().text);
    let what = if count == 1 { "1 colour".to_string() } else { format!("{count} colours") };
    let font = FontId::proportional(11.5);
    let after = p.text(
        Pos2::new(title.right() + 8.0, row.center().y),
        Align2::LEFT_CENTER,
        what,
        font.clone(),
        pal().text_faint,
    );
    if changed > 0 {
        let text = format!("· {changed} changed");
        p.text(Pos2::new(after.right() + 5.0, row.center().y), Align2::LEFT_CENTER, text, font, pal().accent);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn colours(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &crate::ext::Extensions,
    f: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let all = hit(f, &["Colours", "colors"]);
    let current = theme::get();
    let fields: Vec<&theme::Field> = theme::FIELDS
        .iter()
        .filter(|field| {
            let hex = theme::to_hex((field.get)(&current));
            all || hit(f, &[field.label, field.group, field.key, &hex])
        })
        .filter(|field| !st.settings.only_changed || st.theme.is_changed(field.key))
        .collect();
    let bar_shown = f.is_empty() || all;
    if fields.is_empty() && !bar_shown {
        return false;
    }
    let (base_key, base) = {
        let (key, colors) = st.theme.base();
        (key.to_string(), colors)
    };
    let name = theme_name(st, ext, &base_key);
    let changed = st.theme.changed();
    let mut reset = false;
    if bar_shown {
        ui.add_space(6.0);
        well(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icon(icons::PALETTE, 14.0, pal().text_dim));
                ui.label(RichText::new(format!("Based on {name}")).size(13.0).color(pal().text));
                let (what, color) = match changed {
                    0 => ("nothing changed".to_string(), pal().text_faint),
                    1 => ("1 colour changed".to_string(), pal().accent),
                    n => (format!("{n} colours changed"), pal().accent),
                };
                ui.label(RichText::new(format!("· {what}")).size(12.0).color(color));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if changed > 0 && super::ext_panels::pill(ui, &format!("Reset all to {name}"), false) {
                        reset = true;
                    }
                    let enabled = changed > 0 || st.settings.only_changed;
                    check(ui, &mut st.settings.only_changed, "Only the changed ones", enabled, true);
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Save as a theme of mine").size(12.0).color(pal().text_dim));
                let hint = format!("My {name}");
                let field = egui::TextEdit::singleline(&mut st.settings.new_name).hint_text(&hint).desired_width(170.0);
                let typed = ui.add(field);
                let entered = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if super::ext_panels::pill(ui, "Save", true) || entered {
                    let wanted = st.settings.new_name.trim();
                    actions.push(Action::SaveTheme(if wanted.is_empty() { hint } else { wanted.to_string() }));
                    st.settings.new_name.clear();
                }
                ui.add_space(10.0);
                let copy = super::ext_panels::pill(ui, "Copy as JSON", false);
                if copy {
                    ui.ctx().copy_text(crate::themes::as_json(&name, &current));
                }
            });
            ui.add_space(2.0);
            note(ui, "Click a swatch to change a colour. A dot marks one changed from the theme, and the arrow beside it puts the theme's back. Saved themes are listed under Appearance.");
        });
    }
    if reset {
        ask_reset(st, ext);
    }
    if fields.is_empty() {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            note(ui, "No colour is changed from the theme yet.");
        });
        return true;
    }
    let searching = !f.is_empty();
    let mut start = 0;
    while start < fields.len() {
        let group = fields[start].group;
        let rows = &fields[start..start + fields[start..].iter().take_while(|field| field.group == group).count()];
        start += rows.len();
        let folded = !searching && st.settings.folded.contains(group);
        let in_group = rows.iter().filter(|field| st.theme.is_changed(field.key)).count();
        if group_header(ui, group, folded, rows.len(), in_group) && !searching && !st.settings.folded.remove(group) {
            st.settings.folded.insert(group);
        }
        if folded {
            continue;
        }
        let half = rows.len().div_ceil(2);
        ui.columns(2, |columns| {
            for (i, field) in rows.iter().enumerate() {
                let ui = &mut columns[usize::from(i >= half)];
                let changed = st.theme.is_changed(field.key);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let mut rgba = theme::unpack((field.get)(&current));
                    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                        actions.push(Action::SetColor(field.key, u32::from_be_bytes(rgba)));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        if changed {
                            let theirs = (field.get)(&base);
                            let tip = format!("Back to {name}'s {}", theme::to_hex(theirs));
                            if small_icon(ui, icons::HISTORY, &tip) {
                                actions.push(Action::SetColor(field.key, theirs));
                            }
                        }
                        let hex = theme::to_hex(u32::from_be_bytes(rgba));
                        ui.label(RichText::new(hex).monospace().size(11.0).color(pal().text_faint));
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            if changed {
                                let (dot, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), Sense::hover());
                                ui.painter().circle_filled(dot.center(), 3.0, pal().accent);
                            }
                            let color = if changed { pal().text } else { pal().text_dim };
                            ui.add(egui::Label::new(RichText::new(field.label).size(12.5).color(color)).truncate());
                        });
                    });
                });
            }
        });
    }
    true
}

fn keyboard(ui: &mut egui::Ui, st: &mut UiState, f: &str, actions: &mut Vec<Action>) -> bool {
    let all = hit(f, &["Keyboard shortcuts"]);
    let mut rows: Vec<&keys::CommandInfo> = keys::COMMANDS
        .iter()
        .filter(|info| {
            let chords: Vec<String> = st.keys.chords(info.command).map(|c| c.label()).collect();
            all || hit(f, &[info.title, info.group, &chords.join(" ")])
        })
        .collect();
    let place = |group: &str| keys::COMMANDS.iter().position(|info| info.group == group);
    rows.sort_by_key(|info| place(info.group));
    if rows.is_empty() {
        return false;
    }
    section(ui, icons::CORNER_DOWN_LEFT, "Keyboard shortcuts");
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        note(
            ui,
            match st.rebinding {
                Some(command) if command.info().scope == Scope::Canvas => "Press the new key. Esc to keep the old one.",
                Some(_) => "Press the new keys, with Ctrl or Alt. Esc to keep the old ones.",
                None => "Click a shortcut to change it.",
            },
        );
    });
    let mut group = "";
    for info in rows {
        if info.group != group {
            group = info.group;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(group).size(11.5).color(pal().text_faint));
            });
        }
        let waiting = st.rebinding == Some(info.command);
        let changed = st.key_file.is_changed(info.command);
        let (row, hover) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
        if waiting || hover.hovered() {
            ui.painter().rect_filled(row, 6.0, pal().row);
        }
        let inner = egui::UiBuilder::new().max_rect(row.shrink2(Vec2::new(8.0, 0.0)));
        ui.scope_builder(inner.layout(Layout::right_to_left(Align::Center)), |ui| {
            if changed && small_icon(ui, icons::HISTORY, "Back to its own shortcut") {
                actions.push(Action::ResetKey(info.command));
            }
            let has_one = st.keys.chords(info.command).next().is_some();
            if has_one && small_icon(ui, icons::X, "Leave it without a shortcut") {
                actions.push(Action::UnbindKey(info.command));
            }
            let all: Vec<String> = st.keys.chords(info.command).map(|c| c.label()).collect();
            let (text, color) = match (waiting, all.is_empty()) {
                (true, _) => ("Press keys…".to_string(), pal().accent),
                (false, true) => ("none".to_string(), pal().text_faint),
                (false, false) => (all.join("  "), if changed { pal().text } else { pal().text_dim }),
            };
            let button = egui::Button::new(RichText::new(text).size(11.5).color(color)).fill(pal().well);
            if ui.add(button).on_hover_text("Click, then press the new shortcut").clicked() {
                st.rebinding = if waiting { None } else { Some(info.command) };
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let color = if changed { pal().text } else { pal().text_dim };
                ui.add(egui::Label::new(RichText::new(info.title).size(12.5).color(color)).truncate());
            });
        });
    }
    true
}

fn commits(ui: &mut egui::Ui, st: &mut UiState, f: &str, actions: &mut Vec<Action>) -> bool {
    let about = "Who writes the message when you press the sparkle in source control.";
    if !hit(f, &["Commit messages", about]) && !PROVIDERS.iter().any(|(_, label)| hit(f, &[label])) {
        return false;
    }
    section(ui, icons::SPARKLES, "Commit messages");
    well(ui, |ui| {
        note(ui, about);
        ui.add_space(2.0);
        let mut next = st.commit_ai.clone();
        for (id, label) in PROVIDERS {
            let mut on = st.commit_ai.provider == id;
            if check(ui, &mut on, label, true, false).clicked() && on {
                next.provider = id.to_string();
            }
        }
        if next.provider == "custom" {
            ui.add_space(2.0);
            search_field(ui, &mut next.custom, "ollama run llama3", icons::CODE);
            note(ui, "It gets the request on its standard input and prints the message.");
        }
        if next != st.commit_ai {
            actions.push(Action::SetCommitAi(next));
        }
    });
    true
}
