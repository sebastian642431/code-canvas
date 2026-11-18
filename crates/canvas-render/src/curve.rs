use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CurveVertex {
    pub pos: [f32; 2],
    pub normal: [f32; 2],
    pub side: f32,
    pub half_width: f32,
    pub color: u32,
    pub along: f32,
    pub dash: f32,
    pub arrow: f32,
    pub shift: [f32; 2],
}

const SEGMENTS: usize = 28;
pub const DASH_PX: f32 = 7.0;
const ARROW_HALF: f32 = 0.42;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stroke {
    pub dashed: bool,
    pub arrow: f32,
}

pub fn control_points(a: [f32; 2], b: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    if dx.abs() > dy.abs() * 0.6 {
        let k = dx * 0.5;
        ([a[0] + k, a[1]], [b[0] - k, b[1]])
    } else {
        let k = (dy.abs() * 0.35).max((dx * dx + dy * dy).sqrt() * 0.25);
        ([a[0] - k, a[1]], [b[0] - k, b[1]])
    }
}

fn bezier(a: [f32; 2], c1: [f32; 2], c2: [f32; 2], b: [f32; 2], t: f32) -> ([f32; 2], [f32; 2]) {
    let u = 1.0 - t;
    let p = |i: usize| u * u * u * a[i] + 3.0 * u * u * t * c1[i] + 3.0 * u * t * t * c2[i] + t * t * t * b[i];
    let d = |i: usize| 3.0 * u * u * (c1[i] - a[i]) + 6.0 * u * t * (c2[i] - c1[i]) + 3.0 * t * t * (b[i] - c2[i]);
    ([p(0), p(1)], [d(0), d(1)])
}

pub fn curve_point(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    let (c1, c2) = control_points(a, b);
    bezier(a, c1, c2, b, t).0
}

pub fn push_curve(out: &mut Vec<CurveVertex>, a: [f32; 2], b: [f32; 2], color: u32, width_px: f32) {
    push_edge(out, a, b, color, width_px, Stroke::default());
}

