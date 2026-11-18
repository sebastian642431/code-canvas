use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::geom::Rect;
use crate::refs::Site;
use crate::scan::{Dir, Repo};
use crate::{CHAR_W, LINE_H};

pub const CARD_PAD: f32 = 6.0;
pub const FILE_LABEL_H: f32 = 16.0;
pub const DIR_PAD: f32 = 16.0;
pub const DIR_HEADER: f32 = 24.0;
pub const GAP: f32 = 16.0;
pub const MIN_COLS: u32 = 24;
pub const MAX_COLS: u32 = 160;
pub const ASPECT: f32 = 1.6;
pub const FOLD_LINES: usize = 2048;
pub const FOLD_GAP: f32 = 24.0;
pub const GUTTER_COLS: u32 = 6;
pub const NONE: u32 = u32::MAX;

static LINE_NUMBERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_line_numbers(on: bool) {
    LINE_NUMBERS.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn gutter_of(f: &crate::scan::SourceFile) -> f32 {
    let numbered = f.note.as_ref().is_none_or(|note| note.starts_with("truncated"));
    match numbered && LINE_NUMBERS.load(std::sync::atomic::Ordering::Relaxed) {
        true => GUTTER_COLS as f32 * CHAR_W,
        false => 0.0,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutFilter {
    pub collapsed: HashSet<String>,
    pub hidden: HashSet<String>,
    pub only_files: Option<HashSet<u32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    File(usize),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub name: String,
    pub rel_path: String,
    pub rect: Rect,
    pub depth: u16,
    pub parent: Option<u32>,
    pub subtree_end: u32,
    pub col_stride: f32,
    pub fold: u32,
    pub gutter: f32,
    pub collapsed: bool,
    pub files: u32,
    pub lines: u64,
}

pub fn rows_per_column(fold: usize) -> usize {
    match fold {
        0 => FOLD_LINES,
        rows => rows.min(FOLD_LINES),
    }
}

impl Node {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Dir)
    }

    pub fn code_origin(&self) -> [f32; 2] {
        [self.rect.x + CARD_PAD + self.gutter, self.rect.y + CARD_PAD]
    }

    pub fn shown_cols(&self) -> usize {
        ((self.col_stride - FOLD_GAP - self.gutter) / CHAR_W).round().max(0.0) as usize
    }

    pub fn cell_rect(&self, line: u32, col: u32, cols: u32) -> Rect {
        let o = self.code_origin();
        let fold = line as usize / self.fold as usize;
        let row = line as usize % self.fold as usize;
        Rect::new(
            o[0] + fold as f32 * self.col_stride + col.min(MAX_COLS) as f32 * CHAR_W,
            o[1] + row as f32 * LINE_H,
            cols as f32 * CHAR_W,
            LINE_H,
        )
    }

    pub fn line_rects(&self, lines: Range<u32>) -> Vec<Rect> {
        let mut out = Vec::new();
        if lines.is_empty() {
            return out;
        }
        let o = self.code_origin();
        let width = (self.col_stride - FOLD_GAP - self.gutter).max(CHAR_W);
        let per = self.fold as usize;
        let (first, last) = (lines.start as usize / per, (lines.end as usize - 1) / per);
        for fold in first..=last {
            let a = (lines.start as usize).max(fold * per) - fold * per;
            let b = (lines.end as usize).min((fold + 1) * per) - fold * per;
            out.push(Rect::new(
                o[0] + fold as f32 * self.col_stride,
                o[1] + a as f32 * LINE_H,
                width,
                (b - a) as f32 * LINE_H,
            ));
        }
        out
    }

    pub fn cell_at(&self, p: [f32; 2], line_count: usize) -> Option<(u32, u32)> {
        let o = self.code_origin();
        let (dx, dy) = (p[0] - o[0], p[1] - o[1]);
        if dx < 0.0 || dy < 0.0 || self.col_stride <= 0.0 {
            return None;
        }
        let fold = (dx / self.col_stride) as usize;
        let in_col = dx - fold as f32 * self.col_stride;
        if in_col > self.col_stride - FOLD_GAP - self.gutter {
            return None;
        }
        let row = (dy / LINE_H) as usize;
        if row >= self.fold as usize {
            return None;
        }
        let line = fold * self.fold as usize + row;
        (line < line_count).then_some((line as u32, (in_col / CHAR_W) as u32))
    }

    pub fn visible_lines(&self, view: &Rect, line_count: usize) -> Vec<(usize, Range<usize>)> {
        let mut out = Vec::new();
        let o = self.code_origin();
        let first = ((view.y - o[1]) / LINE_H).floor().max(0.0) as usize;
        let per = self.fold as usize;
        let last = (((view.bottom() - o[1]) / LINE_H).ceil().max(0.0) as usize).min(per);
        if first >= last {
            return out;
        }
        for c in 0..line_count.div_ceil(per).max(1) {
            let x0 = o[0] + c as f32 * self.col_stride;
            let x1 = x0 + self.col_stride - FOLD_GAP - self.gutter;
            if x1 < view.x || x0 > view.right() {
                continue;
            }
            let base = c * per;
            let r = (base + first).min(line_count)..(base + last).min(line_count);
            if !r.is_empty() {
                out.push((c, r));
            }
        }
        out
    }
}

#[derive(Default)]
pub struct Layout {
    pub nodes: Vec<Node>,
    pub bounds: Rect,
    pub file_node: Vec<u32>,
    pub file_anchor: Vec<u32>,
    pub dir_node: HashMap<String, u32>,
}

struct Laid {
    w: f32,
    h: f32,
    kind: NodeKind,
    name: String,
    rel_path: String,
    col_stride: f32,
    collapsed: bool,
    files: u32,
    lines: u64,
    paths: Vec<String>,
    inner_files: Vec<u32>,
    children: Vec<(f32, f32, Laid)>,
}

fn card_size(repo: &Repo, idx: usize) -> (f32, f32, f32) {
    let f = &repo.files[idx];
    let col_w = f.max_cols.clamp(MIN_COLS, MAX_COLS) as f32 * CHAR_W + gutter_of(f);
    let lines = f.line_count().max(1);
    let per = rows_per_column(f.fold);
    let folds = lines.div_ceil(per) as f32;
    let rows = lines.min(per) as f32;
    let w = folds * col_w + (folds - 1.0) * FOLD_GAP;
    (w + CARD_PAD * 2.0, rows * LINE_H + CARD_PAD * 2.0, col_w + FOLD_GAP)
}

fn file_visible(repo: &Repo, fi: usize, filter: &LayoutFilter) -> bool {
    !filter.hidden.contains(&repo.files[fi].rel_path)
        && filter.only_files.as_ref().is_none_or(|only| only.contains(&(fi as u32)))
}

fn collect_inner(dir: &Dir, repo: &Repo, filter: &LayoutFilter, files: &mut Vec<u32>, paths: &mut Vec<String>) {
    for &fi in &dir.files {
        if file_visible(repo, fi, filter) {
            files.push(fi as u32);
        }
    }
    for d in &dir.dirs {
        if !filter.hidden.contains(&d.rel_path) {
            paths.push(d.rel_path.clone());
            collect_inner(d, repo, filter, files, paths);
        }
    }
}

fn lay_dir(dir: &Dir, repo: &Repo, filter: &LayoutFilter, is_root: bool) -> Option<Laid> {
    if !is_root && filter.hidden.contains(&dir.rel_path) {
        return None;
    }

    if !is_root && filter.collapsed.contains(&dir.rel_path) {
        let mut inner_files = Vec::new();
        let mut paths = vec![dir.rel_path.clone()];
        collect_inner(dir, repo, filter, &mut inner_files, &mut paths);
        if inner_files.is_empty() {
            return None;
        }
        let lines: u64 = inner_files.iter().map(|&f| repo.files[f as usize].line_count() as u64).sum();
        let side = ((lines as f32).sqrt() * 6.0).clamp(140.0, 1600.0);
        return Some(Laid {
            w: side * 1.5,
            h: side,
            kind: NodeKind::Dir,
            name: dir.name.clone(),
            rel_path: dir.rel_path.clone(),
            col_stride: 0.0,
            collapsed: true,
            files: inner_files.len() as u32,
            lines,
            paths,
            inner_files,
            children: Vec::new(),
        });
    }

    let mut items: Vec<Laid> = dir.dirs.iter().filter_map(|d| lay_dir(d, repo, filter, false)).collect();
    let own_files: Vec<usize> = dir.files.iter().copied().filter(|&fi| file_visible(repo, fi, filter)).collect();

    if !is_root && own_files.is_empty() && items.len() == 1 {
        let mut only = items.pop().unwrap();
        only.name = format!("{}/{}", dir.name, only.name);
        only.paths.push(dir.rel_path.clone());
        return Some(only);
    }
    if !is_root && own_files.is_empty() && items.is_empty() {
        return None;
    }

    for &fi in &own_files {
        let (w, h, col_stride) = card_size(repo, fi);
        let f = &repo.files[fi];
        items.push(Laid {
            w,
            h: h + FILE_LABEL_H,
            kind: NodeKind::File(fi),
            name: f.name.clone(),
            rel_path: f.rel_path.clone(),
            col_stride,
            collapsed: false,
            files: 1,
            lines: f.line_count() as u64,
            paths: Vec::new(),
            inner_files: Vec::new(),
            children: Vec::new(),
        });
    }

    let files = items.iter().map(|c| c.files).sum();
    let lines = items.iter().map(|c| c.lines).sum();
    let area: f32 = items.iter().map(|c| (c.w + GAP) * (c.h + GAP)).sum();
    let widest = items.iter().map(|c| c.w).fold(0.0, f32::max);
    let target_w = (area * ASPECT).sqrt().max(widest);

    items.sort_by(|a, b| b.h.total_cmp(&a.h));
    let mut sky = Skyline::new(target_w);
    let mut placed = Vec::with_capacity(items.len());
    let (mut content_w, mut content_h) = (0.0f32, 0.0f32);
    for item in items {
        let (ix, iy) = sky.place(item.w + GAP, item.h + GAP);
        content_w = content_w.max(ix + item.w);
        content_h = content_h.max(iy + item.h);
        placed.push((DIR_PAD + ix, DIR_HEADER + iy, item));
    }

    Some(Laid {
        w: content_w.max(120.0) + DIR_PAD * 2.0,
        h: content_h.max(40.0) + DIR_HEADER + DIR_PAD,
        kind: NodeKind::Dir,
        name: dir.name.clone(),
        rel_path: dir.rel_path.clone(),
        col_stride: 0.0,
        collapsed: false,
        files,
        lines,
        paths: vec![dir.rel_path.clone()],
        inner_files: Vec::new(),
        children: placed,
    })
}

struct Skyline {
    width: f32,
    segs: Vec<(f32, f32, f32)>,
}

impl Skyline {
    fn new(width: f32) -> Self {
        Self { width, segs: vec![(0.0, 0.0, width)] }
    }

    fn place(&mut self, w: f32, h: f32) -> (f32, f32) {
        let mut best: Option<(f32, f32)> = None;
        for i in 0..self.segs.len() {
            let x = self.segs[i].0;
            if x + w > self.width + 0.01 && x > 0.0 {
                break;
            }
            let mut y = 0.0f32;
            for s in &self.segs[i..] {
                if s.0 >= x + w {
                    break;
                }
                y = y.max(s.1);
            }
            if best.is_none_or(|(by, bx)| y < by - 0.01 || (y - by).abs() <= 0.01 && x < bx) {
                best = Some((y, x));
            }
        }
        let (y, x) = best.unwrap_or((0.0, 0.0));
        self.raise(x, w, y + h);
        (x, y)
    }

    fn raise(&mut self, x: f32, w: f32, top: f32) {
        let end = x + w;
        let mut out = Vec::with_capacity(self.segs.len() + 2);
        for &(sx, sy, sw) in &self.segs {
            let se = sx + sw;
            if se <= x || sx >= end {
                out.push((sx, sy, sw));
                continue;
            }
            if sx < x {
                out.push((sx, sy, x - sx));
            }
            if se > end {
                out.push((end, sy, se - end));
            }
        }
        out.push((x, top, w));
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32, f32)> = Vec::with_capacity(out.len());
        for s in out {
            if let Some(last) = merged.last_mut() {
                if (last.1 - s.1).abs() < 0.01 && (last.0 + last.2 - s.0).abs() < 0.01 {
                    last.2 += s.2;
                    continue;
                }
            }
            merged.push(s);
        }
        self.segs = merged;
    }
}

