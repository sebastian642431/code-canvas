//! A file open in a tab, shown as an editor shows it: line numbers, the colours of the code, the caret and selection
//! of the file's editor, what git sees changed and what extensions report wrong. Only the rows in sight are read and
//! coloured, so a long file costs what a short one does.

use std::path::Path;

use canvas_core::CHUNK_LINES;
use canvas_render::theme;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::file_tabs::{codicon, locate};
use super::{c, pal, text_fit, Action, UiState, ROUND};
use crate::ext::{Extensions, Severity};
use crate::file_tabs::{Cell, Reveal, TabOp};
use crate::git::{HunkKind, RowKind};
use crate::icons;
use crate::page::{Page, Ready};

const HEAD_H: f32 = 32.0;
const PAD: f32 = 10.0;
const MOST_MARKS: usize = 500;

struct Line {
    row: usize,
    text: Vec<char>,
    kinds: Vec<u8>,
    len: usize,
    number: Option<u32>,
    tint: Option<Color32>,
}

struct Span {
    row: usize,
    from: usize,
    to: usize,
    fill: Color32,
    under: Option<Color32>,
}

struct Problem {
    row: usize,
    from: usize,
    to: usize,
    message: String,
}

fn severity_color(severity: Severity) -> u32 {
    let t = theme::get();
    match severity {
        Severity::Error => t.problem_error,
        Severity::Warning => t.problem_warning,
        Severity::Info | Severity::Hint => t.problem_info,
    }
}

fn faint(color: u32, alpha: f32) -> Color32 {
    c((color & 0xffff_ff00) | (alpha.clamp(0.0, 1.0) * 255.0) as u32)
}

fn ordered(a: Cell, b: Cell) -> (Cell, Cell) {
    if (a.0, a.1) <= (b.0, b.1) {
        (a, b)
    } else {
        (b, a)
    }
}

pub(super) fn view(
    ui: &mut egui::Ui,
    st: &mut UiState,
    pages: &mut [Page],
    ext: &Extensions,
    canvas: Rect,
    area: Rect,
    actions: &mut Vec<Action>,
) {
    let Some(path) = st.tabs.front_path().map(Path::to_path_buf) else { return };
    let t = theme::get();
    let back = c(theme::mix(t.background, t.card_fill | 0xff, (t.card_fill & 0xff) as f32 / 255.0));
    ui.painter().rect_filled(canvas.shrink(1.0), ROUND - 1.0, back);
    let inner = area.shrink(1.0);
    let head = Rect::from_min_max(inner.min, Pos2::new(inner.right(), inner.top() + HEAD_H));
    let body = Rect::from_min_max(Pos2::new(inner.left(), head.bottom() + 1.0), inner.max);
    ui.painter().hline(head.x_range(), head.bottom(), Stroke::new(1.0, pal().border));
    let found = locate(pages, &path);
    header(ui, st, head, &path, found.as_ref().map(|(i, rel)| (&pages[*i], rel.as_str())), actions);
    let Some((i, rel)) = found else {
        let close = st.tabs.front().map(|front| ("Close the tab", Action::Tabs(TabOp::Close(front))));
        return say(ui, body, "This file is not in an open folder any more.", close, actions);
    };
    let Some(r) = pages[i].ready() else {
        return say(ui, body, "Reading the folder…", None, actions);
    };
    let Some(fi) = r.repo.file_index(&rel) else {
        let text = "This file is not read: git ignores it, or the settings leave it out.";
        return say(ui, body, text, Some(("Open it with its app", Action::OpenExternally(path.clone()))), actions);
    };
    if let Some(note) = r.repo.files[fi].note.as_deref().filter(|note| !note.starts_with("truncated")) {
        let text = format!("This file is not shown as text ({note}).");
        return say(ui, body, &text, Some(("Show its card", Action::Tabs(TabOp::Reveal))), actions);
    }
    code(ui, st, pages, i, &rel, fi, ext, &path, body, back, actions);
}

