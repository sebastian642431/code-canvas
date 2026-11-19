use std::collections::HashSet;

use canvas_core::deps::Edge;
use canvas_core::layout::{rest_of_line, LineCut, NodeKind, RestOfLine, CARD_PAD, DIR_HEADER, FILE_LABEL_H, NONE};
use canvas_core::{Layout, Rect, RelKind, Repo, Site, SymRef, CHAR_W, LINE_H};
use canvas_render::text::TextLayer;
use canvas_render::{
    curve_point, push_curve, push_edge, theme, CurveVertex, Label, Lod, RectInstance, StripRow, Stroke, View,
};

use crate::ext::{Extensions, Severity};
use crate::git::{Change, HunkKind, RowKind};
use crate::page::{DepFilters, Diffed, Page, PageState, Ready, RelFilters, Selection};

fn row_of(shown: Option<&Diffed>, line: u32) -> u32 {
    shown.map_or(line, |shown| shown.row(line))
}

fn folded_away(shown: Option<&Diffed>, line: u32) -> bool {
    shown.is_some_and(|shown| !shown.shows(line))
}

const MAX_WORLD_LABELS: usize = 600;
const MAX_CHIPS: usize = 260;
const MAX_BODIES: usize = 400;
const MAX_MARKERS: usize = 3000;
const MAX_DECORATIONS: usize = 600;
pub const SYMBOL_HOVER_MIN_PX: f32 = 0.6;

pub struct OwnedLabel {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub size: f32,
    pub color: u32,
    pub max_w: f32,
    pub bold: bool,
}

impl OwnedLabel {
    pub fn as_label(&self) -> Label<'_> {
        Label {
            text: &self.text,
            left: self.left,
            top: self.top,
            size: self.size,
            color: self.color,
            max_w: self.max_w,
            bold: self.bold,
        }
    }
}

#[derive(Default)]
pub struct CanvasOverlay {
    pub under: Vec<RectInstance>,
    pub over: Vec<RectInstance>,
    pub chips: Vec<RectInstance>,
    pub labels: Vec<OwnedLabel>,
    pub chip_labels: Vec<OwnedLabel>,
    pub chip_hits: Vec<(Rect, SymRef)>,
    pub toggle_hits: Vec<(Rect, String)>,
    pub preview_hits: Vec<(Rect, String)>,
    /// The code lenses drawn that run something, in screen pixels.
    pub lens_hits: Vec<(Rect, crate::ext::views::Lens)>,
    pub version_hits: Vec<(Rect, String, u32)>,
}

pub struct LongLine {
    pub file: usize,
    pub line: usize,
    pub rest: RestOfLine,
    card_row: Rect,
}

pub fn long_line(page: &Page, view: View, area: Rect, s: f32, file: usize, line: usize) -> Option<LongLine> {
    if Lod::for_zoom(view.zoom) != Lod::Text {
        return None;
    }
    let r = page.ready()?;
    let n = r.layout.file_node.get(file).filter(|&&n| n != NONE).map(|&n| &r.layout.nodes[n as usize])?;
    let f = r.repo.files.get(file)?;
    let shown = n.shown_cols();
    if line >= f.line_count() || f.max_cols as usize <= shown || n.wrap.is_some() {
        return None;
    }
    let card_row = n.cell_rect(line as u32, 0, shown as u32);
    let cut = LineCut {
        origin: view.to_screen([card_row.x, card_row.y]),
        char_w: CHAR_W * view.zoom,
        line_h: LINE_H * view.zoom,
        shown,
        len: f.line(line).chars().count(),
    };
    let rest = rest_of_line(&cut, area, (4.0 * s).round())?;
    Some(LongLine { file, line, rest, card_row })
}

impl LongLine {
    pub fn draw(&self, out: &mut CanvasOverlay, caret: Option<usize>, s: f32) -> Vec<StripRow> {
        let t = theme::get();
        if caret.is_none() {
            out.under.push(RectInstance::new(self.card_row, t.current_line, 0, 0.0, 0.0));
        }
        let fill = theme::mix(t.background, t.card_fill | 0xff, (t.card_fill & 0xff) as f32 / 255.0);
        out.chips.push(RectInstance::new(self.rest.frame, fill, t.card_border, 3.0 * s, 1.0));
        if let Some(at) = caret.and_then(|col| self.rest.place_of(col)) {
            let w = (2.0 * s).round().max(1.0);
            out.chips.push(RectInstance::new(Rect::new(at.x - w * 0.5, at.y, w, at.h), t.caret, 0, 0.0, 0.0));
        }
        self.rest
            .rows
            .iter()
            .map(|row| StripRow {
                left: row.text_left,
                top: row.rect.y,
                bounds: [
                    row.rect.x.floor() as i32,
                    row.rect.y.floor() as i32,
                    row.rect.right().ceil() as i32,
                    row.rect.bottom().ceil() as i32,
                ],
            })
            .collect()
    }
}

fn severity_color(severity: Severity) -> u32 {
    match severity {
        Severity::Error => theme::get().problem_error,
        Severity::Warning => theme::get().problem_warning,
        Severity::Info | Severity::Hint => theme::get().problem_info,
    }
}

fn with_alpha(color: u32, alpha: f32) -> u32 {
    (color & 0xffff_ff00) | (alpha.clamp(0.0, 1.0) * 255.0) as u32
}

fn rel_color(kind: RelKind) -> u32 {
    theme::get().rel_colors()[kind.index()]
}

const MAX_DEP_CURVES: usize = 4000;
const BUNDLE_FROM: usize = 300;
const GROUP_PX: f32 = 150.0;
const ARROW_WORLD: f32 = LINE_H * 0.9;
const MAX_DEP_MARKS: usize = 300;

#[derive(Clone, Copy, Debug)]
pub struct DepMark {
    pub at: [f32; 2],
    pub count: u32,
    pub color: u32,
}

pub fn zoom_step(zoom: f32) -> i32 {
    (zoom.max(1e-9).log2() * 2.0).round() as i32
}

fn edge_width(count: u32, scale: f32) -> f32 {
    (1.25 + 0.7 * (count.max(1) as f32).log2()).min(7.0) * scale
}

