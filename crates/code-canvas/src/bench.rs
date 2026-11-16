//! Built-in frame-time benchmark: `CODE_CANVAS_BENCH=1 code-canvas <folder>`.
//!
//! Once the active page is loaded, the camera sweeps continuously from
//! "whole repository" down to readable text and back, while drifting
//! sideways so new content keeps entering the screen (worst case for the
//! streaming paths: token chunks, text shaping, glyph rasterisation).
//! Every few seconds it logs frame-interval statistics.

use std::time::Instant;

use crate::camera::Camera;

/// Seconds for one zoom-in + zoom-out cycle.
const PERIOD: f32 = 12.0;
const REPORT_EVERY: f32 = 3.0;

pub struct Bench {
    started: Option<Instant>,
    home: Option<Camera>,
    samples: Vec<f32>,
    cpu: Vec<f32>,
    last_report: Option<Instant>,
    refresh_hz: Option<f32>,
}

impl Bench {
    pub fn from_env() -> Option<Bench> {
        std::env::var_os("CODE_CANVAS_BENCH").map(|_| Bench {
            started: None,
            home: None,
            samples: Vec::new(),
            cpu: Vec::new(),
            last_report: None,
            refresh_hz: None,
        })
    }

    pub fn set_refresh_hz(&mut self, hz: Option<f32>) {
        self.refresh_hz = hz;
    }

    /// Move the camera for this frame. `fit` frames the whole page;
    /// `max_zoom` is a zoom at which code is readable.
    pub fn drive(&mut self, cam: &mut Camera, fit: Camera, max_zoom: f32, viewport: [f32; 2]) {
        let now = Instant::now();
        let start = *self.started.get_or_insert(now);
        let home = *self.home.get_or_insert(fit);
        let t = now.duration_since(start).as_secs_f32();
        // 0 → 1 → 0 over one period, eased.
        let k = 0.5 - 0.5 * (t * std::f32::consts::TAU / PERIOD).cos();
        let ratio = (max_zoom / home.zoom).max(1.0);
        cam.zoom = home.zoom * ratio.powf(k);
        // Drift ~30% of the screen around the centre, at the current zoom.
        let dx = (t * 0.7).sin() * viewport[0] * 0.3 / cam.zoom;
        let dy = (t * 0.45).cos() * viewport[1] * 0.3 / cam.zoom;
        cam.center = [home.center[0] + dx, home.center[1] + dy];
    }

    /// Record one frame: `interval_ms` since the previous frame and the CPU
    /// time spent building + submitting it.
    pub fn record(&mut self, interval_ms: f32, cpu_ms: f32) {
        if self.started.is_none() {
            return;
        }
        self.samples.push(interval_ms);
        self.cpu.push(cpu_ms);
        let now = Instant::now();
        let last = *self.last_report.get_or_insert(now);
        if now.duration_since(last).as_secs_f32() >= REPORT_EVERY {
            self.report();
            self.last_report = Some(now);
        }
    }

    fn report(&mut self) {
        if self.samples.len() < 2 {
            return;
        }
        // The first sample of a window includes the report itself; skip it.
        let mut s: Vec<f32> = self.samples[1..].to_vec();
        let mut c = self.cpu.clone();
        s.sort_by(f32::total_cmp);
        c.sort_by(f32::total_cmp);
        let pct = |v: &[f32], p: f32| v[((v.len() - 1) as f32 * p) as usize];
        let avg = s.iter().sum::<f32>() / s.len() as f32;
        let budget = 1000.0 / 60.0;
        let over_60 = s.iter().filter(|&&x| x > budget * 1.5).count();
        log::info!(
            "bench: {:.1} fps avg | frame p50 {:.2} ms, p99 {:.2} ms, worst {:.2} ms | cpu p50 {:.2} ms, p99 {:.2} ms, worst {:.2} ms | {} of {} frames missed a 60 Hz vsync{}",
            1000.0 / avg,
            pct(&s, 0.5),
            pct(&s, 0.99),
            s[s.len() - 1],
            pct(&c, 0.5),
            pct(&c, 0.99),
            c[c.len() - 1],
            over_60,
            s.len(),
            self.refresh_hz.map(|hz| format!(" | display {hz:.0} Hz")).unwrap_or_default(),
        );
        self.samples.clear();
        self.cpu.clear();
    }
}
