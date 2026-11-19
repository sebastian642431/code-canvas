use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::geom::Rect;
use crate::order::Order;
use crate::refs::Site;
use crate::scan::{Dir, Repo, SourceFile};
use crate::wrap::Wrap;
use crate::{CHAR_W, LINE_H};

pub const CARD_PAD: f32 = 6.0;
pub const FILE_LABEL_H: f32 = 16.0;
pub const DIR_PAD: f32 = 16.0;
pub const DIR_HEADER: f32 = 24.0;
pub const GAP: f32 = 16.0;
pub const MIN_COLS: u32 = 24;
pub const MAX_COLS: u32 = 160;
pub const ASPECT: f32 = 1.6;
pub const ONE_COLUMN: u32 = u32::MAX;
pub const FOLD_GAP: f32 = 24.0;
pub const GUTTER_COLS: u32 = 6;
pub const NONE: u32 = u32::MAX;

static LINE_NUMBERS: AtomicBool = AtomicBool::new(true);
static WRAP: AtomicBool = AtomicBool::new(false);

pub fn set_line_numbers(on: bool) {
    LINE_NUMBERS.store(on, Ordering::Relaxed);
}

pub fn set_wrap(on: bool) {
    WRAP.store(on, Ordering::Relaxed);
}

fn is_text(f: &SourceFile) -> bool {
    f.note.as_ref().is_none_or(|note| note.starts_with("truncated"))
}

fn gutter_of(f: &SourceFile) -> f32 {
    match is_text(f) && LINE_NUMBERS.load(Ordering::Relaxed) {
        true => GUTTER_COLS as f32 * CHAR_W,
        false => 0.0,
    }
}

pub fn wrap_of(f: &SourceFile) -> Option<Arc<Wrap>> {
    wrap_if(f, WRAP.load(Ordering::Relaxed))
}

