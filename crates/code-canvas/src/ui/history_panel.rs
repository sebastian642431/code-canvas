//! The "All Commits" card: every commit of the repository with the lines of its branches, a search,
//! what to show or hide, and a right click for what can be done with a commit. A click on a commit
//! shows on the canvas what it changed.

use egui::{Align2, Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

use super::{c, menu_gap, menu_row, pal, panel_title, search_field, small_icon, text_fit, Action, NameFor};
use crate::git::{GitOp, GitState, GraphRow, HistoryCommit, ResetMode};
use crate::history::{HistoryView, Mode};
use crate::icons;
use crate::page::Page;
use canvas_render::theme;

const ROW: f32 = 24.0;
const LANE: f32 = 14.0;
const MAX_LANES: usize = 14;
const DETAILS: f32 = 108.0;

/// The color of a lane: the palette goes round, so a lane keeps its color from where it starts to where it ends.
fn lane_color(index: usize) -> Color32 {
    let t = theme::get();
    let palette = [
        t.accent,
        t.git_added,
        t.problem_warning,
        t.tok_keyword,
        t.git_removed,
        t.tok_type,
        t.tok_number,
        t.tok_function,
    ];
    c(palette[index % palette.len()])
}

/// Changes when the repository does (a commit, a branch, a tag): then the list is read again.
fn stamp_of(g: &GitState) -> String {
    let mut stamp = format!("{}\u{1f}{}", g.head, g.branch);
    for name in g.locals.iter().map(|l| l.name.as_str()).chain(g.tags.iter().map(String::as_str)) {
        stamp.push('\u{1f}');
        stamp.push_str(name);
    }
    for remote in &g.remote_branches {
        stamp.push_str(&format!("\u{1f}{}/{}", remote.remote, remote.name));
    }
    stamp
}

fn lane_x(rect: Rect, lane: usize) -> f32 {
    rect.left() + 8.0 + lane.min(MAX_LANES) as f32 * LANE + LANE * 0.5
}

/// The lines and the dot of one row, in the lanes it runs through.
fn draw_lanes(ui: &Ui, rect: Rect, row: &GraphRow, head: bool, on: bool) {
    let painter = ui.painter();
    let (top, mid, bottom) = (rect.top(), rect.center().y, rect.bottom());
    let x = |lane: usize| lane_x(rect, lane);
    let line =
        |from: Pos2, to: Pos2, color: usize| painter.line_segment([from, to], Stroke::new(1.7, lane_color(color)));
    for i in 0..row.width().min(MAX_LANES + 1) {
        let (above, below) = (row.above.get(i).copied().flatten(), row.below.get(i).copied().flatten());
        if i == row.lane {
            if let Some(color) = above {
                line(Pos2::new(x(i), top), Pos2::new(x(i), mid), color);
            }
            if let Some(color) = below {
                line(Pos2::new(x(i), mid), Pos2::new(x(i), bottom), color);
            }
        } else if let Some((_, color)) = row.joins.iter().find(|(lane, _)| *lane == i) {
            line(Pos2::new(x(i), top), Pos2::new(x(row.lane), mid), *color);
        } else if let (Some(color), Some(_)) = (above, below) {
            line(Pos2::new(x(i), top), Pos2::new(x(i), bottom), color);
        }
    }
    for (to, color) in &row.forks {
        line(Pos2::new(x(row.lane), mid), Pos2::new(x(*to), bottom), *color);
    }
    let at = Pos2::new(x(row.lane), mid);
    let color = lane_color(row.color);
    if head {
        painter.circle_filled(at, 5.0, pal().card);
        painter.circle_stroke(at, 5.0, Stroke::new(2.0, color));
    } else {
        painter.circle_filled(at, 4.0, color);
    }
    if on {
        painter.circle_stroke(at, 7.0, Stroke::new(1.2, pal().text));
    }
}

/// A name drawn on a commit: the branch, the tag or the head it carries.
struct Badge {
    label: String,
    fill: Color32,
    text: Color32,
    ring: Option<Color32>,
}

fn badges_of(g: &GitState, refs: &[String]) -> Vec<Badge> {
    let filled =
        |label: &str, fill: Color32| Badge { label: label.to_string(), fill, text: pal().on_accent, ring: None };
    let mut out = Vec::new();
    for name in refs {
        if let Some(branch) = name.strip_prefix("HEAD -> ") {
            out.push(Badge { label: "HEAD".into(), fill: pal().card, text: pal().accent, ring: Some(pal().accent) });
            out.push(filled(branch, pal().accent));
        } else if name == "HEAD" {
            out.push(Badge { label: "HEAD".into(), fill: pal().card, text: pal().accent, ring: Some(pal().accent) });
        } else if let Some(tag) = name.strip_prefix("tag: ") {
            out.push(filled(tag, c(theme::get().git_modified)));
        } else if g.remote_branches.iter().any(|b| format!("{}/{}", b.remote, b.name) == *name) {
            out.push(filled(name, c(theme::get().git_added)));
        } else {
            out.push(filled(name, pal().accent));
        }
    }
    out
}

/// Draws the badges from `x` and gives the x where the next thing can start.
fn draw_badges(ui: &Ui, mut x: f32, mid: f32, limit: f32, badges: &[Badge]) -> f32 {
    let painter = ui.painter();
    for badge in badges {
        let galley = painter.layout_no_wrap(badge.label.clone(), FontId::proportional(10.5), badge.text);
        let width = galley.size().x + 12.0;
        if x + width > limit {
            break;
        }
        let rect = Rect::from_min_size(Pos2::new(x, mid - 8.0), Vec2::new(width, 16.0));
        painter.rect_filled(rect, 8.0, badge.fill);
        if let Some(ring) = badge.ring {
            painter.rect_stroke(rect, 8.0, Stroke::new(1.2, ring), egui::StrokeKind::Inside);
        }
        painter.galley(rect.center() - galley.size() * 0.5, galley, badge.text);
        x = rect.right() + 4.0;
    }
    x
}

fn confirm(op: GitOp, title: String, text: &str, button: &'static str, danger: bool) -> Action {
    Action::GitConfirm { op, title, text: text.to_string(), button, danger }
}

/// What a right click on a commit offers.
fn commit_menu(ui: &mut Ui, g: &GitState, commit: &HistoryCommit, actions: &mut Vec<Action>) {
    ui.set_width(270.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let hash = &commit.hash;
    let short = &hash[..hash.len().min(8)];
    let mut chosen = None;
    if menu_row(ui, icons::ARROW_RIGHT, "Checkout…") {
        let text = "You leave your branch and stand on this commit with no branch: what you commit there belongs to no branch. Switch to a branch to come back.";
        chosen = Some(confirm(
            GitOp::CheckoutCommit(hash.clone()),
            format!("Switch to the commit {short}?"),
            text,
            "Switch",
            false,
        ));
    }
    menu_gap(ui);
    if menu_row(ui, icons::GIT_BRANCH, "New branch here…") {
        chosen = Some(Action::GitName(NameFor::NewBranch { from: Some(hash.clone()) }));
    }
    let tag = crate::codicons::glyph("tag").unwrap_or(icons::GIT_BRANCH);
    if menu_row(ui, tag, "New tag here…") {
        chosen = Some(Action::GitName(NameFor::NewTag { at: Some(hash.clone()) }));
    }
    menu_gap(ui);
    let pick = crate::codicons::glyph("git-pull-request-create").unwrap_or(icons::GIT_BRANCH);
    if menu_row(ui, pick, &format!("Cherry-pick onto '{}'", g.branch)) {
        chosen = Some(Action::Git(GitOp::CherryPick(hash.clone())));
    }
    if menu_row(ui, icons::ROTATE_CCW, "Revert") {
        chosen = Some(Action::Git(GitOp::Revert(hash.clone())));
    }
    menu_gap(ui);
    ui.label(RichText::new(format!("   Reset '{}' to here", g.branch)).size(11.5).color(pal().text_faint));
    let reset = |mode| GitOp::Reset { to: hash.clone(), mode };
    if menu_row(ui, icons::HISTORY, "Soft: keep the changes staged") {
        chosen = Some(Action::Git(reset(ResetMode::Soft)));
    }
    if menu_row(ui, icons::HISTORY, "Mixed: keep the changes unstaged") {
        chosen = Some(Action::Git(reset(ResetMode::Mixed)));
    }
    if menu_row(ui, icons::TRASH_2, "Hard: throw the changes away…") {
        let text = format!(
            "'{}' goes back to {short} and every change you have not committed is lost, with the commits after it (they can still be found with git reflog). This cannot be undone here.",
            g.branch
        );
        chosen =
            Some(confirm(reset(ResetMode::Hard), format!("Reset '{}' to {short}?", g.branch), &text, "Reset", true));
    }
    menu_gap(ui);
    if menu_row(ui, icons::COPY, "Copy hash") {
        ui.ctx().copy_text(hash.clone());
        ui.close();
    }
    if menu_row(ui, icons::COPY, "Copy subject") {
        ui.ctx().copy_text(commit.subject.clone());
        ui.close();
    }
    if let Some(action) = chosen {
        actions.push(action);
        ui.close();
    }
}

/// A small pill with a name and a cross; a click on it takes the choice away.
fn chip(ui: &mut Ui, label: &str, tip: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.5), pal().text);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 32.0, 20.0), Sense::click());
    let fill = if resp.hovered() { pal().hover } else { pal().raised };
    ui.painter().rect(rect, 10.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    let at = Pos2::new(rect.left() + 10.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(at, galley, pal().text);
    let x = Pos2::new(rect.right() - 11.0, rect.center().y);
    ui.painter().text(x, Align2::CENTER_CENTER, icons::X, FontId::proportional(11.0), pal().text_dim);
    resp.on_hover_text(tip).clicked()
}

/// The search box: how to search, and what.
fn search_bar(ui: &mut Ui, history: &mut HistoryView) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(84.0, 28.0), Sense::click());
        let fill = if resp.hovered() { pal().hover } else { pal().well };
        ui.painter().rect(rect, 7.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
        let font = FontId::proportional(12.0);
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            history.mode.label(),
            font,
            pal().text,
        );
        let chevron = FontId::proportional(11.0);
        ui.painter().text(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            Align2::CENTER_CENTER,
            icons::CHEVRON_DOWN,
            chevron,
            pal().text_dim,
        );
        let mut picked = None;
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_width(150.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            for mode in Mode::ALL {
                let mark = if mode == history.mode { icons::CHECK } else { "" };
                if menu_row(ui, mark, mode.label()) {
                    picked = Some(mode);
                    ui.close();
                }
            }
        });
        resp.on_hover_text("What the search looks at");
        if let Some(mode) = picked {
            history.set_mode(mode);
        }
        let hint = history.mode.hint();
        let field = search_field(ui, &mut history.draft, hint, icons::SEARCH);
        if field.changed() {
            history.typed();
        }
    });
}