fn flatten(l: Laid, x: f32, y: f32, depth: u16, parent: Option<u32>, out: &mut Layout) {
    let me = out.nodes.len() as u32;
    let rect = match l.kind {
        NodeKind::File(fi) => {
            out.file_node[fi] = me;
            out.file_anchor[fi] = me;
            Rect::new(x, y + FILE_LABEL_H, l.w, l.h - FILE_LABEL_H)
        }
        NodeKind::Dir => Rect::new(x, y, l.w, l.h),
    };
    for fi in &l.inner_files {
        out.file_anchor[*fi as usize] = me;
    }
    for p in l.paths {
        out.dir_node.insert(p, me);
    }
    out.nodes.push(Node {
        kind: l.kind,
        name: l.name,
        rel_path: l.rel_path,
        rect,
        depth,
        parent,
        subtree_end: 0,
        col_stride: l.col_stride,
        fold: FOLD_LINES as u32,
        gutter: 0.0,
        collapsed: l.collapsed,
        files: l.files,
        lines: l.lines,
    });
    for (cx, cy, child) in l.children {
        flatten(child, x + cx, y + cy, depth + 1, Some(me), out);
    }
    out.nodes[me as usize].subtree_end = out.nodes.len() as u32;
}

impl Layout {
    pub fn build(repo: &Repo) -> Layout {
        Self::build_filtered(repo, &LayoutFilter::default())
    }

