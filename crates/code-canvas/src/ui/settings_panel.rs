//! Settings: which theme, and every colour of it.
//!
//! The panel only asks for changes (`Action::SetTheme`, `Action::SetColor`,
//! `Action::ResetColors`); the app applies and saves them (`themes.rs`).

use canvas_render::theme;
use egui::{Align, Frame, Layout, Margin, RichText};

use super::{check, pal, panel_title, section, small_icon, well, Action, UiState};
use crate::icons;

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

pub(super) fn settings_panel(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
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
        });
    });
}
