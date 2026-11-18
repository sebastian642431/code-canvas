use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use canvas_core::{SourceFile, TokenCache, Tokens, Wrap as LineWrap, CHAR_W, CHUNK_LINES, LINE_H};
use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea, TextAtlas,
    TextBounds, TextRenderer, Viewport, Weight, Wrap,
};

use crate::theme;

pub const NEW_CHUNKS_PER_FRAME: usize = 12;
const EVICT_AFTER: u64 = 300;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkKey {
    pub page: u32,
    pub file: u32,
    pub chunk: u32,
}

pub struct CodeText<'a> {
    pub page: u32,
    pub file_index: usize,
    pub file: &'a SourceFile,
    pub lines: std::ops::Range<usize>,
    pub left: f32,
    pub top: f32,
    pub zoom: f32,
    pub clip: [i32; 4],
    pub numbers: Option<[i32; 3]>,
    pub wrap: Option<&'a LineWrap>,
}

const SPACES: &str = "                                                                                ";

fn cuts_of(file: &SourceFile, lines: std::ops::Range<usize>, wrap: Option<&LineWrap>) -> Vec<(usize, usize)> {
    let mut cuts = Vec::new();
    let Some(wrap) = wrap else { return cuts };
    for line in lines {
        let breaks = wrap.breaks(line);
        if breaks.is_empty() {
            continue;
        }
        let base = file.line_starts()[line] as usize;
        let mut next = 0;
        for (col, (byte, _)) in file.line(line).char_indices().enumerate() {
            if breaks[next][0] as usize == col {
                cuts.push((base + byte, (breaks[next][1] as usize).min(SPACES.len())));
                next += 1;
                if next == breaks.len() {
                    break;
                }
            }
        }
    }
    cuts
}

struct Spans<'t, 'a> {
    text: &'t str,
    cuts: Vec<(usize, usize)>,
    next: usize,
    plain: Attrs<'a>,
    out: Vec<(&'t str, Attrs<'a>)>,
}

impl<'t, 'a> Spans<'t, 'a> {
    fn push(&mut self, mut from: usize, to: usize, attrs: Attrs<'a>) {
        while let Some(&(at, indent)) = self.cuts.get(self.next).filter(|(at, _)| *at < to) {
            let at = at.max(from);
            if at > from {
                self.out.push((&self.text[from..at], attrs.clone()));
            }
            self.out.push(("\n", self.plain.clone()));
            if indent > 0 {
                self.out.push((&SPACES[..indent], self.plain.clone()));
            }
            (from, self.next) = (at, self.next + 1);
        }
        if to > from {
            self.out.push((&self.text[from..to], attrs));
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StripRow {
    pub left: f32,
    pub top: f32,
    pub bounds: [i32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct CodeStrip<'a> {
    pub page: u32,
    pub file_index: usize,
    pub line: usize,
    pub zoom: f32,
    pub rows: &'a [StripRow],
}

pub struct Label<'a> {
    pub text: &'a str,
    pub left: f32,
    pub top: f32,
    pub size: f32,
    pub color: u32,
    pub max_w: f32,
    pub bold: bool,
}

struct Cached {
    buffer: Buffer,
    last_used: u64,
}

pub struct TextLayer {
    pub font_system: FontSystem,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    world: TextRenderer,
    ui: TextRenderer,
    chunks: HashMap<ChunkKey, Cached>,
    numbers: HashMap<ChunkKey, Cached>,
    labels: HashMap<u64, Cached>,
    frame: u64,
    code_font_size: f32,
    pub pending: bool,
    pub chunks_shaped_last_frame: usize,
}

fn label_key(text: &str, size: f32, color: u32, bold: bool) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    size.to_bits().hash(&mut h);
    color.hash(&mut h);
    bold.hash(&mut h);
    h.finish()
}

impl TextLayer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let mut font_system = FontSystem::new();
        #[cfg(target_os = "windows")]
        font_system.db_mut().set_monospace_family("Consolas");
        #[cfg(target_os = "macos")]
        font_system.db_mut().set_monospace_family("Menlo");

        let probe = "MMMMMMMMMMMMMMMMMMMM";
        let mut b = Buffer::new(&mut font_system, Metrics::new(10.0, 12.0));
        b.set_text(probe, &Attrs::new().family(Family::Monospace), Shaping::Basic, None);
        b.shape_until_scroll(&mut font_system, false);
        let width = b.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        let advance = if width > 0.0 { width / probe.len() as f32 } else { 6.0 };
        let code_font_size = 10.0 * CHAR_W / advance;
        log::info!("monospace advance {advance:.3} at 10px -> code font size {code_font_size:.2}");

        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::with_color_mode(device, queue, &cache, format, glyphon::ColorMode::Web);
        let world = TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let ui = TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        Self {
            font_system,
            swash: SwashCache::new(),
            viewport,
            atlas,
            world,
            ui,
            chunks: HashMap::new(),
            numbers: HashMap::new(),
            labels: HashMap::new(),
            frame: 0,
            code_font_size,
            pending: false,
            chunks_shaped_last_frame: 0,
        }
    }

