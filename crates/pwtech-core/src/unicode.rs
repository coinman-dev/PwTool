//! String helpers mirroring `upstream/src/util/UnicodeUtil.*`.
//!
//! The original generates passwords on "w32strings": one 32-bit value per
//! character. These are **not** Unicode code points. A character of the Basic
//! Multilingual Plane is stored as its UTF-16 code unit; a character outside
//! of it is stored as its two UTF-16 surrogates packed into one value,
//! `(low_surrogate << 16) | high_surrogate` (`WCharToW32Char()`). Character
//! sets are sorted by these values, so the packing affects which character a
//! given random number selects. [`W32`] values in this crate always use the
//! same packing.

use zeroize::Zeroizing;

/// One "w32" character value (see module docs).
pub type W32 = u32;

/// A string of [`W32`] values (`w32string`).
pub type W32String = Vec<W32>;

/// A [`W32String`] wiped on drop (`SecureW32String`).
pub type SecureW32String = Zeroizing<Vec<W32>>;

/// Pack a Unicode scalar value into a [`W32`] value.
#[inline]
pub fn char_to_w32(c: char) -> W32 {
    let mut buf = [0u16; 2];
    let units = c.encode_utf16(&mut buf);
    if units.len() == 1 {
        units[0] as u32
    } else {
        ((units[1] as u32) << 16) | units[0] as u32
    }
}

/// Unpack a [`W32`] value. Returns `None` for values that are not a valid
/// character (lone surrogates, garbage).
#[inline]
pub fn w32_to_char(v: W32) -> Option<char> {
    if v <= 0xffff {
        char::from_u32(v)
    } else {
        let hi = (v & 0xffff) as u16;
        let lo = (v >> 16) as u16;
        char::decode_utf16([hi, lo]).next()?.ok()
    }
}

/// `WStringToW32String()` / `WCharToW32Char()`.
pub fn str_to_w32(s: &str) -> W32String {
    s.chars().map(char_to_w32).collect()
}

/// [`str_to_w32`] into a buffer that is wiped on drop.
pub fn str_to_w32_secure(s: &str) -> SecureW32String {
    Zeroizing::new(str_to_w32(s))
}

/// `W32StringToWString()`; invalid values become U+FFFD.
pub fn w32_to_string(s: &[W32]) -> String {
    s.iter().map(|&v| w32_to_char(v).unwrap_or('\u{fffd}')).collect()
}

/// [`w32_to_string`] into a string that is wiped on drop.
pub fn w32_to_string_secure(s: &[W32]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(s.len()));
    for &v in s {
        out.push(w32_to_char(v).unwrap_or('\u{fffd}'));
    }
    out
}

/// `AsciiCharToW32String()`: bytes of an ASCII/Latin-1 string as values.
pub fn ascii_to_w32(s: &str) -> W32String {
    s.bytes().map(|b| b as u32).collect()
}

/// Number of UTF-16 code units needed for `s` (`GetNumOfUtf16Chars()`).
pub fn utf16_len_w32(s: &[W32]) -> usize {
    s.iter().map(|&v| if v > 0xffff { 2 } else { 1 }).sum()
}

/// Length of a string in UTF-16 code units (`WString::Length()`).
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `FormatW()`: replace `%1`…`%99` with the corresponding argument (1-based),
/// `%%` with `%`. Unknown indices are dropped; a `%` followed by anything else
/// is kept.
pub fn format_w(fmt: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(fmt.len() + args.iter().map(|a| a.len()).sum::<usize>());
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '%' {
            if i + 1 < chars.len() && chars[i + 1] == '%' {
                out.push('%');
                i += 2;
                continue;
            }
            if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                i += 1;
                let mut n = chars[i] as usize - '0' as usize;
                if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                    i += 1;
                    n = n * 10 + (chars[i] as usize - '0' as usize);
                }
                if n >= 1 && n <= args.len() {
                    out.push_str(args[n - 1]);
                }
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Delphi `Trim()`: remove leading and trailing characters `<= ' '`.
pub fn vcl_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_of_astral_chars() {
        // U+1F600 = D83D DE00 in UTF-16 -> 0xDE00_D83D
        assert_eq!(char_to_w32('😀'), 0xDE00_D83D);
        assert_eq!(w32_to_char(0xDE00_D83D), Some('😀'));
        assert_eq!(char_to_w32('ß'), 0xDF);
        assert_eq!(w32_to_string(&str_to_w32("aß😀")), "aß😀");
        assert_eq!(utf16_len_w32(&str_to_w32("a😀")), 3);
    }

    #[test]
    fn format_w_cases() {
        assert_eq!(format_w("%1 characters / %2 bits", &["90", "6.4"]), "90 characters / 6.4 bits");
        assert_eq!(format_w("%2-%1 100%% %3 %x", &["a", "b"]), "b-a 100%  %x");
    }

    #[test]
    fn trim() {
        assert_eq!(vcl_trim("\t word\r\n"), "word");
    }
}
