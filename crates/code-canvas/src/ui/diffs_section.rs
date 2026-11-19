//! The "Diffs" section of Source Control: how the changes are drawn on the canvas (only
//! marked, inside the files one under the other, or side by side), which files the canvas
//! narrows to, and whether the lines that did not change are folded away.

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

use super::{c, check, pal, Action, ROW_H};
use crate::icons;
use crate::page::DiffView;
use canvas_render::theme;

/// What the section says in its small texts.
pub struct DiffsInfo {
    /// Files with changes.
    pub changed: usize,
    /// How many files the canvas shows while it is narrowed to the changes; None when it is not.
    pub shown: Option<usize>,
    /// Files whose unchanged lines are folded away.
    pub folded: usize,
}

/// How many lines can be kept around a change, in the list of the chip.
const AROUND: [u32; 8] = [0, 1, 2, 3, 5, 10, 20, 40];

fn files(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

fn header(ui: &mut Ui) -> bool {
    let id = egui::Id::unique(("source control section", "diffs"));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let (painter, mid) = (ui.painter(), rect.center().y);
    let chevron = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
    painter.text(
        Pos2::new(rect.left() + 8.0, mid),
        Align2::LEFT_CENTER,
        chevron,
        FontId::proportional(12.0),
        pal().text_dim,
    );
    painter.text(
        Pos2::new(rect.left() + 26.0, mid),
        Align2::LEFT_CENTER,
        icons::SETTINGS,
        FontId::proportional(13.0),
        pal().text_dim,
    );
    painter.text(
        Pos2::new(rect.left() + 46.0, mid),
        Align2::LEFT_CENTER,
        "Diffs",
        FontId::proportional(12.5),
        pal().text,
    );
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    open
}

/// A rounded card tinted with `tint`.
fn card(ui: &mut Ui, tint: Color32, add: impl FnOnce(&mut Ui)) {
    Frame::NONE
        .fill(tint.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0, tint.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// The two colours of a segment's picture: what was removed and what was added.
fn picture(ui: &Ui, mode: usize, at: Rect) {
    let (red, green) = (c(theme::get().git_removed), c(theme::get().git_added));
    let painter = ui.painter();
    let bar = |rect: Rect, color: Color32| painter.rect_filled(rect, 1.5, color);
    match mode {
        0 => {
            painter.circle_filled(at.left_center() + Vec2::new(3.0, 0.0), 3.0, red);
            painter.circle_filled(at.right_center() - Vec2::new(3.0, 0.0), 3.0, green);
        }
        1 => {
            bar(Rect::from_min_size(at.min + Vec2::new(0.0, 1.0), Vec2::new(at.width(), 4.0)), red);
            bar(Rect::from_min_size(at.min + Vec2::new(0.0, 7.0), Vec2::new(at.width(), 4.0)), green);
        }
        _ => {
            let half = Vec2::new(at.width() * 0.5 - 1.0, at.height() - 2.0);
            bar(Rect::from_min_size(at.min + Vec2::new(0.0, 1.0), half), red);
            bar(Rect::from_min_size(at.min + Vec2::new(at.width() * 0.5 + 1.0, 1.0), half), green);
        }
    }
}

/// One choice of a row of choices: the chosen one is raised.
fn choice(ui: &mut Ui, at: Rect, id: &str, chosen: bool, label: &str, draw: impl FnOnce(&Ui, Rect)) -> egui::Response {
    let resp = ui.interact(at, ui.scope_id().with(("diffs choice", id)), Sense::click());
    if chosen {
        ui.painter().rect_filled(at, 6.0, pal().raised);
        ui.painter().rect_stroke(at, 6.0, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(at, 6.0, pal().hover);
    }
    let color = if chosen { pal().text } else { pal().text_dim };
    let text = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), color);
    let width = 14.0 + 6.0 + text.size().x;
    let left = at.center().x - width * 0.5;
    draw(ui, Rect::from_min_size(Pos2::new(left, at.center().y - 6.0), Vec2::new(14.0, 12.0)));
    ui.painter().galley(Pos2::new(left + 20.0, at.center().y - text.size().y * 0.5), text, color);
    resp
}

/// The row of `n` equal choices inside one rounded well; the rectangle of each.
fn row_of(ui: &mut Ui, n: usize, height: f32) -> Vec<Rect> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, pal().well);
    let inner = rect.shrink(3.0);
    let width = inner.width() / n as f32;
    (0..n)
        .map(|i| Rect::from_min_size(inner.min + Vec2::new(width * i as f32, 0.0), Vec2::new(width, inner.height())))
        .collect()
}

fn modes(ui: &mut Ui, view: &mut DiffView) {
    let now = match (view.on, view.split) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    };
    let rects = row_of(ui, 3, 30.0);
    let labels = ["Changes", "Unified", "Split"];
    let tips = [
        "Only mark the files that changed on the canvas",
        "What changed inside the files: the removed lines in red above what replaced them",
        "What changed inside the files: what it was on the left and what it is on the right",
    ];
    for (mode, at) in rects.into_iter().enumerate() {
        let resp = choice(ui, at.shrink2(Vec2::new(1.0, 0.0)), labels[mode], mode == now, labels[mode], |ui, icon| {
            picture(ui, mode, icon)
        });
        if resp.on_hover_text(tips[mode]).clicked() {
            (view.on, view.split) = (mode > 0, mode == 2);
        }
    }
}

