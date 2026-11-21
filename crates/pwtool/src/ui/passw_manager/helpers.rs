//! Helpers of the password manager: fuzzy search, expiry dates, file
//! names, text conversions.

use pwtool_core::passw::database::encode_expiry_date;

/// `levenshteinDist()` with the original costs (insert 2, delete 1,
/// replace 1), on UTF-16 code units like the original.
pub fn levenshtein_dist(src: &[u16], target: &[u16]) -> u32 {
    const INSERT_COST: u32 = 2;
    const DELETE_COST: u32 = 1;
    const REPLACE_COST: u32 = 1;
    if src.len() > target.len() {
        return levenshtein_dist(target, src);
    }
    let min_size = src.len();
    let mut lev = vec![0u32; min_size + 1];
    for i in 1..=min_size {
        lev[i] = lev[i - 1] + DELETE_COST;
    }
    for &t in target {
        let mut prev = lev[0];
        lev[0] += INSERT_COST;
        for i in 1..=min_size {
            let prev_save = lev[i];
            if src[i - 1] == t {
                lev[i] = prev;
            } else {
                lev[i] = (lev[i] + INSERT_COST).min(lev[i - 1] + DELETE_COST).min(prev + REPLACE_COST);
            }
            prev = prev_save;
        }
    }
    *lev.last().unwrap()
}

