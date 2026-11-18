use egui::{Align2, Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Vec2};

use super::{
    c, check, icon, menu_gap, menu_row, pal, panel_title, small_icon, text_fit, Action, FileIcons, FileOp, ROW_H,
};
use crate::git::{After, Change, FileChange, GitOp};
use crate::icons;
use crate::page::Page;
use canvas_render::theme;

pub(super) fn change_color(change: Change) -> Color32 {
    c(match change {
        Change::Added | Change::Untracked => theme::get().git_added,
        Change::Deleted | Change::Conflicted => theme::get().git_removed,
        Change::Modified | Change::Renamed => theme::get().git_modified,
    })
}

pub fn timeline(ui: &mut egui::Ui, page: Option<&Page>, prefs: &crate::prefs::Prefs, actions: &mut Vec<Action>) {
    let Some(page) = page else { return };
    let Some(path) = &page.timeline.path else { return };
    let open = prefs.timeline;
    let panel = match open {
        true => egui::Panel::bottom("timeline").resizable(true).default_size(190.0).size_range(90.0..=520.0),
        false => egui::Panel::bottom("timeline closed").resizable(false).exact_size(ROW_H + 12.0),
    };
    panel.frame(Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 6 })).show(ui, |ui| {
        let top = ui.max_rect().top();
        ui.painter().hline(ui.max_rect().x_range(), top - 6.0, (1.0, pal().border));
        let (head, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(head.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
        }
        let mut row = ui
            .new_child(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::left_to_right(egui::Align::Center)));
        row.add_space(10.0);
        let chevron = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
        row.label(icon(chevron, 13.0, pal().text_dim));
        row.label(icon(icons::GIT_BRANCH, 13.0, pal().text_dim));
        row.label(RichText::new("Timeline").size(12.5).color(pal().text));
        let name = path.rsplit('/').next().unwrap_or(path);
        row.label(RichText::new(name).size(11.5).color(pal().text_faint));
        let tip = if open { "Hide the history of the file" } else { "Show the history of the file" };
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
            actions.push(Action::SetPrefs(crate::prefs::Prefs { timeline: !open, ..prefs.clone() }));
        }
        if !open {
            return;
        }
        let pending = page.git.as_ref().and_then(|g| g.changes.iter().find(|ch| ch.path == *path));
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, color: Color32, title: &str, right: &str, tip: String, on: bool| {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                let painter = ui.painter();
                if resp.hovered() || on {
                    painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
                }
                let mid = rect.center().y;
                painter.circle_filled(Pos2::new(rect.left() + 18.0, mid), 3.0, color);
                let rg = painter.layout_no_wrap(right.to_string(), FontId::proportional(11.0), pal().text_faint);
                let rx = rect.right() - 12.0 - rg.size().x;
                painter.galley(Pos2::new(rx, mid - rg.size().y * 0.5), rg, pal().text_faint);
                let from = rect.left() + 30.0;
                text_fit(painter, Pos2::new(from, mid), title, FontId::proportional(12.0), pal().text, rx - 8.0 - from);
                resp.on_hover_text(tip).clicked()
            };
            let shown = page.showing.as_ref().map(|showing| showing.commit.hash.as_str());
            if let Some(ch) = pending {
                let detail = match ch.insertions + ch.deletions {
                    0 => ch.change.label().to_string(),
                    _ => format!("+{} −{}", ch.insertions, ch.deletions),
                };
                row(
                    ui,
                    change_color(ch.change),
                    "Uncommitted changes",
                    &detail,
                    format!("{} · {}", ch.path, ch.change.label()),
                    false,
                );
            }
            for commit in &page.timeline.commits {
                let tip = format!(
                    "{}\n{} · {}\nClick to see on the canvas what it changed",
                    commit.subject,
                    commit.author,
                    &commit.hash[..commit.hash.len().min(10)]
                );
                let on = shown == Some(commit.hash.as_str());
                let color = if on { pal().accent } else { pal().text_faint };
                if row(ui, color, &commit.subject, &crate::git::ago(now - commit.time), tip, on) {
                    actions.push(Action::ShowCommit((!on).then(|| commit.clone())));
                }
            }
            if page.timeline.loading {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new("Reading the history …").size(11.5).color(pal().text_faint));
                });
            } else if page.timeline.commits.is_empty() && pending.is_none() {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new("No history: not committed, or not a git repository.")
                            .size(11.5)
                            .color(pal().text_faint),
                    );
                });
            }
        });
    });
}

