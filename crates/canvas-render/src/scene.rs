use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use canvas_core::layout::{Layout, Node, NodeKind, FILE_LABEL_H, NONE};
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
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
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
    origins_buf: wgpu::Buffer,
    origins: Vec<[f32; 4]>,
    slots: Vec<u32>,
    free: Vec<u32>,
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
    let blocks = f.analysis.painted.as_deref().unwrap_or(&f.analysis.blocks);
    for (b, block) in blocks.iter().enumerate() {
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
    slots: Vec<u32>,
}

fn frame_of(n: &Node, repo: &Repo, colors: &theme::Theme) -> RectInstance {
    match n.kind {
        NodeKind::Dir => RectInstance::new(
            n.rect,
            if n.collapsed { colors.dir_collapsed_fill } else { colors.dir_fill },
            colors.dir_border(n.depth as usize),
            6.0,
            1.0,
        ),
        NodeKind::File(fi) => {
            let fill = if repo.files[fi].note.is_some() { colors.card_note_fill } else { colors.card_fill };
            RectInstance::new(n.rect, fill, colors.card_border, 3.0, 1.0)
        }
    }
}

fn origin_of(n: &Node) -> [f32; 4] {
    let [x, y] = n.code_origin();
    [x, y, n.col_stride, n.fold as f32]
}

fn blocks_of(n: &Node, repo: &Repo, slots: &[u32]) -> Vec<CodeInstance> {
    let mut out = Vec::new();
    if let NodeKind::File(fi) = n.kind {
        push_blocks(slots[fi] as usize, &repo.files[fi], n.wrap.as_deref(), &mut out);
    }
    out
}

fn origins_of(layout: &Layout, slots: &[u32], len: usize) -> Vec<[f32; 4]> {
    let mut origins = vec![[0.0f32; 4]; len.max(1)];
    for (fi, &ni) in layout.file_node.iter().enumerate().filter(|(_, ni)| **ni != NONE) {
        origins[slots[fi] as usize] = origin_of(&layout.nodes[ni as usize]);
    }
    origins
}

impl SceneData {
    pub fn build(repo: &Repo, layout: &Layout) -> Self {
        let slots: Vec<u32> = (0..repo.files.len() as u32).collect();
        let mut frames = Vec::with_capacity(layout.nodes.len());
        let mut blocks = Vec::new();
        let mut block_start = Vec::with_capacity(layout.nodes.len() + 1);
        let colors = theme::get();
        for n in &layout.nodes {
            block_start.push(blocks.len() as u32);
            frames.push(frame_of(n, repo, &colors));
            if let NodeKind::File(fi) = n.kind {
                push_blocks(fi, &repo.files[fi], n.wrap.as_deref(), &mut blocks);
            }
        }
        block_start.push(blocks.len() as u32);
        let origins = origins_of(layout, &slots, repo.files.len());
        SceneData { frames, blocks, block_start, origins, slots }
    }
}

trait Moves {
    fn copy(&mut self, from: u32, to: u32, count: u32);
    fn write(&mut self, at: u32, data: &[CodeInstance]);
}

fn move_blocks(
    old_start: &[u32],
    from: &[u32],
    mut fresh: impl FnMut(usize) -> Vec<CodeInstance>,
    out: &mut impl Moves,
) -> Vec<u32> {
    let mut start = Vec::with_capacity(from.len() + 1);
    let mut at = 0u32;
    let mut run: Option<(u32, u32, u32)> = None;
    let flush = |run: &mut Option<(u32, u32, u32)>, out: &mut dyn FnMut(u32, u32, u32)| {
        if let Some((src, dst, count)) = run.take().filter(|r| r.2 > 0) {
            out(src, dst, count);
        }
    };
    let mut copies = Vec::new();
    for (i, &was) in from.iter().enumerate() {
        start.push(at);
        if was == NONE {
            flush(&mut run, &mut |s, d, c| copies.push((s, d, c)));
            let data = fresh(i);
            out.write(at, &data);
            at += data.len() as u32;
            continue;
        }
        let (a, b) = (old_start[was as usize], old_start[was as usize + 1]);
        match &mut run {
            Some((src, _, count)) if *src + *count == a => *count += b - a,
            _ => {
                flush(&mut run, &mut |s, d, c| copies.push((s, d, c)));
                run = Some((a, at, b - a));
            }
        }
        at += b - a;
    }
    flush(&mut run, &mut |s, d, c| copies.push((s, d, c)));
    for (src, dst, count) in copies {
        out.copy(src, dst, count);
    }
    start.push(at);
    start
}