    fn shape_chunk(&mut self, file: &SourceFile, chunk: usize, tokens: &Tokens, wrap: Option<&LineWrap>) -> Buffer {
        let first = chunk * CHUNK_LINES;
        let last = ((chunk + 1) * CHUNK_LINES).min(file.line_count());
        let start = file.line_starts()[first] as usize;
        let end = file.line_starts().get(last).map(|&e| e as usize - 1).unwrap_or(file.text().len());

        let colors = theme::get().token_colors();
        let default = Attrs::new().family(Family::Monospace).color(theme::glyph_color(colors[8]));
        let cuts = cuts_of(file, first..last, wrap);
        let mut spans = Spans { text: file.text(), cuts, next: 0, plain: default.clone(), out: Vec::new() };
        let mut cursor = start;
        for line in first..last {
            for t in tokens.line(line - first) {
                let (s, e) = (t.start as usize, t.end() as usize);
                if s > cursor {
                    spans.push(cursor, s, default.clone());
                }
                let color = colors[theme::kind_index(t.kind) as usize];
                spans.push(s, e, default.clone().color(theme::glyph_color(color)));
                cursor = e;
            }
        }
        if end > cursor {
            spans.push(cursor, end, default.clone());
        }

        let mut buffer = Buffer::new_empty(Metrics::new(self.code_font_size, LINE_H));
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        buffer.set_rich_text(spans.out, &default, Shaping::Basic, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }

    fn shape_numbers(&mut self, file: &SourceFile, chunk: usize, wrap: Option<&LineWrap>) -> Buffer {
        let first = chunk * CHUNK_LINES;
        let width = canvas_core::layout::GUTTER_COLS as usize - 1;
        let mut text = String::with_capacity(CHUNK_LINES * (width + 1));
        for row in first..first + CHUNK_LINES {
            let number = match &file.numbers {
                Some(own) => own.get(row).copied().unwrap_or(0),
                None => row as u32 + 1,
            };
            match number {
                0 => text.push('\n'),
                n => text.push_str(&format!("{n:>width$}\n")),
            }
            for _ in 1..wrap.map_or(1, |wrap| wrap.rows(row)) {
                text.push('\n');
            }
        }
        let mut buffer = Buffer::new_empty(Metrics::new(self.code_font_size, LINE_H));
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        buffer.set_text(&text, &Attrs::new().family(Family::Monospace), Shaping::Basic, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }

    fn label_buffer(&mut self, l: &Label) -> u64 {
        let key = label_key(l.text, l.size, l.color, l.bold);
        let frame = self.frame;
        if let Some(c) = self.labels.get_mut(&key) {
            c.last_used = frame;
            return key;
        }
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(l.size, (l.size * 1.3).ceil()));
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        let attrs = Attrs::new()
            .family(Family::SansSerif)
            .weight(if l.bold { Weight::SEMIBOLD } else { Weight::NORMAL })
            .color(theme::glyph_color(l.color));
        buffer.set_text(l.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        self.labels.insert(key, Cached { buffer, last_used: frame });
        key
    }

    pub fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        let l = Label { text, left: 0.0, top: 0.0, size, color: theme::get().ui_text, max_w: f32::MAX, bold };
        let key = self.label_buffer(&l);
        self.labels[&key].buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resolution: [u32; 2],
        code: &[CodeText],
        mut tokens: Option<&mut TokenCache>,
        world_labels: &[Label],
        ui_labels: &[Label],
        strip: Option<&CodeStrip>,
    ) {
        self.frame += 1;
        let frame = self.frame;
        self.viewport.update(queue, Resolution { width: resolution[0], height: resolution[1] });

        let mut budget = NEW_CHUNKS_PER_FRAME;
        self.pending = false;
        let mut areas_code: Vec<(ChunkKey, f32, f32, f32, TextBounds)> = Vec::new();
        let mut areas_numbers: Vec<(ChunkKey, f32, f32, f32, TextBounds)> = Vec::new();
        for c in code {
            if c.lines.is_empty() {
                continue;
            }
            let first_chunk = c.lines.start / CHUNK_LINES;
            let last_chunk = (c.lines.end - 1) / CHUNK_LINES;
            for chunk in first_chunk..=last_chunk {
                let row = c.wrap.map_or(chunk * CHUNK_LINES, |wrap| wrap.row(chunk * CHUNK_LINES) as usize);
                let top = c.top + row as f32 * LINE_H * c.zoom;
                if let Some([left, right, bottom]) = c.numbers.filter(|[left, right, _]| right > left) {
                    let own = c.file.numbers.is_some() || c.wrap.is_some();
                    let key = ChunkKey {
                        page: if own { c.page } else { u32::MAX },
                        file: if own { c.file_index as u32 } else { u32::MAX },
                        chunk: chunk as u32,
                    };
                    if !self.numbers.contains_key(&key) {
                        let buffer = self.shape_numbers(c.file, chunk, c.wrap);
                        self.numbers.insert(key, Cached { buffer, last_used: frame });
                    }
                    if let Some(shaped) = self.numbers.get_mut(&key) {
                        shaped.last_used = frame;
                    }
                    let bounds = TextBounds { left, top: c.clip[1], right, bottom };
                    areas_numbers.push((key, left as f32, top, c.zoom, bounds));
                }
                let key = ChunkKey { page: c.page, file: c.file_index as u32, chunk: chunk as u32 };
                if let Some(e) = self.chunks.get_mut(&key) {
                    e.last_used = frame;
                } else if let (true, Some(cache)) = (budget > 0, tokens.as_deref_mut()) {
                    budget -= 1;
                    let toks = cache.chunk(c.file_index, c.file, chunk);
                    let buffer = self.shape_chunk(c.file, chunk, toks, c.wrap);
                    self.chunks.insert(key, Cached { buffer, last_used: frame });
                } else {
                    self.pending = true;
                    continue;
                }
                let bounds = TextBounds { left: c.clip[0], top: c.clip[1], right: c.clip[2], bottom: c.clip[3] };
                areas_code.push((key, c.left, top, c.zoom, bounds));
            }
        }
        self.chunks_shaped_last_frame = NEW_CHUNKS_PER_FRAME - budget;

        let world_keys: Vec<u64> = world_labels.iter().map(|l| self.label_buffer(l)).collect();
        let ui_keys: Vec<u64> = ui_labels.iter().map(|l| self.label_buffer(l)).collect();

        fn label_area<'b>(buf: &'b Buffer, l: &Label) -> TextArea<'b> {
            let right = if l.max_w.is_finite() { (l.left + l.max_w) as i32 } else { i32::MAX };
            TextArea {
                buffer: buf,
                left: l.left,
                top: l.top,
                scale: 1.0,
                bounds: TextBounds {
                    left: l.left.floor() as i32,
                    top: l.top.floor() as i32,
                    right,
                    bottom: (l.top + l.size * 1.5) as i32,
                },
                default_color: theme::glyph_color(l.color),
                custom_glyphs: &[],
            }
        }

