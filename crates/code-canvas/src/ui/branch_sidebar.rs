//! The part of Source Control under the changes: the branches of this computer, the ones on each
//! remote, the tags, the stashes and the reflog, each in a section that opens and closes. A click on a branch
//! switches to it and a right click offers what can be done with it.

use std::collections::{HashMap, HashSet};

use egui::{Align2, FontId, Pos2, Sense, Ui, Vec2};

use super::branch_picker::{local_row, remote_row, row, section_head, tree};
use super::{menu_gap, menu_row, pal, text_fit, Action, NameFor};
use crate::git::{GitOp, GitState, HistoryQuery, LocalBranch, ReflogEntry, RemoteBranch, ResetMode, Stash};
use crate::icons;

/// Which sections and which folders inside them the person closed or opened.
#[derive(Default)]
pub(super) struct Side {
    sections: HashMap<&'static str, bool>,
    shut: HashSet<String>,
}

fn glyph(name: &str, fallback: &'static str) -> &'static str {
    crate::codicons::glyph(name).unwrap_or(fallback)
}

/// A section head like the one of WORKSPACE: a chevron, the title in small capitals and the count.
fn head(ui: &mut Ui, open: bool, title: &str, count: usize) -> bool {
    head_with(ui, open, title, count, None).0
}

/// A section's title with a button of its own before the count; gives (title clicked, button clicked).
fn head_with(ui: &mut Ui, open: bool, title: &str, count: usize, button: Option<(&str, &str)>) -> (bool, bool) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    let mid = rect.center().y;
    let chevron = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
    let painter = ui.painter();
    painter.text(
        Pos2::new(rect.left() + 8.0, mid),
        Align2::LEFT_CENTER,
        chevron,
        FontId::proportional(12.0),
        pal().text_faint,
    );
    let label = title.to_uppercase();
    painter.text(
        Pos2::new(rect.left() + 26.0, mid),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.5),
        pal().text_faint,
    );
    let counted = painter.text(
        Pos2::new(rect.right() - 10.0, mid),
        Align2::RIGHT_CENTER,
        count.to_string(),
        FontId::proportional(11.0),
        pal().text_faint,
    );
    let mut pressed = false;
    if let Some((glyph, tip)) = button {
        let at = egui::Rect::from_center_size(Pos2::new(counted.left() - 14.0, mid), Vec2::splat(20.0));
        let hit = ui.interact(at, ui.scope_id().with(("section button", title)), Sense::click());
        if hit.hovered() {
            ui.painter().rect_filled(at, 5.0, pal().hover);
        }
        let color = if hit.hovered() { pal().text } else { pal().text_dim };
        ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), color);
        pressed = hit.on_hover_text(tip).clicked();
    }
    let toggled = !pressed && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    (toggled, pressed)
}

/// The remote that a branch is pushed to: the one it follows, or else the first one there is.
fn remote_of(g: &GitState, branch: &LocalBranch) -> String {
    let follows = branch.upstream.as_deref().and_then(|up| {
        g.remote_branches.iter().find(|r| up.strip_prefix(r.remote.as_str()).is_some_and(|rest| rest.starts_with('/')))
    });
    let first = g.remote_branches.first();
    follows.or(first).map(|r| r.remote.clone()).unwrap_or_else(|| "origin".to_string())
}