pub fn push_edge(out: &mut Vec<CurveVertex>, a: [f32; 2], b: [f32; 2], color: u32, width_px: f32, stroke: Stroke) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    if dx * dx + dy * dy < 1e-6 {
        return;
    }
    let (c1, c2) = control_points(a, b);
    let half_width = width_px * 0.5;
    let dash = if stroke.dashed { DASH_PX * width_px.max(1.0).sqrt() } else { 0.0 };
    let mut prev: Option<([f32; 2], [f32; 2], f32)> = None;
    let mut last_normal = [0.0f32, 1.0];
    for i in 0..=SEGMENTS {
        let (p, d) = bezier(a, c1, c2, b, i as f32 / SEGMENTS as f32);
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let n = if len > 1e-6 { [-d[1] / len, d[0] / len] } else { last_normal };
        last_normal = n;
        let mut along = 0.0;
        if let Some((pp, pn, pa)) = prev {
            along = pa + ((p[0] - pp[0]).powi(2) + (p[1] - pp[1]).powi(2)).sqrt();
            let v = |pos: [f32; 2], normal: [f32; 2], side: f32, along: f32| CurveVertex {
                pos,
                normal,
                side,
                half_width,
                color,
                along,
                dash,
                arrow: 0.0,
                shift: [0.0, 0.0],
            };
            out.extend_from_slice(&[
                v(pp, pn, -1.0, pa),
                v(pp, pn, 1.0, pa),
                v(p, n, -1.0, along),
                v(p, n, -1.0, along),
                v(pp, pn, 1.0, pa),
                v(p, n, 1.0, along),
            ]);
        }
        prev = Some((p, n, along));
    }
    if stroke.arrow > 0.0 {
        let d = [b[0] - c2[0], b[1] - c2[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let dir = if len > 1e-6 {
            [d[0] / len, d[1] / len]
        } else {
            let whole = (dx * dx + dy * dy).sqrt();
            [dx / whole, dy / whole]
        };
        push_arrow(out, b, dir, color, width_px, stroke.arrow);
    }
}

fn push_arrow(out: &mut Vec<CurveVertex>, tip: [f32; 2], dir: [f32; 2], color: u32, width_px: f32, arrow: f32) {
    let normal = [-dir[1], dir[0]];
    let back = [-dir[0], -dir[1]];
    let v = |shift: [f32; 2], side: f32, half_width: f32| CurveVertex {
        pos: tip,
        normal,
        side,
        half_width,
        color,
        along: 0.0,
        dash: width_px,
        arrow,
        shift,
    };
    out.extend_from_slice(&[
        v(back, -1.0, ARROW_HALF),
        v(back, 1.0, ARROW_HALF),
        v([0.0, 0.0], -1.0, 0.0),
        v([0.0, 0.0], -1.0, 0.0),
        v(back, 1.0, ARROW_HALF),
        v([0.0, 0.0], 1.0, 0.0),
    ]);
}

pub fn arrow_px(arrow: f32, zoom: f32, width_px: f32) -> f32 {
    let least = (width_px * 3.2).max(6.0);
    (arrow * zoom).clamp(least, least * 2.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_is_a_gapless_ribbon_from_a_to_b() {
        let mut v = Vec::new();
        push_curve(&mut v, [0.0, 0.0], [100.0, 40.0], 0xffffffff, 2.0);
        assert_eq!(v.len(), SEGMENTS * 6);
        assert_eq!(v[0].pos, [0.0, 0.0]);
        let end = v.last().unwrap().pos;
        assert!((end[0] - 100.0).abs() < 1e-3 && (end[1] - 40.0).abs() < 1e-3);
        for q in 1..SEGMENTS {
            assert_eq!(v[q * 6].pos, v[q * 6 - 1].pos);
            assert_eq!(v[q * 6].normal, v[q * 6 - 1].normal);
            assert_eq!(v[q * 6].along, v[q * 6 - 1].along);
        }
        assert!(v.iter().all(|v| v.dash == 0.0 && v.arrow == 0.0));
        let mut e = Vec::new();
        push_curve(&mut e, [5.0, 5.0], [5.0, 5.0], 0, 1.0);
        assert!(e.is_empty());
    }

    #[test]
    fn an_edge_can_be_dashed_and_end_in_an_arrowhead() {
        let mut v = Vec::new();
        push_edge(&mut v, [0.0, 0.0], [100.0, 0.0], 0xff00ffff, 2.0, Stroke { dashed: true, arrow: 10.0 });
        assert_eq!(v.len(), SEGMENTS * 6 + 6);
        let (ribbon, head) = v.split_at(SEGMENTS * 6);
        assert!(ribbon.iter().all(|v| v.dash > 0.0 && v.arrow == 0.0));
        let length = ribbon.last().unwrap().along;
        assert!((length - 100.0).abs() < 0.5, "along measures the curve: {length}");
        assert!(head.iter().all(|v| v.pos == [100.0, 0.0] && v.arrow == 10.0), "every corner hangs off the tip");
        assert!(head.iter().any(|v| v.shift == [-1.0, 0.0]), "the head points the way the curve arrives");
        assert!(head.iter().filter(|v| v.shift == [0.0, 0.0]).all(|v| v.half_width == 0.0));
        assert_eq!(curve_point([0.0, 0.0], [100.0, 0.0], 0.5), [50.0, 0.0]);
    }

    #[test]
    fn arrowheads_are_never_huge_nor_lost() {
        assert_eq!(arrow_px(10.0, 0.001, 1.25), 6.0);
        assert_eq!(arrow_px(10.0, 1000.0, 1.25), 6.0 * 2.2);
        assert_eq!(arrow_px(10.0, 0.75, 1.25), 7.5);
        assert!(arrow_px(10.0, 0.001, 6.0) > 12.0, "a thick bundle gets a head wider than itself");
    }
}
