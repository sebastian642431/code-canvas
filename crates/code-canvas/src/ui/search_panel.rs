use std::collections::HashSet;
use std::time::{Duration, Instant};

use canvas_core::grep::{FileHits, Hit, Matcher, Query};
use canvas_render::theme;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Align2, CornerRadius, FontId, Frame, Key, Layout, Margin, Modifiers, Pos2, Rect};
use egui::{RichText, Sense, Stroke, Vec2};

use super::git_panel::row_button;
use super::{c, file_glyph, pal, panel_title, small_icon, text_fit, Action, FileIcons, UiState, ROW_H};
use crate::icons;
use crate::page::Page;
use crate::search::{Grep, Request, MAX_HITS};

/// How long the query waits after a key before it is searched.
const SETTLE: Duration = Duration::from_millis(300);
const KEPT: usize = 30;

#[derive(Clone, Debug, PartialEq)]
pub enum SearchOp {
    Clear,
    Show {
        file: usize,
        hit: usize,
    },
    /// F4 / Shift+F4: the next or the previous result.
    Step(bool),
    Dismiss {
        file: usize,
        hit: Option<usize>,
    },
    /// Every result (asked first), a file's, or one.
    Replace {
        file: Option<usize>,
        hit: Option<usize>,
    },
}

#[derive(Default)]
pub struct SearchView {
    pub replace: String,
    pub include: String,
    pub exclude: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    /// The field to replace with is shown.
    pub replacing: bool,
    /// The files to include and to exclude are shown.
    pub details: bool,
    /// The results as one list instead of a tree of files.
    pub list: bool,
    /// The query field takes the keyboard, with what is in it selected.
    pub focus: bool,
    refocus: bool,
    shut: HashSet<String>,
    history: Vec<String>,
    back: Option<usize>,
    typed: Option<Instant>,
    follow: Option<Option<(usize, usize)>>,
}

impl SearchView {
    pub fn request(&self, text: &str) -> Request {
        let query = Query { text: text.to_string(), case: self.case, word: self.word, regex: self.regex };
        Request { query, include: self.include.clone(), exclude: self.exclude.clone() }
    }

    /// Something was typed a moment ago and is to be searched now.
    pub fn due(&mut self) -> bool {
        self.typed.take_if(|at| at.elapsed() >= SETTLE).is_some()
    }

    pub fn waiting(&self) -> bool {
        self.typed.is_some()
    }

    /// A search ran: what was typed is not waiting any more.
    pub fn ran(&mut self) {
        self.typed = None;
    }

    pub fn changed(&mut self) {
        self.typed = Some(Instant::now());
    }

    fn remember(&mut self, text: &str) {
        self.back = None;
        if text.trim().is_empty() {
            return;
        }
        self.history.retain(|old| old != text);
        self.history.push(text.to_string());
        if self.history.len() > KEPT {
            self.history.remove(0);
        }
    }

