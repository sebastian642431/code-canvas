// Code "minimap" bars: one instance per block of lines, per line, or per
// token depending on the level of detail. Instances are 12 bytes and are
// positioned relative to their file's origin, read from a storage buffer.

struct Camera {
    offset: vec2<f32>,
    zoom: f32,
    alpha: f32,
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> cam: Camera;
@group(0) @binding(1) var<uniform> palette: array<vec4<f32>, 16>;
// x, y: first character of the file; z: x distance between folded columns.
@group(1) @binding(0) var<storage, read> origins: array<vec4<f32>>;

const CHAR_W: f32 = 6.0;
const LINE_H: f32 = 12.0;
const BAR: f32 = 0.72; // fraction of the line height covered by a bar
const FOLD_LINES: f32 = 2048.0; // must match canvas_core::layout::FOLD_LINES

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) file: u32,
    @location(1) line_kind: u32, // line:20 | kind:4 | span:8
    @location(2) col_cols: u32,  // col:16 | cols:16
) -> Out {
    let line = f32(line_kind & 0xfffffu);
    let kind = (line_kind >> 20u) & 0xfu;
    let span = f32(max((line_kind >> 24u) & 0xffu, 1u));
    let col = f32(col_cols & 0xffffu);
    let cols = f32(col_cols >> 16u);

    let o = origins[file];
    let fold = floor(line / FOLD_LINES);
    let row = line - fold * FOLD_LINES;
    let top = o.y + row * LINE_H + LINE_H * (1.0 - BAR) * 0.5;
    let world_size = vec2<f32>(cols * CHAR_W, (span - 1.0) * LINE_H + LINE_H * BAR);
    let p0 = (vec2<f32>(o.x + fold * o.z + col * CHAR_W, top) - cam.offset) * cam.zoom;
    var size = world_size * cam.zoom;
    var color = palette[kind];
    color.a = color.a * min(size.x, 1.0) * min(size.y, 1.0) * cam.alpha;
    size = max(size, vec2<f32>(1.0));

    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let s = p0 + corner * size;
    var out: Out;
    out.clip = vec4<f32>(s.x / cam.viewport.x * 2.0 - 1.0, 1.0 - s.y / cam.viewport.y * 2.0, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    return in.color;
}