fn crossing<'r>(r: &'r Ready, node: usize, deps: &DepFilters) -> (Vec<&'r Edge>, Vec<&'r Edge>) {
    let Some(n) = r.layout.nodes.get(node) else { return (Vec::new(), Vec::new()) };
    let range = node as u32..n.subtree_end;
    let inside = |f: u32| r.layout.file_anchor.get(f as usize).is_some_and(|a| range.contains(a));
    let files: Vec<u32> = match n.kind {
        NodeKind::File(fi) => vec![fi as u32],
        NodeKind::Dir => (0..r.repo.files.len() as u32).filter(|f| inside(*f)).collect(),
    };
    let shown = |e: &&Edge| deps.kinds[e.kind.index()];
    let mut outgoing = Vec::new();
    let mut incoming = Vec::new();
    for &f in &files {
        outgoing.extend(r.deps.outgoing(f).iter().filter(|e| !inside(e.to)).filter(shown));
        incoming.extend(r.deps.incoming(f).filter(|e| !inside(e.from)).filter(shown));
    }
    (outgoing, incoming)
}

fn box_end(rect: Rect, toward: [f32; 2]) -> [f32; 2] {
    let x = if toward[0] > rect.right() { rect.right() } else { rect.x };
    let y = toward[1].clamp(rect.y + rect.h * 0.2, rect.bottom() - rect.h * 0.2);
    [x, y]
}

#[derive(Default)]
struct DepsDrawn {
    edges: usize,
    curves: usize,
    bundled: bool,
}

struct DepPen<'a> {
    out: &'a mut Vec<CurveVertex>,
    marks: &'a mut Vec<DepMark>,
    scale: f32,
    groups: Option<&'a [u32]>,
}

impl DepPen<'_> {
    fn draw(&mut self, r: &Ready, edges: &[&Edge], color: u32, room: usize) -> DepsDrawn {
        let site = |file: u32, line: u32| Site { file, line, col: 0, len: 0 };
        let anchor = |file: u32, line: u32| r.layout.anchor(&site(file, line)).map(|(p, _)| p);
        let order = r.layout.order.as_deref();
        let dashed = |e: &Edge| r.deps.in_cycle(e) || order.is_some_and(|o| o.against(e));
        let arrow = ARROW_WORLD;
        let mut done = DepsDrawn::default();
        let Some(groups) = self.groups else {
            for e in edges {
                if done.curves >= room {
                    break;
                }
                let (Some(a), Some(b)) = (anchor(e.from, e.line), anchor(e.to, 0)) else { continue };
                let stroke = Stroke { dashed: dashed(e), arrow };
                push_edge(self.out, a, b, color, edge_width(1, self.scale), stroke);
                done.edges += 1;
                done.curves += 1;
            }
            return done;
        };
        done.bundled = true;
        let group = |f: u32| r.layout.file_anchor.get(f as usize).filter(|a| **a != NONE).map(|&a| groups[a as usize]);
        let pairs =
            edges.iter().enumerate().filter_map(|(i, e)| Some((group(e.from)?, group(e.to)?, dashed(e), i as u32)));
        let bundles = canvas_core::deps::bundle(pairs);
        for b in bundles.iter().take(room) {
            let e = edges[b.first as usize];
            let (from, to) = (&r.layout.nodes[b.from as usize], &r.layout.nodes[b.to as usize]);
            let middle = |n: &canvas_core::Node, file: u32, line: u32| match n.is_dir() {
                true => Some(n.rect.center()),
                false => anchor(file, line),
            };
            let (Some(ma), Some(mb)) = (middle(from, e.from, e.line), middle(to, e.to, 0)) else { continue };
            let a = if from.is_dir() { box_end(from.rect, mb) } else { ma };
            let z = if to.is_dir() { box_end(to.rect, ma) } else { mb };
            let stroke = Stroke { dashed: b.cyclic, arrow };
            push_edge(self.out, a, z, color, edge_width(b.count, self.scale), stroke);
            if b.count > 1 && (from.is_dir() || to.is_dir()) && self.marks.len() < MAX_DEP_MARKS {
                self.marks.push(DepMark { at: curve_point(a, z, 0.5), count: b.count, color: with_alpha(color, 1.0) });
            }
            done.edges += b.count as usize;
            done.curves += 1;
        }
        done
    }
}

pub fn build_curves(page: &mut Page, filters: &RelFilters, deps: &DepFilters, scale: f32) -> Vec<CurveVertex> {
    let mut out = Vec::new();
    page.dep_marks.clear();
    page.dep_zoom = None;
    page.dep_edges = (0, 0);
    let PageState::Ready(r) = &page.state else { return out };
    let step = zoom_step(page.camera.zoom);
    let min_size = GROUP_PX * scale / 2f32.powf(step as f32 / 2.0);
    let mut marks = Vec::new();
    let mut bundled = false;
    if deps.all {
        let edges: Vec<&Edge> = r.deps.edges.iter().filter(|e| deps.kinds[e.kind.index()]).collect();
        let groups = (edges.len() > BUNDLE_FROM).then(|| r.layout.groups(min_size, page.selected));
        let mut pen = DepPen { out: &mut out, marks: &mut marks, scale, groups: groups.as_deref() };
        let color = with_alpha(theme::get().dep_out, 0.35);
        bundled |= pen.draw(r, &edges, color, MAX_DEP_CURVES).bundled;
    }
    if let (true, Some(node)) = (deps.selected, page.selected) {
        let (outgoing, incoming) = crossing(r, node, deps);
        let total = outgoing.len() + incoming.len();
        let groups = (total > BUNDLE_FROM).then(|| r.layout.groups(min_size, Some(node)));
        let mut pen = DepPen { out: &mut out, marks: &mut marks, scale, groups: groups.as_deref() };
        let colors = theme::get();
        let went = pen.draw(r, &outgoing, with_alpha(colors.dep_out, 0.7), MAX_DEP_CURVES);
        let came = pen.draw(r, &incoming, with_alpha(colors.dep_in, 0.7), MAX_DEP_CURVES - went.curves);
        bundled |= went.bundled || came.bundled;
        page.dep_edges = (went.edges + came.edges, total);
    }
    page.dep_marks = marks;
    if bundled {
        page.dep_zoom = Some(step);
    }
    for sel in &mut page.selections {
        sel.outside = 0;
        let def = r.repo.sym_site(sel.sym);
        let def_start = r.repo.sym_line_start(sel.sym);
        for rel in &sel.rel.items {
            let to = if rel.to == def {
                def_start
            } else {
                rel.other.filter(|o| r.repo.sym_site(*o) == rel.to).map(|o| r.repo.sym_line_start(o)).unwrap_or(rel.to)
            };
            let (a, b) = (r.layout.anchor(&rel.from), r.layout.anchor(&to));
            let (Some((a, _)), Some((b, _))) = (a, b) else {
                sel.outside += 1;
                continue;
            };
            if !filters.enabled[rel.kind.index()] {
                continue;
            }
            let color = with_alpha(rel_color(rel.kind), filters.opacity[rel.kind.index()] * 0.9);
            push_curve(&mut out, a, b, color, 1.5 * scale);
        }
    }
    out
}