    fn recall(&mut self, query: &mut String, older: bool) {
        let Some(last) = self.history.len().checked_sub(1) else { return };
        let at = match (self.back, older) {
            (None, true) if self.history[last] == *query => last.saturating_sub(1),
            (None, true) => last,
            (Some(at), true) => at.saturating_sub(1),
            (Some(at), false) if at < last => at + 1,
            _ => {
                self.back = None;
                return;
            }
        };
        self.back = Some(at);
        *query = self.history[at].clone();
        self.changed();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    File(usize),
    Hit(usize, usize),
}

fn glyph(name: &str, or: &'static str) -> &'static str {
    crate::codicons::glyph(name).unwrap_or(or)
}

fn plural(n: usize, word: &str) -> String {
    match n {
        1 => format!("1 {word}"),
        n => format!("{n} {word}s"),
    }
}

fn field_frame(error: bool) -> Frame {
    let border = if error { c(theme::get().problem_error) } else { pal().border };
    Frame::NONE
        .fill(pal().well)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(6, 3))
}

fn toggle(ui: &mut egui::Ui, glyph: &str, on: bool, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
    if on {
        ui.painter().rect(rect, 4.0, pal().raised, Stroke::new(1.0, pal().accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, pal().hover);
    }
    let color = if on || resp.hovered() { pal().text } else { pal().text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), color);
    resp.on_hover_text(tip).clicked()
}

fn query_row(ui: &mut egui::Ui, view: &mut SearchView, query: &mut String, error: bool, actions: &mut Vec<Action>) {
    let id = ui.make_persistent_id("search in files query");
    if ui.memory(|m| m.has_focus(id)) {
        let flipped = ui.input_mut(|i| {
            let mut flipped = None;
            for (key, which) in [(Key::C, 0), (Key::W, 1), (Key::R, 2)] {
                if i.consume_key(Modifiers::ALT, key) {
                    flipped = Some(which);
                }
            }
            if flipped.is_some() {
                i.events.retain(|event| !matches!(event, egui::Event::Text(_)));
            }
            flipped
        });
        match flipped {
            Some(0) => view.case = !view.case,
            Some(1) => view.word = !view.word,
            Some(2) => view.regex = !view.regex,
            _ => {}
        }
        if flipped.is_some() {
            view.changed();
        }
        for (key, older) in [(Key::ArrowUp, true), (Key::ArrowDown, false)] {
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                view.recall(query, older);
            }
        }
    }
    let mut flipped = false;
    let shown = field_frame(error).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = (ui.available_width() - 3.0 * 22.0).max(40.0);
            let edit = egui::TextEdit::singleline(query).id(id).hint_text("Search").frame(Frame::NONE);
            let shown = edit.font(FontId::proportional(13.0)).desired_width(width).show(ui);
            let options = [
                (&mut view.case, "case-sensitive", "Match Case (Alt+C)"),
                (&mut view.word, "whole-word", "Match Whole Word (Alt+W)"),
                (&mut view.regex, "regex", "Use Regular Expression (Alt+R)"),
            ];
            for (on, name, tip) in options {
                if toggle(ui, glyph(name, icons::CODE), *on, tip) {
                    *on = !*on;
                    flipped = true;
                }
            }
            shown
        })
        .inner
    });
    let mut shown = shown.inner;
    if flipped {
        view.changed();
    }
    if std::mem::take(&mut view.focus) {
        shown.response.request_focus();
        let all =
            egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(query.chars().count()));
        shown.state.cursor.set_char_range(Some(all));
        shown.state.store(ui.ctx(), shown.response.id);
    } else if std::mem::take(&mut view.refocus) {
        shown.response.request_focus();
    }
    if shown.response.changed() {
        view.back = None;
        view.changed();
    }
    if shown.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        view.remember(query);
        view.refocus = true;
        actions.push(Action::Search(query.clone()));
    }
}

fn replace_row(ui: &mut egui::Ui, view: &mut SearchView, can: bool, actions: &mut Vec<Action>) {
    field_frame(false).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = (ui.available_width() - 22.0).max(40.0);
            let edit = egui::TextEdit::singleline(&mut view.replace).hint_text("Replace").frame(Frame::NONE);
            let resp = ui.add(edit.font(FontId::proportional(13.0)).desired_width(width));
            let all = resp.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::ALT, Key::Enter));
            let (rect, button) =
                ui.allocate_exact_size(Vec2::splat(20.0), if can { Sense::click() } else { Sense::hover() });
            if can && button.hovered() {
                ui.painter().rect_filled(rect, 4.0, pal().hover);
            }
            let color = if can { pal().text } else { pal().text_faint };
            let icon = glyph("replace-all", icons::COPY);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(14.0), color);
            if (button.on_hover_text("Replace All (Ctrl+Alt+Enter)").clicked() || all) && can {
                actions.push(Action::SearchPanel(SearchOp::Replace { file: None, hit: None }));
            }
        });
    });
}

fn path_field(ui: &mut egui::Ui, view: &mut SearchView, label: &str, include: bool) {
    ui.label(RichText::new(label).size(11.5).color(pal().text_dim));
    let (text, hint) = match include {
        true => (&mut view.include, "e.g. src/**/*.ts, *.rs"),
        false => (&mut view.exclude, "e.g. *.md, tests"),
    };
    let resp = field_frame(false)
        .show(ui, |ui| {
            let edit = egui::TextEdit::singleline(text).hint_text(hint).frame(Frame::NONE);
            ui.add(edit.font(FontId::proportional(12.5)).desired_width(f32::INFINITY))
        })
        .inner;
    if resp.changed() {
        view.changed();
    }
}

