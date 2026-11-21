//! Password strength meter: 16 blocks coloured from weak (dark red) to
//! strong (green), drawn into the `TCGauge` control of the form.

use std::cell::Cell;
use std::rc::Rc;

use egui::{Color32, Rect, Stroke, StrokeKind, Ui, pos2};

const BLOCKS: usize = 16;
/// Anchor colours from weak to strong across the 16 blocks.
const COLORS: [u32; 7] = [0x7a0000, 0xe00000, 0xff7000, 0xffe600, 0xa8e000, 0x30d020, 0x00ff40];

fn block_color(idx: usize) -> Color32 {
    let t = (idx.min(BLOCKS - 1) as f32) / ((BLOCKS - 1) as f32);
    let scaled = t * ((COLORS.len() - 1) as f32);
    let seg = (scaled.floor() as usize).min(COLORS.len() - 2);
    let f = scaled - seg as f32;
    let (c0, c1) = (COLORS[seg], COLORS[seg + 1]);
    let lerp = |shift: u32| {
        let a = ((c0 >> shift) & 0xff) as f32;
        let b = ((c1 >> shift) & 0xff) as f32;
        (a + (b - a) * f).round() as u8
    };
    Color32::from_rgb(lerp(16), lerp(8), lerp(0))
}

fn filled_blocks(progress: i32, max: i32) -> usize {
    if progress <= 0 || max <= 0 {
        return 0;
    }
    let frac = (progress as f32 / max as f32).clamp(0.0, 1.0);
    ((frac * BLOCKS as f32).ceil() as usize).clamp(1, BLOCKS)
}

/// State of a strength meter; clone it into the painter of the control.
#[derive(Clone)]
pub struct QualityBar {
    progress: Rc<Cell<i32>>,
    max: i32,
}

impl QualityBar {
    pub fn new(max: i32) -> Self {
        QualityBar { progress: Rc::new(Cell::new(0)), max }
    }

    /// `SetPasswQualityBar()`.
    pub fn set_quality(&self, bits: f64) {
        self.progress.set(pwtool_app::quality::quality_color(bits).0);
    }

    pub fn clear(&self) {
        self.progress.set(0);
    }

    pub fn paint(&self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter();
        let grey = Color32::from_rgba_unmultiplied(128, 128, 128, 40);
        p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(128, 128, 128, 36));
        let pad = 2.0;
        let gap = 2.0;
        let inner = rect.shrink(pad);
        let lit = filled_blocks(self.progress.get(), self.max);
        if inner.width() > 0.0 && inner.height() > 0.0 {
            let stride = (inner.width() + gap) / BLOCKS as f32;
            for i in 0..BLOCKS {
                let x0 = (inner.min.x + stride * i as f32).round();
                let x1 = (inner.min.x + stride * (i + 1) as f32 - gap).round().max(x0 + 1.0);
                let r = Rect::from_min_max(pos2(x0, inner.min.y), pos2(x1, inner.max.y));
                p.rect_filled(r, 0.0, if i < lit { block_color(i) } else { grey });
            }
        }
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(102, 102, 102, 200)), StrokeKind::Inside);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks() {
        assert_eq!(filled_blocks(0, 128), 0);
        assert_eq!(filled_blocks(4, 128), 1);
        assert_eq!(filled_blocks(64, 128), 8);
        assert_eq!(filled_blocks(200, 128), 16);
        assert_eq!(block_color(0), Color32::from_rgb(0x7a, 0, 0));
        assert_eq!(block_color(BLOCKS - 1), Color32::from_rgb(0, 0xff, 0x40));
    }
}