fn group_header(
    ui: &mut egui::Ui,
    title: &str,
    key: &str,
    count: Option<usize>,
    button: Option<(&str, &str)>,
) -> (bool, bool) {
    let id = egui::Id::unique(("source control section", key));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let painter = ui.painter();
    let mid = rect.center().y;
    painter.text(
        Pos2::new(rect.left() + 8.0, mid),
        Align2::LEFT_CENTER,
        if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT },
        FontId::proportional(12.0),
        pal().text_dim,
    );
    painter.text(
        Pos2::new(rect.left() + 26.0, mid),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(12.5),
        pal().text,
    );
    let mut right = rect.right() - 10.0;
    if let Some(n) = count {
        let g = painter.layout_no_wrap(n.to_string(), FontId::proportional(11.0), pal().text_dim);
        let badge = Rect::from_center_size(Pos2::new(right - 9.0, mid), Vec2::new(g.size().x.max(8.0) + 10.0, 16.0));
        painter.rect_filled(badge, 8.0, pal().raised);
        painter.galley(badge.center() - g.size() * 0.5, g, pal().text_dim);
        right = badge.left() - 4.0;
    }
    let mut clicked = false;
    if let Some((glyph, tip)) = button {
        if ui.rect_contains_pointer(rect) {
            let at = Rect::from_center_size(Pos2::new(right - 11.0, mid), Vec2::splat(22.0));
            let r = ui.interact(at, ui.scope_id().with(("group", key)), Sense::click());
            if r.hovered() {
                ui.painter().rect_filled(at, 5.0, pal().hover);
            }
            ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), pal().text);
            clicked = r.on_hover_text(tip).clicked();
        }
    }
    if resp.clicked() && !clicked {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    (open, clicked)
}

fn row_button(
    ui: &mut egui::Ui,
    right: &mut f32,
    mid: f32,
    id: impl std::hash::Hash + std::fmt::Debug,
    glyph: &str,
    tip: &str,
) -> bool {
    let at = Rect::from_center_size(Pos2::new(*right - 11.0, mid), Vec2::splat(22.0));
    let r = ui.interact(at, ui.scope_id().with(id), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(at, 5.0, pal().hover);
    }
    ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), pal().text);
    *right = at.left() - 2.0;
    r.on_hover_text(tip).clicked()
}