fn summary(ui: &mut egui::Ui, grep: Option<&Grep>, view: &SearchView) {
    let Some(grep) = grep else { return };
    let dim = pal().text_dim;
    let (text, color) = match (&grep.error, &grep.results) {
        (Some(why), _) if view.regex => (format!("Invalid regular expression: {why}"), c(theme::get().problem_error)),
        (Some(why), _) => (why.clone(), c(theme::get().problem_error)),
        _ if grep.replacing() => ("Replacing …".to_string(), dim),
        (None, None) if grep.searching() => ("Searching …".to_string(), dim),
        (None, None) => return,
        (None, Some(results)) => {
            let count = results.count();
            let filtered = !results.asked.include.trim().is_empty() || !results.asked.exclude.trim().is_empty();
            let mut text = match (count, filtered) {
                (0, true) => "No results found. Review the files to include and exclude.".to_string(),
                (0, false) => "No results found.".to_string(),
                (n, _) => format!("{} in {}", plural(n, "result"), plural(results.files.len(), "file")),
            };
            if results.truncated {
                text.push_str(&format!(" · only the first {MAX_HITS} are listed"));
            }
            if grep.searching() {
                text.push_str(" · searching …");
            }
            (text, dim)
        }
    };
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(color)).wrap());
}

fn preview(hit: &Hit, matcher: &Matcher, with: Option<&str>, width: f32) -> LayoutJob {
    let plain = TextFormat::simple(FontId::monospace(12.0), pal().text);
    let tidy = |text: &str| text.replace('\t', "    ").replace('\n', "\\n");
    let mut job = LayoutJob::default();
    job.append(&tidy(&hit.before), 0.0, plain.clone());
    match with {
        None => {
            let found = c(theme::get().find_match).gamma_multiply(0.55);
            job.append(&tidy(&hit.found), 0.0, TextFormat { background: found, ..plain.clone() });
        }
        Some(with) => {
            let (gone, new) = (c(theme::get().git_removed), c(theme::get().git_added));
            let struck = TextFormat {
                background: gone.gamma_multiply(0.3),
                strikethrough: Stroke::new(1.0, gone),
                ..plain.clone()
            };
            job.append(&tidy(&hit.found), 0.0, struck);
            let added = TextFormat { background: new.gamma_multiply(0.3), ..plain.clone() };
            job.append(&tidy(&matcher.replaced(&hit.found, with)), 0.0, added);
        }
    }
    job.append(&tidy(&hit.after), 0.0, plain);
    job.wrap =
        TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    job
}

fn file_row(
    ui: &mut egui::Ui,
    file_icons: &mut FileIcons,
    view: &mut SearchView,
    (fi, file): (usize, &FileHits),
    can_replace: bool,
    actions: &mut Vec<Action>,
) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
    }
    let mid = rect.center().y;
    let shut = view.shut.contains(&file.rel);
    let chevron = if shut { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN };
    let at = Pos2::new(rect.left() + 16.0, mid);
    ui.painter().text(at, Align2::CENTER_CENTER, chevron, FontId::proportional(12.0), pal().text_dim);
    let count = ui.painter().layout_no_wrap(file.hits.len().to_string(), FontId::proportional(11.0), pal().text_dim);
    let size = Vec2::new(count.size().x.max(8.0) + 10.0, 16.0);
    let badge = Rect::from_center_size(Pos2::new(rect.right() - 12.0 - size.x * 0.5, mid), size);
    ui.painter().rect_filled(badge, 8.0, pal().raised);
    ui.painter().galley(badge.center() - count.size() * 0.5, count, pal().text_dim);
    let mut right = badge.left() - 4.0;
    let mut action = None;
    if hovered {
        if row_button(ui, &mut right, mid, ("search dismiss file", fi), glyph("close", icons::X), "Dismiss") {
            action = Some(SearchOp::Dismiss { file: fi, hit: None });
        }
        let tip = "Replace All in This File";
        if can_replace
            && row_button(ui, &mut right, mid, ("search replace file", fi), glyph("replace-all", icons::COPY), tip)
        {
            action = Some(SearchOp::Replace { file: Some(fi), hit: None });
        }
    }
    let (dir, name) = file.rel.rsplit_once('/').unwrap_or(("", &file.rel));
    let x = rect.left() + 28.0;
    let icon_at = Rect::from_center_size(Pos2::new(x + 8.0, mid), Vec2::splat(16.0));
    if !file_icons.paint(ui, name, false, false, icon_at, false) {
        let font = FontId::proportional(13.0);
        ui.painter().text(icon_at.center(), Align2::CENTER_CENTER, file_glyph(name), font, pal().text_dim);
    }
    let x = x + 22.0;
    let painter = ui.painter();
    let name_w = text_fit(painter, Pos2::new(x, mid), name, FontId::proportional(12.5), pal().text, right - x);
    let dir_x = x + name_w + 6.0;
    text_fit(painter, Pos2::new(dir_x, mid), dir, FontId::proportional(11.5), pal().text_faint, right - dir_x);
    if action.is_none() && resp.clicked() {
        if shut {
            view.shut.remove(&file.rel);
        } else {
            view.shut.insert(file.rel.clone());
        }
    }
    resp.on_hover_text(&file.rel);
    if let Some(op) = action {
        actions.push(Action::SearchPanel(op));
    }
}

