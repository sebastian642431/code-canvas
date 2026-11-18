//! The banner at the top of Source Control and of the All Commits card while a merge, a cherry-pick,
//! a revert or a rebase waits to be finished: what it is doing, how many files are in conflict, and
//! the buttons to carry on, leave this step out or give up.

use egui::{Align2, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

use super::{c, pal, Action};
use crate::git::{Carry, GitOp, GitState, InProgress};
use canvas_render::theme;

/// "Rebasing — step 2 of 5 — 1 conflict".
pub(super) fn sentence(progress: &InProgress, conflicts: usize) -> String {
    let mut parts = vec![progress.operation.doing().to_string()];
    if let Some((now, of)) = progress.step {
        parts.push(format!("step {now} of {of}"));
    }
    parts.push(match conflicts {
        0 => "no conflicts left".to_string(),
        1 => "1 conflict".to_string(),
        n => format!("{n} conflicts"),
    });
    parts.join(" — ")
}

/// One small button of the banner; a disabled one still shows its tip.
fn button(ui: &mut Ui, label: &str, enabled: bool, main: bool, tip: &str) -> bool {
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), pal().text);
    let size = Vec2::new(galley.size().x + 20.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
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
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    let resp = resp.on_hover_text(tip);
    resp.clicked() && enabled
}

/// Draws the banner when something is in progress. `busy` is true while git is at something else.
pub(super) fn banner(ui: &mut Ui, g: &GitState, busy: bool, actions: &mut Vec<Action>) {
    let Some(progress) = g.in_progress else { return };
    let conflicts = g.conflicts();
    let warning = c(theme::get().problem_warning);
    let operation = progress.operation;
    Frame::NONE
        .fill(warning.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, warning.gamma_multiply(0.7)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .outer_margin(Margin { left: 10, right: 10, top: 2, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            ui.label(RichText::new(sentence(&progress, conflicts)).size(12.5).color(pal().text));
            ui.horizontal_wrapped(|ui| {
                let can = !busy && conflicts == 0;
                let tip = match (busy, conflicts) {
                    (true, _) => "Git is still at something else",
                    (_, 0) => "Finish it with what is staged",
                    _ => "Fix the files marked in red first: take a side or mark them resolved",
                };
                if button(ui, "Continue", can, true, tip) {
                    actions.push(Action::Git(GitOp::Carry { operation, how: Carry::Continue }));
                }
                if operation.can_skip() {
                    let tip = "Leave this step out and go on with the next one";
                    if button(ui, "Skip", !busy, false, tip) {
                        actions.push(Action::Git(GitOp::Carry { operation, how: Carry::Skip }));
                    }
                }
                let tip = "Give it up: everything goes back to how it was before it started";
                if button(ui, "Abort", !busy, false, tip) {
                    let what = operation.noun().to_lowercase();
                    let text = format!(
                        "The {what} is given up and the folder goes back to how it was before it started. What you fixed in the conflicts is lost."
                    );
                    actions.push(Action::GitConfirm {
                        op: GitOp::Carry { operation, how: Carry::Abort },
                        title: format!("Abort the {what}?"),
                        text,
                        button: "Abort",
                        danger: true,
                    });
                }
            });
        });
}
