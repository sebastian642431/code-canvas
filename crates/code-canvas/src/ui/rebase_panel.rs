//! The "Interactive rebase" card: the commits from the one chosen up to HEAD, oldest first, each
//! with what to do to it (pick, reword, squash, fixup, drop). Rows are dragged by their handle to
//! change the order (Alt+Up and Alt+Down move the selected row too). "Start rebase" runs it.

use egui::{Align2, Color32, FontId, Frame, Key, Margin, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

use super::{c, menu_row, pal, panel_title, text_fit, Action};
use crate::icons;
use crate::page::Page;
use crate::rebase::{Plan, Verb};
use canvas_render::theme;

const ROW: f32 = 30.0;
const HANDLE: f32 = 26.0;
const PICKER: f32 = 84.0;
const HASH: f32 = 62.0;

fn pick_color(pick: Verb) -> Color32 {
    match pick {
        Verb::Pick => pal().text,
        Verb::Reword => pal().accent,
        Verb::Squash | Verb::Fixup => c(theme::get().git_modified),
        Verb::Drop => c(theme::get().git_removed),
    }
}

fn empty(ui: &mut Ui, why: &str) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
        panel_title(ui, "Interactive rebase", |_| {});
        ui.add_space(8.0);
        ui.label(RichText::new(why).size(12.0).color(pal().text_faint));
    });
}

/// A plain button for the foot of the card.
fn button(ui: &mut Ui, label: &str, enabled: bool, main: bool, tip: &str) -> bool {
    let font = FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), pal().text);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 28.0, 28.0), Sense::click());
    let hovered = resp.hovered() && enabled;
    let fill = match (enabled, main, hovered) {
        (false, ..) => pal().raised,
        (_, true, false) => pal().accent.gamma_multiply(0.85),
        (_, true, true) => pal().accent,
        (_, false, false) => pal().raised,
        (_, false, true) => pal().hover,
    };
    let color = match (enabled, main) {
        (false, _) => pal().text_faint,
        (_, true) => pal().on_accent,
        _ => pal().text,
    };
    ui.painter().rect(rect, 7.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    resp.on_hover_text(tip).clicked() && enabled
}

/// One row; says what to do about it: pick another action for it, or move it to another row.
struct RowResult {
    pick: Option<Verb>,
    moved_to: Option<usize>,
}

fn row(ui: &mut Ui, plan: &mut Plan, i: usize, top: f32) -> RowResult {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::hover());
    let mut result = RowResult { pick: None, moved_to: None };
    let mid = rect.center().y;
    let key = plan.entries[i].hash.clone();
    let id = ui.scope_id().with(("rebase row", &key));
    let on = plan.selected == Some(i);
    let dragged = plan.dragging == Some(i);
    let pick = plan.entries[i].pick;
    let body = ui.interact(rect, id.with("body"), Sense::click());
    if body.clicked() {
        plan.selected = Some(i);
    }
    if on || dragged {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(0.0, 1.0)), 5.0, pal().row);
    } else if body.hovered() {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(0.0, 1.0)), 5.0, pal().hover);
    }
    let strip = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 4.0), Vec2::new(3.0, ROW - 8.0));
    ui.painter().rect_filled(strip, 1.5, pick_color(pick));

    let handle = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.top()), Vec2::new(HANDLE, ROW));
    let grip = ui.interact(handle, id.with("handle"), Sense::drag());
    let grip_glyph = crate::codicons::glyph("gripper").unwrap_or(icons::ELLIPSIS_VERTICAL);
    let grip_color = if grip.hovered() || dragged { pal().text } else { pal().text_faint };
    ui.painter().text(handle.center(), Align2::CENTER_CENTER, grip_glyph, FontId::proportional(14.0), grip_color);
    let grip = grip.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("Drag to change the order");
    if grip.drag_started() {
        plan.dragging = Some(i);
        plan.selected = Some(i);
    }
    if grip.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let target = ((pointer.y - top) / ROW).floor().clamp(0.0, plan.entries.len() as f32 - 1.0) as usize;
            if target != i {
                result.moved_to = Some(target);
            }
        }
    }
    if grip.drag_stopped() {
        plan.dragging = None;
    }

    let picker = Rect::from_min_size(Pos2::new(handle.right() + 2.0, mid - 11.0), Vec2::new(PICKER, 22.0));
    let button = ui.interact(picker, id.with("picker"), Sense::click());
    let fill = if button.hovered() { pal().hover } else { pal().well };
    ui.painter().rect(picker, 6.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    let font = FontId::proportional(12.0);
    ui.painter().text(Pos2::new(picker.left() + 9.0, mid), Align2::LEFT_CENTER, pick.label(), font, pick_color(pick));
    ui.painter().text(
        Pos2::new(picker.right() - 10.0, mid),
        Align2::CENTER_CENTER,
        icons::CHEVRON_DOWN,
        FontId::proportional(11.0),
        pal().text_dim,
    );
    let can_join = plan.can_join(i);
    egui::Popup::menu(&button).show(|ui| {
        ui.set_width(250.0);
        ui.spacing_mut().item_spacing.y = 1.0;
        for option in Verb::ALL {
            if matches!(option, Verb::Squash | Verb::Fixup) && !can_join {
                continue;
            }
            let mark = if option == pick { icons::CHECK } else { "" };
            if menu_row(ui, mark, &format!("{}  ·  {}", option.label(), option.hint())) {
                result.pick = Some(option);
                ui.close();
            }
        }
    });
    button.on_hover_text(pick.hint());

    let hash_x = picker.right() + 10.0;
    let short = &plan.entries[i].hash[..plan.entries[i].hash.len().min(7)];
    text_fit(ui.painter(), Pos2::new(hash_x, mid), short, FontId::monospace(11.0), pal().text_faint, HASH);
    let text_x = hash_x + HASH;
    let width = rect.right() - text_x - 10.0;
    if pick == Verb::Reword {
        let edit_rect = Rect::from_min_size(Pos2::new(text_x, mid - 11.0), Vec2::new(width.max(40.0), 22.0));
        let message = &mut plan.entries[i].message;
        let edit = egui::TextEdit::singleline(message).hint_text("The new message").desired_width(edit_rect.width());
        ui.put(edit_rect, edit);
    } else {
        let entry = &plan.entries[i];
        let color = match pick {
            Verb::Drop => pal().text_faint,
            Verb::Squash | Verb::Fixup => pal().text_dim,
            _ => pal().text,
        };
        let shown =
            text_fit(ui.painter(), Pos2::new(text_x, mid), &entry.subject, FontId::proportional(12.5), color, width);
        if pick == Verb::Drop {
            ui.painter()
                .line_segment([Pos2::new(text_x, mid), Pos2::new(text_x + shown, mid)], (1.0, pal().text_faint));
        }
    }
    result
}

