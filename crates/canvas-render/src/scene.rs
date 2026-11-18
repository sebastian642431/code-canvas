use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use canvas_core::layout::{Layout, NodeKind, FILE_LABEL_H};
use canvas_core::{Rect, Repo, SourceFile, TokenCache, Tokens, Wrap, BLOCK_LINES, CHUNK_LINES, LINE_H};

use crate::gpu_util::DynBuffer;
use crate::theme;

pub const MAX_DYNAMIC_INSTANCES: usize = 4_000_000;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct RectInstance {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub fill: u32,
    pub border: u32,
    pub radius: f32,
    pub border_w: f32,
}

impl RectInstance {
    pub fn new(r: Rect, fill: u32, border: u32, radius: f32, border_w: f32) -> Self {
        Self { pos: [r.x, r.y], size: [r.w, r.h], fill, border, radius, border_w }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CodeInstance {
    pub file: u32,
    pub line_kind: u32,
    pub col_cols: u32,
}

impl CodeInstance {
    fn new(file: usize, line: usize, kind: u32, span: usize, col: u32, cols: u32) -> Self {
        Self {
            file: file as u32,
            line_kind: (line as u32 & 0xfffff) | (kind & 0xf) << 20 | (span.min(255) as u32) << 24,
            col_cols: col.min(0xffff) | cols.min(0xffff) << 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lod {
    Cards,
    Blocks,
    Lines,
    Tokens,
    Text,
}

impl Lod {
    pub fn for_zoom(zoom: f32) -> Lod {
        let px_per_line = zoom * LINE_H;
        if px_per_line < 0.08 {
            Lod::Cards
        } else if px_per_line < 1.0 {
            Lod::Blocks
        } else if px_per_line < 2.5 {
            Lod::Lines
        } else if px_per_line < 6.5 {
            Lod::Tokens
        } else {
            Lod::Text
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Lod::Cards => "cards",
            Lod::Blocks => "blocks",
            Lod::Lines => "lines",
            Lod::Tokens => "tokens",
            Lod::Text => "text",
        }
    }
}

pub struct SceneGpu {
    pub(crate) frames: wgpu::Buffer,
    pub(crate) blocks: wgpu::Buffer,
    block_start: Vec<u32>,
    pub(crate) origins_bind: wgpu::BindGroup,
    _origins: wgpu::Buffer,
    pub(crate) dynamic: DynBuffer,
    pub(crate) dynamic_len: u32,
    pub(crate) frame_ranges: Vec<Range<u32>>,
    pub(crate) code_ranges: Vec<Range<u32>>,
    scratch: Vec<CodeInstance>,
    pub frame_count: u32,
    pub block_count: u32,
}

fn dominant(counts: &[u32; 11]) -> u32 {
    theme::kind_index(canvas_core::syntax::dominant_kind(counts))
}

fn push_blocks(fi: usize, f: &SourceFile, wrap: Option<&Wrap>, out: &mut Vec<CodeInstance>) {
    let n = f.line_count();
    for (b, block) in f.analysis.blocks.iter().enumerate() {
        let line = b * BLOCK_LINES;
        let span = BLOCK_LINES.min(n - line);
        let hi = block.hi.min(canvas_core::layout::MAX_COLS as u16);
        if hi > block.lo {
            let kind = theme::kind_index(block.kind);
            let (mut row, mut rows) = match wrap {
                Some(wrap) => {
                    let (first, last) = (wrap.row(line), wrap.row(line + span - 1) + wrap.rows(line + span - 1));
                    (first as usize, (last - first) as usize)
                }
                None => (line, span),
            };
            while rows > 0 {
                let part = rows.min(255);
                out.push(CodeInstance::new(fi, row, kind, part, block.lo as u32, (hi - block.lo) as u32));
                (row, rows) = (row + part, rows - part);
            }
        }
    }
}

fn push_bar(fi: usize, wrap: Option<&Wrap>, line: usize, kind: u32, start: u32, end: u32, out: &mut Vec<CodeInstance>) {
    match wrap {
        Some(wrap) => {
            for (row, col, cols) in wrap.pieces(line, start, end.saturating_sub(start)) {
                out.push(CodeInstance::new(fi, row as usize, kind, 1, col, cols));
            }
        }
        None => {
            let end = end.min(canvas_core::layout::MAX_COLS);
            if end > start {
                out.push(CodeInstance::new(fi, line, kind, 1, start, end - start));
            }
        }
    }
}

pub struct SceneData {
    frames: Vec<RectInstance>,
    blocks: Vec<CodeInstance>,
    block_start: Vec<u32>,
    origins: Vec<[f32; 4]>,
}

impl SceneData {
    pub fn build(repo: &Repo, layout: &Layout) -> Self {
        let mut frames = Vec::with_capacity(layout.nodes.len());
        let mut blocks = Vec::new();
        let mut block_start = Vec::with_capacity(layout.nodes.len() + 1);
        let colors = theme::get();
        for n in &layout.nodes {
            block_start.push(blocks.len() as u32);
            match n.kind {
                NodeKind::Dir => frames.push(RectInstance::new(
                    n.rect,
                    if n.collapsed { colors.dir_collapsed_fill } else { colors.dir_fill },
                    colors.dir_border(n.depth as usize),
                    6.0,
                    1.0,
                )),
                NodeKind::File(fi) => {
                    let f = &repo.files[fi];
                    let fill = if f.note.is_some() { colors.card_note_fill } else { colors.card_fill };
                    frames.push(RectInstance::new(n.rect, fill, colors.card_border, 3.0, 1.0));
                    push_blocks(fi, f, n.wrap.as_deref(), &mut blocks);
                }
            }
        }
        block_start.push(blocks.len() as u32);

        let mut origins = vec![[0.0f32; 4]; repo.files.len().max(1)];
        for (fi, &ni) in layout.file_node.iter().enumerate() {
            if ni == canvas_core::layout::NONE {
                continue;
            }
            let n = &layout.nodes[ni as usize];
            let [x, y] = n.code_origin();
            origins[fi] = [x, y, n.col_stride, n.fold as f32];
        }
        SceneData { frames, blocks, block_start, origins }
    }
}

impl SceneGpu {
    pub fn new(device: &wgpu::Device, origins_layout: &wgpu::BindGroupLayout, name: &str, data: SceneData) -> Self {
        use wgpu::util::DeviceExt;
        let SceneData { frames, blocks, block_start, origins } = data;

        let mk = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: if contents.is_empty() { &[0u8; 16] } else { contents },
                usage,
            })
        };
        let frames_buf = mk("frames", bytemuck::cast_slice(&frames), wgpu::BufferUsages::VERTEX);
        let blocks_buf = mk("blocks", bytemuck::cast_slice(&blocks), wgpu::BufferUsages::VERTEX);
        let origins_buf = mk("origins", bytemuck::cast_slice(&origins), wgpu::BufferUsages::STORAGE);
        let origins_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("origins"),
            layout: origins_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: origins_buf.as_entire_binding() }],
        });

        log::info!(
            "scene '{}': {} nodes, {} block instances ({:.1} MB on GPU)",
            name,
            frames.len(),
            blocks.len(),
            (frames.len() * 32 + blocks.len() * 12 + origins.len() * 16) as f64 / 1e6
        );

        SceneGpu {
            frame_count: frames.len() as u32,
            block_count: blocks.len() as u32,
            frames: frames_buf,
            blocks: blocks_buf,
            block_start,
            origins_bind,
            _origins: origins_buf,
            dynamic: DynBuffer::new(device, "dynamic code", wgpu::BufferUsages::VERTEX),
            dynamic_len: 0,
            frame_ranges: Vec::new(),
            code_ranges: Vec::new(),
            scratch: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        repo: &Repo,
        layout: &Layout,
        tokens: &mut TokenCache,
        view: &Rect,
        lod: Lod,
    ) -> u32 {
        self.frame_ranges.clear();
        self.code_ranges.clear();
        cull_ranges(layout, view, |i| i as u32, &mut self.frame_ranges);

        match lod {
            Lod::Cards | Lod::Text => 0,
            Lod::Blocks => {
                let starts = &self.block_start;
                cull_ranges(layout, view, |i| starts[i], &mut self.code_ranges);
                self.code_ranges.iter().map(|r| r.end - r.start).sum()
            }
            Lod::Lines | Lod::Tokens => {
                self.scratch.clear();
                let scratch = &mut self.scratch;
                layout.visit_visible(view, |_, n| {
                    if let NodeKind::File(fi) = n.kind {
                        let f = &repo.files[fi];
                        for (_, lines) in n.visible_lines(view, f.line_count()) {
                            for c in lines.start / CHUNK_LINES..=(lines.end - 1) / CHUNK_LINES {
                                let toks = tokens.chunk(fi, f, c);
                                let cl = f.chunk_lines(c);
                                let r = lines.start.max(cl.start)..lines.end.min(cl.end);
                                let wrap = n.wrap.as_deref();
                                if lod == Lod::Lines {
                                    push_lines(fi, wrap, toks, cl.start, r, scratch);
                                } else {
                                    push_tokens(fi, wrap, toks, cl.start, r, scratch);
                                }
                            }
                        }
                    }
                    scratch.len() < MAX_DYNAMIC_INSTANCES
                });
                self.dynamic.write(device, queue, bytemuck::cast_slice(&self.scratch));
                self.dynamic_len = self.scratch.len() as u32;
                if self.dynamic_len > 0 {
                    self.code_ranges.push(0..self.dynamic_len);
                }
                self.dynamic_len
            }
        }
    }
}