fn wrap_if(f: &SourceFile, on: bool) -> Option<Arc<Wrap>> {
    if !on || !is_text(f) || f.fold != 0 || f.max_cols <= MAX_COLS {
        return None;
    }
    f.peek(|text, starts| Wrap::of(text, starts, MAX_COLS)).map(Arc::new)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutFilter {
    pub collapsed: HashSet<String>,
    pub hidden: HashSet<String>,
    pub only_files: Option<HashSet<u32>>,
    pub order: Option<Arc<Order>>,
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
    pub wrap: Option<Arc<Wrap>>,
    pub collapsed: bool,
    pub files: u32,
    pub lines: u64,
    pub offset: [f32; 2],
    pub slot: [f32; 2],
}

fn rows_per_column(f: &SourceFile) -> u32 {
    match f.fold {
        0 => ONE_COLUMN,
        rows => rows.min(ONE_COLUMN as usize) as u32,
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

    fn row_rect(&self, row: u32, col: u32, cols: u32) -> Rect {
        let o = self.code_origin();
        let per = self.fold.max(1) as usize;
        let (fold, row) = (row as usize / per, row as usize % per);
        Rect::new(
            o[0] + fold as f32 * self.col_stride + col.min(MAX_COLS) as f32 * CHAR_W,
            o[1] + row as f32 * LINE_H,
            cols as f32 * CHAR_W,
            LINE_H,
        )
    }

    pub fn cell_rect(&self, line: u32, col: u32, cols: u32) -> Rect {
        let (row, col) = match &self.wrap {
            Some(wrap) => wrap.place(line as usize, col),
            None => (line, col),
        };
        self.row_rect(row, col, cols)
    }

    pub fn cell_rects(&self, line: u32, col: u32, cols: u32) -> Vec<Rect> {
        let pieces: Vec<_> = match &self.wrap {
            Some(wrap) => wrap.pieces(line as usize, col, cols).collect(),
            None => Vec::new(),
        };
        match pieces.is_empty() {
            true => vec![self.cell_rect(line, col, cols)],
            false => pieces.into_iter().map(|(row, col, cols)| self.row_rect(row, col, cols)).collect(),
        }
    }

    pub fn first_row(&self, line: usize) -> usize {
        self.wrap.as_ref().map_or(line, |wrap| wrap.row(line) as usize)
    }

    fn per(&self) -> usize {
        self.fold.max(1) as usize
    }

    pub fn columns(&self, line_count: usize) -> usize {
        match &self.wrap {
            Some(_) => 1,
            None => line_count.div_ceil(self.per()).max(1),
        }
    }

    pub fn column_rows(&self, col: usize, line_count: usize) -> usize {
        match &self.wrap {
            Some(wrap) => wrap.all_rows() as usize,
            None => line_count.saturating_sub(col.saturating_mul(self.per())).min(self.per()),
        }
    }

    pub fn line_rects(&self, lines: Range<u32>) -> Vec<Rect> {
        let mut out = Vec::new();
        if lines.is_empty() {
            return out;
        }
        let o = self.code_origin();
        let width = (self.col_stride - FOLD_GAP - self.gutter).max(CHAR_W);
        if let Some(wrap) = &self.wrap {
            let (a, b) = (lines.start as usize, (lines.end as usize).min(wrap.lines()));
            if a < b {
                let (ra, rb) = (wrap.row(a), wrap.row(b - 1) + wrap.rows(b - 1));
                out.push(Rect::new(o[0], o[1] + ra as f32 * LINE_H, width, (rb - ra) as f32 * LINE_H));
            }
            return out;
        }
        let per = self.per();
        let (first, last) = (lines.start as usize / per, (lines.end as usize - 1) / per);
        for fold in first..=last {
            let a = (lines.start as usize).max(fold * per) - fold * per;
            let b = (lines.end as usize).min((fold + 1).saturating_mul(per)) - fold * per;
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
        if fold >= self.columns(line_count) || in_col > self.col_stride - FOLD_GAP - self.gutter {
            return None;
        }
        let row = (dy / LINE_H) as usize;
        let col = (in_col / CHAR_W) as u32;
        if let Some(wrap) = &self.wrap {
            let (line, sub) = wrap.line_at(u32::try_from(row).ok()?)?;
            return (line < line_count).then(|| (line as u32, wrap.col_at(line, sub, col)));
        }
        let per = self.per();
        let line = fold * per + row;
        (row < per && line < line_count).then_some((line as u32, col))
    }

    pub fn visible_lines(&self, view: &Rect, line_count: usize) -> Vec<(usize, Range<usize>)> {
        let mut out = Vec::new();
        let o = self.code_origin();
        let first = ((view.y - o[1]) / LINE_H).floor().max(0.0) as usize;
        let last = ((view.bottom() - o[1]) / LINE_H).ceil().max(0.0) as usize;
        let per = match &self.wrap {
            Some(wrap) => wrap.all_rows() as usize,
            None => self.per(),
        };
        let last = last.min(per);
        if first >= last {
            return out;
        }
        for c in 0..self.columns(line_count) {
            let x0 = o[0] + c as f32 * self.col_stride;
            let x1 = x0 + self.col_stride - FOLD_GAP - self.gutter;
            if x1 < view.x || x0 > view.right() {
                continue;
            }
            let r = match &self.wrap {
                Some(wrap) => {
                    let r = wrap.lines_in(first as u32..last as u32);
                    r.start.min(line_count)..r.end.min(line_count)
                }
                None => (c * per + first).min(line_count)..(c * per + last).min(line_count),
            };
            if !r.is_empty() {
                out.push((c, r));
            }
        }
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub nodes: Vec<Node>,
    pub bounds: Rect,
    pub file_node: Vec<u32>,
    pub file_anchor: Vec<u32>,
    pub dir_node: HashMap<String, u32>,
    pub order: Option<Arc<Order>>,
}

struct Laid {
    w: f32,
    h: f32,
    kind: NodeKind,
    name: String,
    rel_path: String,
    col_stride: f32,
    wrap: Option<Arc<Wrap>>,
    collapsed: bool,
    files: u32,
    lines: u64,
    paths: Vec<String>,
    inner_files: Vec<u32>,
    children: Vec<(f32, f32, Laid)>,
    kept: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
enum Shift {
    Insert(usize),
    Remove(usize),
}

impl Shift {
    fn new_of(self, old: usize) -> Option<usize> {
        match self {
            Shift::Insert(k) => Some(old + (old >= k) as usize),
            Shift::Remove(k) if old == k => None,
            Shift::Remove(k) => Some(old - (old > k) as usize),
        }
    }

    fn old_of(self, new: usize) -> Option<usize> {
        match self {
            Shift::Insert(k) if new == k => None,
            Shift::Insert(k) => Some(new - (new > k) as usize),
            Shift::Remove(k) => Some(new + (new >= k) as usize),
        }
    }
}

struct Relay<'a> {
    old: &'a Layout,
    on_path: HashSet<String>,
    shift: Shift,
}

fn ancestors_of(rel_path: &str) -> HashSet<String> {
    let mut out: HashSet<String> = [String::new()].into();
    let mut at = 0;
    while let Some(slash) = rel_path[at..].find('/') {
        out.insert(rel_path[..at + slash].to_string());
        at += slash + 1;
    }
    out
}

fn card_laid(repo: &Repo, fi: usize) -> Laid {
    let f = &repo.files[fi];
    let wrap = wrap_of(f);
    let (w, h, col_stride) = card_size(repo, fi, wrap.as_deref());
    Laid {
        w,
        h: h + FILE_LABEL_H,
        kind: NodeKind::File(fi),
        name: f.name.clone(),
        rel_path: f.rel_path.clone(),
        col_stride,
        wrap,
        collapsed: false,
        files: 1,
        lines: f.line_count() as u64,
        paths: Vec::new(),
        inner_files: Vec::new(),
        children: Vec::new(),
        kept: None,
    }
}

fn kept_laid(old: &Layout, n: u32, kind: NodeKind, name: String) -> Laid {
    let node = &old.nodes[n as usize];
    Laid {
        w: node.slot[0],
        h: node.slot[1],
        kind,
        name,
        rel_path: String::new(),
        col_stride: 0.0,
        wrap: None,
        collapsed: node.collapsed,
        files: node.files,
        lines: node.lines,
        paths: Vec::new(),
        inner_files: Vec::new(),
        children: Vec::new(),
        kept: Some(n),
    }
}

fn collapsed_size(repo: &Repo, inner_files: &[u32]) -> (f32, f32, u64) {
    let lines: u64 = inner_files.iter().map(|&f| repo.files[f as usize].line_count() as u64).sum();
    let side = ((lines as f32).sqrt() * 6.0).clamp(140.0, 1600.0);
    (side * 1.5, side, lines)
}

fn card_size(repo: &Repo, idx: usize, wrap: Option<&Wrap>) -> (f32, f32, f32) {
    let f = &repo.files[idx];
    let col_w = f.max_cols.clamp(MIN_COLS, MAX_COLS) as f32 * CHAR_W + gutter_of(f);
    let lines = f.line_count().max(1);
    let per = rows_per_column(f) as usize;
    let folds = lines.div_ceil(per) as f32;
    let rows = wrap.map_or(lines.min(per), |wrap| wrap.all_rows() as usize) as f32;
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

fn lay_dir(dir: &Dir, repo: &Repo, filter: &LayoutFilter, is_root: bool, relay: Option<&Relay>) -> Option<Laid> {
    if let Some(relay) = relay.filter(|relay| !is_root && !relay.on_path.contains(&dir.rel_path)) {
        let &n = relay.old.dir_node.get(&dir.rel_path)?;
        let deepest = &relay.old.nodes[n as usize].rel_path;
        let above = dir.rel_path.len() - dir.name.len();
        return Some(kept_laid(relay.old, n, NodeKind::Dir, deepest[above..].to_string()));
    }
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
        let (w, h, lines) = collapsed_size(repo, &inner_files);
        return Some(Laid {
            w,
            h,
            kind: NodeKind::Dir,
            name: dir.name.clone(),
            rel_path: dir.rel_path.clone(),
            col_stride: 0.0,
            wrap: None,
            collapsed: true,
            files: inner_files.len() as u32,
            lines,
            paths,
            inner_files,
            children: Vec::new(),
            kept: None,
        });
    }

    let arranged = filter.order.as_deref();
    let mut ranks: Vec<u32> = Vec::new();
    let mut items: Vec<Laid> = Vec::new();
    for d in &dir.dirs {
        if let Some(laid) = lay_dir(d, repo, filter, false, relay) {
            items.push(laid);
            ranks.push(arranged.map_or(0, |o| o.dir_rank(&d.rel_path)));
        }
    }
    let own_files: Vec<usize> = dir.files.iter().copied().filter(|&fi| file_visible(repo, fi, filter)).collect();
    ranks.extend(own_files.iter().map(|&fi| arranged.map_or(0, |o| o.file_rank(fi))));

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
        let kept = relay.and_then(|relay| {
            let old = relay.shift.old_of(fi)?;
            let n = *relay.old.file_node.get(old)?;
            (n != NONE).then(|| kept_laid(relay.old, n, NodeKind::File(fi), repo.files[fi].name.clone()))
        });
        items.push(kept.unwrap_or_else(|| card_laid(repo, fi)));
    }

    let files = items.iter().map(|c| c.files).sum();
    let lines = items.iter().map(|c| c.lines).sum();
    let sizes: Vec<[f32; 2]> = items.iter().map(|c| [c.w, c.h]).collect();
    let (order, places, w, h) = pack(&sizes, arranged.map(|_| ranks.as_slice()));
    let mut items: Vec<Option<Laid>> = items.into_iter().map(Some).collect();
    let placed = order.into_iter().map(|i| (places[i][0], places[i][1], items[i].take().unwrap())).collect();

    Some(Laid {
        w,
        h,
        kind: NodeKind::Dir,
        name: dir.name.clone(),
        rel_path: dir.rel_path.clone(),
        col_stride: 0.0,
        wrap: None,
        collapsed: false,
        files,
        lines,
        paths: vec![dir.rel_path.clone()],
        inner_files: Vec::new(),
        children: placed,
        kept: None,
    })
}

fn pack(sizes: &[[f32; 2]], ranks: Option<&[u32]>) -> (Vec<usize>, Vec<[f32; 2]>, f32, f32) {
    let area: f32 = sizes.iter().map(|[w, h]| (w + GAP) * (h + GAP)).sum();
    let widest = sizes.iter().map(|s| s[0]).fold(0.0, f32::max);
    let tallest = sizes.iter().map(|s| s[1] + GAP).fold(GAP, f32::max);
    // A card taller than the box would be (a long file) sets its height: the rest goes in a strip beside
    // it, not in a row as wide as the area wants, which would leave most of that height empty.
    let target_w = (area * ASPECT).sqrt().min(area / tallest + widest + GAP).max(widest);

    let mut order: Vec<usize> = (0..sizes.len()).collect();
    let rank = |i: usize| ranks.map_or(0, |r| r[i]);
    order.sort_by(|&a, &b| rank(a).cmp(&rank(b)).then(sizes[b][1].total_cmp(&sizes[a][1])));
    let mut sky = Skyline::new(target_w);
    let mut places = vec![[0.0f32; 2]; sizes.len()];
    let (mut content_w, mut content_h) = (0.0f32, 0.0f32);
    for &i in &order {
        let [w, h] = sizes[i];
        let (ix, iy) = sky.place(w + GAP, h + GAP);
        content_w = content_w.max(ix + w);
        content_h = content_h.max(iy + h);
        places[i] = [DIR_PAD + ix, DIR_HEADER + iy];
    }
    (order, places, content_w.max(120.0) + DIR_PAD * 2.0, content_h.max(40.0) + DIR_HEADER + DIR_PAD)
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

fn place_rect(kind: NodeKind, x: f32, y: f32, slot: [f32; 2]) -> Rect {
    match kind {
        NodeKind::File(_) => Rect::new(x, y + FILE_LABEL_H, slot[0], slot[1] - FILE_LABEL_H),
        NodeKind::Dir => Rect::new(x, y, slot[0], slot[1]),
    }
}

struct Flat<'a> {
    out: Layout,
    old: Option<&'a mut Layout>,
    shift: Option<Shift>,
    from: Vec<u32>,
}

impl Flat<'_> {
    fn flatten(&mut self, l: Laid, at: [f32; 2], offset: [f32; 2], depth: u16, parent: Option<u32>) {
        if let Some(kept) = l.kept {
            return self.keep(l, kept as usize, at, offset, depth, parent);
        }
        let out = &mut self.out;
        let me = out.nodes.len() as u32;
        if let NodeKind::File(fi) = l.kind {
            out.file_node[fi] = me;
            out.file_anchor[fi] = me;
        }
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
            rect: place_rect(l.kind, at[0], at[1], [l.w, l.h]),
            depth,
            parent,
            subtree_end: 0,
            col_stride: l.col_stride,
            fold: ONE_COLUMN,
            gutter: 0.0,
            wrap: l.wrap,
            collapsed: l.collapsed,
            files: l.files,
            lines: l.lines,
            offset,
            slot: [l.w, l.h],
        });
        self.from.push(NONE);
        for (cx, cy, child) in l.children {
            self.flatten(child, [at[0] + cx, at[1] + cy], [cx, cy], depth + 1, Some(me));
        }
        self.out.nodes[me as usize].subtree_end = self.out.nodes.len() as u32;
    }

    fn keep(&mut self, l: Laid, kept: usize, at: [f32; 2], offset: [f32; 2], depth: u16, parent: Option<u32>) {
        let (Some(old), Some(shift)) = (self.old.as_deref_mut(), self.shift) else { return };
        let out = &mut self.out;
        let base = out.nodes.len() as u32;
        let end = old.nodes[kept].subtree_end as usize;
        let top_depth = old.nodes[kept].depth;
        let mut name = Some(l.name);
        for j in kept..end {
            let src = &mut old.nodes[j];
            let (up, offset, [x, y]) = match j == kept {
                true => (parent, offset, at),
                false => {
                    let up = src.parent.map(|p| p - kept as u32 + base);
                    let above = &out.nodes[up.unwrap_or(base) as usize];
                    (up, src.offset, [above.rect.x + src.offset[0], above.rect.y + src.offset[1]])
                }
            };
            let kind = match src.kind {
                NodeKind::File(fi) => NodeKind::File(shift.new_of(fi).unwrap_or(fi)),
                NodeKind::Dir => NodeKind::Dir,
            };
            let me = out.nodes.len() as u32;
            if let NodeKind::File(fi) = kind {
                out.file_node[fi] = me;
                out.file_anchor[fi] = me;
            }
            out.nodes.push(Node {
                kind,
                name: name.take().unwrap_or_else(|| std::mem::take(&mut src.name)),
                rel_path: std::mem::take(&mut src.rel_path),
                rect: place_rect(kind, x, y, src.slot),
                depth: depth + (src.depth - top_depth),
                parent: up,
                subtree_end: src.subtree_end - kept as u32 + base,
                col_stride: src.col_stride,
                fold: src.fold,
                gutter: src.gutter,
                wrap: src.wrap.take(),
                collapsed: src.collapsed,
                files: src.files,
                lines: src.lines,
                offset,
                slot: src.slot,
            });
            self.from.push(j as u32);
        }
        for p in l.paths {
            out.dir_node.insert(p, base);
        }
    }
}

