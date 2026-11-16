//! Everything the app draws on the canvas besides the code itself: folder
//! and file names, collapse markers, hover and selection highlights, and the
//! symbol relations (curves, name chips, body highlights).

use std::collections::HashSet;

use canvas_core::layout::{NodeKind, DIR_HEADER, FILE_LABEL_H, NONE};
use canvas_core::{Layout, Rect, RelKind, Repo, Site, SymRef, LINE_H};
use canvas_render::text::TextLayer;
use canvas_render::{push_curve, theme, CurveVertex, Label, RectInstance, View};

use crate::page::{Page, PageState, RelFilters, Selection};

const MAX_WORLD_LABELS: usize = 600;
const MAX_CHIPS: usize = 260;
const MAX_BODIES: usize = 400;
const MAX_MARKERS: usize = 3000;
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

fn with_alpha(color: u32, alpha: f32) -> u32 {
    (color & 0xffff_ff00) | (alpha.clamp(0.0, 1.0) * 255.0) as u32
}

fn rel_color(kind: RelKind) -> u32 {
    theme::REL_COLORS[kind.index()]
}

/// Rebuild the relation curves of a page and update each selection's
/// "outside the canvas" count.
pub fn build_curves(page: &mut Page, filters: &RelFilters, scale: f32) -> Vec<CurveVertex> {
    let mut out = Vec::new();
    let PageState::Ready(r) = &page.state else { return out };
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
            text_color: theme::CHIP_TEXT,
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
                text_color: theme::CHIP_TEXT_DIM,
                bold: false,
                sym: Some(other),
            });
        }
    }
}

/// Build the per-frame overlay for the active page.
pub fn build(page: &Page, view: View, area: Rect, s: f32, filters: &RelFilters, text: &mut TextLayer) -> CanvasOverlay {
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
                            color: theme::LABEL_DIR,
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
                                color: theme::UI_TEXT_DIM,
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
                            color: if is_root { theme::UI_TEXT } else { theme::LABEL_DIR },
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
                                color: theme::UI_TEXT_DIM,
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
                        out.labels.push(OwnedLabel {
                            text: n.name.clone(),
                            left,
                            top,
                            size,
                            color: theme::LABEL_FILE,
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
        out.over.push(RectInstance::new(n.rect, 0, theme::HOVER, if n.is_dir() { 6.0 } else { 3.0 }, 1.5));
    }
    if let Some(n) = page.selected.and_then(|i| layout.nodes.get(i)) {
        let radius = if n.is_dir() { 6.0 } else { 3.0 };
        out.over.push(RectInstance::new(n.rect, theme::SELECT_FILL, theme::SELECT, radius, 2.0));
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
                            text_color: theme::CHIP_TEXT,
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
            theme::CHIP_FILL,
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