fn push_lines(
    fi: usize,
    wrap: Option<&Wrap>,
    tokens: &Tokens,
    first: usize,
    lines: Range<usize>,
    out: &mut Vec<CodeInstance>,
) {
    for line in lines {
        let toks = tokens.line(line - first);
        let mut i = 0;
        while i < toks.len() {
            let start = toks[i].col as u32;
            let mut end = start + toks[i].cols as u32;
            let mut counts = [0u32; 11];
            counts[toks[i].kind as usize] += toks[i].cols as u32;
            i += 1;
            while i < toks.len() && (toks[i].col as u32) <= end + 2 {
                end = toks[i].col as u32 + toks[i].cols as u32;
                counts[toks[i].kind as usize] += toks[i].cols as u32;
                i += 1;
            }
            push_bar(fi, wrap, line, dominant(&counts), start, end, out);
        }
    }
}

fn push_tokens(
    fi: usize,
    wrap: Option<&Wrap>,
    tokens: &Tokens,
    first: usize,
    lines: Range<usize>,
    out: &mut Vec<CodeInstance>,
) {
    for line in lines {
        let toks = tokens.line(line - first);
        let mut i = 0;
        while i < toks.len() {
            let kind = theme::kind_index(toks[i].kind);
            let start = toks[i].col as u32;
            let mut end = start + toks[i].cols as u32;
            i += 1;
            while i < toks.len() && theme::kind_index(toks[i].kind) == kind && (toks[i].col as u32) <= end + 1 {
                end = toks[i].col as u32 + toks[i].cols as u32;
                i += 1;
            }
            push_bar(fi, wrap, line, kind, start, end, out);
        }
    }
}

fn cull_ranges(layout: &Layout, view: &Rect, start: impl Fn(usize) -> u32, out: &mut Vec<Range<u32>>) {
    let push = |r: Range<u32>, out: &mut Vec<Range<u32>>| {
        if r.start >= r.end {
            return;
        }
        if let Some(last) = out.last_mut() {
            if last.end == r.start {
                last.end = r.end;
                return;
            }
        }
        out.push(r);
    };
    let nodes = &layout.nodes;
    let mut i = 0;
    while i < nodes.len() {
        let n = &nodes[i];
        let end = n.subtree_end as usize;
        let mut r = n.rect;
        if !n.is_dir() {
            r.y -= FILE_LABEL_H;
            r.h += FILE_LABEL_H;
        }
        if !r.intersects(view) {
            i = end;
        } else if view.contains_rect(&r) || end == i + 1 {
            push(start(i)..start(end), out);
            i = end;
        } else {
            push(start(i)..start(i + 1), out);
            i += 1;
        }
    }
}