fn details(ui: &mut Ui, history: &HistoryView, jump: &mut Option<String>) {
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    Frame::NONE
        .fill(pal().well)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(DETAILS - 16.0);
            let commit =
                history.selected.as_deref().and_then(|hash| history.index_of(hash)).map(|i| &history.commits[i]);
            let Some(commit) = commit else {
                let hint =
                    "Click a commit to see on the canvas what it changed. Right click for what can be done with it.";
                ui.label(RichText::new(hint).size(11.5).color(pal().text_faint));
                return;
            };
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.add(egui::Label::new(RichText::new(&commit.subject).size(13.0).color(pal().text)).truncate());
            let who = format!(
                "{} <{}> · {} · {}",
                commit.author,
                commit.email,
                crate::git::date_of(commit.time),
                crate::git::ago(now - commit.time)
            );
            ui.add(egui::Label::new(RichText::new(who).size(11.5).color(pal().text_dim)).truncate());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Hash").size(11.5).color(pal().text_faint));
                let text = RichText::new(&commit.hash).monospace().size(11.5).color(pal().accent);
                let hash = ui.add(egui::Label::new(text).sense(Sense::click()));
                if hash.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Click to copy").clicked() {
                    ui.ctx().copy_text(commit.hash.clone());
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Parents").size(11.5).color(pal().text_faint));
                for parent in &commit.parents {
                    let loaded = history.index_of(parent).is_some();
                    let text = RichText::new(&parent[..parent.len().min(10)]).monospace().size(11.5);
                    let text = text.color(if loaded { pal().accent } else { pal().text_dim });
                    let tip = if loaded { "Click to select it" } else { "Not read yet: scroll the list down" };
                    let label = ui.add(egui::Label::new(text).sense(Sense::click()));
                    if label.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() && loaded {
                        *jump = Some(parent.clone());
                    }
                }
                if commit.parents.is_empty() {
                    ui.label(RichText::new("none: the first commit").size(11.5).color(pal().text_dim));
                }
            });
        });
}

