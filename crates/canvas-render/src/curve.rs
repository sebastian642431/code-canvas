//! Relation curves: cubic Béziers tessellated on the CPU into a ribbon.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CurveVertex {
    pub pos: [f32; 2],
    pub normal: [f32; 2],
    /// -1 or +1: which side of the centre line.
    pub side: f32,
    /// Half of the line width, in pixels.
    pub half_width: f32,
    pub color: u32,
}

const SEGMENTS: usize = 28;

/// Control points for a curve from `a` to `b` (world units): a horizontal
/// S when the ends are side by side, an arc bulging to the left when they
/// are stacked (typical for two places in the same file or column).
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

/// Append the triangles of a curve from `a` to `b` to `out`.
pub fn push_curve(out: &mut Vec<CurveVertex>, a: [f32; 2], b: [f32; 2], color: u32, width_px: f32) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    if dx * dx + dy * dy < 1e-6 {
        return;
    }
    let (c1, c2) = control_points(a, b);
    let half_width = width_px * 0.5;
    let mut prev: Option<([f32; 2], [f32; 2])> = None;
    let mut last_normal = [0.0f32, 1.0];
    for i in 0..=SEGMENTS {
        let (p, d) = bezier(a, c1, c2, b, i as f32 / SEGMENTS as f32);
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let n = if len > 1e-6 { [-d[1] / len, d[0] / len] } else { last_normal };
        last_normal = n;
        if let Some((pp, pn)) = prev {
            let v = |pos: [f32; 2], normal: [f32; 2], side: f32| CurveVertex { pos, normal, side, half_width, color };
            out.extend_from_slice(&[
                v(pp, pn, -1.0),
                v(pp, pn, 1.0),
                v(p, n, -1.0),
                v(p, n, -1.0),
                v(pp, pn, 1.0),
                v(p, n, 1.0),
            ]);
        }
        prev = Some((p, n));
    }
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
        // Neighbouring quads share their edge exactly: no gaps, no overlap.
        for q in 1..SEGMENTS {
            assert_eq!(v[q * 6].pos, v[q * 6 - 1].pos);
            assert_eq!(v[q * 6].normal, v[q * 6 - 1].normal);
        }
        // A degenerate curve produces nothing.
        let mut e = Vec::new();
        push_curve(&mut e, [5.0, 5.0], [5.0, 5.0], 0, 1.0);
        assert!(e.is_empty());
    }
}
