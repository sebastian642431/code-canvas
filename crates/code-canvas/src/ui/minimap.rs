use std::sync::mpsc::{self, Receiver};

use canvas_core::{Lang, Node, NodeKind, Rect as World, LINE_H};
use canvas_render::theme;
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::{c, pal, text_fit, Action, UiState};
use crate::ext::{Extensions, Severity};
use crate::git::HunkKind;
use crate::page::Page;

const WIDTH: f32 = 104.0;
const AWAY: f32 = 8.0;
/// How much of the canvas's right side the minimap takes, in points.
pub const REACH: f32 = WIDTH + AWAY;
const HEADER: f32 = 24.0;
const LANE: f32 = 3.0;
const LINE_PT: f32 = 2.0;
const PICTURE_W: usize = 88;
const MOST_ROWS: usize = 2048;
const TOKENIZED_LINES: usize = 100_000;
const MOST_MARKS: usize = 4000;
const LEAST_MARK: f32 = 2.0;
const BAR_FROM: f32 = 140.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    page: u32,
    file: usize,
    serial: u64,
    theme: u64,
}

/// The selected file in small, on the right of the canvas. Its picture is made away from the UI
/// thread once per text and theme; each frame only draws it and at most a strip's height of marks.
#[derive(Default)]
pub struct Minimap {
    /// The part of the active folder's world on screen, set by the app before each frame.
    pub seen: Option<World>,
    picture: Option<(Key, egui::TextureHandle)>,
    making: Option<(Key, Receiver<egui::ColorImage>)>,
}