fn find_sub(hay: &[u16], needle: &[u16]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

/// `strFindFuzzy()`: score of `pattern` in `src` (2 = identical, 1 =
/// contained, otherwise the best Levenshtein score in 0..1).
pub fn str_find_fuzzy(src: &[u16], pattern: &[u16]) -> f32 {
    let src_len = src.len();
    let pat_len = pattern.len();
    if pat_len == 0 {
        return 0.0;
    }
    let lev_score = |p: &[u16]| {
        let d = levenshtein_dist(p, pattern) as usize;
        if d < pat_len { (pat_len - d) as f32 / pat_len as f32 } else { 0.0 }
    };
    let mut best = 0.0f32;
    if src_len <= pat_len {
        if src == pattern {
            best = 2.0;
        } else {
            best = lev_score(src);
        }
    } else {
        let mut i = 0;
        let mut pos = 0;
        while let Some(off) = src[pos..].iter().position(|&c| c == pattern[0]) {
            let next = pos + off;
            if i == 0 && find_sub(&src[next..], pattern) {
                best = 1.0;
                break;
            }
            let len = pat_len.min(src_len - next);
            best = best.max(lev_score(&src[next..next + len]));
            i += 1;
            pos = next + 1;
            if pos >= src_len {
                break;
            }
        }
    }
    best
}

/// Lower-case a string (`CharLower` / `AnsiLowerCase`).
pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `_wcsicmp`: case-insensitive comparison of UTF-16 code units (ASCII
/// letters folded, as in the C locale).
pub fn wcsicmp(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: u16| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c };
    a.encode_utf16().map(fold).cmp(b.encode_utf16().map(fold))
}

/// `wcscmp`.
pub fn wcscmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn now() -> jiff::Zoned {
    jiff::Zoned::now()
}

/// Today's local date as `(year, month, day)`.
pub fn today() -> (i32, u32, u32) {
    let d = now().date();
    (d.year() as i32, d.month() as u32, d.day() as u32)
}

/// Local date `days` days from today.
pub fn today_plus_days(days: i32) -> (i32, u32, u32) {
    let d = now().date().saturating_add(jiff::Span::new().days(days as i64));
    (d.year() as i32, d.month() as u32, d.day() as u32)
}

/// `getExpiryCheckDates()`: encoded current date and the "expires soon"
/// limit (today + `warn_days`, clamped to 1..100).
pub fn expiry_check_dates(warn_days: i32) -> (u32, u32) {
    let (y, m, d) = today();
    let (y2, m2, d2) = today_plus_days(warn_days.clamp(1, 100));
    (encode_expiry_date(y, m, d), encode_expiry_date(y2, m2, d2))
}

/// `getTimeStampString()`: `YYYYMMDDhhmmss[_mmm]` in local time.
pub fn time_stamp_string(millisec: bool) -> String {
    let t = now();
    let mut s = t.strftime("%Y%m%d%H%M%S").to_string();
    if millisec {
        s.push_str(&format!("_{:03}", t.millisecond()));
    }
    s
}

/// `ShortenFileName()`.
pub fn shorten_file_name(name: &str, max_len: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    let len = chars.len();
    if len <= max_len.max(10) {
        return name.to_string();
    }
    let start_chars = 10.min((max_len.saturating_sub(4)) / 2);
    let end_chars = max_len - start_chars - 3;
    let mut result: Vec<char> = chars[..start_chars].to_vec();
    result.extend("...".chars());
    result.extend_from_slice(&chars[len - end_chars..]);
    // remove the partial directory name after "..." (1-based indices in the
    // original: search starts behind "...")
    let rlen = result.len();
    let start = start_chars + 4; // 1-based position
    let mut del_pos = 0;
    let mut i = start;
    while i + 10 <= rlen {
        if result[i - 1] == '/' {
            del_pos = i;
            break;
        }
        i += 1;
    }
    if del_pos > start {
        result.drain(start - 1..del_pos - 1);
    }
    result.into_iter().collect()
}


/// Convert `\r\n` (file) to `\n` (text controls).
pub fn crlf_to_lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Convert `\n` (text controls) to `\r\n` (file, as the original stores the
/// text of memo controls).
pub fn lf_to_crlf(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut prev_cr = false;
    for c in s.chars() {
        if c == '\n' && !prev_cr {
            out.push('\r');
        }
        out.push(c);
        prev_cr = c == '\r';
    }
    out
}

/// Local `FILETIME` (wall clock as UTC) of a file system time, for
/// `time_stamp_to_string()`.
pub fn system_time_to_local_filetime(t: std::time::SystemTime) -> u64 {
    use pwtool_core::random::hrtimer::{FILETIME_UNIX_EPOCH, utc_offset_seconds};
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let local = secs + utc_offset_seconds(secs as _);
    FILETIME_UNIX_EPOCH + local.max(0) as u64 * 10_000_000 + (d.subsec_nanos() / 100) as u64
}

/// Integer with thousands separators (`Format("%.0n")`).
pub fn format_thousands(n: u64) -> String {
    let sep = if pwtool_core::util::decimal_separator() == ',' { '.' } else { ',' };
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn fuzzy() {
        assert_eq!(str_find_fuzzy(&u("hello"), &u("hello")), 2.0);
        assert_eq!(str_find_fuzzy(&u("say hello world"), &u("hello")), 1.0);
        assert!(str_find_fuzzy(&u("say hallo world"), &u("hello")) >= 0.5);
        assert_eq!(str_find_fuzzy(&u("xyz"), &u("hello")), 0.0);
        assert_eq!(levenshtein_dist(&u("abc"), &u("abc")), 0);
        assert_eq!(levenshtein_dist(&u("ab"), &u("abc")), 2);
    }

    #[test]
    fn shorten() {
        assert_eq!(shorten_file_name("/short/name.pwdb", 60), "/short/name.pwdb");
        let long = "/home/user/some/very/long/directory/structure/that/goes/on/and/on/file.pwdb";
        let s = shorten_file_name(long, 60);
        assert!(s.chars().count() <= 60, "{s}");
        assert!(s.starts_with("/home/user"));
        assert!(s.ends_with("file.pwdb"));
    }

    #[test]
    fn crlf() {
        assert_eq!(lf_to_crlf("a\nb\r\nc"), "a\r\nb\r\nc");
        assert_eq!(crlf_to_lf("a\r\nb"), "a\nb");
    }
}
