//! Password quality colours (`PasswQualityColor` of the original).

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