/// The buttons that come up on a branch row, for the All Commits card: show only the commits of
/// this branch, or leave it out. A hidden branch is drawn faint with the eye to bring it back.
fn history_buttons(ui: &mut Ui, row: egui::Rect, key: &str, query: &HistoryQuery, actions: &mut Vec<Action>) {
    let hidden = query.hidden.iter().any(|name| name == key);
    let only = query.only.iter().any(|name| name == key);
    if hidden {
        ui.painter().rect_filled(row, 5.0, pal().row.gamma_multiply(0.6));
    }
    if !(ui.rect_contains_pointer(row) || hidden || only) {
        return;
    }
    let mut x = row.right() - 6.0;
    let mut button = |ui: &mut Ui, id: &str, glyph: &str, on: bool, tip: &str| -> bool {
        let at = egui::Rect::from_center_size(Pos2::new(x - 10.0, row.center().y), Vec2::splat(20.0));
        x -= 22.0;
        let resp = ui.interact(at, ui.scope_id().with(("history button", key, id)), Sense::click());
        ui.painter().rect_filled(at, 5.0, if resp.hovered() { pal().active } else { pal().row });
        let color = if on {
            pal().accent
        } else if resp.hovered() {
            pal().text
        } else {
            pal().text_dim
        };
        ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), color);
        resp.on_hover_text(tip).clicked()
    };
    let tip = if hidden { "Show this branch in the commit list again" } else { "Hide from the commit list" };
    if (hidden || ui.rect_contains_pointer(row))
        && button(ui, "hide", if hidden { icons::EYE_OFF } else { icons::EYE }, hidden, tip)
    {
        actions.push(Action::HistoryHide(key.to_string()));
    }
    if !hidden {
        let tip = if only { "Show every branch in the commit list again" } else { "Show only this branch's commits" };
        let glyph = crate::codicons::glyph("filter").unwrap_or(icons::GIT_GRAPH);
        if button(ui, "only", glyph, only, tip) {
            actions.push(Action::HistoryOnly(key.to_string()));
        }
    }
}

fn confirm(op: GitOp, title: String, text: &str, button: &'static str, danger: bool) -> Action {
    Action::GitConfirm { op, title, text: text.to_string(), button, danger }
}

