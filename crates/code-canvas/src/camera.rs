use canvas_core::Rect;
use canvas_render::View;

pub const MIN_ZOOM: f32 = 0.0002;
pub const MAX_ZOOM: f32 = 12.0;

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

pub fn origins(extents: &[Option<Rect>]) -> Vec<[f32; 2]> {
    let widest = extents.iter().flatten().map(|extent| extent.w).fold(0.0f32, f32::max);
    let gap = (widest * 0.06).max(240.0);
    let mut left = 0.0;
    extents
        .iter()
        .map(|extent| match extent {
            Some(extent) => {
                let origin = [left - extent.x, -extent.y];
                left += extent.w + gap;
                origin
            }
            None => [left, 0.0],
        })
        .collect()
}

impl Camera {
    pub fn seen_from(&self, own_origin: [f32; 2], other_origin: [f32; 2]) -> Camera {
        Camera {
            center: [
                self.center[0] + own_origin[0] - other_origin[0],
                self.center[1] + own_origin[1] - other_origin[1],
            ],
            zoom: self.zoom,
        }
    }

    pub fn view(&self, viewport: [f32; 2]) -> View {
        View {
            offset: [self.center[0] - viewport[0] * 0.5 / self.zoom, self.center[1] - viewport[1] * 0.5 / self.zoom],
            zoom: self.zoom,
            viewport,
        }
    }

    pub fn zoom_at(&mut self, screen: [f32; 2], viewport: [f32; 2], factor: f32) {
        let anchor = self.view(viewport).to_world(screen);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.center = [
            anchor[0] - (screen[0] - viewport[0] * 0.5) / self.zoom,
            anchor[1] - (screen[1] - viewport[1] * 0.5) / self.zoom,
        ];
    }

    pub fn rested(&self, screen: [f32; 2], viewport: [f32; 2], line: f32) -> Option<Camera> {
        const READABLE: f32 = 6.0;
        let tall = line * self.zoom;
        if tall < READABLE {
            return None;
        }
        let mut at = *self;
        at.zoom_at(screen, viewport, tall.round() / tall);
        let (offset, zoom) = (at.view(viewport).offset, at.zoom);
        for (centre, offset) in at.center.iter_mut().zip(offset) {
            *centre += (offset * zoom).round() / zoom - offset;
        }
        let moved = (at.zoom - self.zoom).abs() > self.zoom * 1.0e-4
            || (0..2).any(|axis| ((at.center[axis] - self.center[axis]) * at.zoom).abs() > 0.05);
        moved.then_some(at)
    }

    pub fn keep_middle(&mut self, old_middle: [f32; 2], old_viewport: [f32; 2], middle: [f32; 2], viewport: [f32; 2]) {
        for axis in 0..2 {
            let moved = (old_middle[axis] - old_viewport[axis] * 0.5) - (middle[axis] - viewport[axis] * 0.5);
            self.center[axis] += moved / self.zoom;
        }
    }

    pub fn pan_pixels(&mut self, dx: f32, dy: f32) {
        self.center[0] -= dx / self.zoom;
        self.center[1] -= dy / self.zoom;
    }

    pub fn fit(rect: Rect, area: Rect, viewport: [f32; 2], margin_px: f32) -> Camera {
        let aw = (area.w - margin_px * 2.0).max(10.0);
        let ah = (area.h - margin_px * 2.0).max(10.0);
        let zoom = (aw / rect.w.max(1.0)).min(ah / rect.h.max(1.0)).clamp(MIN_ZOOM, MAX_ZOOM);
        let c = rect.center();
        let ac = area.center();
        Camera { center: [c[0] - (ac[0] - viewport[0] * 0.5) / zoom, c[1] - (ac[1] - viewport[1] * 0.5) / zoom], zoom }
    }

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
    fn a_zoom_rests_where_lines_are_whole_pixels_tall() {
        let vp = [800.0, 600.0];
        let camera = Camera { center: [103.37, 51.9], zoom: 1.2345 };
        let rested = camera.rested([200.0, 150.0], vp, 13.0).expect("it was between pixels");
        let tall = 13.0 * rested.zoom;
        assert!((tall - tall.round()).abs() < 1e-3, "{tall}");
        for at in rested.view(vp).offset.map(|offset| offset * rested.zoom) {
            assert!((at - at.round()).abs() < 1e-2, "{at}");
        }
        assert!(rested.rested([200.0, 150.0], vp, 13.0).is_none(), "and it stays there");
        assert!(Camera { center: [0.0, 0.0], zoom: 0.1 }.rested([0.0, 0.0], vp, 13.0).is_none(), "nothing to read");
    }

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
    fn the_middle_of_the_pane_stays_when_the_window_or_a_panel_changes() {
        let mut c = Camera { center: [100.0, 50.0], zoom: 2.0 };
        let (wide, small) = ([1706.0, 920.0], [1100.0, 700.0]);
        let (whole, beside_a_panel) = ([853.0, 478.0], [722.0, 368.0]);
        let seen = c.view(wide).to_world(whole);
        c.keep_middle(whole, wide, [550.0, 368.0], small);
        let after_resize = c.view(small).to_screen(seen);
        assert!((after_resize[0] - 550.0).abs() < 1e-3 && (after_resize[1] - 368.0).abs() < 1e-3);
        c.keep_middle([550.0, 368.0], small, beside_a_panel, small);
        let after_panel = c.view(small).to_screen(seen);
        assert!((after_panel[0] - 722.0).abs() < 1e-3 && (after_panel[1] - 368.0).abs() < 1e-3);
    }

    #[test]
    fn folders_sit_side_by_side_without_overlapping() {
        let extents = [Some(Rect::new(10.0, 20.0, 1000.0, 500.0)), None, Some(Rect::new(-5.0, 0.0, 4000.0, 900.0))];
        let at = origins(&extents);
        let placed = |i: usize| {
            let extent = extents[i].unwrap();
            Rect::new(extent.x + at[i][0], extent.y + at[i][1], extent.w, extent.h)
        };
        let (first, last) = (placed(0), placed(2));
        assert_eq!([first.x, first.y, last.y], [0.0, 0.0, 0.0]);
        assert_eq!(last.x, first.right() + 240.0);
        assert!(!first.intersects(&last));
    }

    #[test]
    fn a_point_is_at_the_same_place_for_every_folder() {
        let vp = [800.0, 600.0];
        let (own, other) = ([0.0, 0.0], [1240.0, -30.0]);
        let leader = Camera { center: [300.0, 200.0], zoom: 0.5 };
        let follower = leader.seen_from(own, other);
        let on_screen = leader.view(vp).to_screen([1500.0 - own[0], 100.0 - own[1]]);
        let same_point = follower.view(vp).to_screen([1500.0 - other[0], 100.0 - other[1]]);
        assert!((on_screen[0] - same_point[0]).abs() < 1e-3 && (on_screen[1] - same_point[1]).abs() < 1e-3);
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
