//! The source control panel: what git says changed in the active folder.

use egui::{Align2, Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Vec2};

use super::{c, check, icon, pal, panel_title, small_icon, Action, ROW_H};
use crate::git::{Change, FileChange, GitOp};
use crate::icons;
use crate::page::Page;
use canvas_render::theme;

fn change_color(change: Change) -> Color32 {
    c(match change {
        Change::Added | Change::Untracked => theme::get().git_added,
        Change::Deleted | Change::Conflicted => theme::get().git_removed,
        Change::Modified | Change::Renamed => theme::get().git_modified,
    })
}

/// History of the selected file, under the explorer: the commits that
/// touched it, newest first, and whether it has changes not committed yet.
pub fn timeline(ui: &mut egui::Ui, page: Option<&Page>) {
    let Some(page) = page else { return };
    let Some(path) = &page.timeline.path else { return };
    let panel = egui::Panel::bottom("timeline").resizable(true).default_size(190.0).size_range(90.0..=520.0);
    panel.frame(Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 6 })).show(ui, |ui| {
        let top = ui.max_rect().top();
        ui.painter().hline(ui.max_rect().x_range(), top - 6.0, (1.0, pal().border));
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(icon(icons::GIT_BRANCH, 13.0, pal().text_dim));
            ui.label(RichText::new("Timeline").size(12.5).color(pal().text));
            let name = path.rsplit('/').next().unwrap_or(path);
            ui.label(RichText::new(name).size(11.5).color(pal().text_faint));
        });
        let pending = page.git.as_ref().and_then(|g| g.changes.iter().find(|ch| ch.path == *path));
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, color: Color32, title: &str, right: &str, tip: String| {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
                let painter = ui.painter();
                if resp.hovered() {
                    painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
                }
                let mid = rect.center().y;
                painter.circle_filled(Pos2::new(rect.left() + 18.0, mid), 3.0, color);
                let rg = painter.layout_no_wrap(right.to_string(), FontId::proportional(11.0), pal().text_faint);
                let rx = rect.right() - 12.0 - rg.size().x;
                painter.galley(Pos2::new(rx, mid - rg.size().y * 0.5), rg, pal().text_faint);
                let clip =
                    Rect::from_min_max(Pos2::new(rect.left() + 30.0, rect.top()), Pos2::new(rx - 8.0, rect.bottom()));
                painter.with_clip_rect(clip).text(
                    clip.left_center(),
                    Align2::LEFT_CENTER,
                    title,
                    FontId::proportional(12.0),
                    pal().text,
                );
                resp.on_hover_text(tip);
            };
            if let Some(ch) = pending {
                let detail = format!("+{} −{}", ch.insertions, ch.deletions);
                row(
                    ui,
                    change_color(ch.change),
                    "Uncommitted changes",
                    &detail,
                    format!("{} · {}", ch.path, ch.change.label()),
                );
            }
            for commit in &page.timeline.commits {
                let tip =
                    format!("{}\n{} · {}", commit.subject, commit.author, &commit.hash[..commit.hash.len().min(10)]);
                row(ui, pal().text_faint, &commit.subject, &crate::git::ago(now - commit.time), tip);
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

/// A section header like VS Code's: a chevron, a title, and a count.
fn group_header(ui: &mut egui::Ui, title: &str, count: Option<usize>, button: Option<(&str, &str)>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    let painter = ui.painter();
    let mid = rect.center().y;
    painter.text(
        Pos2::new(rect.left() + 8.0, mid),
        Align2::LEFT_CENTER,
        icons::CHEVRON_DOWN,
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
        if resp.hovered() || ui.rect_contains_pointer(rect) {
            let at = Rect::from_center_size(Pos2::new(right - 11.0, mid), Vec2::splat(22.0));
            let r = ui.interact(at, ui.scope_id().with(("group", title)), Sense::click());
            if r.hovered() {
                ui.painter().rect_filled(at, 5.0, pal().hover);
            }
            ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), pal().text);
            clicked = r.on_hover_text(tip).clicked();
        }
    }
    clicked
}

