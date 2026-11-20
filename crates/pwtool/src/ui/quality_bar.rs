//! Password quality bar (`TCGauge` with `SetPasswQualityBar()` of
//! `upstream/src/util/Util.cpp`): length ~ entropy (full = 128 bits). The
//! filled part shows a gradient over the whole scale, from dark red over
//! orange, yellow and green to bright green in equal steps, so a strong
//! password shows the whole spectrum.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;

#[derive(Clone)]
pub struct QualityBar {
    pub area: gtk::DrawingArea,
    state: Rc<Cell<(i32, u32)>>,
    max: i32,
}

/// Colour (0xRRGGBB) and progress for an entropy value.
pub fn quality_color(bits: f64) -> (i32, u32) {
    const THRESH_RED: f64 = 56.0;
    const THRESH_YELLOW: f64 = 70.0;
    const THRESH_YELLOWGREEN: f64 = 75.0;
    const THRESH_GREEN: f64 = 80.0;
    const THRESH_DARKGREEN: f64 = 112.0;
    let progress = (bits as i32).max(4);
    // TColor values of the original are BGR
    let bgr: u32 = if bits >= THRESH_DARKGREEN {
        0xe000
    } else if bits >= THRESH_GREEN {
        0xf000
    } else if bits >= THRESH_YELLOWGREEN {
        0xffd0
    } else if bits >= THRESH_YELLOW {
        0xfff0
    } else if bits >= THRESH_RED {
        let g = ((bits - THRESH_RED) / (THRESH_YELLOW - THRESH_RED) * 255.0) as u32;
        (g << 8) | 0xff
    } else {
        0xff
    };
    let rgb = ((bgr & 0xff) << 16) | (bgr & 0xff00) | ((bgr >> 16) & 0xff);
    (progress, rgb)
}

/// Colours of the bar from weak to strong. Every colour gets an equal band
/// of the scale with a short plateau of the pure colour, so that all colours
/// (also yellow) are equally visible.
const COLORS: [u32; 7] = [0x7a0000, 0xe00000, 0xff7000, 0xffe600, 0xa8e000, 0x30d020, 0x00ff40];
/// Share of a colour band that shows the pure colour.
const PLATEAU: f64 = 0.35;

/// Gradient stops (position 0..1, colour).
fn gradient_stops() -> Vec<(f64, u32)> {
    let n = COLORS.len() as f64;
    let mut v = Vec::new();
    for (i, &c) in COLORS.iter().enumerate() {
        let center = (i as f64 + 0.5) / n;
        let half = PLATEAU / n / 2.0;
        let a = if i == 0 { 0.0 } else { center - half };
        let b = if i + 1 == COLORS.len() { 1.0 } else { center + half };
        v.push((a, c));
        v.push((b, c));
    }
    v
}

impl QualityBar {
    pub fn new(area: gtk::DrawingArea, max: i32) -> Self {
        let state = Rc::new(Cell::new((0, 0u32)));
        let s = state.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (progress, rgb) = s.get();
            cr.set_source_rgba(0.5, 0.5, 0.5, 0.25);
            cr.rectangle(0.0, 0.0, w as f64, h as f64);
            let _ = cr.fill();
            if progress > 0 {
                let frac = (progress as f64 / max as f64).min(1.0);
                let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, w as f64, 0.0);
                for (pos, rgb) in gradient_stops() {
                    grad.add_color_stop_rgb(
                        pos,
                        ((rgb >> 16) & 0xff) as f64 / 255.0,
                        ((rgb >> 8) & 0xff) as f64 / 255.0,
                        (rgb & 0xff) as f64 / 255.0,
                    );
                }
                let _ = cr.set_source(&grad);
                cr.rectangle(0.0, 0.0, w as f64 * frac, h as f64);
                let _ = cr.fill();
                let _ = rgb;
            }
            cr.set_source_rgba(0.4, 0.4, 0.4, 0.8);
            cr.set_line_width(1.0);
            cr.rectangle(0.5, 0.5, w as f64 - 1.0, h as f64 - 1.0);
            let _ = cr.stroke();
        });
        QualityBar { area, state, max }
    }

    /// `SetPasswQualityBar()`.
    pub fn set_quality(&self, bits: f64) {
        self.state.set(quality_color(bits));
        self.area.queue_draw();
    }

    /// `Progress = 0`.
    pub fn clear(&self) {
        self.state.set((0, 0));
        self.area.queue_draw();
    }

    pub fn max(&self) -> i32 {
        self.max
    }
}