fn change_row(
    ui: &mut egui::Ui,
    file_icons: &mut FileIcons,
    change: &FileChange,
    side: Change,
    staged: bool,
) -> Option<Action> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
    }
    let mid = rect.center().y;
    let color = change_color(side);
    let deleted = side == Change::Deleted;
    ui.painter().text(
        Pos2::new(rect.right() - 14.0, mid),
        Align2::RIGHT_CENTER,
        side.letter(),
        FontId::monospace(12.0),
        color,
    );
    let mut right = rect.right() - 32.0;
    if change.unsaved && !staged {
        ui.painter().circle_filled(Pos2::new(right + 2.0, mid), 3.2, pal().text);
        right -= 12.0;
    }
    let mut action = None;
    if hovered {
        let path = &change.path;
        let (glyph, tip) = if staged { (icons::MINUS, "Unstage") } else { (icons::PLUS, "Stage") };
        if row_button(ui, &mut right, mid, ("stage", staged, path), glyph, tip) {
            let op = if staged { GitOp::Unstage(path.clone()) } else { GitOp::Stage(path.clone()) };
            action = Some(Action::Git(op));
        }
        if !staged {
            let glyph = crate::codicons::glyph("discard").unwrap_or(icons::ARROW_LEFT);
            if side == Change::Untracked {
                if row_button(ui, &mut right, mid, ("discard", path), glyph, "Delete (to the recycle bin)") {
                    action = Some(Action::File(FileOp::Delete(path.clone())));
                }
            } else if row_button(ui, &mut right, mid, ("discard", path), glyph, "Discard changes") {
                action = Some(Action::Git(GitOp::Discard(path.clone())));
            }
        }
        if !deleted {
            let glyph = crate::codicons::glyph("go-to-file").unwrap_or(icons::FILE);
            if row_button(ui, &mut right, mid, ("open", staged, path), glyph, "Go to the file") {
                action = Some(Action::FlyToPath(path.clone()));
            }
        }
        right -= 2.0;
    }
    let (dir, name) = change.path.rsplit_once('/').unwrap_or(("", &change.path));
    let mut x = rect.left() + 26.0;
    let icon_at = Rect::from_center_size(Pos2::new(x + 8.0, mid), Vec2::splat(16.0));
    if file_icons.paint(ui, name, false, false, icon_at, deleted) {
        x += 22.0;
    }
    let painter = ui.painter();
    let name_color = if deleted { pal().text_dim } else { color.lerp_to_gamma(pal().text, 0.35) };
    let name_w = text_fit(painter, Pos2::new(x, mid), name, FontId::proportional(12.5), name_color, right - x);
    if deleted {
        painter.line_segment([Pos2::new(x, mid), Pos2::new(x + name_w, mid)], (1.0, pal().text_dim));
    }
    let dir_x = x + name_w + 8.0;
    text_fit(painter, Pos2::new(dir_x, mid), dir, FontId::proportional(11.5), pal().text_faint, right - dir_x);
    if action.is_none() && resp.clicked() && !deleted {
        action = Some(Action::FlyToPath(change.path.clone()));
    }
    let unsaved = if change.unsaved && !staged { " · not saved yet (staging or committing it saves it)" } else { "" };
    resp.on_hover_text(format!("{} · {}{unsaved}", change.path, side.label()));
    action
}

enum Primary {
    Commit,
    Publish,
    Sync,
    Idle,
}