fn splice_blocks(start: &mut [u32], n: usize, data: &[CodeInstance], out: &mut impl Moves) {
    let (a, b, end) = (start[n], start[n + 1], *start.last().unwrap_or(&0));
    let len = data.len() as u32;
    out.copy(0, 0, a);
    out.write(a, data);
    out.copy(b, a + len, end - b);
    for s in &mut start[n + 1..] {
        *s = *s + len - (b - a);
    }
}

struct GpuMoves<'a> {
    queue: &'a wgpu::Queue,
    encoder: &'a mut wgpu::CommandEncoder,
    old: &'a wgpu::Buffer,
    new: &'a wgpu::Buffer,
}

const CODE: u64 = std::mem::size_of::<CodeInstance>() as u64;

impl Moves for GpuMoves<'_> {
    fn copy(&mut self, from: u32, to: u32, count: u32) {
        if count > 0 {
            self.encoder.copy_buffer_to_buffer(
                self.old,
                from as u64 * CODE,
                self.new,
                to as u64 * CODE,
                count as u64 * CODE,
            );
        }
    }

    fn write(&mut self, at: u32, data: &[CodeInstance]) {
        if !data.is_empty() {
            self.queue.write_buffer(self.new, at as u64 * CODE, bytemuck::cast_slice(data));
        }
    }
}

fn buffer(device: &wgpu::Device, label: &str, bytes: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(16).next_multiple_of(4),
        usage,
        mapped_at_creation: false,
    })
}

const BLOCKS: wgpu::BufferUsages =
    wgpu::BufferUsages::VERTEX.union(wgpu::BufferUsages::COPY_DST).union(wgpu::BufferUsages::COPY_SRC);
const FRAMES: wgpu::BufferUsages = wgpu::BufferUsages::VERTEX.union(wgpu::BufferUsages::COPY_DST);
const ORIGINS: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST);