#[allow(clippy::too_many_arguments)]
fn code(
    ui: &mut egui::Ui,
    st: &mut UiState,
    pages: &mut [Page],
    i: usize,
    rel: &str,
    fi: usize,
    ext: &Extensions,
    path: &Path,
    body: Rect,
    back: Color32,
    actions: &mut Vec<Action>,
) {
    let t = theme::get();
    let font = FontId::monospace(13.0);
    let (char_w, glyph_h) = ui.fonts_mut(|fonts| (fonts.glyph_width(&font, '0'), fonts.row_height(&font)));
    let (char_w, row_h) = (char_w.max(1.0), (glyph_h * 1.45).round().max(16.0));
    let page = &pages[i];
    let Some(r) = page.ready() else { return };
    let f = &r.repo.files[fi];
    let rows = f.line_count().max(1);
    let gutter = rows.to_string().len().max(3) as f32 * char_w + 30.0;
    let code_left = body.left() + gutter + PAD;
    let code_clip = Rect::from_min_max(Pos2::new(body.left() + gutter, body.top()), body.max);
    let view_w = (code_clip.width() - PAD).max(char_w);
    let full_w = f.max_cols as f32 * char_w + PAD + 6.0 * char_w;
    let full_h = rows as f32 * row_h + body.height() * 0.5;
    st.tabs.page_rows = (body.height() / row_h).floor().max(1.0);

    let editing = page.editing.as_deref() == Some(rel);
    let editor = page.editors.get(rel).filter(|_| editing);
    let cell = |line: usize, col: usize| (page.row_of(rel, line as u32) as usize, col);
    let carets: Vec<(Cell, Option<(Cell, Cell)>)> = editor
        .map(|e| {
            let at = |p: crate::editor::Pos| cell(p.line, e.display_col(p));
            e.carets().iter().map(|caret| (at(caret.at), caret.selection().map(|(a, b)| (at(a), at(b))))).collect()
        })
        .unwrap_or_default();

    let resp = ui.interact(body, egui::Id::unique(("file view", path)), Sense::click_and_drag());
    let mut scroll = st.tabs.front_tab_mut().map_or(Vec2::ZERO, |tab| Vec2::from(tab.scroll));
    if resp.hovered() {
        scroll -= ui.input(|input| input.smooth_scroll_delta);
    }
    scroll.y += std::mem::take(&mut st.tabs.nudge) * row_h;
    let ensure = |scroll: &mut Vec2, (row, col): Cell, margin: f32| {
        let (y, x) = (row as f32 * row_h, col as f32 * char_w);
        if y < scroll.y + margin {
            scroll.y = y - margin;
        } else if y + row_h > scroll.y + body.height() - margin {
            scroll.y = y + row_h - body.height() + margin;
        }
        if x < scroll.x + 4.0 * char_w {
            scroll.x = x - 4.0 * char_w;
        } else if x > scroll.x + view_w - 4.0 * char_w {
            scroll.x = x - view_w + 4.0 * char_w;
        }
    };
    match st.tabs.reveal.take() {
        Some(Reveal::Row(row, col)) => {
            let y = row as f32 * row_h;
            if y < scroll.y || y + row_h > scroll.y + body.height() {
                scroll.y = y - body.height() * 0.35;
            }
            ensure(&mut scroll, (row, col), 0.0);
        }
        Some(Reveal::Caret) => {
            if let Some(&(at, _)) = carets.first() {
                ensure(&mut scroll, at, 2.0 * row_h);
            }
        }
        None => {}
    }
    let dragged = resp.dragged_by(egui::PointerButton::Primary);
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    if let (true, Some(at)) = (dragged, pointer) {
        let push = |out: f32| (out / 3.0).clamp(-row_h * 2.0, row_h * 2.0);
        if at.y > body.bottom() {
            scroll.y += push(at.y - body.bottom());
        } else if at.y < body.top() {
            scroll.y += push(at.y - body.top());
        }
        if at.y > body.bottom() || at.y < body.top() {
            ui.ctx().request_repaint();
        }
    }
    scroll.x = scroll.x.clamp(0.0, (full_w - view_w).max(0.0));
    scroll.y = scroll.y.clamp(0.0, (full_h - body.height()).max(0.0));
    if let Some(tab) = st.tabs.front_tab_mut() {
        tab.scroll = [scroll.x, scroll.y];
    }

    let cell_at = |p: Pos2| -> Cell {
        let row = ((p.y - body.top() + scroll.y) / row_h).floor().max(0.0) as usize;
        let col = ((p.x - code_left + scroll.x) / char_w).round().max(0.0) as usize;
        (row.min(rows - 1), col)
    };
    if resp.hovered() && ui.input(|input| input.pointer.primary_pressed()) {
        if let Some((row, col)) = pointer.map(cell_at) {
            let mods = ui.input(|input| input.modifiers);
            actions.push(Action::Tabs(TabOp::Press { row, col, shift: mods.shift, alt: mods.alt, double: false }));
        }
    } else if resp.double_clicked() {
        if let Some((row, col)) = pointer.map(cell_at) {
            actions.push(Action::Tabs(TabOp::Press { row, col, shift: false, alt: false, double: true }));
        }
    } else if dragged {
        if let Some((row, col)) = pointer.map(cell_at) {
            actions.push(Action::Tabs(TabOp::DragTo { row, col }));
        }
    }

    let first = (scroll.y / row_h).floor() as usize;
    let last = (((scroll.y + body.height()) / row_h).ceil() as usize + 1).min(rows);
    let c0 = (scroll.x / char_w).floor() as usize;
    let c1 = c0 + (view_w / char_w).ceil() as usize + 2;
    let diff_rows = page.rows(rel);
    let tint = |kind: RowKind| match kind {
        RowKind::Added => Some(faint(t.git_added, 0.16)),
        RowKind::Removed => Some(faint(t.git_removed, 0.19)),
        RowKind::Gap => Some(faint(t.ui_text_dim, 0.08)),
        RowKind::Blank => Some(c(if t.is_light() { 0x0000000f } else { 0x00000029 })),
        RowKind::Same | RowKind::Before => None,
    };
    let mut lines: Vec<Line> = (first..last.max(first))
        .map(|row| {
            let all = f.line(row);
            let text: Vec<char> = all.chars().skip(c0).take(c1 - c0).collect();
            let number = match &f.numbers {
                Some(numbers) => numbers.get(row).copied().filter(|&n| n > 0),
                None => Some(row as u32 + 1),
            };
            let tint = diff_rows.and_then(|shown| shown.get(row)).and_then(|shown| tint(shown.kind));
            Line {
                row,
                kinds: vec![theme::kind_index(canvas_core::TokenKind::Ident) as u8; text.len()],
                text,
                len: all.chars().count(),
                number,
                tint,
            }
        })
        .collect();
    let seen = |row: usize| (first..last).contains(&row);
    let len_of = |row: usize| lines.get(row.wrapping_sub(first)).map_or(0, |line| line.len);

    let mut spans: Vec<Span> = Vec::new();
    if let Some(found) = &page.search {
        let hits = found.hits.iter().enumerate().filter(|(_, hit)| hit.file as usize == fi && seen(hit.line as usize));
        for (k, hit) in hits.take(MOST_MARKS) {
            let alpha = if found.current == Some(k) { 0.5 } else { 0.19 };
            let (from, to) = (hit.col as usize, hit.col as usize + hit.len.max(1) as usize);
            spans.push(Span { row: hit.line as usize, from, to, fill: faint(t.find_match, alpha), under: None });
        }
    }
    if let Some((row, col, len)) = st.tabs.mark.filter(|(row, ..)| seen(*row)) {
        spans.push(Span { row, from: col, to: col + len.max(1), fill: faint(t.find_match, 0.5), under: None });
    }
    let picks: Vec<(Cell, Cell)> = match editor {
        Some(_) => carets.iter().filter_map(|(_, picked)| *picked).collect(),
        None => st.tabs.pick.map(|(a, b)| ordered(a, b)).into_iter().collect(),
    };
    for (a, b) in picks {
        for row in a.0.max(first)..=b.0.min(last.saturating_sub(1)) {
            let from = if row == a.0 { a.1 } else { 0 };
            let to = if row == b.0 { b.1 } else { len_of(row) + 1 };
            if to > from {
                spans.push(Span { row, from, to, fill: c(t.code_selection), under: None });
            }
        }
    }
    let mut problems: Vec<Problem> = Vec::new();
    let reported = ext.diagnostics.get(&crate::ext::path_key(&path.to_string_lossy()));
    for d in reported.map(|found| found.items.as_slice()).unwrap_or_default().iter().take(MOST_MARKS) {
        let row = page.row_of(rel, d.line) as usize;
        let folded = page.diffs.get(rel).is_some_and(|shown| !shown.shows(d.line));
        if !seen(row) || folded {
            continue;
        }
        let from = d.col as usize;
        let to = match d.end_line > d.line {
            true => len_of(row).max(from + 1),
            false => (d.end_col as usize).max(from + 1),
        };
        let color = severity_color(d.severity);
        spans.push(Span { row, from, to, fill: faint(color, 0.14), under: Some(c(color | 0xff)) });
        problems.push(Problem { row, from, to, message: d.message.clone() });
    }
    let mut bars: Vec<(usize, usize, Color32, bool)> = Vec::new();
    match diff_rows {
        Some(shown) => {
            for row in first..last {
                match shown.get(row).map(|shown| shown.kind) {
                    Some(RowKind::Added) => bars.push((row, row + 1, c(t.git_added), false)),
                    Some(RowKind::Removed) => bars.push((row, row + 1, c(t.git_removed), false)),
                    _ => {}
                }
            }
        }
        None if page.commit_shown().is_none() => {
            let change = page.git.as_ref().and_then(|git| git.change(rel));
            for h in &change.map(crate::git::FileChange::every_hunk).unwrap_or_default() {
                let (from, to) = (h.line as usize, (h.line + h.count) as usize);
                let color = match h.kind {
                    HunkKind::Added => t.git_added,
                    HunkKind::Modified => t.git_modified,
                    HunkKind::Removed => t.git_removed,
                };
                let removed = h.kind == HunkKind::Removed;
                if (removed && seen(from)) || (!removed && from < last && to > first) {
                    bars.push((from, to, c(color), removed));
                }
            }
        }
        None => {}
    }
    let lit_rows: Vec<usize> = match editor {
        Some(_) => carets.iter().map(|((row, _), _)| *row).collect(),
        None => st.tabs.pick.map(|(_, (row, _))| row).into_iter().collect(),
    };
    let caret_cells: Vec<Cell> = carets.iter().map(|(at, _)| *at).collect();

    if let Some(r) = pages[i].ready_mut() {
        let Ready { repo, tokens, .. } = r;
        let file = &repo.files[fi];
        for line in &mut lines {
            let chunk = line.row / CHUNK_LINES;
            let got = tokens.chunk(fi, file, chunk);
            let within = line.row - chunk * CHUNK_LINES;
            if within >= got.lines() {
                continue;
            }
            let end = c0 + line.text.len();
            for token in got.line(within) {
                let (a, b) = ((token.col as usize).max(c0), (token.col as usize + token.cols as usize).min(end));
                let kind = theme::kind_index(token.kind) as u8;
                for k in a..b.max(a) {
                    line.kinds[k - c0] = kind;
                }
            }
        }
    }

    let top_of = |row: usize| body.top() + row as f32 * row_h - scroll.y;
    let x_of = |col: usize| code_left + col as f32 * char_w - scroll.x;
    let painter = ui.painter().with_clip_rect(body.intersect(ui.clip_rect()));
    let code_painter = painter.with_clip_rect(code_clip.intersect(painter.clip_rect()));
    for &row in lit_rows.iter().filter(|row| seen(**row)) {
        let band =
            Rect::from_min_max(Pos2::new(code_clip.left(), top_of(row)), Pos2::new(body.right(), top_of(row) + row_h));
        painter.rect_filled(band, 0.0, c(t.current_line));
    }
    for line in lines.iter().filter(|line| line.tint.is_some()) {
        let band = Rect::from_min_max(
            Pos2::new(code_clip.left(), top_of(line.row)),
            Pos2::new(body.right(), top_of(line.row) + row_h),
        );
        painter.rect_filled(band, 0.0, line.tint.unwrap_or_default());
    }
    for span in &spans {
        let rect = Rect::from_min_max(
            Pos2::new(x_of(span.from), top_of(span.row)),
            Pos2::new(x_of(span.to), top_of(span.row) + row_h),
        );
        code_painter.rect_filled(rect, 2.0, span.fill);
    }
    let colors = t.token_colors();
    for line in &lines {
        let mut job = egui::text::LayoutJob::default();
        let mut k = 0;
        while k < line.text.len() {
            let kind = line.kinds[k];
            let end = (k..line.text.len()).find(|&e| line.kinds[e] != kind).unwrap_or(line.text.len());
            let run: String = line.text[k..end].iter().collect();
            let format =
                egui::TextFormat { font_id: font.clone(), color: c(colors[kind as usize]), ..Default::default() };
            job.append(&run, 0.0, format);
            k = end;
        }
        if job.text.is_empty() {
            continue;
        }
        let galley = code_painter.layout_job(job);
        let at = Pos2::new(x_of(c0), top_of(line.row) + (row_h - glyph_h) * 0.5);
        code_painter.galley(at, galley, pal().text);
    }
    for span in spans.iter().filter(|span| span.under.is_some()) {
        let y = top_of(span.row) + row_h - 3.0;
        let line = Rect::from_min_max(Pos2::new(x_of(span.from), y), Pos2::new(x_of(span.to), y + 1.5));
        code_painter.rect_filled(line, 0.0, span.under.unwrap_or_default());
    }
    for &(row, col) in caret_cells.iter().filter(|(row, _)| seen(*row)) {
        let x = x_of(col).round();
        let caret =
            Rect::from_min_max(Pos2::new(x - 1.0, top_of(row) + 1.0), Pos2::new(x + 1.0, top_of(row) + row_h - 1.0));
        code_painter.rect_filled(caret, 0.0, c(t.caret));
    }
    if let (Some(offer), Some(&((row, col), _))) = (st.completion.as_mut(), carets.first()) {
        offer.at = Pos2::new(x_of(col), top_of(row) + row_h);
    }

    let gutter_rect = Rect::from_min_max(body.min, Pos2::new(body.left() + gutter, body.bottom()));
    painter.rect_filled(gutter_rect, 0.0, back);
    let numbers = FontId::monospace(12.0);
    for line in &lines {
        let Some(number) = line.number else { continue };
        let lit = lit_rows.contains(&line.row);
        let color = if lit { pal().text_dim } else { pal().text_faint };
        let at = Pos2::new(gutter_rect.right() - 12.0, top_of(line.row) + row_h * 0.5);
        painter.text(at, Align2::RIGHT_CENTER, number.to_string(), numbers.clone(), color);
    }
    for &(from, to, color, removed) in &bars {
        let x = gutter_rect.left() + 5.0;
        let rect = match removed {
            true => Rect::from_center_size(Pos2::new(x + 3.0, top_of(from)), Vec2::new(8.0, 3.0)),
            false => {
                Rect::from_min_max(Pos2::new(x, top_of(from.max(first))), Pos2::new(x + 3.0, top_of(to.min(last))))
            }
        };
        painter.rect_filled(rect, 1.0, color);
    }

    let thumb = |track: f32, seen: f32, full: f32, at: f32| {
        let size = (track * seen / full).clamp(24.0_f32.min(track), track);
        (size, (track - size) * at / (full - seen).max(1.0))
    };
    if full_h > body.height() {
        let track =
            Rect::from_min_max(Pos2::new(body.right() - 10.0, body.top()), Pos2::new(body.right(), body.bottom()));
        let (size, at) = thumb(track.height(), body.height(), full_h, scroll.y);
        let grip = ui.interact(track, egui::Id::unique(("file view rows", path)), Sense::drag());
        if grip.dragged() {
            let step = (full_h - body.height()) / (track.height() - size).max(1.0);
            if let Some(tab) = st.tabs.front_tab_mut() {
                tab.scroll[1] = (tab.scroll[1] + grip.drag_delta().y * step).clamp(0.0, full_h - body.height());
            }
        }
        let bar = Rect::from_min_size(Pos2::new(track.left() + 3.0, track.top() + at), Vec2::new(5.0, size));
        let lit = grip.hovered() || grip.dragged();
        painter.rect_filled(bar, 3.0, if lit { pal().border_strong } else { pal().border });
    }
    if full_w > view_w {
        let track = Rect::from_min_max(
            Pos2::new(code_clip.left(), body.bottom() - 10.0),
            Pos2::new(body.right() - 10.0, body.bottom()),
        );
        let (size, at) = thumb(track.width(), view_w, full_w, scroll.x);
        let grip = ui.interact(track, egui::Id::unique(("file view columns", path)), Sense::drag());
        if grip.dragged() {
            let step = (full_w - view_w) / (track.width() - size).max(1.0);
            if let Some(tab) = st.tabs.front_tab_mut() {
                tab.scroll[0] = (tab.scroll[0] + grip.drag_delta().x * step).clamp(0.0, full_w - view_w);
            }
        }
        let bar = Rect::from_min_size(Pos2::new(track.left() + at, track.top() + 3.0), Vec2::new(size, 5.0));
        let lit = grip.hovered() || grip.dragged();
        painter.rect_filled(bar, 3.0, if lit { pal().border_strong } else { pal().border });
    }

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        let at = resp.hover_pos().map(cell_at);
        let said = at.and_then(|(row, col)| problems.iter().find(|p| p.row == row && (p.from..p.to).contains(&col)));
        if let Some(problem) = said {
            resp.on_hover_text_at_pointer(problem.message.as_str());
        }
    }
}