pub fn changes_panel(
    ui: &mut egui::Ui,
    file_icons: &mut FileIcons,
    page: Option<&mut Page>,
    actions: &mut Vec<Action>,
) {
    let busy = page.as_ref().is_some_and(|p| p.git_busy());
    let in_repository = page.as_ref().is_some_and(|p| p.git.is_some());
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 2 }).show(ui, |ui| {
        panel_title(ui, "Source control", |ui| {
            if in_repository {
                let more = ui
                    .scope(|ui| {
                        ui.visuals_mut().button_frame = false;
                        let dots = egui::Button::new(icon(icons::ELLIPSIS_VERTICAL, 14.0, pal().text_dim));
                        let dots = dots.min_size(Vec2::splat(24.0)).frame(false);
                        egui::containers::menu::MenuButton::from_button(dots).ui(ui, |ui| {
                            ui.set_width(210.0);
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let mut op = None;
                            for (glyph, label, what) in [
                                (icons::DOWNLOAD, "Pull", GitOp::Pull),
                                (icons::ARROW_UP, "Push", GitOp::Push),
                                (icons::REFRESH_CW, "Fetch", GitOp::Fetch),
                            ] {
                                if menu_row(ui, glyph, label) {
                                    op = Some(what);
                                }
                            }
                            menu_gap(ui);
                            for (glyph, label, what) in [
                                (icons::PLUS, "Stage all changes", GitOp::StageAll),
                                (icons::MINUS, "Unstage all changes", GitOp::UnstageAll),
                                (icons::ARROW_LEFT, "Discard all changes", GitOp::DiscardAll),
                            ] {
                                if menu_row(ui, glyph, label) {
                                    op = Some(what);
                                }
                            }
                            menu_gap(ui);
                            for (glyph, label, what) in [
                                (icons::FILE_ARCHIVE, "Stash (with new files)", GitOp::Stash),
                                (icons::HISTORY, "Pop the latest stash", GitOp::StashPop),
                            ] {
                                if menu_row(ui, glyph, label) {
                                    op = Some(what);
                                }
                            }
                            if let Some(op) = op {
                                if !busy {
                                    actions.push(Action::Git(op));
                                }
                                ui.close();
                            }
                        })
                    })
                    .inner;
                let (more, _) = more;
                if more.hovered() {
                    let at = Rect::from_center_size(more.rect.center(), Vec2::splat(24.0));
                    ui.painter().rect_filled(at, 6.0, pal().hover);
                    let glyph = icons::ELLIPSIS_VERTICAL;
                    ui.painter().text(
                        at.center(),
                        Align2::CENTER_CENTER,
                        glyph,
                        FontId::proportional(14.0),
                        pal().text,
                    );
                }
                more.on_hover_text("More actions");
            }
            if small_icon(ui, icons::REFRESH_CW, "Ask git again") {
                actions.push(Action::RefreshGit);
            }
        });
    });
    let Some(page) = page else {
        ui.label(RichText::new("    Open a folder to see its changes.").size(11.5).color(pal().text_dim));
        return;
    };
    if page.git_loading() {
        ui.label(RichText::new("    Reading the repository …").size(11.5).color(pal().text_dim));
        return;
    }
    let writing = page.writing_message();
    let Page { git, commit_message, only_changed, git_note, diff, showing, .. } = page;
    let Some(g) = git.as_ref() else {
        ui.label(RichText::new("    This folder is not in a git repository.").size(11.5).color(pal().text_dim));
        return;
    };
    let staged: Vec<&FileChange> = g.changes.iter().filter(|ch| ch.staged.is_some()).collect();
    let unstaged: Vec<&FileChange> = g.changes.iter().filter(|ch| ch.unstaged.is_some()).collect();

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
            let mid = rect.center().y;
            let title = RichText::new("Repositories").size(12.5).color(pal().text);
            let galley = ui.painter().layout_no_wrap(title.text().to_string(), FontId::proportional(12.5), pal().text);
            ui.painter().galley(Pos2::new(rect.left() + 26.0, mid - galley.size().y * 0.5), galley, pal().text);
            let mut said = icons::ARROW_UP_DOWN.to_string();
            if g.behind > 0 {
                said.push_str(&format!(" {}{}", g.behind, icons::ARROW_DOWN));
            }
            if g.ahead > 0 {
                said.push_str(&format!(" {}{}", g.ahead, icons::ARROW_UP));
            }
            let shown = ui.painter().layout_no_wrap(said, FontId::proportional(13.0), pal().text_dim);
            let width = (shown.size().x + 12.0).max(22.0);
            let at = Rect::from_center_size(Pos2::new(rect.right() - 10.0 - width * 0.5, mid), Vec2::new(width, 22.0));
            let sync = ui.interact(at, ui.scope_id().with("sync"), Sense::click());
            if sync.hovered() {
                ui.painter().rect_filled(at, 5.0, pal().hover);
            }
            let color = if sync.hovered() { pal().text } else { pal().text_dim };
            ui.painter().galley(at.center() - shown.size() * 0.5, shown, color);
            let tip = match (g.behind, g.ahead) {
                (0, 0) => "Sync: pull (fast-forward only), then push".to_string(),
                (behind, ahead) => format!("Sync: pull {behind} and push {ahead} (pull fast-forward only, then push)"),
            };
            if sync.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() && !busy {
                actions.push(Action::Git(GitOp::Sync));
            }
        }

        if group_header(ui, "Changes", "changes", None, None).0 {
            Frame::NONE.inner_margin(Margin { left: 26, right: 10, top: 2, bottom: 6 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let mut commit = false;
                let hint = format!("Message (Ctrl+Enter to commit on \"{}\")", g.branch);
                Frame::NONE
                    .fill(pal().well)
                    .stroke(egui::Stroke::new(1.0, pal().border))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(Margin { left: 8, right: 3, top: 5, bottom: 5 })
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let edit = ui.add(
                                egui::TextEdit::multiline(commit_message)
                                    .hint_text(if writing { "Writing the message …".to_string() } else { hint })
                                    .frame(Frame::NONE)
                                    .desired_rows(1)
                                    .desired_width(ui.available_width() - 26.0),
                            );
                            commit |= edit.has_focus()
                                && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
                            let (at, spark) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                            let can = !writing && !g.changes.is_empty();
                            if spark.hovered() && can {
                                ui.painter().rect_filled(at, 5.0, pal().hover);
                            }
                            let color = match (writing, can, spark.hovered()) {
                                (true, _, _) => pal().accent,
                                (_, false, _) => pal().text_faint,
                                (_, true, true) => pal().text,
                                (_, true, false) => pal().text_dim,
                            };
                            let glyph = if writing { icons::ELLIPSIS_VERTICAL } else { icons::SPARKLES };
                            ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), color);
                            let tip = if writing {
                                "An assistant is writing the message"
                            } else {
                                "Write the message from the change (Claude Code, Codex or your own command: see Settings)"
                            };
                            if spark.on_hover_text(tip).clicked() && can {
                                actions.push(Action::WriteCommitMessage);
                            }
                        });
                    });

                let has_message = !commit_message.trim().is_empty();
                let primary = match (g.changes.is_empty(), g.upstream) {
                    (false, _) => Primary::Commit,
                    (true, false) if !g.branch.is_empty() && !g.graph.is_empty() => Primary::Publish,
                    (true, true) if g.ahead + g.behind > 0 => Primary::Sync,
                    _ => Primary::Idle,
                };
                let all = staged.is_empty();
                let ready = !busy
                    && match primary {
                        Primary::Commit => has_message,
                        Primary::Publish | Primary::Sync => true,
                        Primary::Idle => false,
                    };
                let label = match (&primary, busy) {
                    (_, true) => "Working …".to_string(),
                    (Primary::Commit | Primary::Idle, _) => format!("{}  Commit", icons::CHECK),
                    (Primary::Publish, _) => format!("{}  Publish Branch", icons::ARROW_UP),
                    (Primary::Sync, _) => {
                        let (down, up) = (icons::ARROW_DOWN, icons::ARROW_UP);
                        format!("{}  Sync Changes  {}{down} {}{up}", icons::REPEAT_2, g.behind, g.ahead)
                    }
                };
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
                let with_menu = matches!(primary, Primary::Commit);
                let menu_w = if with_menu { 26.0 } else { 0.0 };
                let main_at = Rect::from_min_max(rect.min, Pos2::new(rect.right() - menu_w, rect.bottom()));
                let menu_at = Rect::from_min_max(Pos2::new(rect.right() - menu_w, rect.top()), rect.max);
                let main = ui.interact(main_at, ui.scope_id().with("commit"), Sense::click());
                let fill = |hovered: bool| match (ready, hovered) {
                    (false, _) => pal().raised,
                    (true, false) => pal().accent.gamma_multiply(0.85),
                    (true, true) => pal().accent,
                };
                let text_color = if ready { pal().on_accent } else { pal().text_faint };
                ui.painter().rect_filled(rect, 6.0, fill(false));
                if main.hovered() && ready {
                    let round = egui::CornerRadius { nw: 6, sw: 6, ne: if with_menu { 0 } else { 6 }, se: if with_menu { 0 } else { 6 } };
                    ui.painter().rect_filled(main_at, round, fill(true));
                }
                ui.painter().text(main_at.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.5), text_color);
                let tip = match primary {
                    Primary::Commit if all => "Nothing is staged: this stages every change and commits it",
                    Primary::Commit => "Commit the staged changes",
                    Primary::Publish => "Push this branch to the remote for the first time",
                    Primary::Sync => "Pull (fast-forward only), then push",
                    Primary::Idle => "Nothing to commit",
                };
                commit |= main.on_hover_text(tip).clicked();
                let commit_op = |amend: bool, then: After| {
                    Action::Git(GitOp::Commit { message: commit_message.trim().to_string(), all, amend, then })
                };
                if commit && ready {
                    actions.push(match primary {
                        Primary::Commit => commit_op(false, After::Nothing),
                        Primary::Publish => Action::Git(GitOp::Publish(g.branch.clone())),
                        Primary::Sync | Primary::Idle => Action::Git(GitOp::Sync),
                    });
                }
                if with_menu {
                    let arrow = ui.interact(menu_at, ui.scope_id().with("commit menu"), Sense::click());
                    if arrow.hovered() && !busy {
                        let round = egui::CornerRadius { nw: 0, sw: 0, ne: 6, se: 6 };
                        ui.painter().rect_filled(menu_at, round, pal().accent);
                    }
                    let line = if ready { pal().on_accent.gamma_multiply(0.35) } else { pal().border };
                    ui.painter().vline(menu_at.left(), menu_at.shrink(5.0).y_range(), (1.0, line));
                    let color = if ready || arrow.hovered() { pal().on_accent } else { pal().text_faint };
                    ui.painter().text(
                        menu_at.center(),
                        Align2::CENTER_CENTER,
                        icons::CHEVRON_DOWN,
                        FontId::proportional(12.0),
                        if busy { pal().text_faint } else { color },
                    );
                    egui::Popup::menu(&arrow).show(|ui| {
                        ui.set_width(200.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let mut chosen = None;
                        if has_message {
                            if menu_row(ui, icons::CHECK, "Commit") {
                                chosen = Some(commit_op(false, After::Nothing));
                            }
                            if menu_row(ui, icons::ARROW_UP, "Commit & Push") {
                                chosen = Some(commit_op(false, After::Push));
                            }
                            if menu_row(ui, icons::REPEAT_2, "Commit & Sync") {
                                chosen = Some(commit_op(false, After::Sync));
                            }
                            menu_gap(ui);
                        }
                        let amend = if has_message { "Commit (Amend)" } else { "Amend with the same message" };
                        if menu_row(ui, icons::HISTORY, amend) {
                            chosen = Some(commit_op(true, After::Nothing));
                        }
                        if let Some(action) = chosen {
                            if !busy {
                                actions.push(action);
                            }
                            ui.close();
                        }
                    });
                    arrow.on_hover_text("More ways to commit");
                }

                if let Some((ok, text)) = git_note.as_ref() {
                    let color = if *ok { c(theme::get().git_added) } else { c(theme::get().git_removed) };
                    ui.label(RichText::new(text).size(11.5).color(color));
                }
                if !g.changes.is_empty() {
                    let mut only = *only_changed;
                    if check(ui, &mut only, "Show only changed files on canvas", true, true).clicked() {
                        actions.push(Action::SetOnlyChanged(only));
                    }
                }
                if !g.changes.is_empty() || diff.on {
                    let mut view = *diff;
                    let shown = check(ui, &mut view.on, "Show what changed inside the files", true, true);
                    shown.on_hover_text("The removed lines too, in red above what replaced them. A file is as it is on disk again while it is edited.");
                    if view.on {
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            check(ui, &mut view.split, "What it was and what it is, side by side", true, true);
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            check(ui, &mut view.fold, "Fold the lines that did not change", true, true);
                        });
                    }
                    if view.on && view.fold {
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.label(RichText::new("Lines kept around a change").size(12.0).color(pal().text_dim));
                            ui.add_space(6.0);
                            if small_icon(ui, icons::MINUS, "Fewer") {
                                view.around = view.around.saturating_sub(1);
                            }
                            ui.label(RichText::new(view.around.to_string()).monospace().size(12.0).color(pal().text));
                            if small_icon(ui, icons::PLUS, "More") {
                                view.around = (view.around + 1).min(40);
                            }
                        });
                    }
                    if view != *diff {
                        actions.push(Action::SetDiff(view));
                    }
                }
            });

            if !staged.is_empty() {
                let (open, all) =
                    group_header(ui, "Staged Changes", "staged", Some(staged.len()), Some((icons::MINUS, "Unstage all")));
                if all && !busy {
                    actions.push(Action::Git(GitOp::UnstageAll));
                }
                if open {
                    for ch in &staged {
                        actions.extend(change_row(ui, file_icons, ch, ch.staged.unwrap_or(ch.change), true));
                    }
                }
            }
            if !unstaged.is_empty() || staged.is_empty() {
                let (open, all) =
                    group_header(ui, "Changes", "unstaged", Some(unstaged.len()), Some((icons::PLUS, "Stage all")));
                if all && !busy {
                    actions.push(Action::Git(GitOp::StageAll));
                }
                if open {
                    for ch in &unstaged {
                        actions.extend(change_row(ui, file_icons, ch, ch.unstaged.unwrap_or(ch.change), false));
                    }
                }
            }
            if g.truncated {
                ui.label(RichText::new("        More changes than are listed here.").size(11.5).color(pal().text_faint));
            }
        }

        ui.add_space(8.0);
        if group_header(ui, "Graph", "graph", None, None).0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let shown = showing.as_ref().map(|showing| showing.commit.hash.as_str());
            for (i, node) in g.graph.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let on = shown == Some(node.commit.hash.as_str());
                if resp.clicked() {
                    actions.push(Action::ShowCommit((!on).then(|| node.commit.clone())));
                }
                let painter = ui.painter();
                if resp.hovered() || on {
                    painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
                }
                let mid = rect.center().y;
                let x = rect.left() + 18.0;
                let top = if i == 0 { mid } else { rect.top() };
                let bottom = if i + 1 == g.graph.len() { mid } else { rect.bottom() };
                painter.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], (1.5, pal().accent.gamma_multiply(0.7)));
                if i == 0 {
                    painter.circle_filled(Pos2::new(x, mid), 4.5, pal().card);
                    painter.circle_stroke(Pos2::new(x, mid), 4.5, (1.5, pal().accent));
                } else {
                    painter.circle_filled(Pos2::new(x, mid), 3.5, pal().accent);
                }
                let mut right = rect.right() - 10.0;
                for name in node.refs.iter().take(2) {
                    let galley = painter.layout_no_wrap(name.clone(), FontId::proportional(10.5), pal().on_accent);
                    let badge = Rect::from_min_size(
                        Pos2::new(right - galley.size().x - 12.0, mid - 8.0),
                        Vec2::new(galley.size().x + 12.0, 16.0),
                    );
                    let fill = if name.contains('/') { c(theme::get().git_added) } else { pal().accent };
                    painter.rect_filled(badge, 8.0, fill);
                    painter.galley(badge.center() - galley.size() * 0.5, galley, pal().on_accent);
                    right = badge.left() - 5.0;
                }
                let subject = &node.commit.subject;
                text_fit(painter, Pos2::new(x + 12.0, mid), subject, FontId::proportional(12.0), pal().text, right - x - 12.0);
                let hash = &node.commit.hash[..node.commit.hash.len().min(10)];
                resp.on_hover_text(format!(
                    "{}\n{} · {} · {hash}\nClick to see on the canvas what it changed",
                    node.commit.subject,
                    node.commit.author,
                    crate::git::ago(now - node.commit.time)
                ));
            }
            if g.graph.is_empty() {
                ui.label(RichText::new("        No commits yet.").size(11.5).color(pal().text_faint));
            }
        }
        ui.add_space(8.0);
    });
}