struct HitLook<'a> {
    matcher: &'a Matcher,
    with: Option<&'a str>,
    list: bool,
    can_replace: bool,
    current: bool,
}

fn hit_row(ui: &mut egui::Ui, (fi, file): (usize, &FileHits), hi: usize, look: &HitLook, actions: &mut Vec<Action>) {
    let hit = &file.hits[hi];
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let hovered = ui.rect_contains_pointer(rect);
    if look.current || hovered {
        let fill = if look.current { pal().active } else { pal().row };
        ui.painter().rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, fill);
    }
    let mid = rect.center().y;
    let mut right = rect.right() - 8.0;
    let mut action = None;
    if hovered {
        if row_button(ui, &mut right, mid, ("search dismiss", fi, hi), glyph("close", icons::X), "Dismiss") {
            action = Some(SearchOp::Dismiss { file: fi, hit: Some(hi) });
        }
        let replace = glyph("replace", icons::CORNER_DOWN_LEFT);
        if look.can_replace && row_button(ui, &mut right, mid, ("search replace", fi, hi), replace, "Replace") {
            action = Some(SearchOp::Replace { file: Some(fi), hit: Some(hi) });
        }
    }
    let mut x = rect.left() + if look.list { 14.0 } else { 44.0 };
    if look.list {
        let name = file.rel.rsplit('/').next().unwrap_or(&file.rel);
        let place = format!("{name}:{}", hit.line + 1);
        let font = FontId::proportional(12.0);
        x += text_fit(ui.painter(), Pos2::new(x, mid), &place, font, pal().text_dim, (right - x) * 0.45) + 8.0;
    }
    let galley = ui.painter().layout_job(preview(hit, look.matcher, look.with, right - x - 4.0));
    ui.painter().galley(Pos2::new(x, mid - galley.size().y * 0.5), galley, pal().text);
    if action.is_none() && resp.clicked() {
        action = Some(SearchOp::Show { file: fi, hit: hi });
    }
    resp.on_hover_text(format!("{}:{}", file.rel, hit.line + 1));
    if let Some(op) = action {
        actions.push(Action::SearchPanel(op));
    }
}

