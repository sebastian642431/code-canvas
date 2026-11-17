use canvas_render::theme;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use super::{check, icon, pal, search_field, section, small_icon, well, Action, UiState};
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
}

const HEAD: f32 = 46.0;
const NAV: f32 = 190.0;

const EDITING: [(&str, &str); 6] = [
    (
        "Show newer versions of dependencies",
        "In package.json and Cargo.toml. Asks npm and crates.io: only package names are sent.",
    ),
    ("Offer file names while typing a path", "Inside quotes, after ./ or ../ or a folder name and a slash."),
    (
        "Follow what an assistant is writing",
        "Its panel stays scrolled to the end while it answers. Scroll up to stop following.",
    ),
    (
        "Travel to what is changed from outside",
        "When an assistant or another program changes a file, the canvas goes to the lines that changed and lights them. Moving the canvas yourself holds it for a moment; it never moves while you edit.",
    ),
    ("Number the lines", "To the left of the code, when it is close enough to read."),
    (
        "Offer code while typing",
        "What the language's extension knows fits there (rust-analyzer, TypeScript and the like). Enter or Tab takes it.",
    ),
];

const UPDATES: (&str, &str) =
    ("Look for new versions of Code Canvas", "Asks GitHub for the latest release when the app starts.");

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
    ui.label(RichText::new(text).size(11.5).color(pal().text_faint));
}

fn option(ui: &mut egui::Ui, filter: &str, on: &mut bool, (label, about): (&str, &str)) {
    if !hit(filter, &[label, about]) {
        return;
    }
    check(ui, on, label, true, false);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        note(ui, about);
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

    let modal = egui::Modal::new(egui::Id::unique("settings"))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(frame)
        .show(ctx, |ui| {
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
                                    | colours(ui, st, f, actions)
                                    | keyboard(ui, st, f, actions)
                                    | commits(ui, st, f, actions)
                            }
                            (false, Tab::General) => general(ui, st, f, actions),
                            (false, Tab::Appearance) => appearance(ui, st, ext, f, actions),
                            (false, Tab::Colours) => colours(ui, st, f, actions),
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
    let about = hit(f, &["Code Canvas", "version", "update", UPDATES.0, UPDATES.1]);
    if about {
        section(ui, icons::INFO, "Code Canvas");
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
    let editing = EDITING.iter().any(|(label, about)| hit(f, &[label, about, "Help while editing"]));
    if editing {
        let f = if hit(f, &["Help while editing"]) { "" } else { f };
        section(ui, icons::PACKAGE, "Help while editing");
        well(ui, |ui| {
            option(ui, f, &mut next.check_versions, EDITING[0]);
            option(ui, f, &mut next.complete_paths, EDITING[1]);
            option(ui, f, &mut next.complete_code, EDITING[5]);
            option(ui, f, &mut next.follow_chat, EDITING[2]);
            option(ui, f, &mut next.follow_changes, EDITING[3]);
            option(ui, f, &mut next.line_numbers, EDITING[4]);
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
    about || editing || reading
}

fn appearance(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &crate::ext::Extensions,
    f: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let base_key = st.theme.base().0.to_string();
    let all = hit(f, &["Theme"]);
    let presets: Vec<(&str, &str)> = theme::PRESETS.iter().map(|(key, name, _)| (*key, *name)).collect();
    let from_extensions: Vec<(&str, &str)> = ext.color_themes().map(|t| (t.key.as_str(), t.label.as_str())).collect();
    let presets: Vec<(&str, &str)> =
        presets.into_iter().chain(from_extensions.iter().copied()).filter(|(_, name)| all || hit(f, &[name])).collect();
    if !presets.is_empty() {
        section(ui, icons::PALETTE, "Theme");
        well(ui, |ui| {
            for (key, name) in presets.iter().copied() {
                let mut on = key == base_key;
                if check(ui, &mut on, name, true, false).clicked() && key != base_key {
                    actions.push(Action::SetTheme(key.to_string()));
                }
            }
            ui.add_space(2.0);
            if from_extensions.is_empty() {
                note(ui, "A colour theme installed as an extension (Dracula, Tokyo Night, GitHub Theme…) is listed here too.");
            }
            note(ui, "Every colour of a theme can be changed under Colours.");
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
    !presets.is_empty() || icons_shown
}

fn colours(ui: &mut egui::Ui, st: &mut UiState, f: &str, actions: &mut Vec<Action>) -> bool {
    let all = hit(f, &["Colours", "colors"]);
    let fields: Vec<&theme::Field> =
        theme::FIELDS.iter().filter(|field| all || hit(f, &[field.label, field.group])).collect();
    if fields.is_empty() {
        return false;
    }
    let (_, base) = st.theme.base();
    let current = theme::get();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let changed = match st.theme.colors.len() {
            0 => "Click a colour to change it.".to_string(),
            1 => "1 colour changed.".to_string(),
            n => format!("{n} colours changed."),
        };
        ui.add_space(4.0);
        note(ui, &changed);
        if !st.theme.colors.is_empty() {
            ui.add_space(6.0);
            if super::ext_panels::pill(ui, "Back to the theme's own colours", false) {
                actions.push(Action::ResetColors);
            }
        }
    });
    let mut start = 0;
    while start < fields.len() {
        let group = fields[start].group;
        let rows = &fields[start..start + fields[start..].iter().take_while(|field| field.group == group).count()];
        start += rows.len();
        section(ui, group_icon(group), group);
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
                        if changed && small_icon(ui, icons::X, "Back to the theme's colour") {
                            actions.push(Action::SetColor(field.key, (field.get)(&base)));
                        }
                        let hex = theme::to_hex(u32::from_be_bytes(rgba));
                        ui.label(RichText::new(hex).monospace().size(11.0).color(pal().text_faint));
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
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
