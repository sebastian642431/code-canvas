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
    @location(3) along: f32,
    @location(4) dash: f32,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) side: f32,
    @location(3) half_width: f32,
    @location(4) color: u32,
    @location(5) along: f32,
    @location(6) dash: f32,
    @location(7) arrow: f32,
    @location(8) shift: vec2<f32>,
) -> Out {
    var hw = half_width;
    var lift = vec2<f32>(0.0, 0.0);
    var dashed = dash;
    if (arrow > 0.0) {
        let least = max(dash * 3.2, 6.0);
        let size = clamp(arrow * cam.zoom, least, least * 2.2);
        hw = half_width * size;
        lift = shift * size;
        dashed = 0.0;
    }
    let extent = hw + 1.0;
    let s = (pos - cam.offset) * cam.zoom + lift + normal * side * extent;
    var out: Out;
    out.clip = vec4<f32>(s.x / cam.viewport.x * 2.0 - 1.0, 1.0 - s.y / cam.viewport.y * 2.0, 0.0, 1.0);
    out.color = vec4<f32>(
        f32((color >> 24u) & 0xffu),
        f32((color >> 16u) & 0xffu),
        f32((color >> 8u) & 0xffu),
        f32(color & 0xffu),
    ) / 255.0;
    out.dist = side * extent;
    out.half_width = hw;
    out.along = along * cam.zoom;
    out.dash = dashed;
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    var a = clamp(in.half_width + 0.5 - abs(in.dist), 0.0, 1.0);
    if (in.dash > 0.0) {
        let at = in.along - in.dash * floor(in.along / in.dash);
        let on = in.dash * 0.6;
        a = a * clamp(min(at, on - at) + 0.5, 0.0, 1.0);
    }
    return vec4<f32>(in.color.rgb, in.color.a * a);
}