/// One changed file in a group. Returns what the user asked for.
fn change_row(ui: &mut egui::Ui, change: &FileChange, side: Change, staged: bool) -> Option<Action> {
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
    let mut action = None;
    if hovered {
        // Stage or unstage, shown while the pointer is on the row.
        let at = Rect::from_center_size(Pos2::new(right - 11.0, mid), Vec2::splat(22.0));
        let (glyph, tip) = if staged { (icons::MINUS, "Unstage") } else { (icons::PLUS, "Stage") };
        let r = ui.interact(at, ui.scope_id().with(("stage", staged, &change.path)), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(at, 5.0, pal().hover);
        }
        ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), pal().text);
        if r.on_hover_text(tip).clicked() {
            let op = if staged { GitOp::Unstage(change.path.clone()) } else { GitOp::Stage(change.path.clone()) };
            action = Some(Action::Git(op));
        }
        right = at.left() - 2.0;
        if !staged && side != Change::Untracked {
            let at = Rect::from_center_size(Pos2::new(right - 11.0, mid), Vec2::splat(22.0));
            let r = ui.interact(at, ui.scope_id().with(("discard", &change.path)), Sense::click());
            if r.hovered() {
                ui.painter().rect_filled(at, 5.0, pal().hover);
            }
            let glyph = crate::codicons::glyph("discard").unwrap_or(icons::ARROW_LEFT);
            ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), pal().text);
            if r.on_hover_text("Discard changes").clicked() {
                action = Some(Action::Git(GitOp::Discard(change.path.clone())));
            }
            right = at.left() - 2.0;
        }
        right -= 2.0;
    }
    let painter = ui.painter();
    let (dir, name) = change.path.rsplit_once('/').unwrap_or(("", &change.path));
    let name_color = if deleted { pal().text_dim } else { color.lerp_to_gamma(pal().text, 0.35) };
    let ng = painter.layout_no_wrap(name.to_string(), FontId::proportional(12.5), name_color);
    let clip = Rect::from_min_max(Pos2::new(rect.left() + 26.0, rect.top()), Pos2::new(right, rect.bottom()));
    let name_w = ng.size().x;
    let name_at = Pos2::new(rect.left() + 26.0, mid - ng.size().y * 0.5);
    let clipped = painter.with_clip_rect(clip);
    clipped.galley(name_at, ng, name_color);
    if deleted {
        clipped.line_segment([Pos2::new(name_at.x, mid), Pos2::new(name_at.x + name_w, mid)], (1.0, pal().text_dim));
    }
    clipped.text(
        Pos2::new(name_at.x + name_w + 8.0, mid),
        Align2::LEFT_CENTER,
        dir,
        FontId::proportional(11.5),
        pal().text_faint,
    );
    if action.is_none() && resp.clicked() && !deleted {
        action = Some(Action::FlyToPath(change.path.clone()));
    }
    resp.on_hover_text(format!("{} · {}", change.path, side.label()));
    action
}