/// What the card shows when the folder has no repository to read.
fn empty_card(ui: &mut Ui, why: &str) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
        panel_title(ui, "All Commits", |_| {});
        ui.add_space(8.0);
        ui.label(RichText::new(why).size(12.0).color(pal().text_faint));
    });
}

pub(super) fn history_panel(ui: &mut Ui, page: Option<&mut Page>, actions: &mut Vec<Action>) {
    let Some(page) = page else { return empty_card(ui, "Open a folder to see its commits.") };
    let Some(g) = page.git.as_ref() else {
        return empty_card(ui, "This folder is not a git repository, or git is still being read.");
    };
    let history = &mut page.history;
    let stamp = stamp_of(g);
    history.update(&page.root, &stamp);
    if let Some(wait) = history.typing_left() {
        ui.ctx().request_repaint_after(wait);
    }
    let shown = page.showing.as_ref().map(|showing| showing.commit.hash.clone());
    let mut reload = false;
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
        panel_title(ui, "All Commits", |ui| {
            reload = small_icon(ui, icons::REFRESH_CW, "Read the history again");
            if history.reading() {
                ui.label(RichText::new("reading …").size(11.0).color(pal().text_faint));
            }
        });
    });
    if reload {
        history.reload();
    }
    Frame::NONE.inner_margin(Margin { left: 12, right: 12, top: 0, bottom: 10 }).show(ui, |ui| {
        search_bar(ui, history);
        let mut query = history.query.clone();
        let hidden = !query.hidden.is_empty();
        if !query.only.is_empty() || hidden {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(5.0, 4.0);
                for name in query.only.clone() {
                    if chip(ui, &format!("Only: {name}"), "Show every branch again") {
                        query.only.retain(|only| *only != name);
                    }
                }
                for name in query.hidden.clone() {
                    if chip(ui, &format!("Hidden: {name}"), "Show this branch again") {
                        query.hidden.retain(|hidden| *hidden != name);
                    }
                }
            });
        }
        history.set_query(query);
        ui.add_space(6.0);

        let total = history.commits.len();
        let height = (ui.available_height() - DETAILS - 8.0).max(80.0);
        let graph = history.graph();
        let wide = if graph { history.width.clamp(1, MAX_LANES + 1) } else { 1 };
        let mut picked: Option<usize> = None;
        let mut last = 0;
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut area =
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(height).id_salt("all commits");
        if let Some(at) = history.scroll_to.take() {
            area = area.vertical_scroll_offset((at as f32 * ROW - height * 0.4).max(0.0));
        }
        let list = &*history;
        area.show_rows(ui, ROW, total, |ui, range| {
            last = range.end;
            for i in range {
                let (commit, lanes) = (&list.commits[i], &list.rows[i]);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let on = list.selected.as_deref() == Some(commit.hash.as_str())
                    || (list.selected.is_none() && shown.as_deref() == Some(commit.hash.as_str()));
                if on {
                    ui.painter().rect_filled(rect, 4.0, pal().row);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, pal().hover);
                }
                let mid = rect.center().y;
                let head = commit.refs.iter().any(|r| r == "HEAD" || r.starts_with("HEAD -> "));
                if graph {
                    draw_lanes(ui, rect, lanes, head, on);
                } else {
                    let dot = Pos2::new(lane_x(rect, 0), mid);
                    ui.painter().circle_filled(dot, 3.5, lane_color(0));
                }
                let first = rect.left() + 8.0 + wide as f32 * LANE + 8.0;
                let hash_x = rect.right() - 68.0;
                let date_x = hash_x - 96.0;
                let author_x = if rect.width() > 560.0 { date_x - 124.0 } else { date_x };
                let badges = badges_of(g, &commit.refs);
                let after = draw_badges(ui, first, mid, author_x - 80.0, &badges);
                let font = FontId::proportional(12.0);
                text_fit(
                    ui.painter(),
                    Pos2::new(after, mid),
                    &commit.subject,
                    font.clone(),
                    pal().text,
                    author_x - after - 8.0,
                );
                if author_x < date_x {
                    text_fit(
                        ui.painter(),
                        Pos2::new(author_x, mid),
                        &commit.author,
                        font.clone(),
                        pal().text_dim,
                        116.0,
                    );
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let ago = crate::git::ago(now - commit.time);
                text_fit(ui.painter(), Pos2::new(date_x, mid), &ago, font, pal().text_faint, 90.0);
                let mono = FontId::monospace(11.0);
                text_fit(
                    ui.painter(),
                    Pos2::new(hash_x, mid),
                    &commit.hash[..commit.hash.len().min(7)],
                    mono,
                    pal().text_faint,
                    60.0,
                );
                if resp.clicked() {
                    picked = Some(i);
                }
                resp.context_menu(|ui| commit_menu(ui, g, commit, actions));
            }
        });
        if last + 60 >= total && history.more {
            history.want_more = true;
        }
        if history.loaded && total == 0 {
            let why = if graph { "No commits yet." } else { "No commits match." };
            ui.label(RichText::new(why).size(12.0).color(pal().text_faint));
        } else if !history.loaded {
            ui.label(RichText::new("Reading the history …").size(12.0).color(pal().text_faint));
        }
        let mut jump = picked.map(|i| history.commits[i].hash.clone());
        ui.add_space(8.0);
        details(ui, history, &mut jump);
        if let Some(hash) = jump {
            if let Some(i) = history.index_of(&hash) {
                if picked.is_none() {
                    history.scroll_to = Some(i);
                }
                actions.push(Action::ShowCommit(Some(history.commits[i].commit())));
                history.selected = Some(hash);
            }
        }
    });
}
