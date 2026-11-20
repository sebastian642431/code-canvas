//! The branch picker that opens from the branch chip of a workspace folder: the branches of this
//! computer and the ones on each remote (GitHub), in folders by their "/" prefixes.

use std::collections::{BTreeSet, HashSet};

use egui::{Align, Align2, FontId, Layout, Pos2, Rect, Response, Sense, Ui, Vec2};

use super::{pal, text_fit, Action};
use crate::git::{GitOp, LocalBranch, RemoteBranch};
use crate::icons;

const ROW: f32 = 22.0;
const INDENT: f32 = 12.0;

/// What the picker is showing and what the person typed in its filter.
pub(super) struct Picker<'a> {
    pub filter: &'a mut String,
    pub shut: &'a mut HashSet<String>,
    pub opening: bool,
    pub switching: bool,
    pub locals: &'a [LocalBranch],
    pub remotes: &'a [RemoteBranch],
}

pub(super) fn matches(filter: &str, name: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(filter)
}

/// One full-width row with the hover shade already painted.
pub(super) fn row(ui: &mut Ui) -> (Rect, Response) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, pal().hover);
    }
    (rect, resp)
}

/// Draws the folders of `names` under `prefix` (open or closed) and then the branches directly in it.
pub(super) fn tree(
    ui: &mut Ui,
    shut: &mut HashSet<String>,
    section: &str,
    force_open: bool,
    names: &[&str],
    prefix: &str,
    leaf: &mut dyn FnMut(&mut Ui, &str, &str, f32),
) {
    let mut folders = BTreeSet::new();
    let mut leaves = Vec::new();
    for full in names {
        let Some(rest) = full.strip_prefix(prefix) else { continue };
        match rest.split_once('/') {
            Some((dir, _)) => {
                folders.insert(dir);
            }
            None => leaves.push((*full, rest)),
        }
    }
    let indent = prefix.matches('/').count() as f32 * INDENT;
    for dir in folders {
        let path = format!("{prefix}{dir}/");
        let key = format!("{section}\u{1f}{path}");
        let open = force_open || !shut.contains(&key);
        let (rect, resp) = row(ui);
        let mid = rect.center().y;
        let glyph = if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
        let font = FontId::proportional(12.0);
        ui.painter().text(
            Pos2::new(rect.left() + 6.0 + indent, mid),
            Align2::LEFT_CENTER,
            glyph,
            font,
            pal().text_faint,
        );
        let folder = if open { icons::FOLDER_OPEN } else { icons::FOLDER };
        let font = FontId::proportional(13.0);
        ui.painter().text(
            Pos2::new(rect.left() + 22.0 + indent, mid),
            Align2::LEFT_CENTER,
            folder,
            font,
            pal().text_dim,
        );
        let font = FontId::proportional(12.5);
        text_fit(
            ui.painter(),
            Pos2::new(rect.left() + 40.0 + indent, mid),
            dir,
            font,
            pal().text_dim,
            rect.width() - 48.0 - indent,
        );
        if resp.clicked() && !force_open && !shut.remove(&key) {
            shut.insert(key);
        }
        if open {
            tree(ui, shut, section, force_open, names, &path, leaf);
        }
    }
    for (full, label) in leaves {
        leaf(ui, full, label, indent);
    }
}

/// A row of a local branch: its name, how far ahead or behind its remote it is, and a check on the
/// branch that is switched to (a spinner while git is still getting there). The tip says what a
/// click does.
pub(super) fn local_row(ui: &mut Ui, branch: &LocalBranch, label: &str, indent: f32, switching: bool) -> Response {
    let full = &branch.name;
    let (rect, resp) = row(ui);
    let mid = rect.center().y;
    let tint = if branch.current { pal().accent } else { pal().text };
    let font = FontId::proportional(13.0);
    ui.painter().text(Pos2::new(rect.left() + 22.0 + indent, mid), Align2::LEFT_CENTER, icons::GIT_BRANCH, font, tint);
    let mut gap = String::new();
    for (count, arrow) in [(branch.ahead, icons::ARROW_UP), (branch.behind, icons::ARROW_DOWN)] {
        if count > 0 {
            gap.push_str(&format!("{}{count}{arrow}", if gap.is_empty() { "" } else { " " }));
        }
    }
    let mut right = rect.right() - 8.0;
    if !gap.is_empty() {
        let galley = ui.painter().layout_no_wrap(gap, FontId::proportional(11.0), pal().text_faint);
        right -= galley.size().x;
        ui.painter().galley(Pos2::new(right, mid - galley.size().y * 0.5), galley, pal().text_faint);
        right -= 6.0;
    }
    if branch.current && switching {
        let at = Rect::from_center_size(Pos2::new(right - 6.0, mid), Vec2::splat(14.0));
        egui::Spinner::new().size(14.0).color(pal().accent).paint_at(ui, at);
        right -= 18.0;
    } else if branch.current {
        let font = FontId::proportional(12.0);
        ui.painter().text(Pos2::new(right, mid), Align2::RIGHT_CENTER, icons::CHECK, font, pal().accent);
        right -= 18.0;
    }
    let font = FontId::proportional(12.5);
    text_fit(
        ui.painter(),
        Pos2::new(rect.left() + 40.0 + indent, mid),
        label,
        font,
        tint,
        right - rect.left() - 44.0 - indent,
    );
    let tip = match (branch.current, switching) {
        (true, true) => format!("Switching to {full}…"),
        (true, false) => format!("{full}: the branch you are on"),
        (false, _) => format!("Switch to {full}"),
    };
    resp.on_hover_text(tip)
}