/// Source control, laid out like VS Code's view: the repository and its
/// branch, the commit message and button, staged and unstaged changes, and
/// the latest commits.
pub fn changes_panel(ui: &mut egui::Ui, page: Option<&mut Page>, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 2 }).show(ui, |ui| {
        panel_title(ui, "Source control", |ui| {
            if small_icon(ui, icons::REFRESH_CW, "Ask git again") {
                actions.push(Action::RefreshGit);
            }
        });
    });
    let Some(page) = page else {
        ui.label(RichText::new("    Open a folder to see its changes.").size(11.5).color(pal().text_dim));
        return;
    };
    let busy = page.git_busy();
    if page.git_loading() {
        ui.label(RichText::new("    Reading the repository …").size(11.5).color(pal().text_dim));
        return;
    }
    let Page { git, commit_message, only_changed, git_note, title, .. } = page;
    let Some(g) = git.as_ref() else {
        ui.label(RichText::new("    This folder is not in a git repository.").size(11.5).color(pal().text_dim));
        return;
    };
    let staged: Vec<&FileChange> = g.changes.iter().filter(|ch| ch.staged.is_some()).collect();
    let unstaged: Vec<&FileChange> = g.changes.iter().filter(|ch| ch.unstaged.is_some()).collect();

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // --- the repository and its branch ---------------------------------
        group_header(ui, "Repositories", None, None);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
        let mid = rect.center().y;
        ui.painter().text(
            Pos2::new(rect.left() + 26.0, mid),
            Align2::LEFT_CENTER,
            icons::BOOK_MARKED,
            FontId::proportional(13.0),
            pal().text_dim,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 46.0, mid),
            Align2::LEFT_CENTER,
            title.as_str(),
            FontId::proportional(12.5),
            pal().text,
        );
        let sync_at = Rect::from_center_size(Pos2::new(rect.right() - 20.0, mid), Vec2::splat(22.0));
        let sync = ui.interact(sync_at, ui.scope_id().with("sync"), Sense::click());
        if sync.hovered() {
            ui.painter().rect_filled(sync_at, 5.0, pal().hover);
        }
        ui.painter().text(
            sync_at.center(),
            Align2::CENTER_CENTER,
            icons::REPEAT_2,
            FontId::proportional(13.0),
            pal().text_dim,
        );
        if sync.on_hover_text("Sync: pull (fast-forward only), then push").clicked() && !busy {
            actions.push(Action::Git(GitOp::Sync));
        }
        let mut branch = format!("{}  {}{}", icons::GIT_BRANCH, g.branch, if g.changes.is_empty() { "" } else { "*" });
        if g.ahead > 0 {
            branch.push_str(&format!("  ↑{}", g.ahead));
        }
        if g.behind > 0 {
            branch.push_str(&format!("  ↓{}", g.behind));
        }
        // The branch, on the right of the row; a click lists the others.
        let galley = ui.painter().layout_no_wrap(branch.clone(), FontId::proportional(12.0), pal().text_dim);
        let at = Rect::from_min_size(
            Pos2::new(sync_at.left() - 12.0 - galley.size().x, rect.top() + 1.0),
            Vec2::new(galley.size().x + 8.0, ROW_H - 2.0),
        );
        let mut inner =
            ui.new_child(egui::UiBuilder::new().max_rect(at).layout(egui::Layout::right_to_left(egui::Align::Center)));
        inner
            .menu_button(RichText::new(branch).size(12.0).color(pal().text_dim), |ui| {
                ui.set_min_width(200.0);
                ui.label(RichText::new("Switch to a branch").size(11.0).color(pal().text_faint));
                for name in &g.branches {
                    if ui.selectable_label(*name == g.branch, name).clicked() {
                        if *name != g.branch && !busy {
                            actions.push(Action::Git(GitOp::Switch(name.clone())));
                        }
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Branch: click to switch");

        // --- message and commit ---------------------------------------------
        group_header(ui, "Changes", None, None);
        let mut commit = false;
        Frame::NONE.inner_margin(Margin { left: 26, right: 10, top: 2, bottom: 6 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let hint = format!("Message (Ctrl+Enter to commit on \"{}\")", g.branch);
            let edit = Frame::NONE
                .fill(pal().well)
                .stroke(egui::Stroke::new(1.0, pal().border))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(commit_message)
                            .hint_text(hint)
                            .frame(Frame::NONE)
                            .desired_rows(1)
                            .desired_width(f32::INFINITY),
                    )
                })
                .inner;
            commit |= edit.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
            let ready = !busy && !commit_message.trim().is_empty() && !g.changes.is_empty();
            let label = match (busy, staged.is_empty()) {
                (true, _) => "Working …".to_string(),
                (false, true) => format!("{}  Commit all changes", icons::CHECK),
                (false, false) => format!("{}  Commit", icons::CHECK),
            };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
            let fill = match (ready, resp.hovered()) {
                (false, _) => pal().raised,
                (true, false) => pal().accent.gamma_multiply(0.85),
                (true, true) => pal().accent,
            };
            ui.painter().rect_filled(rect, 6.0, fill);
            let text_color = if ready { pal().on_accent } else { pal().text_faint };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.5), text_color);
            let tip = if staged.is_empty() {
                "Nothing is staged: this stages every change and commits it"
            } else {
                "Commit the staged changes"
            };
            commit |= resp.on_hover_text(tip).clicked();
            if commit && ready {
                let message = commit_message.trim().to_string();
                if staged.is_empty() {
                    actions.push(Action::Git(GitOp::CommitAll(message)));
                } else {
                    actions.push(Action::Git(GitOp::Commit(message)));
                }
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
        });

        // --- staged and unstaged changes ------------------------------------
        if !staged.is_empty() {
            if group_header(ui, "Staged Changes", Some(staged.len()), Some((icons::MINUS, "Unstage all"))) && !busy {
                actions.push(Action::Git(GitOp::UnstageAll));
            }
            for ch in &staged {
                actions.extend(change_row(ui, ch, ch.staged.unwrap_or(ch.change), true));
            }
        }
        if group_header(ui, "Changes", Some(unstaged.len()), Some((icons::PLUS, "Stage all"))) && !busy {
            actions.push(Action::Git(GitOp::StageAll));
        }
        for ch in &unstaged {
            actions.extend(change_row(ui, ch, ch.unstaged.unwrap_or(ch.change), false));
        }
        if g.changes.is_empty() {
            ui.label(RichText::new("        Nothing to commit.").size(11.5).color(pal().text_faint));
        }
        if g.truncated {
            ui.label(RichText::new("        More changes than are listed here.").size(11.5).color(pal().text_faint));
        }

        // --- the latest commits -----------------------------------------------
        ui.add_space(8.0);
        group_header(ui, "Graph", None, None);
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        for (i, node) in g.graph.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            if resp.hovered() {
                painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
            }
            let mid = rect.center().y;
            let x = rect.left() + 18.0;
            // The line of history, with a ring for the commit the branch is on.
            let top = if i == 0 { mid } else { rect.top() };
            let bottom = if i + 1 == g.graph.len() { mid } else { rect.bottom() };
            painter.line_segment([Pos2::new(x, top), Pos2::new(x, bottom)], (1.5, pal().accent.gamma_multiply(0.7)));
            if i == 0 {
                painter.circle_filled(Pos2::new(x, mid), 4.5, pal().card);
                painter.circle_stroke(Pos2::new(x, mid), 4.5, (1.5, pal().accent));
            } else {
                painter.circle_filled(Pos2::new(x, mid), 3.5, pal().accent);
            }
            // Branch and tag names on the right, the subject in what is left.
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
            let clip = Rect::from_min_max(Pos2::new(x + 12.0, rect.top()), Pos2::new(right, rect.bottom()));
            let clipped = painter.with_clip_rect(clip);
            let sg = clipped.layout_no_wrap(node.commit.subject.clone(), FontId::proportional(12.0), pal().text);
            let sw = sg.size().x;
            clipped.galley(Pos2::new(clip.left(), mid - sg.size().y * 0.5), sg, pal().text);
            clipped.text(
                Pos2::new(clip.left() + sw + 8.0, mid),
                Align2::LEFT_CENTER,
                &node.commit.author,
                FontId::proportional(11.0),
                pal().text_faint,
            );
            let hash = &node.commit.hash[..node.commit.hash.len().min(10)];
            resp.on_hover_text(format!(
                "{}\n{} · {} · {hash}",
                node.commit.subject,
                node.commit.author,
                crate::git::ago(now - node.commit.time)
            ));
        }
        if g.graph.is_empty() {
            ui.label(RichText::new("        No commits yet.").size(11.5).color(pal().text_faint));
        }
        ui.add_space(8.0);
    });
}