        let code_color = theme::get().tok_ident;
        let mut world_areas: Vec<TextArea> = Vec::with_capacity(areas_code.len() + world_labels.len());
        for (key, left, top, scale, bounds) in &areas_code {
            world_areas.push(TextArea {
                buffer: &self.chunks[key].buffer,
                left: *left,
                top: *top,
                scale: *scale,
                bounds: *bounds,
                default_color: theme::glyph_color(code_color),
                custom_glyphs: &[],
            });
        }
        let numbers_color = theme::glyph_color(theme::mix(theme::get().ui_text_dim, theme::get().card_fill, 0.3));
        for (key, left, top, scale, bounds) in &areas_numbers {
            world_areas.push(TextArea {
                buffer: &self.numbers[key].buffer,
                left: *left,
                top: *top,
                scale: *scale,
                bounds: *bounds,
                default_color: numbers_color,
                custom_glyphs: &[],
            });
        }
        for (l, k) in world_labels.iter().zip(&world_keys) {
            world_areas.push(label_area(&self.labels[k].buffer, l));
        }
        let mut ui_areas: Vec<TextArea> =
            ui_labels.iter().zip(&ui_keys).map(|(l, k)| label_area(&self.labels[k].buffer, l)).collect();
        if let Some(s) = strip {
            let chunk = s.line / CHUNK_LINES;
            let key = ChunkKey { page: s.page, file: s.file_index as u32, chunk: chunk as u32 };
            if let Some(shaped) = self.chunks.get(&key) {
                let above = (s.line - chunk * CHUNK_LINES) as f32 * LINE_H * s.zoom;
                for row in s.rows {
                    let [left, top, right, bottom] = row.bounds;
                    ui_areas.push(TextArea {
                        buffer: &shaped.buffer,
                        left: row.left,
                        top: row.top - above,
                        scale: s.zoom,
                        bounds: TextBounds { left, top, right, bottom },
                        default_color: theme::glyph_color(code_color),
                        custom_glyphs: &[],
                    });
                }
            }
        }

