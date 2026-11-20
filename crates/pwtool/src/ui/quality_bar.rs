//! Password quality bar (`TCGauge` with `SetPasswQualityBar()` of
//! `upstream/src/util/Util.cpp`): length ~ entropy (full = 128 bits), split
//! into 16 discrete vertical blocks, each in its own shade from dark red over
//! orange and yellow to bright green so strength gradations are clearly visible.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;

/// Number of vertical blocks in the password strength bar.
pub const BLOCKS: usize = 16;

#[derive(Clone)]
pub struct QualityBar {
    pub area: gtk::DrawingArea,
    state: Rc<Cell<(i32, u32)>>,
    max: i32,
}

pub use pwtool_app::quality::quality_color;

/// Anchor colours from weak to strong across the 16 blocks.
const COLORS: [u32; 7] = [0x7a0000, 0xe00000, 0xff7000, 0xffe600, 0xa8e000, 0x30d020, 0x00ff40];

/// Distinct RGB colour `(r, g, b)` in `0.0..=1.0` for block `idx` (`0..BLOCKS`).
fn block_color(idx: usize) -> (f64, f64, f64) {
    let t = (idx.min(BLOCKS - 1) as f64) / ((BLOCKS - 1) as f64);
    let scaled = t * ((COLORS.len() - 1) as f64);
    let seg = (scaled.floor() as usize).min(COLORS.len() - 2);
    let f = scaled - seg as f64;
    let c0 = COLORS[seg];
    let c1 = COLORS[seg + 1];
    let lerp = |shift: u32| -> f64 {
        let a = ((c0 >> shift) & 0xff) as f64;
        let b = ((c1 >> shift) & 0xff) as f64;
        (a + (b - a) * f) / 255.0
    };
    (lerp(16), lerp(8), lerp(0))
}

/// Number of lit blocks (`0..=BLOCKS`) for a given `progress` and `max`.
fn filled_blocks(progress: i32, max: i32) -> usize {
    if progress <= 0 || max <= 0 {
        return 0;
    }
    let frac = (progress as f64 / max as f64).clamp(0.0, 1.0);
    ((frac * BLOCKS as f64).ceil() as usize).clamp(1, BLOCKS)
}

impl QualityBar {
    pub fn new(area: gtk::DrawingArea, max: i32) -> Self {
        let state = Rc::new(Cell::new((0, 0u32)));
        let s = state.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (progress, _rgb) = s.get();
            let wf = w as f64;
            let hf = h as f64;

            // Subtle bar background
            cr.set_source_rgba(0.5, 0.5, 0.5, 0.14);
            cr.rectangle(0.0, 0.0, wf, hf);
            let _ = cr.fill();

            // 16 discrete vertical blocks with crisp 2px separators
            let pad = 2.0;
            let gap = 2.0;
            let inner_w = (wf - pad * 2.0).max(0.0);
            let inner_h = (hf - pad * 2.0).max(0.0);
            let lit = filled_blocks(progress, max);
            if inner_w > 0.0 && inner_h > 0.0 {
                let stride = (inner_w + gap) / BLOCKS as f64;
                for i in 0..BLOCKS {
                    let x0 = (pad + stride * i as f64).round();
                    let x1 = (pad + stride * (i + 1) as f64 - gap).round();
                    let bw = (x1 - x0).max(1.0);
                    if i < lit {
                        let (r, g, b) = block_color(i);
                        cr.set_source_rgb(r, g, b);
                    } else {
                        cr.set_source_rgba(0.5, 0.5, 0.5, 0.18);
                    }
                    cr.rectangle(x0, pad, bw, inner_h);
                    let _ = cr.fill();
                }
            }

            // Outer border
            cr.set_source_rgba(0.4, 0.4, 0.4, 0.8);
            cr.set_line_width(1.0);
            cr.rectangle(0.5, 0.5, wf - 1.0, hf - 1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_distinct_blocks_and_scaling() {
        assert_eq!(BLOCKS, 16);
        assert_eq!(filled_blocks(0, 128), 0);
        assert_eq!(filled_blocks(4, 128), 1);
        assert_eq!(filled_blocks(64, 128), 8);
        assert_eq!(filled_blocks(128, 128), 16);
        assert_eq!(filled_blocks(200, 128), 16);
        // All 16 blocks have distinct colours
        let colors: Vec<(u8, u8, u8)> = (0..BLOCKS)
            .map(|i| {
                let (r, g, b) = block_color(i);
                ((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
            })
            .collect();
        for w in colors.windows(2) {
            assert_ne!(w[0], w[1]);
        }
        assert_eq!(colors[0], (0x7a, 0x00, 0x00));
        assert_eq!(colors[BLOCKS - 1], (0x00, 0xff, 0x40));
    }
}
