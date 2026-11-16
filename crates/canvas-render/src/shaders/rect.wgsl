// Instanced rounded rectangles with an anti-aliased border.
// Used for directory boxes, file cards, highlights and the UI chrome.

struct Camera {
    offset: vec2<f32>,   // world position at the top-left of the screen
    zoom: f32,           // physical pixels per world unit
    alpha: f32,
    viewport: vec2<f32>, // physical pixels
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> cam: Camera;

struct Inst {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) fill: u32,
    @location(3) border: u32,
    @location(4) radius: f32,    // pixels
    @location(5) border_w: f32,  // pixels
};

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec3<f32>, // radius, border width, coverage
};

fn unpack(c: u32) -> vec4<f32> {
    return vec4<f32>(
        f32((c >> 24u) & 0xffu),
        f32((c >> 16u) & 0xffu),
        f32((c >> 8u) & 0xffu),
        f32(c & 0xffu),
    ) / 255.0;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Inst) -> Out {
    let size_px = inst.size * cam.zoom;
    // Rectangles thinner than a pixel stay one pixel wide but fade out.
    let coverage = min(size_px.x, 1.0) * min(size_px.y, 1.0);
    let half = max(size_px * 0.5, vec2<f32>(0.5));
    let center = (inst.pos + inst.size * 0.5 - cam.offset) * cam.zoom;
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u)) * 2.0 - 1.0;
    let local = corner * (half + 1.0); // +1px for anti-aliasing
    let s = center + local;

    var out: Out;
    out.clip = vec4<f32>(s.x / cam.viewport.x * 2.0 - 1.0, 1.0 - s.y / cam.viewport.y * 2.0, 0.0, 1.0);
    out.local = local;
    out.half = half;
    out.fill = unpack(inst.fill);
    out.border = unpack(inst.border);
    out.params = vec3<f32>(inst.radius, inst.border_w, coverage * cam.alpha);
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let r = min(in.params.x, min(in.half.x, in.half.y));
    let q = abs(in.local) - in.half + r;
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
    let inside = clamp(0.5 - d, 0.0, 1.0);
    var color = in.fill;
    if (in.params.y > 0.0) {
        let t = clamp(d + in.params.y + 0.5, 0.0, 1.0);
        color = mix(in.fill, in.border, t);
    }
    let a = color.a * inside * in.params.z;
    if (a <= 0.001) {
        discard;
    }
    return vec4<f32>(color.rgb, a);
}