pub(crate) fn find_dir<'t>(tree: &'t Dir, rel_path: &str) -> Option<&'t Dir> {
    let mut dir = tree;
    for part in rel_path.split('/').filter(|part| !part.is_empty()) {
        dir = dir.dirs.iter().find(|d| d.name == part)?;
    }
    Some(dir)
}

impl Layout {
    pub fn build(repo: &Repo) -> Layout {
        Self::build_filtered(repo, &LayoutFilter::default())
    }

    fn empty(repo: &Repo, laid: &Laid) -> Layout {
        Layout {
            nodes: Vec::new(),
            bounds: Rect::new(0.0, 0.0, laid.w, laid.h),
            file_node: vec![NONE; repo.files.len()],
            file_anchor: vec![NONE; repo.files.len()],
            dir_node: HashMap::new(),
            order: None,
        }
    }

    fn finish(&mut self, repo: &Repo, fresh: impl Fn(usize) -> bool) {
        let Layout { nodes, file_node, .. } = self;
        for (file, &node) in file_node.iter().enumerate().filter(|(_, node)| **node != NONE) {
            if !fresh(node as usize) {
                continue;
            }
            let n = &mut nodes[node as usize];
            n.fold = rows_per_column(&repo.files[file]);
            n.gutter = gutter_of(&repo.files[file]);
        }
    }

