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
pub const NONE: u32 = u32::MAX;

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
        [self.rect.x + CARD_PAD, self.rect.y + CARD_PAD]
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
        let width = (self.col_stride - FOLD_GAP).max(CHAR_W);
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
        if in_col > self.col_stride - FOLD_GAP {
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
            let x1 = x0 + self.col_stride - FOLD_GAP;
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
    let col_w = f.max_cols.clamp(MIN_COLS, MAX_COLS) as f32 * CHAR_W;
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
}
