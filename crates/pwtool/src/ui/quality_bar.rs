//! Password quality bar (`TCGauge` with `SetPasswQualityBar()` of
//! `upstream/src/util/Util.cpp`): length ~ entropy (full = 128 bits), colour
//! from red over yellow to green.

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
                cr.set_source_rgb(
                    ((rgb >> 16) & 0xff) as f64 / 255.0,
                    ((rgb >> 8) & 0xff) as f64 / 255.0,
                    (rgb & 0xff) as f64 / 255.0,
                );
                cr.rectangle(0.0, 0.0, w as f64 * frac, h as f64);
                let _ = cr.fill();
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