    pub fn build_filtered(repo: &Repo, filter: &LayoutFilter) -> Layout {
        let laid = lay_dir(&repo.tree, repo, filter, true, None).expect("the root is always laid out");
        let mut flat = Flat { out: Self::empty(repo, &laid), old: None, shift: None, from: Vec::new() };
        flat.flatten(laid, [0.0, 0.0], [0.0, 0.0], 0, None);
        let mut out = flat.out;
        out.finish(repo, |_| true);
        out.order = filter.order.clone();
        out
    }

    pub fn insert_file(&mut self, repo: &Repo, filter: &LayoutFilter, file: usize) -> Option<Vec<u32>> {
        let rel_path = repo.files.get(file)?.rel_path.clone();
        self.relay(repo, filter, &rel_path, Shift::Insert(file))
    }

    pub fn remove_file(&mut self, repo: &Repo, filter: &LayoutFilter, file: usize, rel_path: &str) -> Option<Vec<u32>> {
        self.relay(repo, filter, rel_path, Shift::Remove(file))
    }

    fn relay(&mut self, repo: &Repo, filter: &LayoutFilter, rel_path: &str, shift: Shift) -> Option<Vec<u32>> {
        if filter.only_files.is_some() || self.nodes.is_empty() {
            return None;
        }
        let on_path = ancestors_of(rel_path);
        let reordered = self.reordered(filter)?;
        if reordered.iter().any(|&n| !on_path.contains(&self.nodes[n].rel_path)) {
            return None;
        }
        let laid = {
            let relay = Relay { old: self, on_path: ancestors_of(rel_path), shift };
            lay_dir(&repo.tree, repo, filter, true, Some(&relay)).expect("the root is always laid out")
        };
        let mut old = std::mem::take(self);
        let mut flat =
            Flat { out: Self::empty(repo, &laid), old: Some(&mut old), shift: Some(shift), from: Vec::new() };
        flat.flatten(laid, [0.0, 0.0], [0.0, 0.0], 0, None);
        let Flat { mut out, from, .. } = flat;
        let mut new_of_old = vec![NONE; old.nodes.len()];
        for (new, &was) in from.iter().enumerate().filter(|(_, was)| **was != NONE) {
            new_of_old[was as usize] = new as u32;
        }
        for (path, was) in old.dir_node.drain() {
            let now = new_of_old[was as usize];
            if now != NONE && !on_path.contains(&path) {
                out.dir_node.entry(path).or_insert(now);
            }
        }
        for (was_file, &was) in old.file_anchor.iter().enumerate().filter(|(_, was)| **was != NONE) {
            let (Some(file), now) = (shift.new_of(was_file), new_of_old[was as usize]) else { continue };
            if now != NONE && out.file_anchor[file] == NONE {
                out.file_anchor[file] = now;
            }
        }
        out.finish(repo, |node| from[node] == NONE);
        out.order = filter.order.clone();
        *self = out;
        Some(from)
    }