impl SceneGpu {
    pub fn new(device: &wgpu::Device, origins_layout: &wgpu::BindGroupLayout, name: &str, data: SceneData) -> Self {
        use wgpu::util::DeviceExt;
        let SceneData { frames, blocks, block_start, origins, slots } = data;

        let mk = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: if contents.is_empty() { &[0u8; 16] } else { contents },
                usage,
            })
        };
        let frames_buf = mk("frames", bytemuck::cast_slice(&frames), FRAMES);
        let blocks_buf = mk("blocks", bytemuck::cast_slice(&blocks), BLOCKS);
        let origins_buf = mk("origins", bytemuck::cast_slice(&origins), ORIGINS);
        let origins_bind = Self::bind(device, origins_layout, &origins_buf);

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
            origins_buf,
            origins,
            slots,
            free: Vec::new(),
            dynamic: DynBuffer::new(device, "dynamic code", wgpu::BufferUsages::VERTEX),
            dynamic_len: 0,
            frame_ranges: Vec::new(),
            code_ranges: Vec::new(),
            scratch: Vec::new(),
        }
    }

    fn bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, origins: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("origins"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: origins.as_entire_binding() }],
        })
    }

    fn write_origins(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        span: Range<usize>,
    ) {
        let bytes = (self.origins.len() * 16) as u64;
        if bytes > self.origins_buf.size() {
            self.origins_buf = buffer(device, "origins", bytes.next_power_of_two(), ORIGINS);
            self.origins_bind = Self::bind(device, layout, &self.origins_buf);
            queue.write_buffer(&self.origins_buf, 0, bytemuck::cast_slice(&self.origins));
        } else if !span.is_empty() {
            queue.write_buffer(&self.origins_buf, span.start as u64 * 16, bytemuck::cast_slice(&self.origins[span]));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn card_changed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        origins_layout: &wgpu::BindGroupLayout,
        repo: &Repo,
        layout: &Layout,
        file: usize,
        changed: &[Range<u32>],
    ) {
        let colors = theme::get();
        let (mut low, mut high) = (usize::MAX, 0);
        for range in changed {
            let nodes = &layout.nodes[range.start as usize..range.end as usize];
            let frames: Vec<RectInstance> = nodes.iter().map(|n| frame_of(n, repo, &colors)).collect();
            queue.write_buffer(&self.frames, range.start as u64 * 32, bytemuck::cast_slice(&frames));
            for n in nodes {
                if let NodeKind::File(fi) = n.kind {
                    let slot = self.slots[fi] as usize;
                    self.origins[slot] = origin_of(n);
                    (low, high) = (low.min(slot), high.max(slot + 1));
                }
            }
        }
        self.write_origins(device, queue, origins_layout, low.min(high)..high);
        self.blocks_changed(device, queue, repo, layout, file);
    }

    /// The far-away bars of one card made again (from a grammar's colours, say),
    /// nothing else moved.
    pub fn blocks_changed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        repo: &Repo,
        layout: &Layout,
        file: usize,
    ) {
        let Some(&node) = layout.file_node.get(file) else { return };
        if node == NONE {
            return;
        }
        let n = node as usize;
        let data = blocks_of(&layout.nodes[n], repo, &self.slots);
        let (a, b) = (self.block_start[n], self.block_start[n + 1]);
        if data.len() as u32 == b - a {
            if !data.is_empty() {
                queue.write_buffer(&self.blocks, a as u64 * CODE, bytemuck::cast_slice(&data));
            }
            return;
        }
        let total = self.block_count + data.len() as u32 - (b - a);
        let new = buffer(device, "blocks", total as u64 * CODE, BLOCKS);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("blocks") });
        let mut moves = GpuMoves { queue, encoder: &mut encoder, old: &self.blocks, new: &new };
        splice_blocks(&mut self.block_start, n, &data, &mut moves);
        queue.submit([encoder.finish()]);
        (self.blocks, self.block_count) = (new, total);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn relaid(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        origins_layout: &wgpu::BindGroupLayout,
        repo: &Repo,
        layout: &Layout,
        from: &[u32],
        file: usize,
        made: bool,
    ) {
        match made {
            true => {
                let slot = self.free.pop().unwrap_or(self.origins.len() as u32);
                if slot as usize == self.origins.len() {
                    self.origins.push([0.0; 4]);
                }
                self.slots.insert(file, slot);
            }
            false => self.free.push(self.slots.remove(file)),
        }
        let colors = theme::get();
        let frames: Vec<RectInstance> = layout.nodes.iter().map(|n| frame_of(n, repo, &colors)).collect();
        if (frames.len() * 32) as u64 > self.frames.size() {
            self.frames = buffer(device, "frames", (frames.len() * 32).next_power_of_two() as u64, FRAMES);
        }
        queue.write_buffer(&self.frames, 0, bytemuck::cast_slice(&frames));
        self.frame_count = frames.len() as u32;

        let total: u32 = from
            .iter()
            .enumerate()
            .map(|(i, &was)| match was {
                NONE => blocks_of(&layout.nodes[i], repo, &self.slots).len() as u32,
                was => self.block_start[was as usize + 1] - self.block_start[was as usize],
            })
            .sum();
        let new = buffer(device, "blocks", total as u64 * CODE, BLOCKS);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("blocks") });
        let mut moves = GpuMoves { queue, encoder: &mut encoder, old: &self.blocks, new: &new };
        let slots = &self.slots;
        self.block_start =
            move_blocks(&self.block_start, from, |i| blocks_of(&layout.nodes[i], repo, slots), &mut moves);
        queue.submit([encoder.finish()]);
        (self.blocks, self.block_count) = (new, total);

        let len = self.origins.len();
        self.origins = origins_of(layout, &self.slots, len);
        self.write_origins(device, queue, origins_layout, 0..len);
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
                let (scratch, slots) = (&mut self.scratch, &self.slots);
                layout.visit_visible(view, |_, n| {
                    if let NodeKind::File(fi) = n.kind {
                        let f = &repo.files[fi];
                        for (_, lines) in n.visible_lines(view, f.line_count()) {
                            for c in lines.start / CHUNK_LINES..=(lines.end - 1) / CHUNK_LINES {
                                let toks = tokens.chunk(fi, f, c);
                                let cl = f.chunk_lines(c);
                                let r = lines.start.max(cl.start)..lines.end.min(cl.end);
                                let wrap = n.wrap.as_deref();
                                let slot = slots[fi] as usize;
                                if lod == Lod::Lines {
                                    push_lines(slot, wrap, toks, cl.start, r, scratch);
                                } else {
                                    push_tokens(slot, wrap, toks, cl.start, r, scratch);
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use canvas_core::scan::{scan, ScanOptions};
    use canvas_core::LayoutFilter;

    use super::*;

    struct Mirror {
        old: Vec<CodeInstance>,
        new: Vec<CodeInstance>,
    }

    impl Moves for Mirror {
        fn copy(&mut self, from: u32, to: u32, count: u32) {
            let (from, to, count) = (from as usize, to as usize, count as usize);
            self.new[to..to + count].copy_from_slice(&self.old[from..from + count]);
        }

        fn write(&mut self, at: u32, data: &[CodeInstance]) {
            self.new[at as usize..at as usize + data.len()].copy_from_slice(data);
        }
    }

    fn per_file(
        repo: &Repo,
        layout: &Layout,
        blocks: &[CodeInstance],
        start: &[u32],
        slots: &[u32],
    ) -> HashMap<String, Vec<(String, u32, u32)>> {
        let file_of: HashMap<u32, usize> = slots.iter().enumerate().map(|(fi, &slot)| (slot, fi)).collect();
        let mut out = HashMap::new();
        for (n, node) in layout.nodes.iter().enumerate() {
            let NodeKind::File(fi) = node.kind else { continue };
            let mine = &blocks[start[n] as usize..start[n + 1] as usize];
            let named = mine.iter().map(|b| (repo.files[file_of[&b.file]].rel_path.clone(), b.line_kind, b.col_cols));
            out.insert(repo.files[fi].rel_path.clone(), named.collect());
        }
        out
    }

    fn same_as_built(repo: &Repo, layout: &Layout, blocks: &[CodeInstance], start: &[u32], slots: &[u32]) {
        let built = SceneData::build(repo, layout);
        assert_eq!(start.len(), layout.nodes.len() + 1);
        assert_eq!(*start.last().unwrap() as usize, blocks.len());
        assert_eq!(
            per_file(repo, layout, blocks, start, slots),
            per_file(repo, layout, &built.blocks, &built.block_start, &built.slots)
        );
    }

    #[test]
    fn blocks_moved_on_the_gpu_are_the_ones_built_again() {
        let dir = std::env::temp_dir().join(format!("code-canvas-scene-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let text = |n: usize| (0..n).map(|i| format!("fn f{i}() {{ {} }}\n", "x".repeat(i % 40))).collect::<String>();
        for (path, lines) in
            [("a/one.rs", 40), ("a/two.rs", 90), ("b/three.rs", 17), ("b/c/four.rs", 64), ("top.rs", 5)]
        {
            std::fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
            std::fs::write(dir.join(path), text(lines)).unwrap();
        }
        let opts = ScanOptions::default();
        let filter = LayoutFilter::default();
        let mut repo = scan(&dir, &opts, None);
        let mut layout = Layout::build(&repo);
        let data = SceneData::build(&repo, &layout);
        let (mut blocks, mut start, mut slots) = (data.blocks, data.block_start, data.slots);

        std::fs::write(dir.join("a/two.rs"), text(300)).unwrap();
        let at = repo.file_index("a/two.rs").unwrap();
        repo.reload_file(at, &opts);
        layout.resize_file(&repo, &filter, at).unwrap();
        let n = layout.file_node[at] as usize;
        let fresh = blocks_of(&layout.nodes[n], &repo, &slots);
        let total = blocks.len() + fresh.len() - (start[n + 1] - start[n]) as usize;
        let mut mirror = Mirror { old: blocks, new: vec![CodeInstance::zeroed(); total] };
        splice_blocks(&mut start, n, &fresh, &mut mirror);
        blocks = mirror.new;
        same_as_built(&repo, &layout, &blocks, &start, &slots);

        std::fs::write(dir.join("b/c/new.rs"), text(33)).unwrap();
        let at = repo.add_file("b/c/new.rs", &opts).unwrap();
        let from = layout.insert_file(&repo, &filter, at).unwrap();
        slots.insert(at, slots.len() as u32);
        let mut mirror = Mirror { old: blocks, new: vec![CodeInstance::zeroed(); 100_000] };
        start = move_blocks(&start, &from, |i| blocks_of(&layout.nodes[i], &repo, &slots), &mut mirror);
        blocks = mirror.new;
        blocks.truncate(*start.last().unwrap() as usize);
        same_as_built(&repo, &layout, &blocks, &start, &slots);

        std::fs::remove_file(dir.join("a/one.rs")).unwrap();
        let at = repo.file_index("a/one.rs").unwrap();
        repo.remove_file(at);
        let from = layout.remove_file(&repo, &filter, at, "a/one.rs").unwrap();
        slots.remove(at);
        let mut mirror = Mirror { old: blocks, new: vec![CodeInstance::zeroed(); 100_000] };
        start = move_blocks(&start, &from, |i| blocks_of(&layout.nodes[i], &repo, &slots), &mut mirror);
        blocks = mirror.new;
        blocks.truncate(*start.last().unwrap() as usize);
        same_as_built(&repo, &layout, &blocks, &start, &slots);
        std::fs::remove_dir_all(&dir).ok();
    }
}
