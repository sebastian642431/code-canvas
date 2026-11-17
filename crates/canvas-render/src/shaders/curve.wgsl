struct Camera {
    offset: vec2<f32>,
    zoom: f32,
    alpha: f32,
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> cam: Camera;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) dist: f32,
    @location(2) half_width: f32,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) side: f32,
    @location(3) half_width: f32,
    @location(4) color: u32,
) -> Out {
    let extent = half_width + 1.0;
    let s = (pos - cam.offset) * cam.zoom + normal * side * extent;
    var out: Out;
    out.clip = vec4<f32>(s.x / cam.viewport.x * 2.0 - 1.0, 1.0 - s.y / cam.viewport.y * 2.0, 0.0, 1.0);
    out.color = vec4<f32>(
        f32((color >> 24u) & 0xffu),
        f32((color >> 16u) & 0xffu),
        f32((color >> 8u) & 0xffu),
        f32(color & 0xffu),
    ) / 255.0;
    out.dist = side * extent;
    out.half_width = half_width;
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let a = clamp(in.half_width + 0.5 - abs(in.dist), 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * a);
}
