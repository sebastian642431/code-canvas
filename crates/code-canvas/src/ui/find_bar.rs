//! The find bar over the canvas: files and folders by name, or a text in the file selected,
//! with the toggles, the counter and the replace row of an editor's find widget.

use egui::{Align2, CornerRadius, FontId, Frame, Key, Margin, Modifiers, Rect, RichText, Sense, Stroke, Vec2};

use super::{c, pal, small_icon, Action, UiState};
use crate::find::Options;
use crate::icons;
use crate::keys::Command;
use canvas_render::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FindIn {
    #[default]
    Files,
    File,
    Text,
}

#[derive(Default)]
pub struct Find {
    pub query: String,
    pub focus: bool,
    /// Select the whole query when it takes the focus, so that typing replaces it.
    pub select: bool,
    pub at: usize,
    pub count: usize,
    /// Whether the camera went to the match at `at` yet: the first Enter goes there, the next ones move on.
    pub landed: bool,
    /// The file selected when the bar opened, empty when there was none.
    pub file: String,
    pub mode: FindIn,
    pub error: Option<String>,
    pub history_at: Option<usize>,
}

/// What the find bar keeps from one opening to the next.
#[derive(Default)]
pub struct FindKept {
    pub options: Options,
    pub preserve: bool,
    pub replace: bool,
    pub with: String,
    pub history: Vec<String>,
}

impl FindKept {
    pub fn remember(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        self.history.retain(|seen| seen != query);
        self.history.push(query.to_string());
        if self.history.len() > 30 {
            self.history.remove(0);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FindOp {
    Open,
    OpenReplace,
    Changed,
    Next,
    Previous,
    Replace,
    ReplaceAll,
    Close,
    Mode(FindIn),
}

const QUERY: &str = "find bar query";
const WITH: &str = "find bar replacement";

fn toggle(ui: &mut egui::Ui, on: bool, glyph: &str, or: &str, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::click());
    if on {
        ui.painter().rect(rect, 4.0, pal().selection, Stroke::new(1.0, pal().accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, pal().hover);
    }
    let shown = crate::codicons::glyph(glyph).unwrap_or(or);
    let color = if on || resp.hovered() { pal().text } else { pal().text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, shown, FontId::proportional(13.0), color);
    resp.on_hover_text(tip).clicked()
}

fn segment(ui: &mut egui::Ui, chosen: bool, label: &str, tip: &str) -> bool {
    let color = if chosen { pal().text } else { pal().text_dim };
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), color);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 16.0, 20.0), Sense::click());
    if chosen {
        ui.painter().rect(rect, 5.0, pal().raised, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, pal().hover);
    }
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    resp.on_hover_text(tip).clicked() && !chosen
}

/// A text field in a well, with `extra` (its toggles) inside it at the right.
fn field(
    ui: &mut egui::Ui,
    text: &mut String,
    id: egui::Id,
    hint: &str,
    wrong: bool,
    extra: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let edge = if wrong { c(theme::get().problem_error) } else { pal().border };
    let frame = Frame::NONE
        .fill(pal().well)
        .stroke(Stroke::new(1.0, edge))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(6, 2));
    let inner = frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let edit = egui::TextEdit::singleline(text).id(id).hint_text(hint).frame(Frame::NONE).desired_width(190.0);
            let resp = ui.add(edit.font(FontId::proportional(13.0)));
            extra(ui);
            resp
        })
        .inner
    });
    inner.inner
}

fn focus(ui: &egui::Ui, resp: &egui::Response, text: &str, select: bool) {
    resp.request_focus();
    if !select {
        return;
    }
    let mut state = egui::TextEdit::load_state(ui.ctx(), resp.id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(text)));
    state.store(ui.ctx(), resp.id);
}

