//! A file in conflict in the changes list: marked in red, with a button for each way out (take
//! your side, take the other side, or say it is resolved as it is).

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::{c, pal, text_fit, Action, FileIcons, ROW_H};
use crate::git::{FileChange, GitOp, Side};
use canvas_render::theme;

/// A small labelled button at the right of a row; `right` moves left of it.
fn pill(ui: &mut egui::Ui, right: &mut f32, mid: f32, key: &str, label: &str, tip: &str) -> bool {
    let font = FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), pal().text);
    let size = Vec2::new(galley.size().x + 12.0, 18.0);
    let at = Rect::from_min_size(Pos2::new(*right - size.x, mid - 9.0), size);
    let resp = ui.interact(at, ui.scope_id().with(("conflict pill", key, label)), Sense::click());
    let fill = if resp.hovered() { pal().active } else { pal().raised };
    ui.painter().rect(at, 5.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    ui.painter().text(at.center(), Align2::CENTER_CENTER, label, font, pal().text);
    *right = at.left() - 4.0;
    resp.on_hover_text(tip).clicked()
}

/// `rebasing` swaps what git calls ours and theirs: in a rebase, "ours" is what is being rebased onto
/// and "theirs" is your own commit being replayed.
pub(super) fn conflict_row(
    ui: &mut egui::Ui,
    file_icons: &mut FileIcons,
    change: &FileChange,
    rebasing: bool,
    busy: bool,
) -> Option<Action> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let red = c(theme::get().git_removed);
    ui.painter().rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, red.gamma_multiply(0.14));
    let mid = rect.center().y;
    let mark = Pos2::new(rect.right() - 20.0, mid);
    ui.painter().circle_filled(mark, 8.0, red);
    ui.painter().text(mark, Align2::CENTER_CENTER, "!", FontId::proportional(12.0), pal().on_accent);
    let mut right = rect.right() - 34.0;
    let path = &change.path;
    let mut action = None;
    let mut go = |op: GitOp| {
        if !busy {
            action = Some(Action::Git(op));
        }
    };
    if pill(ui, &mut right, mid, path, "Resolved", "Mark it resolved as it is now in the file (git add)") {
        go(GitOp::Stage(path.clone()));
    }
    let (mine, theirs) = if rebasing { (Side::Theirs, Side::Ours) } else { (Side::Ours, Side::Theirs) };
    let other = if rebasing { "the branch you rebase onto" } else { "the branch you bring in" };
    if pill(ui, &mut right, mid, path, "Theirs", &format!("Take the file as it is in {other}, and mark it resolved")) {
        go(GitOp::Take { path: path.clone(), side: theirs });
    }
    let own = if rebasing { "your commit being replayed" } else { "your branch" };
    if pill(ui, &mut right, mid, path, "Mine", &format!("Take the file as it is in {own}, and mark it resolved")) {
        go(GitOp::Take { path: path.clone(), side: mine });
    }
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    let mut x = rect.left() + 26.0;
    let icon_at = Rect::from_center_size(Pos2::new(x + 8.0, mid), Vec2::splat(16.0));
    if file_icons.paint(ui, name, false, false, icon_at, false) {
        x += 22.0;
    }
    let painter = ui.painter();
    let name_w = text_fit(painter, Pos2::new(x, mid), name, FontId::proportional(12.5), red, right - x);
    let dir_x = x + name_w + 8.0;
    text_fit(painter, Pos2::new(dir_x, mid), dir, FontId::proportional(11.5), pal().text_faint, right - dir_x);
    if action.is_none() && resp.clicked() {
        action = Some(Action::FlyToPath(path.clone()));
    }
    resp.on_hover_text(format!("{path} · in conflict: both sides changed it"));
    action
}