impl Minimap {
    fn picture_for(&mut self, ctx: &egui::Context, key: Key, file: &canvas_core::SourceFile, page: &Page) {
        if let Some((made, rx)) = &self.making {
            match rx.try_recv() {
                Ok(image) => {
                    let texture = ctx.load_texture("minimap", image, egui::TextureOptions::LINEAR);
                    self.picture = Some((*made, texture));
                    self.making = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.making = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.making.is_some() || self.picture.as_ref().is_some_and(|(made, _)| *made == key) {
            return;
        }
        let text = file.has_text().then(|| (file.text().to_string(), file.line_starts().to_vec()));
        let (path, lang, colors) = (page.root.join(&file.rel_path), file.lang, theme::get().token_colors());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (text, starts) = text.unwrap_or_else(|| {
                let raw = std::fs::read(&path).unwrap_or_default();
                let (text, starts, _) = canvas_core::scan::normalise(&String::from_utf8_lossy(&raw));
                (text, starts)
            });
            let _ = tx.send(paint(&text, &starts, lang, colors));
            crate::ext::wake_ui();
        });
        self.making = Some((key, rx));
    }
}

fn picture_rows(lines: usize) -> usize {
    (lines * LINE_PT as usize).clamp(1, MOST_ROWS)
}

/// The line each row of the picture shows, and whether it is the faint second row of its line;
/// a file too long for two rows a line gets one row for each of the lines spread over it.
fn sampled(lines: usize, rows: usize) -> Vec<(usize, bool)> {
    let per = LINE_PT as usize;
    match lines * per <= rows {
        true => (0..rows).map(|row| (row / per, row % per == per - 1)).collect(),
        false => (0..rows).map(|row| (row * lines / rows, false)).collect(),
    }
}

fn paint(text: &str, starts: &[u32], lang: Lang, colors: [u32; 16]) -> egui::ColorImage {
    let lines = starts.len();
    let rows = picture_rows(lines);
    let tokens = (lines <= TOKENIZED_LINES).then(|| canvas_core::syntax::tokenize(text, starts, lang));
    let line_text = |at: usize| {
        let start = starts[at] as usize;
        let end = starts.get(at + 1).map_or(text.len(), |&next| next as usize - 1);
        text.get(start..end).unwrap_or("")
    };
    let plain = theme::kind_index(canvas_core::TokenKind::Ident) as usize;
    let mut rgba = vec![0u8; PICTURE_W * rows * 4];
    for (row, (line, faint)) in sampled(lines, rows).into_iter().enumerate().filter(|(_, (line, _))| *line < lines) {
        let mut kinds = [None; PICTURE_W];
        for (col, ch) in line_text(line).chars().take(PICTURE_W).enumerate() {
            if !ch.is_whitespace() {
                kinds[col] = Some(plain);
            }
        }
        for token in tokens.as_ref().map_or(&[][..], |tokens| tokens.line(line)) {
            let cols = token.col as usize..(token.col as usize + token.cols as usize).min(PICTURE_W);
            for kind in kinds.get_mut(cols).into_iter().flatten().filter(|kind| kind.is_some()) {
                *kind = Some(theme::kind_index(token.kind) as usize);
            }
        }
        let alpha = if faint { 90 } else { 200 };
        for (col, kind) in kinds.iter().enumerate() {
            let Some(kind) = kind else { continue };
            let [r, g, b, _] = theme::unpack(colors[*kind]);
            let at = (row * PICTURE_W + col) * 4;
            rgba[at..at + 4].copy_from_slice(&[r, g, b, alpha]);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([PICTURE_W, rows], &rgba)
}

/// Where the lines `from..to` of a file of `lines` lines fall in a picture `height` tall (`to ==
/// from` is a mark between two lines), as bands at least `least` tall, overlapping ones merged:
/// never more than the picture has room for, however many marks there are.
fn bands(spans: impl IntoIterator<Item = (u32, u32)>, lines: u32, height: f32, least: f32) -> Vec<(f32, f32)> {
    let lines = lines.max(1) as f32;
    let mut out: Vec<(f32, f32)> = spans
        .into_iter()
        .map(|(from, to)| {
            let (top, bottom) = (from as f32 * height / lines, to as f32 * height / lines);
            let grow = (least - (bottom - top)).max(0.0) * 0.5;
            ((top - grow).max(0.0), (bottom + grow).min(height))
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::with_capacity(out.len().min(height as usize + 1));
    for (top, bottom) in out {
        match merged.last_mut() {
            Some(last) if top <= last.1 + 0.5 => last.1 = last.1.max(bottom),
            _ => merged.push((top, bottom)),
        }
    }
    merged
}

fn line_at(y: f32, lines: u32, height: f32) -> u32 {
    let at = (y / height.max(1.0) * lines as f32).floor();
    (at.max(0.0) as u32).min(lines.saturating_sub(1))
}

fn line_at_world(n: &Node, y: f32, lines: usize) -> u32 {
    let row = ((y - n.code_origin()[1]) / LINE_H).max(0.0) as u32;
    let line = match &n.wrap {
        Some(wrap) => wrap.line_at(row).map_or(lines, |(line, _)| line),
        None => row as usize,
    };
    line.min(lines) as u32
}

fn marks(page: &Page, ext: &Extensions, file: usize, rel: &str, lines: u32) -> Vec<(Color32, Vec<(u32, u32)>)> {
    let t = theme::get();
    let shown = |line: u32| page.row_of(rel, line).min(lines);
    let (mut added, mut modified, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    let change = page.git.as_ref().and_then(|git| git.changes.iter().find(|change| change.path == rel));
    for hunk in change.map_or(&[][..], |change| change.hunks.as_slice()).iter().take(MOST_MARKS) {
        let covered = hunk.lines();
        let span = (shown(covered.start), shown(covered.end.saturating_sub(1)).max(shown(covered.start)) + 1);
        match hunk.kind {
            HunkKind::Added => added.push(span),
            HunkKind::Modified => modified.push(span),
            HunkKind::Removed => removed.push((span.0, span.0)),
        }
    }
    let (mut errors, mut warnings) = (Vec::new(), Vec::new());
    let key = crate::ext::path_key(&page.root.join(rel).to_string_lossy());
    for problem in ext.diagnostics.get(&key).map_or(&[][..], |found| found.items.as_slice()).iter().take(MOST_MARKS) {
        let span = (shown(problem.line), shown(problem.end_line.max(problem.line)) + 1);
        match problem.severity {
            Severity::Error => errors.push(span),
            Severity::Warning => warnings.push(span),
            _ => {}
        }
    }
    let hits = page.search.as_ref().map_or(&[][..], |search| search.hits.as_slice());
    let found =
        hits.iter().filter(|hit| hit.file as usize == file).take(MOST_MARKS).map(|hit| (hit.line, hit.line + 1));
    vec![
        (c(t.git_added), added),
        (c(t.git_modified), modified),
        (c(t.git_removed), removed),
        (c(t.find_match), found.collect()),
        (c(t.problem_warning), warnings),
        (c(t.problem_error), errors),
    ]
}

/// The strip's place: at the right of the canvas, left of the cards there, and clear of the bars
/// floating over the canvas's top and bottom.
fn place(st: &UiState, canvas: Rect) -> Option<Rect> {
    let right = canvas.right() - st.cards_reach.max(AWAY);
    let (left, mut top, mut bottom) = (right - WIDTH, canvas.top() + AWAY, canvas.bottom() - AWAY);
    for bar in st.floating.iter().filter(|bar| bar.height() < 120.0 && bar.right() > left && bar.left() < right) {
        if bar.top() < canvas.top() + 120.0 {
            top = top.max(bar.bottom() + 6.0);
        } else if bar.bottom() > canvas.bottom() - 120.0 {
            bottom = bottom.min(bar.top() - 6.0);
        }
    }
    (left > canvas.left() + REACH && bottom - top > BAR_FROM)
        .then(|| Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)))
}

pub(super) fn minimap(
    ctx: &egui::Context,
    st: &mut UiState,
    canvas: Rect,
    page: Option<&Page>,
    ext: &Extensions,
    actions: &mut Vec<Action>,
) {
    let Some(page) = page else { return };
    let Some(r) = page.ready() else { return };
    let Some(node) = page.selected.and_then(|at| r.layout.nodes.get(at)) else { return };
    let NodeKind::File(fi) = node.kind else { return };
    let file = &r.repo.files[fi];
    if file.note.is_some() || file.line_count() == 0 {
        return;
    }
    let Some(strip) = place(st, canvas) else { return };
    let key = Key { page: page.id, file: fi, serial: file.serial(), theme: theme::revision() };
    st.minimap.picture_for(ctx, key, file, page);
    let lines = file.line_count() as u32;
    let body = Rect::from_min_max(strip.min + Vec2::new(0.0, HEADER), strip.max - Vec2::new(0.0, 6.0));
    let room = Rect::from_min_max(body.min + Vec2::new(LANE + 3.0, 0.0), body.max - Vec2::new(LANE + 3.0, 0.0));
    let tall = (lines as f32 * LINE_PT).min(room.height());
    let content = Rect::from_min_size(room.min, Vec2::new(room.width(), tall));
    let seen = st.minimap.seen;
    let shown = marks(page, ext, fi, &node.rel_path, lines);
    let id = egui::Id::unique("minimap");
    let resp = egui::Area::new(id).fixed_pos(strip.min).order(egui::Order::Middle).show(ctx, |ui| {
        ui.set_min_size(strip.size());
        let painter = ui.painter().clone();
        painter.rect(strip, 8.0, pal().card, Stroke::new(1.0, pal().border), egui::StrokeKind::Inside);
        let head = Rect::from_min_size(strip.min, Vec2::new(strip.width(), HEADER));
        let title = ui.interact(head, id.with("head"), Sense::click());
        let color = if title.hovered() { pal().text } else { pal().text_dim };
        let at = Pos2::new(head.left() + 9.0, head.center().y + 1.0);
        text_fit(&painter, at, &file.name, FontId::proportional(11.5), color, head.width() - 16.0);
        if title.on_hover_text(format!("{}\nClick to see the whole card", file.rel_path)).clicked() {
            actions.push(Action::FlyToPath(node.rel_path.clone()));
        }
        let made = st.minimap.picture.as_ref().filter(|(made, _)| made.page == key.page && made.file == key.file);
        if let Some((_, texture)) = made {
            let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            let width = (PICTURE_W as f32).min(content.width());
            let drawn = Rect::from_min_size(content.min, Vec2::new(width, content.height()));
            painter.image(texture.id(), drawn, whole, Color32::WHITE);
        }
        for (i, (color, spans)) in shown.into_iter().enumerate() {
            let x = match i {
                0..=2 => body.left() + 2.0,
                _ => body.right() - LANE - 2.0,
            };
            for (top, bottom) in bands(spans, lines, content.height(), LEAST_MARK) {
                let mark =
                    Rect::from_min_max(Pos2::new(x, content.top() + top), Pos2::new(x + LANE, content.top() + bottom));
                painter.rect_filled(mark, 1.0, color);
            }
        }
        if let Some(seen) = seen.filter(|seen| seen.intersects(&node.rect)) {
            let count = file.line_count();
            let (first, last) = (line_at_world(node, seen.y, count), line_at_world(node, seen.bottom(), count));
            let y = |line: u32| content.top() + line as f32 * content.height() / lines as f32;
            let (top, bottom) = (y(first), y(last.max(first + 1)).max(y(first) + LEAST_MARK));
            let slider =
                Rect::from_min_max(Pos2::new(content.left() - 2.0, top), Pos2::new(content.right() + 2.0, bottom));
            painter.rect_filled(slider, 2.0, pal().text.gamma_multiply(0.12));
        }
        let pull = ui.interact(content, id.with("strip"), Sense::click_and_drag());
        if pull.is_pointer_button_down_on() {
            if let Some(p) = pull.interact_pointer_pos() {
                let line = line_at(p.y - content.top(), lines, content.height());
                let cell = node.cell_rect(line, 0, 1);
                let across = seen.is_some_and(|seen| cell.x >= seen.x && cell.x < seen.right());
                let left = (!across).then_some(cell.x - node.gutter - canvas_core::layout::CARD_PAD);
                actions.push(Action::MinimapTo { y: cell.y + cell.h * 0.5, left });
            }
        }
    });
    st.floating.push(resp.response.rect.union(strip));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_file_gets_two_rows_a_line() {
        assert_eq!(picture_rows(10), 20);
        assert_eq!(sampled(3, 6), [(0, false), (0, true), (1, false), (1, true), (2, false), (2, true)]);
    }

    #[test]
    fn a_long_file_is_spread_over_the_rows_there_are() {
        assert_eq!(picture_rows(50_000), MOST_ROWS);
        let rows = sampled(10_000, 4);
        assert_eq!(rows, [(0, false), (2500, false), (5000, false), (7500, false)]);
        assert!(sampled(1_000_000, MOST_ROWS).iter().all(|(line, _)| *line < 1_000_000));
    }

    #[test]
    fn marks_fall_where_their_lines_are_and_merge_when_they_touch() {
        assert_eq!(bands([(0, 10)], 100, 200.0, 2.0), [(0.0, 20.0)]);
        assert_eq!(bands([(50, 50)], 100, 200.0, 2.0), [(99.0, 101.0)], "a removed line is a thin tick");
        assert_eq!(bands([(20, 21), (10, 12), (11, 15)], 100, 200.0, 2.0), [(20.0, 30.0), (40.0, 42.0)]);
    }

    #[test]
    fn many_marks_never_make_more_bands_than_the_strip_has_room_for() {
        let every = (0..100_000u32).step_by(2).map(|line| (line, line + 1));
        let drawn = bands(every, 100_000, 300.0, 2.0);
        assert!(drawn.len() <= 150, "{} bands", drawn.len());
        assert!(bands(Vec::new(), 10, 100.0, 2.0).is_empty());
    }

    #[test]
    fn a_point_of_the_strip_is_a_line_of_the_file() {
        assert_eq!(line_at(0.0, 400, 200.0), 0);
        assert_eq!(line_at(100.0, 400, 200.0), 200);
        assert_eq!(line_at(500.0, 400, 200.0), 399, "below the picture is the last line");
        assert_eq!(line_at(-4.0, 400, 200.0), 0);
    }
}