pub(super) fn rebase_panel(ui: &mut Ui, page: Option<&mut Page>, actions: &mut Vec<Action>) {
    let Some(page) = page else { return empty(ui, "Open a folder to rebase its commits.") };
    let busy = page.git_busy();
    let Page { git, rebase, .. } = page;
    let Some(plan) = rebase.as_mut() else {
        return empty(ui, "Right click a commit in All Commits and choose \"Interactive rebase from here…\".");
    };
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
        panel_title(ui, "Interactive rebase", |_| {});
    });
    let stopped = git.as_ref().is_some_and(|g| g.in_progress.is_some());
    if let Some(g) = git.as_ref() {
        super::progress_banner::banner(ui, g, busy, actions);
    }
    Frame::NONE.inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 10 }).show(ui, |ui| {
        let onto = match &plan.base {
            Some(base) => format!("on top of {}", &base[..base.len().min(7)]),
            None => "from the first commit".to_string(),
        };
        let text = format!("Replayed {onto}, the oldest first. Drag a row by its handle.");
        ui.label(RichText::new(text).size(11.5).color(pal().text_dim));
        ui.add_space(6.0);

        let typing = ui.ctx().memory(|memory| memory.focused().is_some());
        let selected = plan.selected;
        if let (Some(at), false) = (selected, typing) {
            if ui.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowUp)) && at > 0 {
                plan.move_to(at, at - 1);
            } else if ui.input_mut(|input| input.consume_key(Modifiers::ALT, Key::ArrowDown)) {
                plan.move_to(at, at + 1);
            }
        }

        let room = (ui.available_height() - 48.0).max(80.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut pick: Option<(usize, Verb)> = None;
        let mut moved: Option<(usize, usize)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(room).id_salt("rebase rows").show(
            ui,
            |ui| {
                let top = ui.cursor().top();
                for i in 0..plan.entries.len() {
                    let result = row(ui, plan, i, top);
                    pick = pick.or(result.pick.map(|p| (i, p)));
                    moved = moved.or(result.moved_to.map(|to| (i, to)));
                }
            },
        );
        if let Some((at, option)) = pick {
            plan.set(at, option);
        }
        if let Some((from, to)) = moved {
            plan.move_to(from, to);
        }

        ui.add_space(8.0);
        let (can, why) = match (stopped, plan.keeps_one(), plan.changes_something(), busy) {
            (true, ..) => (false, "Finish or abort the operation that is stopped first"),
            (_, false, ..) => (false, "Every commit is dropped: there would be nothing left"),
            (_, _, false, _) => (false, "Nothing is changed yet: reorder, reword, squash or drop something"),
            (_, _, _, true) => (false, "Git is still at something else"),
            _ => (true, "Rewrite the history of this branch as arranged above"),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if button(ui, "Start rebase", can, true, why) {
                actions.push(Action::RebaseStart);
            }
            if button(ui, "Cancel", true, false, "Close this card without changing anything") {
                actions.push(Action::RebaseCancel);
            }
        });
    });
}