pub(super) fn search_panel(ui: &mut egui::Ui, st: &mut UiState, page: Option<&Page>, actions: &mut Vec<Action>) {
    let UiState { search: view, search_query: query, file_icons, .. } = st;
    let grep = page.map(|page| &page.grep);
    let results = grep.and_then(|grep| grep.results.as_ref());
    let files = results.map_or(&[][..], |results| results.files.as_slice());
    let changeable = page.is_some_and(|page| !page.read_only && page.showing.is_none());
    let can_replace = view.replacing && changeable && !grep.is_some_and(Grep::replacing);
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Search", |ui| {
            let (tip, look) = match view.list {
                true => ("View as Tree", glyph("list-tree", icons::LIST_TREE)),
                false => ("View as List", glyph("list-flat", icons::LIST_TREE)),
            };
            if small_icon(ui, look, tip) {
                view.list = !view.list;
            }
            if !view.list && !files.is_empty() {
                let open = files.iter().any(|file| !view.shut.contains(&file.rel));
                let (look, tip) = match open {
                    true => (glyph("collapse-all", icons::FOLD_VERTICAL), "Collapse All"),
                    false => (glyph("expand-all", icons::UNFOLD_VERTICAL), "Expand All"),
                };
                if small_icon(ui, look, tip) {
                    match open {
                        true => view.shut = files.iter().map(|file| file.rel.clone()).collect(),
                        false => view.shut.clear(),
                    }
                }
            }
            if small_icon(ui, glyph("clear-all", icons::X), "Clear Search Results") {
                query.clear();
                view.ran();
                actions.push(Action::SearchPanel(SearchOp::Clear));
            }
            if small_icon(ui, glyph("refresh", icons::REFRESH_CW), "Refresh") {
                actions.push(Action::Search(query.clone()));
            }
        });
        ui.add_space(6.0);
        let error = grep.is_some_and(|grep| grep.error.is_some());
        let inputs = ui
            .horizontal(|ui| {
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    query_row(ui, view, query, error, actions);
                    if view.replacing {
                        replace_row(ui, view, can_replace && !files.is_empty(), actions);
                    }
                });
            })
            .response
            .rect;
        let chevron = Rect::from_min_max(inputs.left_top(), Pos2::new(inputs.left() + 16.0, inputs.bottom()));
        let resp = ui.interact(chevron, ui.scope_id().with("search replace toggle"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(chevron, 4.0, pal().hover);
        }
        let look = if view.replacing { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
        ui.painter().text(chevron.center(), Align2::CENTER_CENTER, look, FontId::proportional(13.0), pal().text_dim);
        if resp.on_hover_text("Toggle Replace").clicked() {
            view.replacing = !view.replacing;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if small_icon(ui, glyph("ellipsis", icons::ELLIPSIS_VERTICAL), "Toggle Search Details") {
                view.details = !view.details;
            }
        });
        if view.details {
            path_field(ui, view, "files to include", true);
            path_field(ui, view, "files to exclude", false);
            ui.add_space(4.0);
        }
        summary(ui, grep, view);
    });
    if results.is_some_and(|results| results.count() > 0) {
        let (back, next) =
            ui.input_mut(|i| (i.consume_key(Modifiers::SHIFT, Key::F4), i.consume_key(Modifiers::NONE, Key::F4)));
        if back || next {
            view.follow = Some(grep.and_then(|grep| grep.at));
            actions.push(Action::SearchPanel(SearchOp::Step(next)));
        }
    }
    let (Some(grep), Some(results)) = (grep, results) else { return };
    let at = grep.at;
    if let Some((file, _)) = at.filter(|_| view.follow.is_some()) {
        if let Some(found) = files.get(file) {
            view.shut.remove(&found.rel);
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    for (fi, file) in files.iter().enumerate() {
        if !view.list {
            rows.push(Row::File(fi));
            if view.shut.contains(&file.rel) {
                continue;
            }
        }
        rows.extend((0..file.hits.len()).map(|hi| Row::Hit(fi, hi)));
    }
    let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if view.follow.is_some_and(|before| before != at) {
        view.follow = None;
        if let Some(row) = at.and_then(|(file, hit)| rows.iter().position(|row| *row == Row::Hit(file, hit))) {
            scroll = scroll.vertical_scroll_offset(((row as f32 - 4.0) * ROW_H).max(0.0));
        }
    }
    let with: Option<String> = view.replacing.then(|| view.replace.clone());
    let list = view.list;
    let look = |current: bool| HitLook { matcher: &results.matcher, with: with.as_deref(), list, can_replace, current };
    ui.spacing_mut().item_spacing.y = 0.0;
    scroll.show_rows(ui, ROW_H, rows.len(), |ui, range| {
        for row in &rows[range] {
            match *row {
                Row::File(fi) => file_row(ui, file_icons, view, (fi, &files[fi]), can_replace, actions),
                Row::Hit(fi, hi) => hit_row(ui, (fi, &files[fi]), hi, &look(at == Some((fi, hi))), actions),
            }
        }
    });
}