struct Chip {
    at: [f32; 2],
    text: String,
    border: u32,
    text_color: u32,
    bold: bool,
    sym: Option<SymRef>,
}

fn body_rects(repo: &Repo, layout: &Layout, sym: SymRef) -> Vec<Rect> {
    let node = layout.file_node[sym.file as usize];
    if node == NONE {
        return Vec::new();
    }
    let s = repo.sym(sym);
    layout.nodes[node as usize].line_rects(s.line..s.body_end)
}

fn site_rects(layout: &Layout, site: &Site) -> Vec<Rect> {
    match layout.file_node.get(site.file as usize).copied().filter(|&node| node != NONE) {
        Some(node) => layout.nodes[node as usize].cell_rects(site.line, site.col as u32, site.len as u32),
        None => Vec::new(),
    }
}

fn fitted(text: &mut TextLayer, name: &str, end: &str, size: f32, bold: bool, room: f32) -> String {
    let whole = format!("{name}{end}");
    let count = name.chars().count();
    if (count + end.chars().count()) as f32 * size <= room {
        return whole;
    }
    let wide = text.measure(&whole, size, bold);
    if wide <= room {
        return whole;
    }
    let mut keep = ((count as f32 * room / wide) as usize).min(count.saturating_sub(1));
    loop {
        let cut: String = name.chars().take(keep).chain("…".chars()).chain(end.chars()).collect();
        if keep == 0 || text.measure(&cut, size, bold) <= room {
            return cut;
        }
        keep -= 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn selection_overlay(
    sel: &Selection,
    repo: &Repo,
    layout: &Layout,
    filters: &RelFilters,
    world: &Rect,
    out: &mut CanvasOverlay,
    chips: &mut Vec<Chip>,
    seen_bodies: &mut HashSet<SymRef>,
    seen_chips: &mut HashSet<(u32, u32)>,
) {
    let def = repo.sym_site(sel.sym);
    let own = RelKind::Definition.index();
    if filters.enabled[own] && filters.bodies[own] && seen_bodies.insert(sel.sym) {
        for rect in body_rects(repo, layout, sel.sym) {
            if rect.intersects(world) {
                let o = filters.body_opacity[own];
                let ink = theme::get().chip_text;
                out.under.push(RectInstance::new(rect, with_alpha(ink, 0.07 * o), with_alpha(ink, 0.45 * o), 2.0, 1.0));
            }
        }
    }
    if let Some((at, _)) = layout.anchor(&repo.sym_line_start(sel.sym)) {
        chips.push(Chip {
            at,
            text: repo.sym_name(sel.sym).to_string(),
            border: theme::get().chip_text,
            text_color: theme::get().chip_text,
            bold: true,
            sym: Some(sel.sym),
        });
        seen_chips.insert((def.file, def.line));
    }

    for rel in &sel.rel.items {
        if !filters.enabled[rel.kind.index()] {
            continue;
        }
        let color = rel_color(rel.kind);
        let opacity = filters.opacity[rel.kind.index()];
        if rel.from != rel.to && out.over.len() < MAX_MARKERS {
            for rect in site_rects(layout, &rel.from).into_iter().filter(|rect| rect.intersects(world)) {
                out.over.push(RectInstance::new(
                    rect.expand(1.0),
                    with_alpha(color, 0.16 * opacity),
                    with_alpha(color, 0.9 * opacity),
                    2.0,
                    1.0,
                ));
            }
        }
        let Some(other) = rel.other else { continue };
        if other == sel.sym {
            continue;
        }
        let other_sym = repo.sym(other);
        let show_body = other_sym.kind.is_callable() || rel.kind == RelKind::Implementation;
        let body_opacity = filters.body_opacity[rel.kind.index()];
        if filters.bodies[rel.kind.index()] && show_body && seen_bodies.len() < MAX_BODIES && seen_bodies.insert(other)
        {
            for rect in body_rects(repo, layout, other) {
                if rect.intersects(world) {
                    out.under.push(RectInstance::new(
                        rect,
                        with_alpha(color, 0.07 * body_opacity),
                        with_alpha(color, 0.45 * body_opacity),
                        2.0,
                        1.0,
                    ));
                }
            }
        }
        if rel.kind == RelKind::Reference && !other_sym.kind.is_callable() {
            continue;
        }
        let other_site = repo.sym_site(other);
        if !seen_chips.insert((other_site.file, other_site.line)) {
            continue;
        }
        if let Some((at, _)) = layout.anchor(&repo.sym_line_start(other)) {
            let name = repo.sym_name(other);
            chips.push(Chip {
                at,
                text: if rel.kind == RelKind::Implementation { format!("impl {name}") } else { name.to_string() },
                border: with_alpha(color, 0.35 + 0.65 * opacity),
                text_color: theme::get().chip_text_dim,
                bold: false,
                sym: Some(other),
            });
        }
    }
}

/// Lenses are drawn once a line is this many pixels tall: the zoom where code is read.
const LENS_MIN_PX: f32 = 6.5;
const MAX_LENS_LINES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LensAt {
    /// On the line above, at this column.
    Above(u32),
    /// The first line's: on the row of the file's name, after the name.
    Name,
    /// After the end of the line itself, at this column.
    After(u32),
}

/// Where the lenses of a line go: on the line above, at the line's indentation when the
/// line above is empty there, else after the text above if it fits in the card; the first
/// line's beside the file's name; otherwise (no room above) after the end of the line.
pub fn lens_place(above: Option<&str>, line: &str, cols: u32, shown: u32) -> LensAt {
    let indent = line.chars().take_while(|c| c.is_whitespace()).count() as u32;
    let end_of = |text: &str| text.trim_end().chars().count() as u32;
    match above {
        None => LensAt::Name,
        Some(above) if end_of(above) <= indent => LensAt::Above(indent),
        Some(above) if end_of(above) + 2 + cols <= shown => LensAt::Above(end_of(above) + 2),
        _ => LensAt::After(end_of(line) + 3),
    }
}

fn code_lenses(
    page: &Page,
    view: View,
    world: &Rect,
    s: f32,
    ext: &Extensions,
    text: &mut TextLayer,
    out: &mut CanvasOverlay,
) {
    let PageState::Ready(r) = &page.state else { return };
    let (repo, layout, z) = (&r.repo, &r.layout, view.zoom);
    let t = theme::get();
    let size = (z * LINE_H * 0.72).round();
    let char_w = CHAR_W * z;
    let bar = " | ";
    let bar_w = text.measure(bar, size, false);
    let mut drawn = 0usize;
    for (rel, found) in ext.lenses_under(&page.root) {
        let Some(fi) = repo.file_index(rel) else { continue };
        let Some(n) = layout.file_node.get(fi).filter(|&&n| n != NONE).map(|&n| &layout.nodes[n as usize]) else {
            continue;
        };
        if !n.rect.intersects(world) || found.items.is_empty() {
            continue;
        }
        let f = &repo.files[fi];
        let rows = page.diffs.get(rel);
        let mut lines: std::collections::BTreeMap<u32, Vec<&crate::ext::views::Lens>> = Default::default();
        for lens in &found.items {
            lines.entry(lens.line).or_default().push(lens);
        }
        for (line, mut lenses) in lines {
            let row = row_of(rows, line);
            if row as usize >= f.line_count() || drawn >= MAX_LENS_LINES || folded_away(rows, line) {
                continue;
            }
            if !n.cell_rect(row, 0, 1).intersects(&world.expand(LINE_H * 2.0)) {
                continue;
            }
            drawn += 1;
            lenses.sort_by_key(|l| l.col);
            let titles: Vec<String> = lenses.iter().map(|l| crate::ext::views::plain_title(&l.title)).collect();
            let width: f32 =
                titles.iter().map(|t| text.measure(t, size, false)).sum::<f32>() + bar_w * (titles.len() - 1) as f32;
            let cols = (width / char_w).ceil() as u32;
            let above = (row > 0).then(|| f.line(row as usize - 1));
            let line_h = z * LINE_H;
            let cell_at = |row: u32, col: u32| {
                let cell = n.cell_rect(row, col.min(canvas_core::layout::MAX_COLS), 1);
                view.to_screen([cell.x, cell.y])
            };
            let (mut x, top) = match lens_place(above, f.line(row as usize), cols, n.shown_cols() as u32) {
                LensAt::Above(col) => {
                    let p = cell_at(row - 1, col);
                    (p[0], p[1] + (line_h - size * 1.25) * 0.9)
                }
                LensAt::After(col) => {
                    let p = cell_at(row, col);
                    (p[0], p[1] + (line_h - size * 1.25) * 0.5)
                }
                LensAt::Name => {
                    let lab_px = FILE_LABEL_H * z;
                    let name_size = (lab_px * 0.7).min(14.0 * s).round();
                    let dirty = page.editors.get(&n.rel_path).is_some_and(|e| e.dirty);
                    let name = if dirty { format!("● {}", n.name) } else { n.name.clone() };
                    let corner = view.to_screen([n.rect.x, n.rect.y]);
                    let after_name = corner[0] + 2.0 + text.measure(&name, name_size, false) + 14.0 * s;
                    let indent = f.line(row as usize).chars().take_while(|c| c.is_whitespace()).count() as u32;
                    (cell_at(row, indent)[0].max(after_name), corner[1] - lab_px + (lab_px - size * 1.25) * 0.5)
                }
            };
            for (i, (lens, title)) in lenses.iter().zip(titles).enumerate() {
                if i > 0 {
                    out.labels.push(OwnedLabel {
                        text: bar.to_string(),
                        left: x,
                        top,
                        size,
                        color: theme::mix(t.ui_text_dim, t.background, 0.45),
                        max_w: bar_w + 2.0,
                        bold: false,
                    });
                    x += bar_w;
                }
                let w = text.measure(&title, size, false);
                let hovered = ext.views.lens_hover.as_deref() == Some(lens.id.as_str());
                let color = match (lens.runs, hovered) {
                    (true, true) => t.accent,
                    (true, false) => t.ui_text_dim,
                    (false, _) => theme::mix(t.ui_text_dim, t.background, 0.3),
                };
                out.labels.push(OwnedLabel { text: title, left: x, top, size, color, max_w: w + 2.0, bold: false });
                if hovered {
                    let under = Rect::new(x, top + size * 1.2, w, s.max(1.0));
                    out.chips.push(RectInstance::new(under, t.accent, 0, 0.0, 0.0));
                }
                if lens.runs {
                    let hit = Rect::new(x - 2.0 * s, top - s, w + 4.0 * s, size * 1.3 + 2.0 * s);
                    out.lens_hits.push((hit, (*lens).clone()));
                }
                x += w;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    page: &Page,
    view: View,
    area: Rect,
    s: f32,
    filters: &RelFilters,
    ext: &Extensions,
    pictures: &crate::pictures::Pictures,
    text: &mut TextLayer,
) -> CanvasOverlay {
    let mut out = CanvasOverlay::default();
    let PageState::Ready(r) = &page.state else { return out };
    let (repo, layout) = (&r.repo, &r.layout);
    let z = view.zoom;
    let mut world = view.world_rect();
    world.x += area.x / z;
    world.w = area.w / z;
    world.y += area.y / z;
    world.h = area.h / z;

    let mut captions: Vec<Rect> = Vec::new();
    let mut placed: Vec<Rect> = Vec::new();
    let free = |r: Rect, placed: &mut Vec<Rect>| {
        if placed.iter().any(|p| p.intersects(&r)) {
            false
        } else {
            placed.push(r);
            true
        }
    };
    layout.visit_visible(&world, |_, n| {
        let tl = view.to_screen([n.rect.x, n.rect.y]);
        let w_px = n.rect.w * z;
        match n.kind {
            NodeKind::Dir if n.collapsed => {
                let h_px = n.rect.h * z;
                if w_px > 46.0 * s && h_px > 22.0 * s {
                    let size = (h_px * 0.16).clamp(11.0 * s, 20.0 * s).round();
                    let label = fitted(text, &n.name, "/  +", size, true, w_px - 8.0 * s);
                    let tw = text.measure(&label, size, true).min(w_px - 8.0 * s);
                    let c = view.to_screen(n.rect.center());
                    let two_lines = h_px > size * 4.0;
                    let top = c[1] - if two_lines { size * 1.2 } else { size * 0.65 };
                    let hit = Rect::new(c[0] - tw * 0.5 - 6.0 * s, top - 3.0 * s, tw + 12.0 * s, size * 1.3 + 6.0 * s);
                    if free(hit, &mut placed) {
                        out.labels.push(OwnedLabel {
                            text: label,
                            left: c[0] - tw * 0.5,
                            top,
                            size,
                            color: theme::get().label_dir,
                            max_w: tw + 2.0,
                            bold: true,
                        });
                        out.toggle_hits.push((hit, n.rel_path.clone()));
                        let mut caption = hit;
                        if two_lines {
                            let sub = format!("{} files · {} lines", n.files, n.lines);
                            let sub_size = (size * 0.72).max(10.0 * s).round();
                            let sw = text.measure(&sub, sub_size, false).min(w_px - 8.0 * s);
                            let wide = tw.max(sw) + 12.0 * s;
                            let tall = top + size * 1.45 + sub_size * 1.3 + 3.0 * s - caption.y;
                            caption = Rect::new(c[0] - wide * 0.5, caption.y, wide, tall);
                            out.labels.push(OwnedLabel {
                                text: sub,
                                left: c[0] - sw * 0.5,
                                top: top + size * 1.45,
                                size: sub_size,
                                color: theme::get().ui_text_dim,
                                max_w: sw + 2.0,
                                bold: false,
                            });
                        }
                        captions.push(caption);
                    }
                }
            }
            NodeKind::Dir => {
                let header_px = DIR_HEADER * z;
                if (header_px >= 10.0 || w_px >= 280.0 * s) && w_px > 50.0 * s {
                    let size = (header_px * 0.55).clamp(11.0 * s, 16.0 * s).round();
                    let left = tl[0] + 8.0 * s;
                    let is_root = n.parent.is_none();
                    let top = if is_root {
                        tl[1] - size * 1.3 - 5.0 * s
                    } else {
                        (tl[1] + (header_px - size * 1.3) * 0.5).max(tl[1] + 2.0)
                    };
                    let max_w = w_px - 34.0 * s;
                    let label = fitted(text, &n.name, "/", size, true, max_w);
                    let tw = text.measure(&label, size, true).min(max_w);
                    let marker = Rect::new(left + tw + 6.0 * s, top - 1.0, size * 1.3 + 2.0, size * 1.3 + 2.0);
                    if free(Rect::new(left - 2.0, top, tw + marker.w + 12.0 * s, size * 1.3), &mut placed) {
                        out.labels.push(OwnedLabel {
                            text: label,
                            left,
                            top,
                            size,
                            color: if is_root { theme::get().ui_text } else { theme::get().label_dir },
                            max_w,
                            bold: true,
                        });
                        if !is_root {
                            out.labels.push(OwnedLabel {
                                text: "−".into(),
                                left: marker.x + size * 0.35,
                                top,
                                size,
                                color: theme::get().ui_text_dim,
                                max_w: f32::INFINITY,
                                bold: true,
                            });
                            out.toggle_hits.push((marker, n.rel_path.clone()));
                        }
                    }
                }
            }
            NodeKind::File(fi) => {
                let h_px = n.rect.h * z;
                let word = match (canvas_core::scan::media_kind(&n.rel_path), repo.files[fi].note.as_deref()) {
                    (Some("picture"), _) => pictures.problem(&page.root.join(&n.rel_path)).map(str::to_string),
                    (Some(what), _) => (h_px >= 90.0).then(|| format!("{what}  ·  click to play")),
                    (None, Some(note)) if !note.starts_with("truncated") => Some(note.to_string()),
                    _ => None,
                };
                let size = (13.0 * s).min((h_px - 2.0) / 1.3).floor();
                let said = word.filter(|_| size >= 9.0 * s).map(|word| (text.measure(&word, size, false), word));
                if let Some((width, word)) = said.filter(|(width, _)| width + 8.0 * s <= w_px) {
                    let middle = view.to_screen(n.rect.center());
                    out.labels.push(OwnedLabel {
                        text: word,
                        left: middle[0] - width * 0.5,
                        top: middle[1] - size * 0.65,
                        size,
                        color: theme::get().ui_text_dim,
                        max_w: width + 2.0,
                        bold: false,
                    });
                }
                let lab_px = FILE_LABEL_H * z;
                if lab_px >= 9.0 && w_px > 30.0 {
                    let size = (lab_px * 0.7).min(14.0 * s).round();
                    let left = tl[0] + 2.0;
                    let top = tl[1] - lab_px + (lab_px - size * 1.3) * 0.5;
                    let tw = (n.name.chars().count() as f32 * size * 0.58).min(w_px);
                    if free(Rect::new(left - 2.0, top, tw + 4.0, size * 1.3), &mut placed) {
                        let unsaved = page.editors.get(&n.rel_path).is_some_and(|e| e.dirty);
                        let shown = if unsaved { format!("● {}", n.name) } else { n.name.clone() };
                        let shown = fitted(text, &shown, "", size, false, w_px);
                        if crate::markdown::is_markdown(std::path::Path::new(&n.name)) {
                            let word = "Preview";
                            let word_w = text.measure(word, size, false);
                            let after = left + text.measure(&shown, size, false).max(tw) + 10.0 * s;
                            let button = Rect::new(after - 3.0 * s, top, word_w + 6.0 * s, size * 1.3);
                            if button.right() <= tl[0] + w_px && free(button, &mut placed) {
                                out.labels.push(OwnedLabel {
                                    text: word.into(),
                                    left: after,
                                    top,
                                    size,
                                    color: theme::get().accent,
                                    max_w: word_w + 2.0,
                                    bold: false,
                                });
                                out.preview_hits.push((button, n.rel_path.clone()));
                            }
                        }
                        out.labels.push(OwnedLabel {
                            text: shown,
                            left,
                            top,
                            size,
                            color: if unsaved { theme::get().ui_text } else { theme::get().label_file },
                            max_w: w_px,
                            bold: false,
                        });
                    }
                }
            }
        }
        out.labels.len() < MAX_WORLD_LABELS
    });

    if let Some(n) = page.hover.and_then(|h| layout.nodes.get(h)) {
        out.over.push(RectInstance::new(n.rect, 0, theme::get().canvas_hover, if n.is_dir() { 6.0 } else { 3.0 }, 1.5));
    }
    if let Some(n) = page.selected.and_then(|i| layout.nodes.get(i)) {
        let radius = if n.is_dir() { 6.0 } else { 3.0 };
        out.over.push(RectInstance::new(n.rect, theme::get().select_fill, theme::get().select, radius, 2.0));
    }

    if let Some(search) = &page.search {
        for (i, hit) in search.hits.iter().enumerate().take(MAX_MARKERS) {
            for rect in site_rects(layout, hit).into_iter().filter(|rect| rect.intersects(&world)) {
                let found = theme::get().find_match;
                let fill = with_alpha(found, if search.current == Some(i) { 0.5 } else { 0.19 });
                out.over.push(RectInstance::new(rect.expand(1.0), fill, found | 0xff, 2.0, 1.0));
            }
        }
    }

    let readable = z * LINE_H >= 2.5;

    if let Some(editor) = page.editor() {
        let node =
            repo.file_index(&editor.rel_path).and_then(|fi| layout.file_node.get(fi).copied()).filter(|&n| n != NONE);
        if let Some(n) = node.map(|n| &layout.nodes[n as usize]) {
            let width = repo.file_index(&editor.rel_path).map_or(1, |fi| repo.files[fi].max_cols.max(1));
            let w = (2.0 * s / z).max(0.6);
            let mut lit = std::collections::HashSet::new();
            for caret in editor.carets() {
                let at = caret.at;
                let rows = match n.wrap.is_some() {
                    true => n.line_rects(at.line as u32..at.line as u32 + 1),
                    false => vec![n.cell_rect(at.line as u32, 0, width)],
                };
                if rows.iter().any(|row| row.intersects(&world)) && lit.insert(at.line) {
                    for row in rows {
                        out.under.push(RectInstance::new(row, theme::get().current_line, 0, 0.0, 0.0));
                    }
                }
                if let Some((a, b)) = caret.selection() {
                    for line in a.line..=b.line {
                        let from = if line == a.line { editor.display_col(a) } else { 0 };
                        let to = if line == b.line {
                            editor.display_col(b)
                        } else {
                            editor.display_col(crate::editor::Pos::new(line, usize::MAX)) + 1
                        };
                        let cells = n.cell_rects(line as u32, from as u32, to.saturating_sub(from).max(1) as u32);
                        for cell in cells.into_iter().filter(|cell| cell.intersects(&world)) {
                            out.under.push(RectInstance::new(cell, theme::get().code_selection, 0, 0.0, 0.0));
                        }
                    }
                }
                let cell = n.cell_rect(at.line as u32, editor.display_col(at) as u32, 1);
                if cell.intersects(&world) {
                    let bar = Rect::new(cell.x - w * 0.5, cell.y, w, cell.h);
                    out.over.push(RectInstance::new(bar, theme::get().caret, 0, 0.0, 0.0));
                }
            }
        }
    }

    {
        let marked: Vec<(&str, Change, &[crate::git::Hunk])> = match (page.commit_shown(), &page.git) {
            (Some(showing), _) => showing.files.iter().map(|file| (file.path.as_str(), file.change, &[][..])).collect(),
            (None, Some(git)) => {
                let some = git.changes.iter().take(MAX_MARKERS);
                some.map(|change| (change.path.as_str(), change.change, change.hunks.as_slice())).collect()
            }
            _ => Vec::new(),
        };
        for (path, kind_of_change, hunks) in marked {
            let Some(fi) = repo.file_index(path) else { continue };
            let Some(n) = layout.file_node.get(fi).filter(|&&n| n != NONE).map(|&n| &layout.nodes[n as usize]) else {
                continue;
            };
            if !n.rect.intersects(&world) {
                continue;
            }
            let color = match kind_of_change {
                Change::Added | Change::Untracked => theme::get().git_added,
                Change::Deleted | Change::Conflicted => theme::get().git_removed,
                Change::Modified | Change::Renamed => theme::get().git_modified,
            };
            let strip = (3.0 * s / z).max(1.5);
            out.over.push(RectInstance::new(
                Rect::new(n.rect.x - strip, n.rect.y, strip, n.rect.h),
                color,
                0,
                0.0,
                0.0,
            ));
            if !readable {
                let side = 7.0 * s / z;
                let dot = Rect::new(n.rect.x - side * 0.5, n.rect.y - side * 0.5, side, side);
                out.over.push(RectInstance::new(dot, color, 0, 3.5 * s, 0.0));
            }
            let lines = repo.files[fi].line_count() as u32;
            let bar = (2.5 * s / z).min(CARD_PAD * 0.6);
            let wash = if readable { 0.16 } else { 0.55 };
            let seen = world.expand(CARD_PAD);
            if let Some(rows) = page.rows(path) {
                let mut first = 0usize;
                while first < rows.len() && out.under.len() < MAX_MARKERS {
                    let kind = rows[first].kind;
                    let mut end = first + 1;
                    while end < rows.len() && rows[end].kind == kind {
                        end += 1;
                    }
                    let tint = match kind {
                        RowKind::Added => Some((theme::get().git_added, wash)),
                        RowKind::Removed => Some((theme::get().git_removed, wash * 1.2)),
                        RowKind::Gap => Some((theme::get().ui_text_dim, 0.08)),
                        RowKind::Blank => Some((0x000000ff, if theme::get().is_light() { 0.06 } else { 0.16 })),
                        RowKind::Same | RowKind::Before => None,
                    };
                    for mut rect in n.line_rects(first as u32..(end as u32).min(lines)) {
                        let Some((c, alpha)) = tint.filter(|_| rect.intersects(&seen)) else { continue };
                        rect.h = rect.h.max(1.5 / z);
                        out.under.push(RectInstance::new(rect, with_alpha(c, alpha), 0, 0.0, 0.0));
                        if readable && matches!(kind, RowKind::Added | RowKind::Removed) {
                            let edge = Rect::new(rect.x - CARD_PAD * 0.75, rect.y, bar, rect.h);
                            out.over.push(RectInstance::new(edge, c, 0, 0.0, 0.0));
                        }
                    }
                    first = end;
                }
                continue;
            }
            for h in hunks {
                if h.line > lines || out.under.len() >= MAX_MARKERS {
                    continue;
                }
                if h.kind == HunkKind::Removed {
                    let at = h.line.min(lines.saturating_sub(1));
                    let cell = n.line_rects(at..at + 1).pop().unwrap_or_else(|| n.cell_rect(at, 0, 1));
                    if readable && cell.intersects(&seen) {
                        let y = if h.line >= lines { cell.bottom() } else { n.cell_rect(at, 0, 1).y };
                        let tick = Rect::new(cell.x - CARD_PAD * 0.9, y - bar * 0.5, CARD_PAD * 0.8, bar);
                        out.over.push(RectInstance::new(tick, theme::get().git_removed, 0, 0.0, 0.0));
                    }
                    continue;
                }
                let c = if h.kind == HunkKind::Added { theme::get().git_added } else { theme::get().git_modified };
                for mut rect in n.line_rects(h.line..(h.line + h.count).min(lines)) {
                    if !rect.intersects(&seen) {
                        continue;
                    }
                    rect.h = rect.h.max(1.5 / z);
                    out.under.push(RectInstance::new(rect, with_alpha(c, wash), 0, 0.0, 0.0));
                    if readable {
                        let edge = Rect::new(rect.x - CARD_PAD * 0.75, rect.y, bar, rect.h);
                        out.over.push(RectInstance::new(edge, c, 0, 0.0, 0.0));
                    }
                }
            }
        }
    }

    for touch in &page.touched {
        let strength = touch.strength();
        let Some(fi) = repo.file_index(&touch.path) else { continue };
        let Some(n) = layout.file_node.get(fi).filter(|&&n| n != NONE).map(|&n| &layout.nodes[n as usize]) else {
            continue;
        };
        if strength <= 0.0 || !n.rect.intersects(&world) {
            continue;
        }
        let lines = repo.files[fi].line_count() as u32;
        let glow = theme::get().accent;
        out.over.push(RectInstance::new(n.rect.expand(2.0 / z), 0, with_alpha(glow, strength), 3.0, 2.0));
        let rows = page.diffs.get(&touch.path);
        for &(first, count) in touch.lines.iter().take(200) {
            let first = row_of(rows, first);
            for mut rect in n.line_rects(first.min(lines)..(first + count).min(lines)) {
                if rect.intersects(&world) {
                    rect.h = rect.h.max(2.0 / z);
                    out.under.push(RectInstance::new(rect, with_alpha(glow, 0.3 * strength), 0, 0.0, 0.0));
                }
            }
        }
    }

    let mut marked = 0usize;
    for (rel, found) in ext.diagnostics_under(&page.root) {
        let Some(fi) = repo.file_index(rel) else { continue };
        let Some(n) = layout.file_node.get(fi).filter(|&&n| n != NONE).map(|&n| &layout.nodes[n as usize]) else {
            continue;
        };
        if !n.rect.intersects(&world) || marked >= MAX_MARKERS {
            continue;
        }
        marked += 1;
        let worst = found.items.iter().map(|d| d.severity).min().unwrap_or(Severity::Hint);
        let color = severity_color(worst);
        out.over.push(RectInstance::new(n.rect, 0, color, 3.0, 1.5));
        if readable {
            let f = &repo.files[fi];
            let rows = page.diffs.get(rel);
            for d in found.items.iter().take(500) {
                let line = row_of(rows, d.line);
                if line as usize >= f.line_count() || folded_away(rows, d.line) {
                    continue;
                }
                let cols = if d.end_line > d.line {
                    (f.line(line as usize).chars().count() as u32).saturating_sub(d.col)
                } else {
                    d.end_col.saturating_sub(d.col)
                };
                let c = severity_color(d.severity);
                let h = (2.0 * s / z).min(LINE_H * 0.3);
                for cell in n.cell_rects(line, d.col, cols.max(1)).into_iter().filter(|cell| cell.intersects(&world)) {
                    out.over.push(RectInstance::new(cell, with_alpha(c, 0.14), 0, 2.0, 0.0));
                    out.over.push(RectInstance::new(Rect::new(cell.x, cell.bottom() - h, cell.w, h), c, 0, 0.0, 0.0));
                }
            }
        } else {
            let side = 7.0 * s / z;
            let dot = Rect::new(n.rect.right() - side * 0.5, n.rect.y - side * 0.5, side, side);
            out.over.push(RectInstance::new(dot, color, 0, 3.5 * s, 0.0));
        }
    }

    if z * LINE_H >= 6.5 {
        let size = (z * LINE_H * 0.8).round();
        let mut drawn = 0usize;
        for (rel, found) in ext.decorations_under(&page.root) {
            let Some(fi) = repo.file_index(rel) else { continue };
            let Some(n) = layout.file_node.get(fi).filter(|&&n| n != NONE).map(|&n| &layout.nodes[n as usize]) else {
                continue;
            };
            if !n.rect.intersects(&world) {
                continue;
            }
            let f = &repo.files[fi];
            let mut next_col: std::collections::HashMap<u32, u32> = Default::default();
            let mut types: Vec<_> = found.by_type.iter().collect();
            types.sort_by(|a, b| a.0.cmp(b.0));
            let rows = page.diffs.get(rel);
            for (kind, d) in types.into_iter().flat_map(|(kind, items)| items.iter().map(move |d| (kind, d))) {
                let line = row_of(rows, d.line);
                if line as usize >= f.line_count() || drawn >= MAX_DECORATIONS || folded_away(rows, d.line) {
                    continue;
                }
                let len = f.line(line as usize).chars().count() as u32;
                let line_cells = n.cell_rects(line, 0, len.max(1));
                if !line_cells.iter().any(|cell| cell.intersects(&world.expand(400.0 / z))) {
                    continue;
                }
                drawn += 1;
                if d.background & 0xff != 0 {
                    let cells = if d.whole_line || d.end_line > d.line {
                        line_cells
                    } else {
                        n.cell_rects(line, d.col, d.end_col.saturating_sub(d.col).max(1))
                    };
                    for cell in cells {
                        out.under.push(RectInstance::new(cell, d.background, 0, 2.0, 0.0));
                    }
                }
                if !d.text.is_empty() {
                    let col = next_col.get(&d.line).copied().unwrap_or(len + 3);
                    let at = n.cell_rect(line, col, 1);
                    let p = view.to_screen([at.x, at.y]);
                    let label: String = d.text.chars().take(160).collect();
                    next_col.insert(d.line, col + label.chars().count() as u32 + 3);
                    let top = p[1] + (z * LINE_H - size * 1.25) * 0.5;
                    let color = if d.color & 0xff != 0 { d.color } else { theme::get().label_dir };
                    if kind == "versions" {
                        let w = text.measure(&label, size, false);
                        let hit = Rect::new(p[0] - 2.0 * s, top - s, w + 4.0 * s, size * 1.3 + 2.0 * s);
                        if page.version_hover.as_ref().is_some_and(|(r, l)| r == rel && *l == d.line) {
                            out.chips.push(RectInstance::new(
                                Rect::new(p[0], top + size * 1.2, w, s.max(1.0)),
                                color,
                                0,
                                0.0,
                                0.0,
                            ));
                        }
                        out.version_hits.push((hit, rel.to_string(), d.line));
                    }
                    out.labels.push(OwnedLabel {
                        text: label,
                        left: p[0],
                        top,
                        size,
                        color,
                        max_w: 2000.0 * s,
                        bold: false,
                    });
                }
            }
        }
    }

    if z * LINE_H >= LENS_MIN_PX {
        code_lenses(page, view, &world, s, ext, text, &mut out);
    }

    let mut chips: Vec<Chip> = Vec::new();
    let mut seen_bodies = HashSet::new();
    let mut seen_chips = HashSet::new();
    for sel in &page.selections {
        selection_overlay(sel, repo, layout, filters, &world, &mut out, &mut chips, &mut seen_bodies, &mut seen_chips);
    }

    if let Some((file, line, _)) = page.hover_cell {
        if z * LINE_H >= SYMBOL_HOVER_MIN_PX {
            if let Some(sym) = repo.sym_at(file, line) {
                for rect in body_rects(repo, layout, sym) {
                    let tint = theme::get().select;
                    out.over.push(RectInstance::new(rect, with_alpha(tint, 0.05), with_alpha(tint, 0.75), 2.0, 1.0));
                }
            }
        }
    }

    let size = (11.0 * s).round();
    let h = (size * 1.3).ceil() + 5.0 * s;
    let mut placed: Vec<Rect> = Vec::new();
    for chip in chips {
        if out.chips.len() >= MAX_CHIPS {
            break;
        }
        let p = view.to_screen(chip.at);
        if p[0] < area.x - 200.0 || p[0] > area.right() || p[1] < area.y || p[1] > area.bottom() {
            continue;
        }
        let w = text.measure(&chip.text, size, chip.bold) + 12.0 * s;
        let mut rect = Rect::new(p[0] - 2.0 * s, p[1] - h - LINE_H * z * 0.5 + 1.0, w, h);
        if let Some(caption) = captions.iter().find(|caption| caption.intersects(&rect)) {
            let (above, below) = (caption.y - h - 2.0 * s, caption.bottom() + 2.0 * s);
            rect.y = if rect.y - above <= below - rect.y { above } else { below };
        }
        if placed.iter().any(|q| q.intersects(&rect)) {
            continue;
        }
        placed.push(rect);
        out.chips.push(RectInstance::new(
            rect,
            theme::get().chip_fill,
            chip.border,
            4.0 * s,
            if chip.bold { 1.5 } else { 1.0 },
        ));
        out.chip_labels.push(OwnedLabel {
            text: chip.text,
            left: rect.x + 6.0 * s,
            top: rect.y + (h - size * 1.3) * 0.5,
            size,
            color: chip.text_color,
            max_w: f32::INFINITY,
            bold: chip.bold,
        });
        if let Some(sym) = chip.sym {
            out.chip_hits.push((rect, sym));
        }
    }
    let size = (10.5 * s).round();
    let h = (size * 1.3).ceil() + 3.0 * s;
    for mark in &page.dep_marks {
        if out.chips.len() >= MAX_CHIPS {
            break;
        }
        let p = view.to_screen(mark.at);
        if !area.contains(p) {
            continue;
        }
        let label = mark.count.to_string();
        let w = text.measure(&label, size, false) + 10.0 * s;
        let rect = Rect::new(p[0] - w * 0.5, p[1] - h * 0.5, w, h);
        if placed.iter().any(|q| q.intersects(&rect)) {
            continue;
        }
        placed.push(rect);
        out.chips.push(RectInstance::new(rect, theme::get().chip_fill, mark.color, h * 0.5, 1.0));
        out.chip_labels.push(OwnedLabel {
            left: rect.x + 5.0 * s,
            top: rect.y + (h - size * 1.3) * 0.5,
            text: label,
            size,
            color: theme::get().chip_text,
            max_w: f32::INFINITY,
            bold: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenses_find_room_above_their_line() {
        assert_eq!(lens_place(None, "fn main() {", 10, 80), LensAt::Name);
        assert_eq!(lens_place(Some(""), "    fn helper() {", 10, 80), LensAt::Above(4));
        assert_eq!(lens_place(Some("}"), "    fn helper() {", 10, 80), LensAt::Above(4));
        assert_eq!(lens_place(Some("/// Doubles it."), "fn double() {", 10, 80), LensAt::Above(17));
        assert_eq!(lens_place(Some(&"x".repeat(75)), "fn double() {", 10, 80), LensAt::After(16));
    }
}
