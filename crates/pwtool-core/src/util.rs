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

pub use imp::{
    current_local_year, format_short_date, init_decimal_separator_from_locale, init_time_locale,
    time_stamp_to_long_string, time_stamp_to_string,
};

#[cfg(unix)]
mod imp {
    use super::set_decimal_separator;

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

    /// Use the user's locale for dates and times (`LC_TIME`), as Windows uses
    /// the regional settings. Call once at program start.
    pub fn init_time_locale() {
        // SAFETY: setlocale with a valid category and empty string.
        unsafe {
            libc::setlocale(libc::LC_TIME, c"".as_ptr());
        }
    }

    fn strftime(fmt: &std::ffi::CStr, tm: &libc::tm) -> String {
        let mut buf = [0u8; 256];
        // SAFETY: buffer and format are valid; tm is initialised.
        let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), tm) };
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    /// Broken-down time of a `FILETIME` value without time zone conversion
    /// (`FileTimeToSystemTime`).
    fn filetime_to_tm(ft: u64) -> Option<libc::tm> {
        let secs = (ft / 10_000_000) as i64 - 11_644_473_600;
        let t = secs as libc::time_t;
        // SAFETY: gmtime_r with valid pointers.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::gmtime_r(&t, &mut tm).is_null() { None } else { Some(tm) }
        }
    }

    /// `TimeStampToString()`: short date and time in the user's locale.
    pub fn time_stamp_to_string(ft: u64) -> String {
        match filetime_to_tm(ft) {
            Some(tm) => format!("{} {}", strftime(c"%x", &tm), strftime(c"%X", &tm)),
            None => String::new(),
        }
    }

    /// `FileTimeToString(..., true)`: long date and time in the user's locale.
    pub fn time_stamp_to_long_string(ft: u64) -> String {
        match filetime_to_tm(ft) {
            Some(tm) => format!("{} {}", strftime(c"%A, %e %B %Y", &tm).replace("  ", " "), strftime(c"%X", &tm)),
            None => String::new(),
        }
    }

    pub fn current_local_year() -> i64 {
        unsafe {
            let t = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&t, &mut tm).is_null() { 2026 } else { tm.tm_year as i64 + 1900 }
        }
    }

    /// Short date of the user's locale (`GetDateFormatEx(DATE_SHORTDATE)`).
    pub fn format_short_date(year: i32, month: u32, day: u32) -> String {
        // SAFETY: zeroed tm is a valid value; fields set below.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = year - 1900;
        tm.tm_mon = month as i32 - 1;
        tm.tm_mday = day as i32;
        strftime(c"%x", &tm)
    }
}

#[cfg(windows)]
mod imp {
    use super::set_decimal_separator;
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::Globalization::{
        DATE_LONGDATE, DATE_SHORTDATE, GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx, LOCALE_SDECIMAL,
    };
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    use windows_sys::Win32::System::Time::FileTimeToSystemTime;

