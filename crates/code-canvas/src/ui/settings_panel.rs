//! Settings: which theme, every colour of it, and the keyboard shortcuts.
//!
//! The panel only asks for changes (`Action::SetTheme`, `Action::SetColor`,
//! `Action::ResetColors`, `Action::ResetKey`, `Action::UnbindKey`); the app
//! applies and saves them (`themes.rs`, `keys.rs`). A new shortcut is the
//! next key pressed after clicking one: `UiState::rebinding` says for which
//! command, and the app's key handler takes it from there.

use canvas_render::theme;
use egui::{Align, Frame, Layout, Margin, RichText};

use super::{check, pal, panel_title, search_field, section, small_icon, well, Action, UiState};
use crate::icons;
use crate::keys::{self, Scope};

/// The icon next to each group of colours (`theme::Field::group`).
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

pub(super) fn settings_panel(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &crate::ext::Extensions,
    actions: &mut Vec<Action>,
) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 10, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Settings", |_| {});
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let (base_key, base) = st.theme.base();

            section(ui, icons::PALETTE, "Theme");
            well(ui, |ui| {
                for (key, name, _) in theme::PRESETS {
                    let mut on = *key == base_key;
                    if check(ui, &mut on, name, true, false).clicked() && *key != base_key {
                        actions.push(Action::SetTheme(key));
                    }
                }
            });

            section(ui, icons::FILE, "File icons");
            well(ui, |ui| {
                // What is in use: the choice, or the theme found for it.
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
                    let note = "Install an icon theme extension (Symbols, Material Icon Theme) for more.";
                    ui.label(RichText::new(note).size(11.5).color(pal().text_faint));
                }
            });

            section(ui, icons::SPARKLES, "Commit messages");
            well(ui, |ui| {
                let note = "Who writes the message when you press the sparkle in source control.";
                ui.label(RichText::new(note).size(11.5).color(pal().text_faint));
                let mut next = st.commit_ai.clone();
                for (id, label) in [
                    ("auto", "Claude Code, or Codex without it"),
                    ("claude", "Claude Code"),
                    ("codex", "Codex"),
                    ("custom", "A command of mine"),
                ] {
                    let mut on = st.commit_ai.provider == id;
                    if check(ui, &mut on, label, true, false).clicked() && on {
                        next.provider = id.to_string();
                    }
                }
                if next.provider == "custom" {
                    search_field(ui, &mut next.custom, "ollama run llama3", icons::CODE);
                    let note = "It gets the request on its standard input and prints the message.";
                    ui.label(RichText::new(note).size(11.5).color(pal().text_faint));
                }
                if next != st.commit_ai {
                    actions.push(Action::SetCommitAi(next));
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let note = match st.theme.colors.len() {
                    0 => "Click a colour to change it.".to_string(),
                    1 => "1 colour changed.".to_string(),
                    n => format!("{n} colours changed."),
                };
                ui.add_space(4.0);
                ui.label(RichText::new(note).size(11.5).color(pal().text_faint));
                if !st.theme.colors.is_empty() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if small_icon(ui, icons::HISTORY, "Back to the theme's own colours") {
                            actions.push(Action::ResetColors);
                        }
                    });
                }
            });

            let current = theme::get();
            let mut group = "";
            for field in theme::FIELDS {
                if field.group != group {
                    group = field.group;
                    section(ui, group_icon(group), group);
                }
                let changed = st.theme.is_changed(field.key);
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let mut rgba = theme::unpack((field.get)(&current));
                    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                        actions.push(Action::SetColor(field.key, u32::from_be_bytes(rgba)));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if changed && small_icon(ui, icons::X, "Back to the theme's colour") {
                            actions.push(Action::SetColor(field.key, (field.get)(&base)));
                        }
                        let hex = theme::to_hex(u32::from_be_bytes(rgba));
                        ui.label(RichText::new(hex).monospace().size(11.0).color(pal().text_faint));
                        // The name takes what is left, cut at the end if it does not fit.
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let color = if changed { pal().text } else { pal().text_dim };
                            ui.add(egui::Label::new(RichText::new(field.label).size(12.5).color(color)).truncate());
                        });
                    });
                });
            }
            ui.add_space(8.0);
            shortcuts(ui, st, actions);
            ui.add_space(8.0);
        });
    });
}

/// Every command with its shortcut. Clicking the shortcut waits for a new
/// one; the icons give the command its own back, or leave it without any.
fn shortcuts(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    section(ui, icons::CORNER_DOWN_LEFT, "Keyboard shortcuts");
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let note = match st.rebinding {
            Some(command) if command.info().scope == Scope::Canvas => "Press the new key. Esc to keep the old one.",
            Some(_) => "Press the new keys, with Ctrl or Alt. Esc to keep the old ones.",
            None => "Click a shortcut to change it.",
        };
        ui.label(RichText::new(note).size(11.5).color(pal().text_faint));
    });
    let mut group = "";
    for info in keys::COMMANDS {
        if info.group != group {
            group = info.group;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(group).size(11.5).color(pal().text_dim));
            });
        }
        let waiting = st.rebinding == Some(info.command);
        let changed = st.key_file.is_changed(info.command);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                // The name takes what is left, cut at the end if it does not fit.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let color = if changed { pal().text } else { pal().text_dim };
                    ui.add(egui::Label::new(RichText::new(info.title).size(12.5).color(color)).truncate());
                });
            });
        });
    }
}