pub(super) fn find_bar(ctx: &egui::Context, st: &mut UiState, canvas: Rect, actions: &mut Vec<Action>) {
    let can_replace = st.prefs.edit;
    let text_tip = st.keys.hint("Search every file for the text, in the Search panel", Command::SearchInFiles);
    let kept = &mut st.find_kept;
    let Some(find) = st.find.as_mut() else { return };
    let (query_id, with_id) = (egui::Id::unique(QUERY), egui::Id::unique(WITH));
    let typing = ctx.memory(|m| m.has_focus(query_id) || m.has_focus(with_id));
    let in_file = find.mode == FindIn::File;

    let key = |mods: Modifiers, pressed: Key| ctx.input_mut(|i| i.consume_key(mods, pressed));
    if key(Modifiers::SHIFT, Key::F3) {
        actions.push(Action::Find(FindOp::Previous));
    } else if key(Modifiers::NONE, Key::F3) {
        actions.push(Action::Find(FindOp::Next));
    }
    let mut toggled = false;
    if typing {
        let options = &mut kept.options;
        for (letter, flag) in [(Key::C, &mut options.case), (Key::W, &mut options.word), (Key::R, &mut options.regex)] {
            if key(Modifiers::ALT, letter) {
                *flag = !*flag;
                toggled = true;
            }
        }
        if in_file && key(Modifiers::ALT, Key::P) {
            kept.preserve = !kept.preserve;
            toggled = true;
        }
        if key(Modifiers::COMMAND, Key::H) {
            kept.replace = !kept.replace || !in_file;
            if !in_file {
                actions.push(Action::Find(FindOp::Mode(FindIn::File)));
            }
        }
        if key(Modifiers::COMMAND, Key::F) {
            find.focus = true;
            find.select = true;
        }
        let older = key(Modifiers::ALT, Key::ArrowUp);
        let newer = key(Modifiers::ALT, Key::ArrowDown);
        let seen = &kept.history;
        let went = match (older, newer, find.history_at) {
            (true, _, None) => seen.iter().rposition(|text| *text != find.query),
            (true, _, Some(at)) => Some(at.saturating_sub(1)),
            (_, true, Some(at)) => Some((at + 1).min(seen.len().saturating_sub(1))),
            _ => None,
        };
        if let Some(text) = went.and_then(|at| seen.get(at)) {
            find.history_at = went;
            find.query = text.clone();
            find.focus = true;
            actions.push(Action::Find(FindOp::Changed));
        }
    }
    if toggled {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
        actions.push(Action::Find(FindOp::Changed));
    }

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
            let glyph = |name: &str, or: &'static str| crate::codicons::glyph(name).unwrap_or(or);
            ui.horizontal(|ui| {
                let well =
                    Frame::NONE.fill(pal().well).corner_radius(CornerRadius::same(7)).inner_margin(Margin::same(2));
                well.show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.horizontal(|ui| {
                        let files = "Files and folders on the canvas, by name";
                        if segment(ui, find.mode == FindIn::Files, "Files", files) {
                            actions.push(Action::Find(FindOp::Mode(FindIn::Files)));
                        }
                        if segment(ui, false, "Text in files", &text_tip) {
                            actions.push(Action::Find(FindOp::Mode(FindIn::Text)));
                        }
                        if !find.file.is_empty() && segment(ui, in_file, "This file", "The text of the file selected") {
                            actions.push(Action::Find(FindOp::Mode(FindIn::File)));
                        }
                    });
                });
                let about = match (in_file, can_replace) {
                    (true, true) => find.file.rsplit('/').next().unwrap_or(&find.file).to_string(),
                    (true, false) => {
                        format!("{}  ·  the pencil replaces", find.file.rsplit('/').next().unwrap_or(&find.file))
                    }
                    (false, _) => String::new(),
                };
                ui.add(egui::Label::new(RichText::new(about).size(11.5).color(pal().text_faint)).truncate());
            });
            ui.horizontal(|ui| {
                let open = in_file && can_replace && kept.replace;
                if in_file && can_replace {
                    let (shown, tip) = if open {
                        (glyph("chevron-down", icons::CHEVRON_DOWN), "Hide the replace row")
                    } else {
                        (glyph("chevron-right", icons::CHEVRON_RIGHT), "Replace (Ctrl+H)")
                    };
                    if small_icon(ui, shown, tip) {
                        kept.replace = !kept.replace;
                    }
                }
                let hint = if in_file { "Find in the file" } else { "Find files and folders by name" };
                let options = &mut kept.options;
                let mut flipped = false;
                let query = field(ui, &mut find.query, query_id, hint, find.error.is_some(), |ui| {
                    let toggles = [
                        (&mut options.case, "case-sensitive", "Aa", "Match Case (Alt+C)"),
                        (&mut options.word, "whole-word", "ab", "Match Whole Word (Alt+W)"),
                        (&mut options.regex, "regex", ".*", "Use Regular Expression (Alt+R)"),
                    ];
                    for (flag, name, or, tip) in toggles {
                        if toggle(ui, *flag, name, or, tip) {
                            *flag = !*flag;
                            flipped = true;
                        }
                    }
                });
                if flipped {
                    actions.push(Action::Find(FindOp::Changed));
                }
                if std::mem::take(&mut find.focus) {
                    focus(ui, &query, &find.query, std::mem::take(&mut find.select));
                }
                if query.changed() {
                    find.history_at = None;
                    actions.push(Action::Find(FindOp::Changed));
                }
                let entered = query.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if entered {
                    let back = ui.input(|i| i.modifiers.shift);
                    actions.push(Action::Find(if back { FindOp::Previous } else { FindOp::Next }));
                    find.focus = true;
                }
                if query.has_focus() {
                    if key(Modifiers::NONE, Key::ArrowDown) {
                        actions.push(Action::Find(FindOp::Next));
                    } else if key(Modifiers::NONE, Key::ArrowUp) {
                        actions.push(Action::Find(FindOp::Previous));
                    }
                }
                let (count, tint) = match (&find.error, find.query.trim().is_empty(), find.count) {
                    (Some(_), ..) => ("Invalid pattern".to_string(), c(theme::get().problem_error)),
                    (None, true, _) => (String::new(), pal().text_dim),
                    (None, false, 0) => ("No results".to_string(), c(theme::get().ui_warn)),
                    (None, false, n) => (format!("{} of {n}", find.at + 1), pal().text_dim),
                };
                let (slot, said) = ui.allocate_exact_size(Vec2::new(78.0, 22.0), Sense::hover());
                ui.painter().text(slot.left_center(), Align2::LEFT_CENTER, count, FontId::proportional(12.0), tint);
                if let Some(error) = &find.error {
                    let _ = said.on_hover_text(error.as_str());
                }
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
            if in_file && can_replace && kept.replace {
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    let preserve = &mut kept.preserve;
                    let with = field(ui, &mut kept.with, with_id, "Replace", false, |ui| {
                        if toggle(ui, *preserve, "preserve-case", "AB", "Preserve Case (Alt+P)") {
                            *preserve = !*preserve;
                        }
                    });
                    if with.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        let all = ui.input(|i| i.modifiers.command);
                        actions.push(Action::Find(if all { FindOp::ReplaceAll } else { FindOp::Replace }));
                        with.request_focus();
                    }
                    if small_icon(ui, glyph("replace", icons::CORNER_DOWN_LEFT), "Replace (Enter)") {
                        actions.push(Action::Find(FindOp::Replace));
                    }
                    if small_icon(ui, glyph("replace-all", icons::COPY), "Replace All (Ctrl+Enter)") {
                        actions.push(Action::Find(FindOp::ReplaceAll));
                    }
                });
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                actions.push(Action::Find(FindOp::Close));
            }
        });
    });
    st.floating.push(resp.response.rect);
}