        if let Err(e) = self.world.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            world_areas,
            &mut self.swash,
        ) {
            log::warn!("text prepare (world): {e:?}");
        }
        if let Err(e) = self.ui.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            ui_areas,
            &mut self.swash,
        ) {
            log::warn!("text prepare (ui): {e:?}");
        }
    }

    pub(crate) fn render_world(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Err(e) = self.world.render(&self.atlas, &self.viewport, pass) {
            log::warn!("text render: {e:?}");
        }
    }

    pub(crate) fn render_ui(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Err(e) = self.ui.render(&self.atlas, &self.viewport, pass) {
            log::warn!("text render: {e:?}");
        }
    }

    pub(crate) fn end_frame(&mut self) {
        self.atlas.trim();
        if self.frame.is_multiple_of(60) {
            let frame = self.frame;
            self.chunks.retain(|_, c| frame - c.last_used < EVICT_AFTER);
            self.numbers.retain(|_, c| frame - c.last_used < EVICT_AFTER);
            self.labels.retain(|_, c| frame - c.last_used < EVICT_AFTER);
        }
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.numbers.clear();
        self.labels.clear();
    }

    pub fn drop_page(&mut self, page: u32) {
        self.chunks.retain(|k, _| k.page != page);
        self.numbers.retain(|k, _| k.page != page);
    }

    pub fn cached_chunks(&self) -> usize {
        self.chunks.len()
    }
}