/// A row of a branch on a remote: its name, "local" when a local branch follows it, and a button
/// to bring it to local when none does.
pub(super) fn remote_row(ui: &mut Ui, branch: &RemoteBranch, label: &str, indent: f32) -> Response {
    let full = &branch.name;
    let (rect, resp) = row(ui);
    let mid = rect.center().y;
    let font = FontId::proportional(13.0);
    ui.painter().text(
        Pos2::new(rect.left() + 22.0 + indent, mid),
        Align2::LEFT_CENTER,
        icons::GIT_BRANCH,
        font,
        pal().text_dim,
    );
    let mut right = rect.right() - 8.0;
    match &branch.local {
        Some(_) => {
            let galley = ui.painter().layout_no_wrap("local".into(), FontId::proportional(11.0), pal().text_faint);
            right -= galley.size().x;
            ui.painter().galley(Pos2::new(right, mid - galley.size().y * 0.5), galley, pal().text_faint);
            right -= 6.0;
        }
        None if resp.hovered() => {
            let button = Rect::from_center_size(Pos2::new(rect.right() - 14.0, mid), Vec2::splat(20.0));
            ui.painter().rect_filled(button, 5.0, pal().border_strong);
            let font = FontId::proportional(13.0);
            ui.painter().text(button.center(), Align2::CENTER_CENTER, icons::DOWNLOAD, font, pal().text);
            right = button.left() - 4.0;
        }
        None => {}
    }
    let font = FontId::proportional(12.5);
    text_fit(
        ui.painter(),
        Pos2::new(rect.left() + 40.0 + indent, mid),
        label,
        font,
        pal().text,
        right - rect.left() - 44.0 - indent,
    );
    let tip = match &branch.local {
        Some(local) => format!("Switch to {local}, which follows {full}"),
        None => format!("Bring '{full}' to local and switch to it"),
    };
    resp.on_hover_text(tip)
}

pub(super) fn section_head(ui: &mut Ui, glyph: &str, title: &str, count: usize) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
    let mid = rect.center().y;
    ui.painter().text(
        Pos2::new(rect.left() + 6.0, mid),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(12.0),
        pal().text_faint,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 24.0, mid),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        FontId::proportional(11.0),
        pal().text_faint,
    );
    ui.painter().text(
        Pos2::new(rect.right() - 6.0, mid),
        Align2::RIGHT_CENTER,
        count.to_string(),
        FontId::proportional(11.0),
        pal().text_faint,
    );
}

impl Picker<'_> {
    pub(super) fn show(self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Picker { filter, shut, opening, switching, locals, remotes } = self;
        ui.set_width(260.0);
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Branches").size(12.0).strong().color(pal().text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 5.0, pal().border_strong);
                }
                let font = FontId::proportional(13.0);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, icons::REFRESH_CW, font, pal().text_dim);
                let tip = "Fetch from the remotes: get the branches that are on GitHub";
                if resp.on_hover_text(tip).clicked() {
                    actions.push(Action::Git(GitOp::Fetch));
                }
            });
        });
        let edit = egui::TextEdit::singleline(filter)
            .hint_text("Filter branches")
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(6, 4));
        let edit = ui.add(edit);
        if opening {
            edit.request_focus();
        }
        ui.add_space(4.0);
        let needle = filter.trim().to_lowercase();
        let force_open = !needle.is_empty();
        let mut shown = 0;
        egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let names: Vec<&str> = locals.iter().map(|l| l.name.as_str()).filter(|n| matches(&needle, n)).collect();
            if !names.is_empty() {
                shown += names.len();
                section_head(ui, icons::GIT_BRANCH, "Local", names.len());
                let mut leaf = |ui: &mut Ui, full: &str, label: &str, indent: f32| {
                    let Some(branch) = locals.iter().find(|l| l.name == full) else { return };
                    if local_row(ui, branch, label, indent, switching).clicked() && !branch.current {
                        actions.push(Action::Git(GitOp::Switch(full.to_string())));
                        ui.close();
                    }
                };
                tree(ui, shut, "local", force_open, &names, "", &mut leaf);
            }
            let mut names_of_remotes: Vec<&str> = Vec::new();
            for remote in remotes {
                if !names_of_remotes.contains(&remote.remote.as_str()) {
                    names_of_remotes.push(&remote.remote);
                }
            }
            for remote_name in names_of_remotes {
                let names: Vec<&str> = remotes
                    .iter()
                    .filter(|r| r.remote == remote_name && matches(&needle, &r.name))
                    .map(|r| r.name.as_str())
                    .collect();
                if names.is_empty() {
                    continue;
                }
                shown += names.len();
                section_head(ui, icons::GITHUB, remote_name, names.len());
                let mut leaf = |ui: &mut Ui, full: &str, label: &str, indent: f32| {
                    let Some(branch) = remotes.iter().find(|r| r.remote == remote_name && r.name == full) else {
                        return;
                    };
                    if remote_row(ui, branch, label, indent).clicked() {
                        actions.push(Action::Git(match &branch.local {
                            Some(local) => GitOp::Switch(local.clone()),
                            None => GitOp::Track { remote: remote_name.to_string(), name: full.to_string() },
                        }));
                        ui.close();
                    }
                };
                tree(ui, shut, remote_name, force_open, &names, "", &mut leaf);
            }
            if shown == 0 {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("No branch matches.").size(11.5).color(pal().text_faint));
            }
        });
    }
}