    /// The folders on the canvas whose children `filter.order` puts in another order
    /// than the one they were laid out in; `None` when one of the two has no order.
    fn reordered(&self, filter: &LayoutFilter) -> Option<Vec<usize>> {
        match (self.order.as_deref(), filter.order.as_deref()) {
            (None, None) => Some(Vec::new()),
            (Some(old), Some(new)) if std::ptr::eq(old, new) => Some(Vec::new()),
            (Some(old), Some(new)) => Some(
                old.changed(new)
                    .into_iter()
                    .filter_map(|path| {
                        let n = *self.dir_node.get(&path)? as usize;
                        let node = &self.nodes[n];
                        (node.rel_path == path && !node.collapsed).then_some(n)
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    pub fn resize_file(&mut self, repo: &Repo, filter: &LayoutFilter, file: usize) -> Option<Vec<Range<u32>>> {
        if filter.only_files.is_some() {
            return None;
        }
        let mut pending = self.reordered(filter)?;
        let anchor = *self.file_anchor.get(file)?;
        if anchor == NONE {
            self.order = filter.order.clone();
            return pending.is_empty().then(Vec::new);
        }
        let n = anchor as usize;
        let mut above = Vec::new();
        let mut up = self.nodes[n].parent;
        while let Some(p) = up {
            above.push(p as usize);
            up = self.nodes[p as usize].parent;
        }
        if pending.iter().any(|r| !above.contains(r)) {
            return None;
        }
        self.order = filter.order.clone();
        let (before, had) = (self.nodes[n].slot, self.nodes[n].lines);
        match self.nodes[n].kind {
            NodeKind::File(fi) => {
                let f = &repo.files[fi];
                let wrap = wrap_of(f);
                let (w, h, col_stride) = card_size(repo, fi, wrap.as_deref());
                let node = &mut self.nodes[n];
                node.slot = [w, h + FILE_LABEL_H];
                node.col_stride = col_stride;
                node.fold = rows_per_column(f);
                node.gutter = gutter_of(f);
                node.wrap = wrap;
                node.lines = f.line_count() as u64;
            }
            NodeKind::Dir => {
                let dir = find_dir(&repo.tree, &self.nodes[n].rel_path)?;
                let (mut inner, mut paths) = (Vec::new(), Vec::new());
                collect_inner(dir, repo, filter, &mut inner, &mut paths);
                let (w, h, lines) = collapsed_size(repo, &inner);
                let node = &mut self.nodes[n];
                (node.slot, node.lines, node.files) = ([w, h], lines, inner.len() as u32);
            }
        }
        let now = self.nodes[n].lines;
        let mut up = self.nodes[n].parent;
        while let Some(p) = up.map(|p| p as usize) {
            self.nodes[p].lines = self.nodes[p].lines + now - had;
            up = self.nodes[p].parent;
        }
        if self.nodes[n].slot == before && pending.is_empty() {
            return Some(std::iter::once(n as u32..n as u32 + 1).collect());
        }
        let (mut chain, mut moved) = (vec![n], Vec::new());
        while let Some(p) = self.nodes[*chain.last().unwrap()].parent.map(|p| p as usize) {
            let before = self.nodes[p].slot;
            moved.push(self.repack(repo, filter, p)?);
            chain.push(p);
            pending.retain(|&r| r != p);
            if self.nodes[p].slot == before && pending.is_empty() {
                break;
            }
        }
        let mut changed = Vec::new();
        for level in (0..chain.len()).rev() {
            let c = chain[level];
            let was = [self.nodes[c].rect.x, self.nodes[c].rect.y];
            if level + 1 < chain.len() || c == 0 {
                self.place(c);
            }
            let shifted = was != [self.nodes[c].rect.x, self.nodes[c].rect.y];
            changed.push(c as u32..c as u32 + 1);
            let Some(below) = level.checked_sub(1).map(|l| chain[l]) else { continue };
            let movers: Vec<usize> = match shifted {
                true => self.children(c).collect(),
                false => moved[level - 1].clone(),
            };
            for m in movers.into_iter().filter(|&m| m != below) {
                let end = self.nodes[m].subtree_end as usize;
                for i in m..end {
                    self.place(i);
                }
                changed.push(m as u32..end as u32);
            }
        }
        self.bounds = Rect::new(0.0, 0.0, self.nodes[0].slot[0], self.nodes[0].slot[1]);
        changed.sort_by_key(|r| r.start);
        Some(changed)
    }

    fn place(&mut self, i: usize) {
        let [x, y] = match self.nodes[i].parent {
            Some(p) => {
                let (above, n) = (&self.nodes[p as usize], &self.nodes[i]);
                [above.rect.x + n.offset[0], above.rect.y + n.offset[1]]
            }
            None => [0.0, 0.0],
        };
        let n = &mut self.nodes[i];
        n.rect = place_rect(n.kind, x, y, n.slot);
    }

    fn children(&self, p: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.nodes[p].subtree_end as usize;
        std::iter::successors(Some(p + 1).filter(|&c| c < end), move |&c| {
            Some(self.nodes[c].subtree_end as usize).filter(|&next| next < end)
        })
    }

    fn repack(&mut self, repo: &Repo, filter: &LayoutFilter, p: usize) -> Option<Vec<usize>> {
        let dir = find_dir(&repo.tree, &self.nodes[p].rel_path)?;
        let is_child = |n: u32| self.nodes[n as usize].parent == Some(p as u32);
        let order = filter.order.as_deref();
        let mut items: Vec<usize> = Vec::new();
        let mut ranks: Vec<u32> = Vec::new();
        for d in &dir.dirs {
            if let Some(&c) = self.dir_node.get(&d.rel_path).filter(|&&c| is_child(c)) {
                items.push(c as usize);
                ranks.push(order.map_or(0, |o| o.dir_rank(&d.rel_path)));
            }
        }
        for &fi in &dir.files {
            let c = self.file_node[fi];
            if c != NONE && is_child(c) {
                items.push(c as usize);
                ranks.push(order.map_or(0, |o| o.file_rank(fi)));
            }
        }
        if items.len() != self.children(p).count() {
            return None;
        }
        let sizes: Vec<[f32; 2]> = items.iter().map(|&c| self.nodes[c].slot).collect();
        let (_, places, w, h) = pack(&sizes, order.map(|_| ranks.as_slice()));
        let mut moved = Vec::new();
        for (&c, place) in items.iter().zip(places) {
            if self.nodes[c].offset != place {
                self.nodes[c].offset = place;
                moved.push(c);
            }
        }
        let files = items.iter().map(|&c| self.nodes[c].files).sum();
        let lines = items.iter().map(|&c| self.nodes[c].lines).sum();
        let node = &mut self.nodes[p];
        (node.slot, node.files, node.lines) = ([w, h], files, lines);
        Some(moved)
    }

    pub fn differences(&self, other: &Layout) -> Vec<String> {
        let by_path = |l: &Layout| -> HashMap<(bool, String), usize> {
            l.nodes.iter().enumerate().map(|(i, n)| ((n.is_dir(), n.rel_path.clone()), i)).collect()
        };
        let (mine, theirs) = (by_path(self), by_path(other));
        let mut out = Vec::new();
        if self.bounds != other.bounds {
            out.push(format!("bounds {:?} and {:?}", self.bounds, other.bounds));
        }
        for (key, &i) in &mine {
            let Some(&j) = theirs.get(key) else {
                out.push(format!("{} only here", key.1));
                continue;
            };
            let (a, b) = (&self.nodes[i], &other.nodes[j]);
            let up = |l: &Layout, n: &Node| n.parent.map(|p| l.nodes[p as usize].rel_path.clone());
            let same = a.kind == b.kind
                && a.name == b.name
                && a.rect == b.rect
                && a.depth == b.depth
                && (a.files, a.lines, a.collapsed, a.fold, a.col_stride, a.gutter)
                    == (b.files, b.lines, b.collapsed, b.fold, b.col_stride, b.gutter)
                && a.subtree_end as usize - i == b.subtree_end as usize - j
                && up(self, a) == up(other, b);
            if !same {
                out.push(format!("{}: {:?} and {:?}", key.1, a.rect, b.rect));
            }
        }
        out.extend(theirs.keys().filter(|key| !mine.contains_key(*key)).map(|key| format!("{} missing", key.1)));
        let anchors = |l: &Layout| -> Vec<Option<(bool, String)>> {
            let node =
                |n: u32| (n != NONE).then(|| (l.nodes[n as usize].is_dir(), l.nodes[n as usize].rel_path.clone()));
            l.file_anchor.iter().map(|&n| node(n)).chain(l.file_node.iter().map(|&n| node(n))).collect()
        };
        if anchors(self) != anchors(other) {
            out.push("files are anchored elsewhere".into());
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
            assert_eq!(n.columns(lines), 1);
            assert!(n.code_origin()[1] + lines as f32 * LINE_H <= n.rect.bottom());
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

    fn numbered_lines(count: usize) -> String {
        (0..count).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn a_long_file_is_one_column_that_goes_down() {
        let repo = temp_repo("tall", &[("big.txt", numbered_lines(9000))]);
        let layout = Layout::build(&repo);

        let n = &layout.nodes[layout.file_node[0] as usize];
        assert_eq!(n.columns(9000), 1);
        assert!(n.rect.h >= 9000.0 * LINE_H);
        assert!(n.rect.w < n.col_stride + 2.0 * CARD_PAD);
        let o = n.code_origin();
        let view = Rect::new(o[0], o[1] + 8000.0 * LINE_H, 10.0, 100.0 * LINE_H);
        assert_eq!(n.visible_lines(&view, 9000), vec![(0, 8000..8100)]);
        let beside = Rect::new(o[0] + n.col_stride, o[1], 10.0, n.rect.h);
        assert!(n.visible_lines(&beside, 9000).is_empty(), "nothing is drawn beside the card");
        for (line, col) in [(0u32, 0u32), (17, 3), (2048, 0), (8999, 5)] {
            let r = n.cell_rect(line, col, 1);
            assert_eq!(r.x, o[0] + col as f32 * CHAR_W);
            assert_eq!(n.cell_at([r.x + 1.0, r.y + 1.0], 9000), Some((line, col)));
        }
        assert_eq!(n.cell_rect(9000, 0, 1).y, o[1] + 9000.0 * LINE_H, "after the last line is under it");
        assert_eq!(
            n.line_rects(2040..2060),
            vec![Rect::new(o[0], o[1] + 2040.0 * LINE_H, n.shown_cols() as f32 * CHAR_W, 20.0 * LINE_H)]
        );
        assert_eq!(n.cell_at([o[0] + n.col_stride + 1.0, o[1] + 1.0], 9000), None);
    }

    #[test]
    fn small_files_go_beside_a_long_one_not_in_a_row_as_wide_as_its_area() {
        let mut files = vec![("big.txt", numbered_lines(9000))];
        let names: Vec<String> = (0..12).map(|i| format!("small_{i:02}.txt")).collect();
        files.extend(names.iter().map(|name| (name.as_str(), "a\nb\n".to_string())));
        let repo = temp_repo("beside", &files);
        let layout = Layout::build(&repo);
        let big_file = repo.file_index("big.txt").unwrap();
        let big = &layout.nodes[layout.file_node[big_file] as usize];
        let root = &layout.nodes[0];
        assert!(root.rect.w < 3.0 * big.rect.w, "the folder is about as wide as the long card and a small one");
        for fi in (0..repo.files.len()).filter(|&fi| fi != big_file) {
            let small = &layout.nodes[layout.file_node[fi] as usize];
            assert!(small.rect.x >= big.rect.right(), "{} is beside the long card", small.name);
            assert!(small.rect.bottom() < big.rect.y + big.rect.h * 0.5, "{} is near the top", small.name);
        }
    }

    #[test]
    fn a_wrapped_card_is_taller_and_its_cells_are_on_its_rows() {
        let text = format!("short\n{}\nend\n", "x".repeat(400));
        let repo = temp_repo("wrap", &[("long.txt", text)]);
        let layout = Layout::build(&repo);
        let mut n = layout.nodes[layout.file_node[0] as usize].clone();
        assert!(n.wrap.is_none(), "off unless asked for");
        assert!(wrap_if(&repo.files[0], false).is_none());
        let wrap = wrap_if(&repo.files[0], true).expect("a line of 400 columns wraps");
        let (_, cut, _) = card_size(&repo, 0, None);
        let (_, tall, _) = card_size(&repo, 0, Some(&wrap));
        assert_eq!(tall - cut, 2.0 * LINE_H, "400 columns take three rows of 160");

        (n.fold, n.wrap) = (ONE_COLUMN, Some(wrap));
        let o = n.code_origin();
        assert_eq!(n.cell_rect(1, 0, 1).y, o[1] + LINE_H);
        let r = n.cell_rect(1, 170, 1);
        assert_eq!((r.x, r.y), (o[0] + 12.0 * CHAR_W, o[1] + 2.0 * LINE_H), "indented on the next row");
        assert_eq!(n.cell_rect(2, 0, 1).y, o[1] + 4.0 * LINE_H, "the next line is under the rest");
        for (line, col) in [(0u32, 0u32), (1, 5), (1, 159), (1, 160), (1, 300), (1, 399), (2, 2)] {
            let r = n.cell_rect(line, col, 1);
            assert_eq!(n.cell_at([r.x + 1.0, r.y + 1.0], 3), Some((line, col)), "{line}:{col} there and back");
        }
        assert_eq!(n.cell_rects(1, 150, 20).len(), 2, "a span across a break is two rectangles");
        let whole = n.line_rects(1..2);
        assert_eq!((whole.len(), whole[0].h), (1, 3.0 * LINE_H), "a line's wash covers all its rows");
        let view = Rect::new(n.rect.x, o[1] + 2.5 * LINE_H, n.rect.w, LINE_H);
        assert_eq!(n.visible_lines(&view, 3), vec![(0, 1..2)]);
        assert_eq!((n.column_rows(0, 3), n.first_row(2)), (5, 4));
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
