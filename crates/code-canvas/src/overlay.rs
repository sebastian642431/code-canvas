//! Everything the app draws on the canvas besides the code itself: folder
//! and file names, collapse markers, hover and selection highlights, and the
//! symbol relations (curves, name chips, body highlights).

use std::collections::HashSet;

use canvas_core::layout::{NodeKind, CARD_PAD, DIR_HEADER, FILE_LABEL_H, NONE};
use canvas_core::{Layout, Rect, RelKind, Repo, Site, SymRef, LINE_H};
use canvas_render::text::TextLayer;
use canvas_render::{push_curve, theme, CurveVertex, Label, RectInstance, View};

use crate::ext::{Extensions, Severity};
use crate::git::{Change, HunkKind};
use crate::page::{DepFilters, Page, PageState, Ready, RelFilters, Selection};

const MAX_WORLD_LABELS: usize = 600;
const MAX_CHIPS: usize = 260;
const MAX_BODIES: usize = 400;
const MAX_MARKERS: usize = 3000;
const MAX_DECORATIONS: usize = 600;
/// Symbols react to the pointer once a line is at least this many pixels tall.
pub const SYMBOL_HOVER_MIN_PX: f32 = 0.6;

/// A label whose text is owned (built per frame).
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
    /// Screen rectangles of clickable chips and the symbol they stand for.
    pub chip_hits: Vec<(Rect, SymRef)>,
    /// Screen rectangles of folder collapse markers and the folder's path.
    pub toggle_hits: Vec<(Rect, String)>,
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

/// Rebuild the relation curves of a page and update each selection's
/// "outside the canvas" count.
/// Most dependency curves drawn for one selection, and for "every edge".
const MAX_DEP_CURVES: usize = 4000;
const MAX_ALL_EDGES: usize = 40_000;

/// Curves for the dependencies of `node` (a file, or everything under a
/// folder): what it imports in one colour, what imports it in another. Each
/// starts at the line of the import. Returns `(drawn, total)`.
fn dep_curves(out: &mut Vec<CurveVertex>, r: &Ready, node: usize, deps: &DepFilters, scale: f32) -> (usize, usize) {
    let Some(n) = r.layout.nodes.get(node) else { return (0, 0) };
    let range = node as u32..n.subtree_end;
    // A file hidden inside a collapsed folder counts as being where that folder is.
    let inside = |f: u32| r.layout.file_anchor.get(f as usize).is_some_and(|a| range.contains(a));
    let anchor = |file: u32, line: u32| r.layout.anchor(&Site { file, line, col: 0, len: 0 }).map(|(p, _)| p);
    let mut files: Vec<u32> = Vec::new();
    match n.kind {
        NodeKind::File(fi) => files.push(fi as u32),
        NodeKind::Dir => {
            // What is drawn under the folder, or all its files when it is collapsed.
            let all = (0..r.repo.files.len() as u32).filter(|f| inside(*f));
            files.extend(all);
        }
    }
    let (mut drawn, mut total) = (0, 0);
    for &f in &files {
        let outgoing = r.deps.outgoing(f).iter().filter(|e| !inside(e.to)).map(|e| (e, true));
        let incoming = r.deps.incoming(f).filter(|e| !inside(e.from)).map(|e| (e, false));
        for (e, out_edge) in outgoing.chain(incoming) {
            if !deps.kinds[e.kind.index()] {
                continue;
            }
            total += 1;
            if drawn >= MAX_DEP_CURVES {
                continue;
            }
            let (Some(a), Some(b)) = (anchor(e.from, e.line), anchor(e.to, 0)) else { continue };
            let color = with_alpha(if out_edge { theme::get().dep_out } else { theme::get().dep_in }, 0.7);
            push_curve(out, a, b, color, 1.25 * scale);
            drawn += 1;
        }
    }
    (drawn, total)
}