/// What a right click on a local branch offers.
fn local_menu(ui: &mut Ui, g: &GitState, branch: &LocalBranch, actions: &mut Vec<Action>) {
    ui.set_width(250.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let (name, current) = (&branch.name, &g.branch);
    let can_join = !branch.current && g.locals.iter().any(|l| l.current);
    let mut chosen = None;
    if !branch.current && menu_row(ui, icons::ARROW_RIGHT, "Checkout") {
        chosen = Some(Action::Git(GitOp::Switch(name.clone())));
    }
    if can_join {
        if menu_row(ui, glyph("git-merge", icons::GIT_BRANCH), &format!("Merge into '{current}'")) {
            chosen = Some(Action::Git(GitOp::Merge(name.clone())));
        }
        if menu_row(ui, glyph("git-compare", icons::GIT_BRANCH), &format!("Rebase '{current}' onto '{name}'")) {
            chosen = Some(Action::Git(GitOp::Rebase(name.clone())));
        }
    }
    if !branch.current {
        menu_gap(ui);
    }
    if menu_row(ui, icons::GIT_BRANCH, "New branch from here…") {
        chosen = Some(Action::GitName(NameFor::NewBranch { from: Some(name.clone()) }));
    }
    if menu_row(ui, glyph("tag", icons::GIT_BRANCH), "New tag here…") {
        chosen = Some(Action::GitName(NameFor::NewTag { at: Some(name.clone()) }));
    }
    if menu_row(ui, icons::PENCIL, "Rename…") {
        chosen = Some(Action::GitName(NameFor::RenameBranch { from: name.clone() }));
    }
    if menu_row(ui, icons::ARROW_UP, "Push") {
        chosen = Some(Action::Git(GitOp::PushBranch { remote: remote_of(g, branch), name: name.clone() }));
    }
    menu_gap(ui);
    if menu_row(ui, icons::COPY, "Copy branch name") {
        ui.ctx().copy_text(name.clone());
        ui.close();
    }
    if !branch.current && menu_row(ui, icons::TRASH_2, "Delete…") {
        let text =
            "Git only deletes it when everything in it is already in another branch; if not, it says so and keeps it.";
        chosen = Some(confirm(
            GitOp::DeleteBranch(name.clone()),
            format!("Delete the branch '{name}'?"),
            text,
            "Delete",
            true,
        ));
    }
    if let Some(action) = chosen {
        actions.push(action);
        ui.close();
    }
}

/// What a right click on a branch of a remote offers.
fn remote_menu(ui: &mut Ui, g: &GitState, branch: &RemoteBranch, actions: &mut Vec<Action>) {
    ui.set_width(250.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let full = format!("{}/{}", branch.remote, branch.name);
    let mut chosen = None;
    let (checkout, on) = match &branch.local {
        Some(local) => (GitOp::Switch(local.clone()), g.branch == *local),
        None => (GitOp::Track { remote: branch.remote.clone(), name: branch.name.clone() }, false),
    };
    if !on && menu_row(ui, icons::ARROW_RIGHT, "Checkout") {
        chosen = Some(Action::Git(checkout));
    }
    let join = format!("Merge into '{}'", g.branch);
    if g.locals.iter().any(|l| l.current) && menu_row(ui, glyph("git-merge", icons::GIT_BRANCH), &join) {
        chosen = Some(Action::Git(GitOp::Merge(full.clone())));
    }
    if menu_row(ui, icons::GIT_BRANCH, "New branch from here…") {
        chosen = Some(Action::GitName(NameFor::NewBranch { from: Some(full.clone()) }));
    }
    menu_gap(ui);
    if menu_row(ui, icons::COPY, "Copy branch name") {
        ui.ctx().copy_text(full.clone());
        ui.close();
    }
    let delete = format!("Delete on '{}'…", branch.remote);
    if menu_row(ui, icons::TRASH_2, &delete) {
        let text =
            "The branch is removed from the remote for everyone who uses it. A local branch of the same name stays.";
        let op = GitOp::DeleteRemoteBranch { remote: branch.remote.clone(), name: branch.name.clone() };
        chosen = Some(confirm(op, format!("Delete '{}' on '{}'?", branch.name, branch.remote), text, "Delete", true));
    }
    if let Some(action) = chosen {
        actions.push(action);
        ui.close();
    }
}

fn tag_menu(ui: &mut Ui, tag: &str, actions: &mut Vec<Action>) {
    ui.set_width(230.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let mut chosen = None;
    if menu_row(ui, icons::ARROW_RIGHT, "Checkout") {
        let text = "You leave your branch and stand on the tag with no branch: what you commit there belongs to no branch. Switch to a branch to come back.";
        chosen = Some(confirm(
            GitOp::CheckoutTag(tag.to_string()),
            format!("Switch to the tag '{tag}'?"),
            text,
            "Switch",
            false,
        ));
    }
    if menu_row(ui, icons::ARROW_UP, "Push to origin") {
        chosen = Some(Action::Git(GitOp::PushTag(tag.to_string())));
    }
    menu_gap(ui);
    if menu_row(ui, icons::COPY, "Copy tag name") {
        ui.ctx().copy_text(tag.to_string());
        ui.close();
    }
    if menu_row(ui, icons::TRASH_2, "Delete…") {
        let text = "Only the tag on this computer goes; if it was pushed, it stays on the remote.";
        chosen =
            Some(confirm(GitOp::DeleteTag(tag.to_string()), format!("Delete the tag '{tag}'?"), text, "Delete", true));
    }
    if let Some(action) = chosen {
        actions.push(action);
        ui.close();
    }
}

fn stash_menu(ui: &mut Ui, stash: &Stash, actions: &mut Vec<Action>) {
    ui.set_width(200.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let mut chosen = None;
    if menu_row(ui, glyph("git-stash-apply", icons::HISTORY), "Apply") {
        chosen = Some(Action::Git(GitOp::StashApply(stash.index)));
    }
    if menu_row(ui, glyph("git-stash-pop", icons::HISTORY), "Pop") {
        chosen = Some(Action::Git(GitOp::StashPopAt(stash.index)));
    }
    menu_gap(ui);
    if menu_row(ui, icons::TRASH_2, "Drop…") {
        let text = format!("The changes saved in \"{}\" are lost. This cannot be undone.", stash.message);
        chosen = Some(confirm(GitOp::StashDrop(stash.index), "Drop this stash?".to_string(), &text, "Drop", true));
    }
    if let Some(action) = chosen {
        actions.push(action);
        ui.close();
    }
}

/// What a right click on a line of the reflog offers: go back to where HEAD was (the "undo").
fn reflog_menu(ui: &mut Ui, g: &GitState, entry: &ReflogEntry, actions: &mut Vec<Action>) {
    ui.set_width(270.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let hash = &entry.hash;
    let short = &hash[..hash.len().min(8)];
    let mut chosen = None;
    if menu_row(ui, icons::HISTORY, "Reset current branch to here…") {
        let text = format!(
            "'{}' goes back to {short} and every change you have not committed is lost. The commits after it can be found again here, in the reflog.",
            g.branch
        );
        let op = GitOp::Reset { to: hash.clone(), mode: ResetMode::Hard };
        chosen = Some(confirm(op, format!("Reset '{}' to {short}?", g.branch), &text, "Reset", true));
    }
    if menu_row(ui, icons::GIT_BRANCH, "New branch here…") {
        chosen = Some(Action::GitName(NameFor::NewBranch { from: Some(hash.clone()) }));
    }
    menu_gap(ui);
    if menu_row(ui, icons::COPY, "Copy hash") {
        ui.ctx().copy_text(hash.clone());
        ui.close();
    }
    if let Some(action) = chosen {
        actions.push(action);
        ui.close();
    }
}

/// A flat row with a glyph, a label and a faint note on the right.
fn plain_row(ui: &mut Ui, glyph: &str, label: &str, note: &str, tip: &str) -> egui::Response {
    let (rect, resp) = row(ui);
    let mid = rect.center().y;
    ui.painter().text(
        Pos2::new(rect.left() + 22.0, mid),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(13.0),
        pal().text_dim,
    );
    let mut right = rect.right() - 8.0;
    if !note.is_empty() {
        let galley = ui.painter().layout_no_wrap(note.to_string(), FontId::proportional(11.0), pal().text_faint);
        right -= galley.size().x;
        ui.painter().galley(Pos2::new(right, mid - galley.size().y * 0.5), galley, pal().text_faint);
        right -= 6.0;
    }
    text_fit(
        ui.painter(),
        Pos2::new(rect.left() + 40.0, mid),
        label,
        FontId::proportional(12.5),
        pal().text,
        right - rect.left() - 44.0,
    );
    resp.on_hover_text(tip)
}

impl Side {
    fn open(&self, key: &'static str) -> bool {
        self.sections.get(key).copied().unwrap_or(key == "branches" || key == "remotes")
    }

    /// Draws the head of a section and says whether its rows are to be shown.
    fn section(&mut self, ui: &mut Ui, key: &'static str, title: &str, count: usize) -> bool {
        let open = self.open(key);
        if head(ui, open, title, count) {
            self.sections.insert(key, !open);
        }
        open
    }

    pub(super) fn show(&mut self, ui: &mut Ui, g: &GitState, query: &HistoryQuery, actions: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add_space(8.0);
        if !g.locals.is_empty() && self.section(ui, "branches", "Branches", g.locals.len()) {
            let names: Vec<&str> = g.locals.iter().map(|l| l.name.as_str()).collect();
            let mut leaf = |ui: &mut Ui, full: &str, label: &str, indent: f32| {
                let Some(branch) = g.locals.iter().find(|l| l.name == full) else { return };
                let resp = local_row(ui, branch, label, indent);
                if resp.clicked() && !branch.current {
                    actions.push(Action::Git(GitOp::Switch(full.to_string())));
                }
                history_buttons(ui, resp.rect, full, query, actions);
                resp.context_menu(|ui| local_menu(ui, g, branch, actions));
            };
            tree(ui, &mut self.shut, "side local", false, &names, "", &mut leaf);
        }
        let mut remote_names: Vec<&str> = Vec::new();
        for remote in &g.remote_branches {
            if !remote_names.contains(&remote.remote.as_str()) {
                remote_names.push(&remote.remote);
            }
        }
        let open = self.open("remotes");
        let fetch = (glyph("repo-fetch", icons::REFRESH_CW), "Fetch from every remote (git fetch --all --prune)");
        let (toggled, fetched) = head_with(ui, open, "Remotes", g.remote_branches.len(), Some(fetch));
        if toggled {
            self.sections.insert("remotes", !open);
        }
        if fetched {
            actions.push(Action::Git(GitOp::Fetch));
        }
        if !remote_names.is_empty() && open {
            for remote_name in remote_names {
                let names: Vec<&str> =
                    g.remote_branches.iter().filter(|r| r.remote == remote_name).map(|r| r.name.as_str()).collect();
                section_head(ui, icons::GITHUB, remote_name, names.len());
                let mut leaf = |ui: &mut Ui, full: &str, label: &str, indent: f32| {
                    let Some(branch) = g.remote_branches.iter().find(|r| r.remote == remote_name && r.name == full)
                    else {
                        return;
                    };
                    let resp = remote_row(ui, branch, label, indent);
                    if resp.clicked() {
                        match &branch.local {
                            Some(local) if *local == g.branch => {}
                            Some(local) => actions.push(Action::Git(GitOp::Switch(local.clone()))),
                            None => actions.push(Action::Git(GitOp::Track {
                                remote: remote_name.to_string(),
                                name: full.to_string(),
                            })),
                        }
                    }
                    let key = format!("{}/{}", branch.remote, branch.name);
                    history_buttons(ui, resp.rect, &key, query, actions);
                    resp.context_menu(|ui| remote_menu(ui, g, branch, actions));
                };
                tree(ui, &mut self.shut, &format!("side {remote_name}"), false, &names, "", &mut leaf);
            }
        }
        if self.section(ui, "tags", "Tags", g.tags.len()) {
            let icon = glyph("tag", icons::GIT_BRANCH);
            for tag in &g.tags {
                plain_row(ui, icon, tag, "", tag).context_menu(|ui| tag_menu(ui, tag, actions));
            }
            if g.tags.is_empty() {
                ui.label(egui::RichText::new("        No tags.").size(11.5).color(pal().text_faint));
            }
        }
        if self.section(ui, "stashes", "Stashes", g.stashes.len()) {
            let icon = glyph("git-stash", icons::HISTORY);
            for stash in &g.stashes {
                let name = format!("stash@{{{}}}", stash.index);
                let label = if stash.message.is_empty() { name.as_str() } else { stash.message.as_str() };
                let tip = format!("{label}\nRight click: apply, pop or drop it");
                plain_row(ui, icon, label, &name, &tip).context_menu(|ui| stash_menu(ui, stash, actions));
            }
            if g.stashes.is_empty() {
                ui.label(egui::RichText::new("        Nothing is stashed.").size(11.5).color(pal().text_faint));
            }
        }
        if self.section(ui, "reflog", "Reflog", g.reflog.len()) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let icon = glyph("history", icons::HISTORY);
            for entry in &g.reflog {
                let (hash, ago) = (&entry.hash[..entry.hash.len().min(8)], crate::git::ago(now - entry.time));
                let tip = format!(
                    "{}\n{hash} · {} · {ago}\nClick to select it in All Commits. Right click to go back to it.",
                    entry.what, entry.name
                );
                let resp = plain_row(ui, icon, &entry.what, &ago, &tip);
                if resp.clicked() {
                    actions.push(Action::HistorySelect(entry.hash.clone()));
                }
                resp.context_menu(|ui| reflog_menu(ui, g, entry, actions));
            }
            if g.reflog.is_empty() {
                ui.label(egui::RichText::new("        Nothing yet.").size(11.5).color(pal().text_faint));
            }
        }
        ui.add_space(4.0);
    }
}