fn collapse(ui: &mut Ui, view: &mut DiffView) {
    let rects = row_of(ui, 2, 28.0);
    let tips = ["Hide the lines that did not change, leaving some around each change", "Show every line of the files"];
    for (i, at) in rects.into_iter().enumerate() {
        let (glyph, label) =
            if i == 0 { (icons::FOLD_VERTICAL, "Collapse") } else { (icons::UNFOLD_VERTICAL, "Expand") };
        let chosen = view.fold == (i == 0);
        let resp = choice(ui, at.shrink2(Vec2::new(1.0, 0.0)), label, chosen, label, |ui, icon| {
            let color = if chosen { pal().text } else { pal().text_dim };
            ui.painter().text(icon.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), color);
        });
        if resp.on_hover_text(tips[i]).clicked() {
            view.fold = i == 0;
        }
    }
}

/// "Lines around changes", with its value as a chip that opens the small list.
fn around(ui: &mut Ui, view: &mut DiffView) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("Lines around changes").size(12.0).color(pal().text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let text = if view.around == 1 { "1 line".to_string() } else { format!("{} lines", view.around) };
            let galley = ui.painter().layout_no_wrap(text, FontId::proportional(12.0), pal().text);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 22.0, 22.0), Sense::click());
            let fill = if resp.hovered() { pal().hover } else { pal().raised };
            ui.painter().rect_filled(rect, 6.0, fill);
            ui.painter().galley(
                Pos2::new(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5),
                galley,
                pal().text,
            );
            ui.painter().text(
                Pos2::new(rect.right() - 9.0, rect.center().y),
                Align2::CENTER_CENTER,
                icons::CHEVRON_DOWN,
                FontId::proportional(10.0),
                pal().text_dim,
            );
            egui::Popup::menu(
                &resp.on_hover_text(
                    "How many lines that did not change stay around each change when they are collapsed",
                ),
            )
            .show(|ui| {
                ui.set_width(120.0);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    if super::small_icon(ui, icons::MINUS, "Fewer") {
                        view.around = view.around.saturating_sub(1);
                    }
                    ui.label(RichText::new(view.around.to_string()).monospace().size(12.5).color(pal().text));
                    if super::small_icon(ui, icons::PLUS, "More") {
                        view.around = (view.around + 1).min(40);
                    }
                });
                for n in AROUND {
                    let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(row, 6.0, pal().hover);
                    }
                    let color = if n == view.around { pal().text } else { pal().text_dim };
                    ui.painter().text(
                        Pos2::new(row.left() + 8.0, row.center().y),
                        Align2::LEFT_CENTER,
                        n.to_string(),
                        FontId::proportional(12.0),
                        color,
                    );
                    if n == view.around {
                        ui.painter().text(
                            Pos2::new(row.right() - 10.0, row.center().y),
                            Align2::CENTER_CENTER,
                            icons::CHECK,
                            FontId::proportional(12.0),
                            color,
                        );
                    }
                    if resp.clicked() {
                        view.around = n;
                        ui.close();
                    }
                }
            });
        });
    });
}

/// Draws the section; `only`, `folders` and `neighbours` are the narrowing of the canvas.
pub fn show(
    ui: &mut Ui,
    diff: &DiffView,
    only: bool,
    folders: bool,
    neighbours: bool,
    info: &DiffsInfo,
    actions: &mut Vec<Action>,
) {
    if !header(ui) {
        return;
    }
    let mut view = *diff;
    let (mut only_now, mut folders_now, mut neighbours_now) = (only, folders, neighbours);
    ui.spacing_mut().item_spacing.y = 5.0;
    modes(ui, &mut view);

    let green = c(theme::get().git_added);
    card(ui, green, |ui| {
        let tip = "Leave the files that did not change off the canvas";
        check(ui, &mut only_now, "Only files with changes", true, false).on_hover_text(tip);
        let said = match (info.shown, info.changed) {
            (Some(n), changed) if folders || neighbours => Some(format!("{} shown, {changed} with changes", files(n))),
            (Some(n), _) => Some(format!("{} with changes shown", files(n))),
            (None, 0) => None,
            (None, n) => Some(format!("{} with changes", files(n))),
        };
        if let Some(said) = said {
            ui.label(RichText::new(said).size(11.0).color(green));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        let tip = "Also the other files of every folder that has a changed file";
        check(ui, &mut folders_now, "All files in folders with changes", only, true).on_hover_text(tip);
    });
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        let tip = "Also the files that import a changed file or are imported by one";
        check(ui, &mut neighbours_now, "Their dependency neighbours", only, true).on_hover_text(tip);
    });

    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(icons_text(icons::FOLD_VERTICAL));
        ui.label(RichText::new("Unchanged lines").size(12.0).color(pal().text));
    });
    let amber = c(theme::get().problem_warning);
    card(ui, amber, |ui| {
        collapse(ui, &mut view);
        if view.on && view.fold && info.folded > 0 {
            let verb = if info.folded == 1 { "has" } else { "have" };
            let said = format!("{} {verb} unchanged lines collapsed", files(info.folded));
            ui.label(RichText::new(said).size(11.0).color(amber));
        }
    });
    around(ui, &mut view);
    ui.spacing_mut().item_spacing.y = 3.0;

    if (only_now, folders_now, neighbours_now) != (only, folders, neighbours) {
        if only_now != only {
            actions.push(Action::SetOnlyChanged(only_now));
        }
        if (folders_now, neighbours_now) != (folders, neighbours) {
            actions.push(Action::SetChangedAround { folders: folders_now, neighbours: neighbours_now });
        }
    }
    if view != *diff {
        actions.push(Action::SetDiff(view));
    }
}

fn icons_text(glyph: &str) -> RichText {
    RichText::new(glyph).size(13.0).color(pal().text_dim)
}