pub fn build_curves(page: &mut Page, filters: &RelFilters, deps: &DepFilters, scale: f32) -> Vec<CurveVertex> {
    let mut out = Vec::new();
    let PageState::Ready(r) = &page.state else { return out };
    // Every edge, faint, under everything else (a sample of them in a huge repository).
    if deps.all {
        let step = r.deps.edges.len().div_ceil(MAX_ALL_EDGES).max(1);
        for e in r.deps.edges.iter().step_by(step).filter(|e| deps.kinds[e.kind.index()]) {
            let a = r.layout.anchor(&Site { file: e.from, line: e.line, col: 0, len: 0 });
            let b = r.layout.anchor(&Site { file: e.to, line: 0, col: 0, len: 0 });
            if let (Some((a, _)), Some((b, _))) = (a, b) {
                push_curve(&mut out, a, b, with_alpha(theme::get().dep_out, 0.16), scale);
            }
        }
    }
    page.dep_edges = match (deps.selected, page.selected) {
        (true, Some(node)) => dep_curves(&mut out, r, node, deps, scale),
        _ => (0, 0),
    };
    for sel in &mut page.selections {
        sel.outside = 0;
        let def = r.repo.sym_site(sel.sym);
        let def_start = r.repo.sym_line_start(sel.sym);
        for rel in &sel.rel.items {
            // Ends at a definition attach to the start of its line.
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

/// Body rectangles of `sym`, if its file is on the canvas.
fn body_rects(repo: &Repo, layout: &Layout, sym: SymRef) -> Vec<Rect> {
    let node = layout.file_node[sym.file as usize];
    if node == NONE {
        return Vec::new();
    }
    let s = repo.sym(sym);
    layout.nodes[node as usize].line_rects(s.line..s.body_end)
}

fn site_rect(layout: &Layout, site: &Site) -> Option<Rect> {
    let node = layout.file_node[site.file as usize];
    (node != NONE).then(|| layout.nodes[node as usize].cell_rect(site.line, site.col as u32, site.len as u32))
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
    // The selected symbol itself: body + bright chip.
    let own = RelKind::Definition.index();
    if filters.enabled[own] && filters.bodies[own] && seen_bodies.insert(sel.sym) {
        for rect in body_rects(repo, layout, sel.sym) {
            if rect.intersects(world) {
                let o = filters.body_opacity[own];
                out.under.push(RectInstance::new(
                    rect,
                    with_alpha(0xffffff00, 0.07 * o),
                    with_alpha(0xffffff00, 0.45 * o),
                    2.0,
                    1.0,
                ));
            }
        }
    }
    if let Some((at, _)) = layout.anchor(&repo.sym_line_start(sel.sym)) {
        chips.push(Chip {
            at,
            text: repo.sym_name(sel.sym).to_string(),
            border: 0xffffffff,
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
        // Marker on the usage site.
        if rel.from != rel.to && out.over.len() < MAX_MARKERS {
            if let Some(rect) = site_rect(layout, &rel.from) {
                if rect.intersects(world) {
                    out.over.push(RectInstance::new(
                        rect.expand(1.0),
                        with_alpha(color, 0.16 * opacity),
                        with_alpha(color, 0.9 * opacity),
                        2.0,
                        1.0,
                    ));
                }
            }
        }
        // The symbol at the far end: its body and a chip with its name.
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
            continue; // a plain mention outside any function needs no chip
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

/// Build the per-frame overlay for the active page.
pub fn build(
    page: &Page,
    view: View,
    area: Rect,
    s: f32,
    filters: &RelFilters,
    ext: &Extensions,
    text: &mut TextLayer,
) -> CanvasOverlay {
    let mut out = CanvasOverlay::default();
    let PageState::Ready(r) = &page.state else { return out };
    let (repo, layout) = (&r.repo, &r.layout);
    let z = view.zoom;
    // Only what is inside the canvas area gets labels and chips.
    let mut world = view.world_rect();
    world.x += area.x / z;
    world.w = area.w / z;
    world.y += area.y / z;
    world.h = area.h / z;

    // --- folder and file names ---------------------------------------------
    // Screen rects of placed labels; parents are visited first and win,
    // overlapping descendants are skipped.
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
                // A collapsed folder is one box: name it in the middle.
                let h_px = n.rect.h * z;
                if w_px > 46.0 * s && h_px > 22.0 * s {
                    let size = (h_px * 0.16).clamp(11.0 * s, 20.0 * s).round();
                    let label = format!("{}/  +", n.name);
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
                        if two_lines {
                            let sub = format!("{} files · {} lines", n.files, n.lines);
                            let sub_size = (size * 0.72).max(10.0 * s).round();
                            let sw = text.measure(&sub, sub_size, false).min(w_px - 8.0 * s);
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
                    }
                }
            }
            NodeKind::Dir => {
                let header_px = DIR_HEADER * z;
                if (header_px >= 10.0 || w_px >= 280.0 * s) && w_px > 50.0 * s {
                    let size = (header_px * 0.55).clamp(11.0 * s, 16.0 * s).round();
                    let label = format!("{}/", n.name);
                    let left = tl[0] + 8.0 * s;
                    let is_root = n.parent.is_none();
                    // The root's name sits above its box, so it never fights
                    // with the first folder inside for the same corner.
                    let top = if is_root {
                        tl[1] - size * 1.3 - 5.0 * s
                    } else {
                        (tl[1] + (header_px - size * 1.3) * 0.5).max(tl[1] + 2.0)
                    };
                    let max_w = w_px - 34.0 * s;
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
                        // The root folder cannot be collapsed.
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
            NodeKind::File(_) => {
                let lab_px = FILE_LABEL_H * z;
                if lab_px >= 9.0 && w_px > 30.0 {
                    let size = (lab_px * 0.7).min(14.0 * s).round();
                    let left = tl[0] + 2.0;
                    let top = tl[1] - lab_px + (lab_px - size * 1.3) * 0.5;
                    let tw = (n.name.chars().count() as f32 * size * 0.58).min(w_px);
                    if free(Rect::new(left - 2.0, top, tw + 4.0, size * 1.3), &mut placed) {
                        // A dot marks a file with changes that are not saved yet.
                        let unsaved = page.editors.get(&n.rel_path).is_some_and(|e| e.dirty);
                        out.labels.push(OwnedLabel {
                            text: if unsaved { format!("● {}", n.name) } else { n.name.clone() },
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

    // --- hover and selection of nodes --------------------------------------
    if let Some(n) = page.hover.and_then(|h| layout.nodes.get(h)) {
        out.over.push(RectInstance::new(n.rect, 0, theme::get().canvas_hover, if n.is_dir() { 6.0 } else { 3.0 }, 1.5));
    }
    if let Some(n) = page.selected.and_then(|i| layout.nodes.get(i)) {
        let radius = if n.is_dir() { 6.0 } else { 3.0 };
        out.over.push(RectInstance::new(n.rect, theme::get().select_fill, theme::get().select, radius, 2.0));
    }

    // --- search hits ---------------------------------------------------------
    if let Some(search) = &page.search {
        for hit in search.hits.iter().take(MAX_MARKERS) {
            if let Some(rect) = site_rect(layout, hit) {
                if rect.intersects(&world) {
                    out.over.push(RectInstance::new(rect.expand(1.0), 0xffd84d30, 0xffd84dff, 2.0, 1.0));
                }
            }
        }
    }

    let readable = z * LINE_H >= 2.5;

    // --- the file being edited: its line, selection and caret ------------------
    if let Some(editor) = page.editor() {
        let node =
            repo.file_index(&editor.rel_path).and_then(|fi| layout.file_node.get(fi).copied()).filter(|&n| n != NONE);
        if let Some(n) = node.map(|n| &layout.nodes[n as usize]) {
            let caret = editor.caret;
            let width = repo.file_index(&editor.rel_path).map_or(1, |fi| repo.files[fi].max_cols.max(1));
            out.under.push(RectInstance::new(n.cell_rect(caret.line as u32, 0, width), 0xffffff0a, 0, 0.0, 0.0));
            if let Some((a, b)) = editor.selection() {
                for line in a.line..=b.line {
                    let from = if line == a.line { editor.display_col(a) } else { 0 };
                    let to = if line == b.line {
                        editor.display_col(b)
                    } else {
                        // Up to the end of the line, plus one cell for the line break.
                        editor.display_col(crate::editor::Pos::new(line, usize::MAX)) + 1
                    };
                    let cell = n.cell_rect(line as u32, from as u32, to.saturating_sub(from).max(1) as u32);
                    if cell.intersects(&world) {
                        out.under.push(RectInstance::new(cell, 0x5a96ff59, 0, 0.0, 0.0));
                    }
                }
            }
            let cell = n.cell_rect(caret.line as u32, editor.display_col(caret) as u32, 1);
            let w = (2.0 * s / z).max(0.6);
            out.over.push(RectInstance::new(Rect::new(cell.x - w * 0.5, cell.y, w, cell.h), 0xffffffff, 0, 0.0, 0.0));
        }
    }

    // --- source control: what changed since the last commit -------------------
    // A coloured strip along the card's left side (and a dot from far away);
    // close up, a bar next to each changed line, like an editor's gutter.
    if let Some(git) = &page.git {
        for change in git.changes.iter().take(MAX_MARKERS) {
            let Some(fi) = repo.file_index(&change.path) else { continue };
            let Some(n) = layout.file_node.get(fi).filter(|&&n| n != NONE).map(|&n| &layout.nodes[n as usize]) else {
                continue;
            };
            if !n.rect.intersects(&world) {
                continue;
            }
            let color = match change.change {
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
                continue;
            }
            let lines = repo.files[fi].line_count() as u32;
            let bar = (2.5 * s / z).min(CARD_PAD * 0.6);
            for h in &change.hunks {
                let (c, first, count) = match h.kind {
                    HunkKind::Added => (theme::get().git_added, h.line, h.count),
                    HunkKind::Modified => (theme::get().git_modified, h.line, h.count),
                    HunkKind::Removed => (theme::get().git_removed, h.line, 0),
                };
                if first > lines {
                    continue;
                }
                for line in first..(first + count.max(1)).min(lines.max(first + 1)) {
                    let cell = n.cell_rect(line.min(lines.saturating_sub(1)), 0, 1);
                    if !cell.intersects(&world.expand(CARD_PAD)) {
                        continue;
                    }
                    let rect = if count == 0 {
                        // Lines were removed here: a short mark between two lines.
                        let y = if line >= lines { cell.bottom() } else { cell.y };
                        Rect::new(cell.x - CARD_PAD * 0.9, y - bar * 0.5, CARD_PAD * 0.8, bar)
                    } else {
                        Rect::new(cell.x - CARD_PAD * 0.75, cell.y, bar, cell.h)
                    };
                    out.over.push(RectInstance::new(rect, c, 0, 0.0, 0.0));
                }
            }
        }
    }

    // --- problems reported by extensions -------------------------------------
    // Close up, each one underlines its code. From any distance the file's
    // card gets a coloured edge and a dot, so problems show on the whole map.
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
            for d in found.items.iter().take(500) {
                if d.line as usize >= f.line_count() {
                    continue;
                }
                let cols = if d.end_line > d.line {
                    (f.line(d.line as usize).chars().count() as u32).saturating_sub(d.col)
                } else {
                    d.end_col.saturating_sub(d.col)
                };
                let cell = n.cell_rect(d.line, d.col, cols.max(1));
                if !cell.intersects(&world) {
                    continue;
                }
                let c = severity_color(d.severity);
                let h = (2.0 * s / z).min(LINE_H * 0.3);
                out.over.push(RectInstance::new(cell, with_alpha(c, 0.14), 0, 2.0, 0.0));
                out.over.push(RectInstance::new(Rect::new(cell.x, cell.bottom() - h, cell.w, h), c, 0, 0.0, 0.0));
            }
        } else {
            let side = 7.0 * s / z;
            let dot = Rect::new(n.rect.right() - side * 0.5, n.rect.y - side * 0.5, side, side);
            out.over.push(RectInstance::new(dot, color, 0, 3.5 * s, 0.0));
        }
    }

    // --- what extensions draw on the code (decorations) -----------------------
    // Readable only close up: a wash over the range, and text after the line.
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
            // Several texts on one line (two problems) follow each other.
            let mut next_col: std::collections::HashMap<u32, u32> = Default::default();
            let mut types: Vec<_> = found.by_type.iter().collect();
            types.sort_by(|a, b| a.0.cmp(b.0));
            for d in types.into_iter().flat_map(|(_, items)| items) {
                if d.line as usize >= f.line_count() || drawn >= MAX_DECORATIONS {
                    continue;
                }
                let len = f.line(d.line as usize).chars().count() as u32;
                let line_cell = n.cell_rect(d.line, 0, len.max(1));
                if !line_cell.intersects(&world.expand(400.0 / z)) {
                    continue;
                }
                drawn += 1;
                if d.background & 0xff != 0 {
                    let cell = if d.whole_line || d.end_line > d.line {
                        line_cell
                    } else {
                        n.cell_rect(d.line, d.col, d.end_col.saturating_sub(d.col).max(1))
                    };
                    out.under.push(RectInstance::new(cell, d.background, 0, 2.0, 0.0));
                }
                if !d.text.is_empty() {
                    let col = next_col.get(&d.line).copied().unwrap_or(len + 3);
                    let at = n.cell_rect(d.line, col.min(canvas_core::layout::MAX_COLS), 1);
                    let p = view.to_screen([at.x, at.y]);
                    let text: String = d.text.chars().take(160).collect();
                    next_col.insert(d.line, col + text.chars().count() as u32 + 3);
                    out.labels.push(OwnedLabel {
                        text,
                        left: p[0],
                        top: p[1] + (z * LINE_H - size * 1.25) * 0.5,
                        size,
                        color: if d.color & 0xff != 0 { d.color } else { theme::get().label_dir },
                        max_w: 2000.0 * s,
                        bold: false,
                    });
                }
            }
        }
    }

    // --- symbols -------------------------------------------------------------
    let mut chips: Vec<Chip> = Vec::new();
    let mut seen_bodies = HashSet::new();
    let mut seen_chips = HashSet::new();
    for sel in &page.selections {
        selection_overlay(sel, repo, layout, filters, &world, &mut out, &mut chips, &mut seen_bodies, &mut seen_chips);
    }

    // The symbol under the pointer: outline its body, name it.
    if let Some((file, line, _)) = page.hover_cell {
        if z * LINE_H >= SYMBOL_HOVER_MIN_PX {
            if let Some(sym) = repo.sym_at(file, line) {
                for rect in body_rects(repo, layout, sym) {
                    out.over.push(RectInstance::new(rect, 0x7aa2f70c, 0x7aa2f7c0, 2.0, 1.0));
                }
                let site = repo.sym_site(sym);
                if !seen_chips.contains(&(site.file, site.line)) {
                    if let Some((at, _)) = layout.anchor(&repo.sym_line_start(sym)) {
                        chips.push(Chip {
                            at,
                            text: format!("{} {}", repo.sym(sym).kind.badge(), repo.sym_name(sym)),
                            border: 0x7aa2f7ff,
                            text_color: theme::get().chip_text,
                            bold: false,
                            sym: None,
                        });
                    }
                }
            }
        }
    }

    // Chips: fixed pixel size, anchored at a world point, no overlaps.
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
        // Sit just above the line it names, like a tag.
        let rect = Rect::new(p[0] - 2.0 * s, p[1] - h - LINE_H * z * 0.5 + 1.0, w, h);
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
    out
}