    fn wide_to_string(buf: &[u16], n: i32) -> String {
        let n = (n.max(1) as usize - 1).min(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }

    /// Decimal separator of the regional settings (`LOCALE_SDECIMAL`).
    pub fn init_decimal_separator_from_locale() {
        let mut buf = [0u16; 8];
        // SAFETY: valid buffer; NULL is LOCALE_NAME_USER_DEFAULT.
        let n = unsafe { GetLocaleInfoEx(std::ptr::null(), LOCALE_SDECIMAL, buf.as_mut_ptr(), buf.len() as i32) };
        if n > 1 {
            if let Some(c) = wide_to_string(&buf, n).chars().next() {
                set_decimal_separator(c);
            }
        }
    }

    /// Windows formats dates with the regional settings; nothing to do.
    pub fn init_time_locale() {}

    fn format_date(st: &SYSTEMTIME, flags: u32) -> String {
        let mut buf = [0u16; 128];
        // SAFETY: valid buffer and SYSTEMTIME; NULL locale = user default.
        let n = unsafe {
            GetDateFormatEx(std::ptr::null(), flags, st, std::ptr::null(), buf.as_mut_ptr(), buf.len() as i32, std::ptr::null())
        };
        if n > 0 { wide_to_string(&buf, n) } else { String::new() }
    }

    fn format_time(st: &SYSTEMTIME) -> String {
        let mut buf = [0u16; 64];
        // SAFETY: as above.
        let n = unsafe { GetTimeFormatEx(std::ptr::null(), 0, st, std::ptr::null(), buf.as_mut_ptr(), buf.len() as i32) };
        if n > 0 { wide_to_string(&buf, n) } else { String::new() }
    }

    /// `FileTimeToSystemTime()` without time zone conversion.
    fn filetime_to_systemtime(ft: u64) -> Option<SYSTEMTIME> {
        let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        // SAFETY: valid pointers.
        unsafe {
            let mut st: SYSTEMTIME = std::mem::zeroed();
            (FileTimeToSystemTime(&f, &mut st) != 0).then_some(st)
        }
    }

    /// `TimeStampToString()`: short date and time in the user's locale.
    pub fn time_stamp_to_string(ft: u64) -> String {
        match filetime_to_systemtime(ft) {
            Some(st) => format!("{} {}", format_date(&st, DATE_SHORTDATE), format_time(&st)),
            None => String::new(),
        }
    }

    /// `FileTimeToString(..., true)`: long date and time in the user's locale.
    pub fn time_stamp_to_long_string(ft: u64) -> String {
        match filetime_to_systemtime(ft) {
            Some(st) => format!("{} {}", format_date(&st, DATE_LONGDATE), format_time(&st)),
            None => String::new(),
        }
    }

    pub fn current_local_year() -> i64 {
        // SAFETY: valid pointer.
        unsafe {
            let mut st: SYSTEMTIME = std::mem::zeroed();
            GetLocalTime(&mut st);
            st.wYear as i64
        }
    }

    /// Short date of the user's locale (`GetDateFormatEx(DATE_SHORTDATE)`).
    pub fn format_short_date(year: i32, month: u32, day: u32) -> String {
        // SAFETY: zeroed SYSTEMTIME is valid; fields set below.
        let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
        st.wYear = year.clamp(1601, 30827) as u16;
        st.wMonth = month as u16;
        st.wDay = day as u16;
        format_date(&st, DATE_SHORTDATE)
    }
}

/// Parse a short date of the user's locale (`StrToDate()`, inverse of
/// [`format_short_date`]): (year, month, day).
pub fn parse_short_date(s: &str) -> Option<(i32, u32, u32)> {
    let nums: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|x| !x.is_empty())
        .filter_map(|x| x.parse().ok())
        .collect();
    if nums.len() != 3 {
        return None;
    }
    // order of the fields in the locale's format
    let sample = format_short_date(2033, 11, 22);
    let pos = |pat: &str| sample.find(pat);
    let (py, pm, pd) = (pos("33").unwrap_or(usize::MAX), pos("11")?, pos("22")?);
    let mut order = [(py, 'y'), (pm, 'm'), (pd, 'd')];
    order.sort();
    let (mut y, mut m, mut d) = (0i64, 0i64, 0i64);
    for (i, (_, k)) in order.iter().enumerate() {
        match k {
            'y' => y = nums[i],
            'm' => m = nums[i],
            _ => d = nums[i],
        }
    }
    if y < 100 {
        let cur_year = current_local_year();
        let pivot = cur_year - 50;
        let century = (pivot / 100) * 100;
        y += century;
        if y < pivot {
            y += 100;
        }
    }
    if !(1..=12).contains(&m) {
        return None;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        _ => 28,
    };
    if d < 1 || d > dim {
        return None;
    }
    Some((y as i32, m as u32, d as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_date_roundtrip() {
        init_time_locale();
        let s = format_short_date(2026, 11, 20);
        assert_eq!(parse_short_date(&s), Some((2026, 11, 20)));
        assert_eq!(parse_short_date("garbage"), None);
        // Reject impossible dates (e.g., Feb 31, non-leap Feb 29)
        let bad = format_short_date(2026, 2, 28).replace("28", "31");
        assert_eq!(parse_short_date(&bad), None);
        let non_leap = format_short_date(2025, 2, 28).replace("28", "29");
        assert_eq!(parse_short_date(&non_leap), None);
        let leap = format_short_date(2024, 2, 29);
        assert_eq!(parse_short_date(&leap), Some((2024, 2, 29)));
        // 2-digit year sliding 50-year window
        let two_digit = format_short_date(2026, 11, 20).replace("2026", "26");
        let parsed = parse_short_date(&two_digit).unwrap();
        assert_eq!(parsed.0 % 100, 26);
        assert!(parsed.0 >= 1970 && parsed.0 <= 2100);
    }

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
