//! The source control panel: what git says changed in the active folder.

use egui::{Align2, Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Vec2};

use super::{c, check, icon, panel_title, small_icon, well, Action, BORDER, ROW_H, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::git::Change;
use crate::icons;
use crate::page::Page;
use canvas_render::theme;

fn change_color(change: Change) -> Color32 {
    c(match change {
        Change::Added | Change::Untracked => theme::GIT_ADDED,
        Change::Deleted | Change::Conflicted => theme::GIT_REMOVED,
        Change::Modified | Change::Renamed => theme::GIT_MODIFIED,
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
        ui.painter().hline(ui.max_rect().x_range(), top - 6.0, (1.0, BORDER));
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(icon(icons::GIT_BRANCH, 13.0, TEXT_DIM));
            ui.label(RichText::new("Timeline").size(12.5).color(TEXT));
            let name = path.rsplit('/').next().unwrap_or(path);
            ui.label(RichText::new(name).size(11.5).color(TEXT_FAINT));
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
                    painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, Color32::from_rgb(20, 22, 27));
                }
                let mid = rect.center().y;
                painter.circle_filled(Pos2::new(rect.left() + 18.0, mid), 3.0, color);
                let rg = painter.layout_no_wrap(right.to_string(), FontId::proportional(11.0), TEXT_FAINT);
                let rx = rect.right() - 12.0 - rg.size().x;
                painter.galley(Pos2::new(rx, mid - rg.size().y * 0.5), rg, TEXT_FAINT);
                let clip =
                    Rect::from_min_max(Pos2::new(rect.left() + 30.0, rect.top()), Pos2::new(rx - 8.0, rect.bottom()));
                painter.with_clip_rect(clip).text(
                    clip.left_center(),
                    Align2::LEFT_CENTER,
                    title,
                    FontId::proportional(12.0),
                    TEXT,
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
                row(ui, TEXT_FAINT, &commit.subject, &crate::git::ago(now - commit.time), tip);
            }
            if page.timeline.loading {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new("Reading the history …").size(11.5).color(TEXT_FAINT));
                });
            } else if page.timeline.commits.is_empty() && pending.is_none() {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new("No history: not committed, or not a git repository.")
                            .size(11.5)
                            .color(TEXT_FAINT),
                    );
                });
            }
        });
    });
}

pub fn changes_panel(ui: &mut egui::Ui, page: Option<&Page>, actions: &mut Vec<Action>) {
    let git = page.and_then(|p| p.git.as_ref());
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Source control", |ui| {
            if small_icon(ui, icons::REFRESH_CW, "Ask git again") {
                actions.push(Action::RefreshGit);
            }
        });
        ui.add_space(4.0);
        let summary = match git {
            None if page.is_none() => "Open a folder to see its changes.".to_string(),
            None => "This folder is not in a git repository.".to_string(),
            Some(g) if g.changes.is_empty() => format!("{}  ·  nothing to commit", g.branch),
            Some(g) => {
                let (ins, del) = g.changes.iter().fold((0, 0), |(i, d), ch| (i + ch.insertions, d + ch.deletions));
                let more = if g.truncated { "+" } else { "" };
                format!("{}  ·  {}{more} changed files  ·  +{ins} −{del}", g.branch, g.changes.len())
            }
        };
        ui.label(RichText::new(summary).size(11.5).color(TEXT_DIM));
        if let (Some(page), Some(g)) = (page, git) {
            if !g.changes.is_empty() {
                ui.add_space(6.0);
                well(ui, |ui| {
                    let mut only = page.only_changed;
                    if check(ui, &mut only, "Show only changed files on canvas", true, false).clicked() {
                        actions.push(Action::SetOnlyChanged(only));
                    }
                });
            }
        }
    });
    let Some(g) = git else { return };

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, g.changes.len(), |ui, range| {
        for change in &g.changes[range] {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            if resp.hovered() {
                painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, Color32::from_rgb(20, 22, 27));
            }
            let mid = rect.center().y;
            let color = change_color(change.change);
            let deleted = change.change == Change::Deleted;
            // Right side: the letter, and how many lines went in and out.
            painter.text(
                Pos2::new(rect.right() - 14.0, mid),
                Align2::RIGHT_CENTER,
                change.change.letter(),
                FontId::monospace(12.0),
                color,
            );
            let mut right = rect.right() - 34.0;
            if change.insertions + change.deletions > 0 {
                let counts = format!("+{} −{}", change.insertions, change.deletions);
                let g = painter.layout_no_wrap(counts, FontId::monospace(11.0), TEXT_FAINT);
                right -= g.size().x;
                painter.galley(Pos2::new(right, mid - g.size().y * 0.5), g, TEXT_FAINT);
                right -= 8.0;
            }
            let (dir, name) = change.path.rsplit_once('/').unwrap_or(("", &change.path));
            let name_color = if deleted { TEXT_DIM } else { TEXT };
            let ng = painter.layout_no_wrap(name.to_string(), FontId::proportional(12.5), name_color);
            let clip = Rect::from_min_max(Pos2::new(rect.left() + 12.0, rect.top()), Pos2::new(right, rect.bottom()));
            let name_w = ng.size().x;
            let name_at = Pos2::new(rect.left() + 12.0, mid - ng.size().y * 0.5);
            let clipped = painter.with_clip_rect(clip);
            clipped.galley(name_at, ng, name_color);
            if deleted {
                // The file is gone: its name is struck through.
                clipped.line_segment([Pos2::new(name_at.x, mid), Pos2::new(name_at.x + name_w, mid)], (1.0, TEXT_DIM));
            }
            clipped.text(
                Pos2::new(name_at.x + name_w + 8.0, mid),
                Align2::LEFT_CENTER,
                dir,
                FontId::proportional(11.5),
                TEXT_FAINT,
            );
            if resp.clicked() && !deleted {
                actions.push(Action::FlyToPath(change.path.clone()));
            }
            resp.on_hover_text(format!("{} · {}", change.path, change.change.label()));
        }
    });
}
