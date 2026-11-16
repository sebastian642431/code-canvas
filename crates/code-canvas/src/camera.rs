use canvas_core::Rect;
use canvas_render::View;

pub const MIN_ZOOM: f32 = 0.0002;
pub const MAX_ZOOM: f32 = 12.0;

/// Camera of one page. `center` is the world point shown at the centre of
/// the window; `zoom` is physical pixels per world unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: [f32; 2],
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { center: [0.0, 0.0], zoom: 1.0 }
    }
}

impl Camera {
    pub fn view(&self, viewport: [f32; 2]) -> View {
        View {
            offset: [self.center[0] - viewport[0] * 0.5 / self.zoom, self.center[1] - viewport[1] * 0.5 / self.zoom],
            zoom: self.zoom,
            viewport,
        }
    }

    /// Zoom by `factor` keeping the world point under `screen` fixed.
    pub fn zoom_at(&mut self, screen: [f32; 2], viewport: [f32; 2], factor: f32) {
        let anchor = self.view(viewport).to_world(screen);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.center = [
            anchor[0] - (screen[0] - viewport[0] * 0.5) / self.zoom,
            anchor[1] - (screen[1] - viewport[1] * 0.5) / self.zoom,
        ];
    }

    pub fn pan_pixels(&mut self, dx: f32, dy: f32) {
        self.center[0] -= dx / self.zoom;
        self.center[1] -= dy / self.zoom;
    }

    /// Camera showing `rect` inside the screen area `area` (physical pixels)
    /// of a window of size `viewport`.
    pub fn fit(rect: Rect, area: Rect, viewport: [f32; 2], margin_px: f32) -> Camera {
        let aw = (area.w - margin_px * 2.0).max(10.0);
        let ah = (area.h - margin_px * 2.0).max(10.0);
        let zoom = (aw / rect.w.max(1.0)).min(ah / rect.h.max(1.0)).clamp(MIN_ZOOM, MAX_ZOOM);
        let c = rect.center();
        let ac = area.center();
        Camera { center: [c[0] - (ac[0] - viewport[0] * 0.5) / zoom, c[1] - (ac[1] - viewport[1] * 0.5) / zoom], zoom }
    }

    /// Smooth step towards `target` (zoom interpolated in log space).
    /// Returns true when it arrived.
    pub fn approach(&mut self, target: &Camera, dt: f32) -> bool {
        let t = 1.0 - (-dt * 14.0).exp();
        let lz = self.zoom.ln() + (target.zoom.ln() - self.zoom.ln()) * t;
        self.zoom = lz.exp();
        self.center[0] += (target.center[0] - self.center[0]) * t;
        self.center[1] += (target.center[1] - self.center[1]) * t;
        let close_zoom = (self.zoom / target.zoom - 1.0).abs() < 0.002;
        let dx = (self.center[0] - target.center[0]) * self.zoom;
        let dy = (self.center[1] - target.center[1]) * self.zoom;
        if close_zoom && dx.abs() < 0.5 && dy.abs() < 0.5 {
            *self = *target;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_anchor() {
        let vp = [800.0, 600.0];
        let mut c = Camera { center: [100.0, 50.0], zoom: 1.0 };
        let p = [200.0, 150.0];
        let before = c.view(vp).to_world(p);
        c.zoom_at(p, vp, 2.5);
        let after = c.view(vp).to_world(p);
        assert!((before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3);
    }

    #[test]
    fn fit_contains_rect() {
        let vp = [1000.0, 800.0];
        let area = Rect::new(0.0, 40.0, 1000.0, 730.0);
        let r = Rect::new(-500.0, 200.0, 3000.0, 900.0);
        let c = Camera::fit(r, area, vp, 20.0);
        let v = c.view(vp);
        let tl = v.to_screen([r.x, r.y]);
        let br = v.to_screen([r.right(), r.bottom()]);
        assert!(tl[0] >= area.x - 0.5 && tl[1] >= area.y - 0.5);
        assert!(br[0] <= area.right() + 0.5 && br[1] <= area.bottom() + 0.5);
    }
}
