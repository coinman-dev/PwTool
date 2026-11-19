//! Small helpers from `upstream/src/util/Util.*` that do not depend on the GUI.

use std::sync::atomic::{AtomicU32, Ordering};

/// `FloorEntropyBits()`: integer part of an entropy value, rounded up if the
/// fractional part exceeds 0.99.
pub fn floor_entropy_bits(val: f64) -> i32 {
    let int = val.trunc();
    let frac = val - int;
    (if frac > 0.99 { int + 1.0 } else { int }) as i32
}

static DECIMAL_SEPARATOR: AtomicU32 = AtomicU32::new('.' as u32);

/// Decimal separator used by [`format_float`] (Delphi `FormatSettings`).
pub fn set_decimal_separator(c: char) {
    DECIMAL_SEPARATOR.store(c as u32, Ordering::Relaxed);
}

pub fn decimal_separator() -> char {
    char::from_u32(DECIMAL_SEPARATOR.load(Ordering::Relaxed)).unwrap_or('.')
}

/// Take the decimal separator from the C locale of the environment
/// (`LC_NUMERIC`), as Windows does from the regional settings.
pub fn init_decimal_separator_from_locale() {
    // SAFETY: setlocale/nl_langinfo with valid arguments; the result is copied.
    unsafe {
        let old = libc::setlocale(libc::LC_NUMERIC, std::ptr::null());
        let saved = if old.is_null() { None } else { Some(std::ffi::CStr::from_ptr(old).to_owned()) };
        libc::setlocale(libc::LC_NUMERIC, c"".as_ptr());
        let radix = libc::nl_langinfo(libc::RADIXCHAR);
        if !radix.is_null() {
            if let Some(c) = std::ffi::CStr::from_ptr(radix).to_str().ok().and_then(|s| s.chars().next()) {
                set_decimal_separator(c);
            }
        }
        if let Some(s) = saved {
            libc::setlocale(libc::LC_NUMERIC, s.as_ptr());
        }
    }
}

/// `FormatFloat(fmt, x)` for the formats `"0"`, `"0.0"`, `"0.00"`…: fixed
/// number of decimals, rounding half away from zero, locale decimal separator.
pub fn format_float(x: f64, decimals: usize) -> String {
    let factor = 10f64.powi(decimals as i32);
    let r = (x * factor).round() / factor;
    let s = format!("{r:.decimals$}");
    let sep = decimal_separator();
    if sep == '.' { s } else { s.replace('.', &sep.to_string()) }
}

/// `FormatFloat("0.0", x)`.
pub fn format_float_1(x: f64) -> String {
    format_float(x, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_bits() {
        assert_eq!(floor_entropy_bits(12.5), 12);
        assert_eq!(floor_entropy_bits(12.995), 13);
        assert_eq!(floor_entropy_bits(0.0), 0);
    }

    #[test]
    fn float_format() {
        assert_eq!(format_float_1(12.25), "12.3");
        assert_eq!(format_float_1(0.15), "0.2");
        assert_eq!(format_float_1(128.0), "128.0");
    }
}