fn header(
    ui: &mut egui::Ui,
    st: &mut UiState,
    head: Rect,
    path: &Path,
    page: Option<(&Page, &str)>,
    actions: &mut Vec<Action>,
) {
    let font = FontId::proportional(12.5);
    let mid = head.center().y;
    let label = "Reveal on the canvas";
    let label_w = ui.painter().layout_no_wrap(label.into(), font.clone(), pal().text).size().x;
    let reveal =
        Rect::from_min_size(Pos2::new(head.right() - label_w - 44.0, mid - 12.0), Vec2::new(label_w + 34.0, 24.0));
    let resp = ui.interact(reveal, egui::Id::unique("file view reveal"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(reveal, 6.0, pal().hover);
    }
    let color = if resp.hovered() { pal().text } else { pal().text_dim };
    let glyph_at = Pos2::new(reveal.left() + 14.0, mid);
    ui.painter().text(glyph_at, Align2::CENTER_CENTER, icons::LOCATE_FIXED, FontId::proportional(13.5), color);
    ui.painter().text(Pos2::new(reveal.left() + 26.0, mid), Align2::LEFT_CENTER, label, font.clone(), color);
    if resp.on_hover_text("Back to the canvas, at this file's card").clicked() {
        actions.push(Action::Tabs(TabOp::Reveal));
    }

    let mut right = reveal.left() - 10.0;
    if let Some((page, rel)) = page {
        let editor = page.editors.get(rel);
        let mut about = Vec::new();
        if editor.is_some_and(|e| e.dirty) {
            about.push("● Not saved".to_string());
        }
        match editor.filter(|_| page.editing.as_deref() == Some(rel)) {
            Some(e) => about.push(format!("Ln {}, Col {}", e.caret.line + 1, e.display_col(e.caret) + 1)),
            None if page.read_only || page.showing.is_some() => about.push("Read-only".to_string()),
            None => {}
        }
        if !about.is_empty() {
            let text = about.join("    ");
            let w = ui.painter().layout_no_wrap(text.clone(), font.clone(), pal().text).size().x;
            if right - w > head.left() + 160.0 {
                let at = Pos2::new(right, mid);
                ui.painter().text(at, Align2::RIGHT_CENTER, text, font.clone(), pal().text_faint);
                right -= w + 14.0;
            }
        }
    }

    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let mut x = head.left() + 12.0;
    let icon_at = Rect::from_center_size(Pos2::new(x + 8.0, mid), Vec2::splat(15.0));
    if !st.file_icons.paint(ui, &name, false, false, icon_at, false) {
        let glyph = codicon("file", icons::FILE);
        ui.painter().text(icon_at.center(), Align2::CENTER_CENTER, glyph, font.clone(), pal().text_dim);
    }
    x += 24.0;
    let Some((page, rel)) = page else {
        text_fit(ui.painter(), Pos2::new(x, mid), &path.to_string_lossy(), font, pal().text, right - x);
        return;
    };
    let parts: Vec<&str> = rel.split('/').collect();
    let mut crumbs: Vec<(String, Option<String>)> = vec![(page.title.clone(), Some(String::new()))];
    for (k, part) in parts.iter().enumerate() {
        let last = k + 1 == parts.len();
        crumbs.push((part.to_string(), (!last).then(|| parts[..=k].join("/"))));
    }
    let painter = ui.painter().with_clip_rect(Rect::from_min_max(head.min, Pos2::new(right, head.max.y)));
    let count = crumbs.len();
    for (k, (crumb, dir)) in crumbs.into_iter().enumerate() {
        if x >= right {
            break;
        }
        let last = k + 1 == count;
        let w = painter.layout_no_wrap(crumb.clone(), font.clone(), pal().text).size().x;
        let rect = Rect::from_min_size(Pos2::new(x - 3.0, mid - 10.0), Vec2::new(w + 6.0, 20.0));
        let color = if last { pal().text } else { pal().text_dim };
        match dir {
            Some(dir) => {
                let resp = ui.interact(
                    rect.intersect(painter.clip_rect()),
                    egui::Id::unique(("crumb", k, &crumb)),
                    Sense::click(),
                );
                let color = if resp.hovered() { pal().text } else { color };
                text_fit(&painter, Pos2::new(x, mid), &crumb, font.clone(), color, right - x);
                if resp.on_hover_text("Show this folder on the canvas").clicked() {
                    actions.push(Action::Tabs(TabOp::Canvas));
                    actions.push(match dir.is_empty() {
                        true => Action::Fit,
                        false => Action::FlyToPath(dir),
                    });
                }
            }
            None => {
                text_fit(&painter, Pos2::new(x, mid), &crumb, font.clone(), color, right - x);
            }
        }
        x += w;
        if !last {
            let arrow = codicon("chevron-right", icons::CHEVRON_RIGHT);
            painter.text(
                Pos2::new(x + 9.0, mid),
                Align2::CENTER_CENTER,
                arrow,
                FontId::proportional(11.0),
                pal().text_faint,
            );
            x += 18.0;
        }
    }
}

fn say(ui: &mut egui::Ui, body: Rect, text: &str, button: Option<(&str, Action)>, actions: &mut Vec<Action>) {
    let at = body.center() - Vec2::new(0.0, 24.0);
    let painter = ui.painter().with_clip_rect(body);
    let wrap = (body.width() - 60.0).max(120.0);
    let galley = painter.layout(text.to_string(), FontId::proportional(13.5), pal().text_dim, wrap);
    painter.galley(at - galley.size() * 0.5, galley, pal().text_dim);
    let Some((label, action)) = button else { return };
    let font = FontId::proportional(13.0);
    let w = painter.layout_no_wrap(label.to_string(), font.clone(), pal().text).size().x + 28.0;
    let rect = Rect::from_center_size(at + Vec2::new(0.0, 40.0), Vec2::new(w, 28.0));
    let resp = ui.interact(rect, egui::Id::unique(("file view button", label)), Sense::click());
    let fill = if resp.hovered() { pal().hover } else { pal().raised };
    painter.rect(rect, 7.0, fill, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, font, pal().text);
    if resp.clicked() {
        actions.push(action);
    }
}