    pub fn build_filtered(repo: &Repo, filter: &LayoutFilter) -> Layout {
        let laid = lay_dir(&repo.tree, repo, filter, true).expect("the root is always laid out");
        let mut out = Layout {
            nodes: Vec::new(),
            bounds: Rect::new(0.0, 0.0, laid.w, laid.h),
            file_node: vec![NONE; repo.files.len()],
            file_anchor: vec![NONE; repo.files.len()],
            dir_node: HashMap::new(),
        };
        flatten(laid, 0.0, 0.0, 0, None, &mut out);
        let Layout { nodes, file_node, .. } = &mut out;
        for (file, &node) in file_node.iter().enumerate().filter(|(_, node)| **node != NONE) {
            nodes[node as usize].fold = rows_per_column(repo.files[file].fold) as u32;
            nodes[node as usize].gutter = gutter_of(&repo.files[file]);
        }
        out
    }

    pub fn node_for_path(&self, repo: &Repo, path: &str) -> Option<usize> {
        if let Some(&n) = self.dir_node.get(path) {
            return Some(n as usize);
        }
        let fi = repo.file_index(path)?;
        let n = self.file_anchor[fi];
        (n != NONE).then_some(n as usize)
    }

    pub fn anchor(&self, site: &Site) -> Option<([f32; 2], bool)> {
        let n = *self.file_anchor.get(site.file as usize)?;
        if n == NONE {
            return None;
        }
        let node = &self.nodes[n as usize];
        if node.is_dir() {
            let h = (site.file as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (site.line as u64 + 1).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            let fx = 0.12 + 0.76 * ((h >> 16) & 0xffff) as f32 / 65535.0;
            let fy = 0.25 + 0.65 * ((h >> 40) & 0xffff) as f32 / 65535.0;
            return Some(([node.rect.x + node.rect.w * fx, node.rect.y + node.rect.h * fy], false));
        }
        let r = node.cell_rect(site.line, site.col as u32, site.len as u32);
        Some(([r.x, r.y + r.h * 0.5], true))
    }

    pub fn hit(&self, p: [f32; 2]) -> Option<usize> {
        let root = self.nodes.first()?;
        if !root.rect.contains(p) {
            return None;
        }
        let mut best = 0usize;
        let mut i = 1usize;
        let mut end = root.subtree_end as usize;
        while i < end {
            let n = &self.nodes[i];
            let mut r = n.rect;
            if !n.is_dir() {
                r.y -= FILE_LABEL_H;
                r.h += FILE_LABEL_H;
            }
            if r.contains(p) {
                best = i;
                end = n.subtree_end as usize;
                i += 1;
            } else {
                i = n.subtree_end as usize;
            }
        }
        Some(best)
    }

    pub fn visit_visible(&self, view: &Rect, mut f: impl FnMut(usize, &Node) -> bool) {
        let mut i = 0usize;
        while i < self.nodes.len() {
            let n = &self.nodes[i];
            let mut r = n.rect;
            if !n.is_dir() {
                r.y -= FILE_LABEL_H;
                r.h += FILE_LABEL_H;
            }
            if r.intersects(view) && f(i, n) {
                i += 1;
            } else {
                i = n.subtree_end as usize;
            }
        }
    }

    pub fn groups(&self, min_size: f32, keep: Option<usize>) -> Vec<u32> {
        let mut open = vec![false; self.nodes.len()];
        if let Some(open_node) = open.first_mut() {
            *open_node = true;
        }
        let mut at = keep.and_then(|k| self.nodes.get(k)).and_then(|n| n.parent);
        while let Some(p) = at {
            open[p as usize] = true;
            at = self.nodes[p as usize].parent;
        }
        let mut group: Vec<u32> = (0..self.nodes.len() as u32).collect();
        let mut i = 0;
        while i < self.nodes.len() {
            let n = &self.nodes[i];
            let end = n.subtree_end as usize;
            if n.is_dir() && !open[i] && n.rect.w.max(n.rect.h) < min_size {
                group[i..end].fill(i as u32);
                i = end;
            } else {
                i += 1;
            }
        }
        group
    }
}

pub const REST_MIN_COLS: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineCut {
    pub origin: [f32; 2],
    pub char_w: f32,
    pub line_h: f32,
    pub shown: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestRow {
    pub from: usize,
    pub to: usize,
    pub rect: Rect,
    pub text_left: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RestOfLine {
    pub rows: Vec<RestRow>,
    pub frame: Rect,
    pub beside: bool,
    pub char_w: f32,
}

impl RestOfLine {
    pub fn place_of(&self, col: usize) -> Option<Rect> {
        let last = self.rows.last()?;
        let row = match self.rows.iter().find(|r| (r.from..r.to).contains(&col)) {
            Some(row) => row,
            None if col == last.to => last,
            None => return None,
        };
        let x = row.text_left + col as f32 * self.char_w;
        Some(Rect::new(x, row.rect.y, self.char_w, row.rect.h))
    }

    pub fn col_at(&self, p: [f32; 2]) -> Option<usize> {
        if !self.frame.contains(p) {
            return None;
        }
        let last = self.rows.last()?;
        let row = self.rows.iter().find(|r| p[1] < r.rect.bottom()).unwrap_or(last);
        let col = ((p[0] - row.text_left) / self.char_w).round().max(0.0) as usize;
        Some(col.clamp(row.from, row.to))
    }
}

pub fn rest_of_line(cut: &LineCut, pane: Rect, pad: f32) -> Option<RestOfLine> {
    let rest = cut.len.saturating_sub(cut.shown);
    let in_view = cut.origin[1] >= pane.y && cut.origin[1] + cut.line_h <= pane.bottom();
    if rest == 0 || cut.char_w <= 0.0 || cut.line_h <= 0.0 || !in_view {
        return None;
    }
    let (cw, lh) = (cut.char_w, cut.line_h);
    let cols = |room: f32| (room / cw).floor().max(0.0) as usize;
    let rows_in = |room: f32| (room / lh).floor().max(0.0) as usize;
    let edge = cut.origin[0] + cut.shown as f32 * cw;
    let beside = edge >= pane.x && cols(pane.right() - pad - edge) >= rest.min(REST_MIN_COLS);
    let (per_row, x) = if beside {
        (rest.min(cols(pane.right() - pad - edge)), edge)
    } else {
        let per_row = rest.min(cols(pane.w - 2.0 * pad));
        (per_row, pane.right() - pad - per_row as f32 * cw)
    };
    if per_row == 0 {
        return None;
    }
    let wanted = rest.div_ceil(per_row);
    let (top, count) = if beside {
        (cut.origin[1], wanted.min(rows_in(pane.bottom() - pad - cut.origin[1])))
    } else {
        let below = rows_in(pane.bottom() - pad - cut.origin[1] - lh);
        let above = rows_in(cut.origin[1] - pad - pane.y);
        match wanted > below && above > below {
            true => (cut.origin[1] - wanted.min(above) as f32 * lh, wanted.min(above)),
            false => (cut.origin[1] + lh, wanted.min(below)),
        }
    };
    if count == 0 {
        return None;
    }
    let rows: Vec<RestRow> = (0..count)
        .map(|k| {
            let from = cut.shown + k * per_row;
            let to = (from + per_row).min(cut.len);
            RestRow {
                from,
                to,
                rect: Rect::new(x, top + k as f32 * lh, (to - from) as f32 * cw, lh),
                text_left: x - from as f32 * cw,
            }
        })
        .collect();
    let body = Rect::new(x, top, rows[0].rect.w, count as f32 * lh);
    let frame = match beside {
        true => Rect::new(x, top - pad, body.w + pad, body.h + 2.0 * pad),
        false => body.expand(pad),
    };
    Some(RestOfLine { rows, frame, beside, char_w: cw })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{scan, ScanOptions};
    use std::path::Path;

    fn temp_repo(tag: &str, files: &[(&str, String)]) -> Repo {
        let dir = std::env::temp_dir().join(format!("code-canvas-layout-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let repo = scan(&dir, &ScanOptions::default(), None);
        std::fs::remove_dir_all(&dir).ok();
        repo
    }

    #[test]
    fn layout_is_nested_and_preorder() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo = scan(root, &ScanOptions::default(), None);
        let layout = Layout::build(&repo);
        assert_eq!(layout.nodes[0].subtree_end as usize, layout.nodes.len());
        for (i, n) in layout.nodes.iter().enumerate() {
            assert!(n.subtree_end as usize > i);
            if let Some(p) = n.parent {
                let parent = &layout.nodes[p as usize];
                assert!(parent.rect.x <= n.rect.x && parent.rect.right() >= n.rect.right() - 0.01);
                assert!(parent.rect.y <= n.rect.y && parent.rect.bottom() >= n.rect.bottom() - 0.01);
                assert!(i < parent.subtree_end as usize);
            }
        }
        for (fi, &ni) in layout.file_node.iter().enumerate() {
            let n = &layout.nodes[ni as usize];
            let lines = repo.files[fi].line_count();
            let cols = lines.div_ceil(FOLD_LINES).max(1);
            let last_x = n.code_origin()[0] + (cols - 1) as f32 * n.col_stride;
            assert!(last_x < n.rect.right());
            let all: usize = n.visible_lines(&n.rect.expand(1.0), lines).iter().map(|(_, r)| r.len()).sum();
            assert_eq!(all, lines);
        }
        for (fi, &ni) in layout.file_node.iter().enumerate() {
            let n = &layout.nodes[ni as usize];
            assert_eq!(n.kind, NodeKind::File(fi));
            assert_eq!(layout.hit(n.rect.center()), Some(ni as usize));
        }
        assert_eq!(layout.nodes[0].files as usize, repo.files.len());
    }

    #[test]
    fn long_files_fold_into_columns() {
        let text: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        let repo = temp_repo("fold", &[("big.txt", text)]);
        let layout = Layout::build(&repo);

        let n = &layout.nodes[layout.file_node[0] as usize];
        assert!(n.rect.h < (FOLD_LINES as f32 + 2.0) * LINE_H);
        assert!(n.rect.w > 2.0 * n.col_stride);
        let x = n.code_origin()[0] + 2.0 * n.col_stride;
        let view = Rect::new(x, n.rect.y, 10.0, n.rect.h);
        assert_eq!(n.visible_lines(&view, 5000), vec![(2, 4096..5000)]);
        for (line, col) in [(0u32, 0u32), (17, 3), (2048, 0), (4999, 5)] {
            let r = n.cell_rect(line, col, 1);
            assert_eq!(n.cell_at([r.x + 1.0, r.y + 1.0], 5000), Some((line, col)));
        }
        assert_eq!(n.line_rects(2040..2060).len(), 2);
    }

    #[test]
    fn chains_merge_and_folders_collapse_hide_and_filter() {
        let repo = temp_repo(
            "filter",
            &[
                ("a/b/c/deep.rs", "fn deep() {}\n".into()),
                ("a/b/c/other.rs", "fn other() {}\n".into()),
                ("src/main.rs", "fn main() {}\n".into()),
                ("src/util/x.rs", "fn x() {}\n".into()),
                ("README.md", "# hi\n".into()),
            ],
        );
        let file = |name: &str| repo.files.iter().position(|f| f.rel_path == name).unwrap();

        let layout = Layout::build(&repo);
        let chain = layout.nodes.iter().find(|n| n.name == "a/b/c").expect("merged chain");
        assert_eq!(chain.rel_path, "a/b/c");
        assert_eq!(chain.files, 2);
        for p in ["a", "a/b", "a/b/c"] {
            assert_eq!(layout.nodes[layout.dir_node[p] as usize].name, "a/b/c");
        }

        let mut filter = LayoutFilter::default();
        filter.collapsed.insert("src".into());
        let layout = Layout::build_filtered(&repo, &filter);
        let src = layout.dir_node["src"] as usize;
        assert!(layout.nodes[src].collapsed);
        assert_eq!(layout.nodes[src].subtree_end as usize, src + 1);
        assert_eq!(layout.file_node[file("src/main.rs")], NONE);
        assert_eq!(layout.file_anchor[file("src/util/x.rs")] as usize, src);
        assert_eq!(layout.dir_node["src/util"] as usize, src);
        let site = Site { file: file("src/main.rs") as u32, line: 0, col: 3, len: 4 };
        let (p, exact) = layout.anchor(&site).unwrap();
        assert!(!exact && layout.nodes[src].rect.contains(p));

        let mut filter = LayoutFilter::default();
        filter.hidden.insert("a".into());
        filter.hidden.insert("README.md".into());
        let layout = Layout::build_filtered(&repo, &filter);
        assert!(!layout.dir_node.contains_key("a/b/c"));
        assert_eq!(layout.file_anchor[file("a/b/c/deep.rs")], NONE);
        assert_eq!(layout.file_anchor[file("README.md")], NONE);
        assert_ne!(layout.file_node[file("src/main.rs")], NONE);

        let filter = LayoutFilter {
            only_files: Some([file("src/util/x.rs") as u32].into_iter().collect()),
            ..Default::default()
        };
        let layout = Layout::build_filtered(&repo, &filter);
        assert_eq!(layout.nodes.iter().filter(|n| !n.is_dir()).count(), 1);
        assert!(layout.nodes.iter().any(|n| n.name == "src/util"));
        assert!(!layout.dir_node.contains_key("a"));
    }

    fn cut(x: f32, y: f32, len: usize) -> LineCut {
        LineCut { origin: [x, y], char_w: 6.0, line_h: 12.0, shown: 160, len }
    }

    #[test]
    fn a_long_line_card_shows_its_whole_width() {
        let repo = temp_repo("cut", &[("wide.txt", format!("{}\nshort\n", "x".repeat(400)))]);
        let layout = Layout::build(&repo);
        let n = &layout.nodes[layout.file_node[0] as usize];
        assert_eq!(n.shown_cols(), MAX_COLS as usize);
        let narrow = temp_repo("narrow", &[("a.txt", "abc\n".into())]);
        let layout = Layout::build(&narrow);
        assert_eq!(layout.nodes[layout.file_node[0] as usize].shown_cols(), MIN_COLS as usize);
    }

    #[test]
    fn the_rest_of_a_line_goes_on_beside_the_card() {
        let pane = Rect::new(0.0, 0.0, 2000.0, 1000.0);
        assert_eq!(rest_of_line(&cut(10.0, 100.0, 160), pane, 4.0), None);
        assert_eq!(rest_of_line(&cut(10.0, 100.0, 100), pane, 4.0), None);
        assert_eq!(rest_of_line(&cut(10.0, -5.0, 400), pane, 4.0), None);
        assert_eq!(rest_of_line(&cut(10.0, 995.0, 400), pane, 4.0), None);

        let rest = rest_of_line(&cut(10.0, 100.0, 200), pane, 4.0).unwrap();
        assert!(rest.beside);
        assert_eq!(rest.rows.len(), 1);
        let row = rest.rows[0];
        assert_eq!((row.from, row.to), (160, 200));
        assert_eq!(row.text_left, 10.0);
        assert_eq!(row.rect, Rect::new(10.0 + 960.0, 100.0, 240.0, 12.0));
        assert_eq!(rest.frame, Rect::new(970.0, 96.0, 244.0, 20.0));
    }

    #[test]
    fn a_rest_wider_than_the_room_wraps_under_its_first_row() {
        let pane = Rect::new(0.0, 0.0, 1574.0, 1000.0);
        let rest = rest_of_line(&cut(10.0, 100.0, 5000), pane, 4.0).unwrap();
        assert!(rest.beside);
        assert_eq!(rest.rows[0].rect.x, 970.0);
        let per_row = 100;
        assert_eq!(rest.rows.len(), (5000 - 160usize).div_ceil(per_row).min(74));
        for (k, row) in rest.rows.iter().enumerate() {
            assert_eq!(row.from, 160 + k * per_row);
            assert_eq!(row.rect.x, 970.0);
            assert_eq!(row.rect.y, 100.0 + k as f32 * 12.0);
            assert_eq!(row.text_left + row.from as f32 * 6.0, 970.0);
            assert!(row.rect.bottom() <= pane.bottom() - 4.0);
        }
        assert_eq!(rest.rows.last().unwrap().to, 5000);
        assert_eq!(rest.frame.w, 604.0);

        let low = rest_of_line(&cut(10.0, 900.0, 5000), pane, 4.0).unwrap();
        assert_eq!(low.rows.len(), 8);
        assert_eq!(low.rows.last().unwrap().to, 160 + 8 * per_row);
    }

    #[test]
    fn with_no_room_beside_the_rest_sits_under_the_line_against_the_right_edge() {
        let pane = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let rest = rest_of_line(&cut(0.0, 100.0, 300), pane, 4.0).unwrap();
        assert!(!rest.beside);
        assert_eq!(rest.rows.len(), 1);
        assert_eq!(rest.rows[0].rect, Rect::new(1000.0 - 4.0 - 140.0 * 6.0, 112.0, 840.0, 12.0));
        assert_eq!(rest.rows[0].text_left + 160.0 * 6.0, rest.rows[0].rect.x);

        let near_bottom = rest_of_line(&cut(0.0, 950.0, 3000), pane, 4.0).unwrap();
        assert!(near_bottom.rows[0].rect.bottom() <= 950.0);
        assert!(near_bottom.rows.len() > 2);
        assert_eq!(near_bottom.rows.last().unwrap().rect.bottom(), 950.0);
    }

    #[test]
    fn a_column_of_the_rest_has_its_place_in_the_rows() {
        let pane = Rect::new(0.0, 0.0, 1574.0, 1000.0);
        let rest = rest_of_line(&cut(10.0, 100.0, 450), pane, 4.0).unwrap();
        assert_eq!(rest.place_of(100), None);
        assert_eq!(rest.place_of(160), Some(Rect::new(970.0, 100.0, 6.0, 12.0)));
        assert_eq!(rest.place_of(265), Some(Rect::new(970.0 + 5.0 * 6.0, 112.0, 6.0, 12.0)));
        assert_eq!(rest.place_of(450), Some(Rect::new(970.0 + 90.0 * 6.0, 124.0, 6.0, 12.0)));
        assert_eq!(rest.place_of(451), None);

        assert_eq!(rest.col_at([2.0, 100.0]), None);
        assert_eq!(rest.col_at([970.0 + 10.0 * 6.0 + 1.0, 105.0]), Some(170));
        assert_eq!(rest.col_at([970.0 + 10.0 * 6.0 + 4.0, 113.0]), Some(271));
        assert_eq!(rest.col_at([1560.0, 130.0]), Some(450));
        assert_eq!(rest.col_at([971.0, 97.0]), Some(160));
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use crate::scan::{scan, ScanOptions};

    #[test]
    fn folders_too_small_to_see_into_are_one_group() {
        let dir = std::env::temp_dir().join(format!("code-canvas-groups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for path in ["a/b/x.rs", "a/b/w.rs", "a/c/y.rs", "d/z.rs", "top.rs"] {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "fn f() {}\n").unwrap();
        }
        let repo = scan(&dir, &ScanOptions::default(), None);
        std::fs::remove_dir_all(&dir).ok();
        let layout = Layout::build(&repo);
        let node = |path: &str| layout.node_for_path(&repo, path).unwrap_or_else(|| panic!("no node {path}")) as u32;
        let of = |groups: &[u32], path: &str| groups[node(path) as usize];

        let none = layout.groups(0.0, None);
        assert!(none.iter().enumerate().all(|(i, g)| *g as usize == i), "big enough: every node is its own");

        let all = layout.groups(f32::INFINITY, None);
        assert_eq!(all[0], 0, "the root never folds into one point");
        assert_eq!(of(&all, "a/b/x.rs"), node("a"));
        assert_eq!(of(&all, "a/c/y.rs"), node("a"));
        assert_eq!(of(&all, "d/z.rs"), node("d"));
        assert_eq!(of(&all, "top.rs"), node("top.rs"), "a file of an open folder stays itself");

        let open = layout.groups(f32::INFINITY, Some(node("a/b/x.rs") as usize));
        assert_eq!(of(&open, "a/b/x.rs"), node("a/b/x.rs"), "the selected file and its folders open up");
        assert_eq!(of(&open, "a/b/w.rs"), node("a/b/w.rs"));
        assert_eq!(of(&open, "a/c/y.rs"), node("a/c"));
        assert_eq!(of(&open, "d/z.rs"), node("d"));

        let small = layout.nodes[node("a/c") as usize].rect;
        let between = layout.groups(small.w.max(small.h) + 1.0, None);
        assert_eq!(of(&between, "a/c/y.rs"), node("a/c"), "only the folders smaller than the size fold");
        assert_ne!(of(&between, "a/b/x.rs"), node("a"));
    }
}
