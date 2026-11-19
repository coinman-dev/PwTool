//! Password generator — port of `upstream/src/passw/PasswGen.*`.
//!
//! The order of random draws, the rejection rules and the character set
//! ordering follow the original line by line, so that the deterministic modes
//! produce identical passwords. Characters are handled as [`W32`] values with
//! the original's packing (see [`crate::unicode`]).
//!
//! Notes on behaviour carried over from the original:
//! - `toupper()`/`tolower()`/`isalpha()`/`isdigit()` of the C runtime act on
//!   ASCII only; non-ASCII letters are left unchanged / not treated as letters.
//! - Delphi `LowerCase()` (used for word lists) converts ASCII only.
//! - `std::sort` on the subset frequency list is assumed to keep the order of
//!   equal elements (true for the small lists involved); a stable sort is used.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use zeroize::Zeroizing;

use super::tables::{DICEWARE8K, PHONETIC_ENTROPY, PHONETIC_SIGMA, PHONETIC_TRIS};
use crate::random::{RandomGenerator, permute};
use crate::text_file::{CharEncoding, read_text_file, split_tokens};
use crate::unicode::{SecureW32String, W32, W32String, ascii_to_w32, char_to_w32, str_to_w32, w32_to_string};

pub const WORDLIST_DEFAULT_SIZE: i32 = 8192;
pub const WORDLIST_MAX_WORDLEN: i32 = 100;
pub const WORDLIST_MAX_SIZE: usize = 1_048_576;

pub const PASSWGEN_NUMCHARSETCODES: usize = 11;
pub const PASSWGEN_NUMINCLUDECHARSETS: usize = 4;
pub const PASSWGEN_NUMFORMATCHARSETS: usize = 22;

/// First character must not be a lower-case letter.
pub const PASSW_FLAG_FIRSTCHARNOTLC: u32 = 0x0001;
/// No repeating consecutive characters.
pub const PASSW_FLAG_EXCLUDEREPCHARS: u32 = 0x0002;
/// Take the "include" characters from subsets of the custom set.
pub const PASSW_FLAG_INCLUDESUBSET: u32 = 0x0004;
pub const PASSW_FLAG_INCLUDEUPPERCASE: u32 = 0x0008;
pub const PASSW_FLAG_INCLUDELOWERCASE: u32 = 0x0010;
pub const PASSW_FLAG_INCLUDEDIGIT: u32 = 0x0020;
pub const PASSW_FLAG_INCLUDESPECIAL: u32 = 0x0040;
pub const PASSW_FLAG_PHONETICUPPERCASE: u32 = 0x0080;
pub const PASSW_FLAG_PHONETICMIXEDCASE: u32 = 0x0100;
pub const PASSW_FLAG_EACHCHARONLYONCE: u32 = 0x0200;
pub const PASSW_FLAG_CHECKDUPLICATESBYSET: u32 = 0x0400;
pub const PASSW_FLAG_REMOVEWHITESPACE: u32 = 0x0800;

pub const PASSPHR_FLAG_COMBINEWCH: u32 = 0x0001;
pub const PASSPHR_FLAG_CAPITALIZEWORDS: u32 = 0x0002;
pub const PASSPHR_FLAG_DONTSEPWORDS: u32 = 0x0004;
pub const PASSPHR_FLAG_DONTSEPWCH: u32 = 0x0008;
pub const PASSPHR_FLAG_REVERSEWCHORDER: u32 = 0x0010;
pub const PASSPHR_FLAG_EACHWORDONLYONCE: u32 = 0x0020;

pub const PASSFORMAT_FLAG_EXCLUDEREPCHARS: u32 = 0x0001;
pub const PASSFORMAT_FLAG_REMOVEWHITESPACE: u32 = 0x0002;

/// Password provided to `GetFormatPassw`, but no `P` specifier.
pub const PASSFORMAT_PWUSED_NOSPECIFIER: i32 = -1;
/// `%P` specified, but no password available.
pub const PASSFORMAT_PWUSED_EMPTYPASSW: i32 = -2;

pub const CHARSET_SYMBOLS: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

const CHARSET_CODES: [&str; 18] = [
    "AZ", "az", "09", "Hex", "HEX", "hex", "base64", "b64", "easytoread", "etr", "symbols", "sym", "brackets",
    "brac", "punctuation", "punct", "highansi", "high",
];
const CHARSET_CODES_MAP: [usize; 18] = [0, 1, 2, 3, 3, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10];
const CHARSET_DECODES: [&str; PASSWGEN_NUMCHARSETCODES] = [
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "abcdefghijklmnopqrstuvwxyz",
    "0123456789",
    "0123456789ABCDEF",
    "0123456789abcdef",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    "",
    "",
    "()[]{}<>",
    ",.;:",
    "",
];
const CHARSET_CODE_PHONETIC: &str = "<phonetic>";
const CHARSET_CODE_PHONETIC_UPPERCASE: &str = "<phoneticu>";
const CHARSET_CODE_PHONETIC_MIXEDCASE: &str = "<phoneticx>";

const CHARSET_CODES_AZ: usize = 0;
const CHARSET_CODES_LC_AZ: usize = 1;
const CHARSET_CODES_09: usize = 2;
const CHARSET_CODES_EASYTOREAD: usize = 6;
const CHARSET_CODES_SYMBOLS: usize = 7;
const CHARSET_CODES_HIGHANSI: usize = 10;

const CHARSET_INCLUDE_AZ: usize = 0;
const CHARSET_INCLUDE_LC_AZ: usize = 1;
const CHARSET_INCLUDE_09: usize = 2;
const CHARSET_INCLUDE_SPECIAL: usize = 3;

/// Default ambiguous characters.
pub const CHARSET_AMBIGUOUS: &str = "B8G6I1l|0OQDS5Z2";

/// Format specifiers, index = format character set.
pub const FORMAT_PLACEHOLDERS: &str = "xaAUEdhHlLuvVZcCzpbsSy";

const CHARSET_FORMAT: [&str; PASSWGEN_NUMFORMATCHARSETS] = [
    "",
    "abcdefghijklmnopqrstuvwxyz0123456789",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
    "",
    "0123456789",
    "0123456789abcdef",
    "0123456789ABCDEF",
    "abcdefghijklmnopqrstuvwxyz",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "aeiou",
    "AEIOUaeiou",
    "AEIOU",
    "bcdfghjklmnpqrstvwxyz",
    "BCDFGHJKLMNPQRSTVWXYZbcdfghjklmnpqrstvwxyz",
    "BCDFGHJKLMNPQRSTVWXYZ",
    ",.;:",
    "()[]{}<>",
    "",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
    "",
];
const CHARSET_FORMAT_X: usize = 0;
const CHARSET_FORMAT_E: usize = 4;
const CHARSET_FORMAT_LC_S: usize = 19;
const CHARSET_FORMAT_S: usize = 20;
const CHARSET_FORMAT_Y: usize = 21;
const CHARSET_FORMAT_CONST: [usize; 17] = [1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];
const FORMAT_REPEAT_MAXDEPTH: usize = 4;

pub const PHONETIC_TRIS_NUM: usize = 17576;

/// Type of the custom character set (`CharSetType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharSetType {
    Standard,
    StandardWithFreq,
    Phonetic,
    PhoneticUpperCase,
    PhoneticMixedCase,
}

/// Subsets with their number of occurrences; the last element is the common
/// set with `i32::MAX` (`CharSetFreq`).
pub type CharSetFreq = Vec<(W32String, i32)>;

/// Result of loading a word list (`LoadWordListFile()` return codes).
#[derive(Debug)]
pub enum WordListError {
    /// I/O or encoding error (return value < 0).
    Io(io::Error),
    /// Fewer than 2 words (return value 0).
    TooFewWords,
}

/// Errors of `CreateTrigramFile()`.
#[derive(Debug, thiserror::Error)]
pub enum TrigramError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("Source file does not contain enough trigrams")]
    NotEnoughTrigrams,
}

/// Result of `GetFormatPassw()`.
pub struct FormatPasswResult {
    pub passw: SecureW32String,
    /// How the provided password was used (`pnPasswUsed`): 0 = no password
    /// given; > 0 = characters copied; or one of `PASSFORMAT_PWUSED_*`.
    pub passw_used: i32,
    /// Unknown format specifiers encountered (`pInvalidSpec`).
    pub invalid_spec: W32String,
}

#[inline]
fn is_ascii_digit(c: W32) -> bool {
    (b'0' as u32..=b'9' as u32).contains(&c)
}

#[inline]
fn is_ascii_alpha(c: W32) -> bool {
    (b'a' as u32..=b'z' as u32).contains(&c) || (b'A' as u32..=b'Z' as u32).contains(&c)
}

#[inline]
fn ascii_upper(c: W32) -> W32 {
    if (b'a' as u32..=b'z' as u32).contains(&c) { c - 32 } else { c }
}

#[inline]
fn ascii_lower(c: W32) -> W32 {
    if (b'A' as u32..=b'Z' as u32).contains(&c) { c + 32 } else { c }
}

const SPACE: W32 = b' ' as W32;

/// `removeWhitespace()`: strip leading/trailing spaces and tabs, keeping at
/// least one character.
fn remove_whitespace(s: &mut Vec<W32>) {
    let len = s.len();
    if len < 2 {
        return;
    }
    let ws = |c: W32| c == SPACE || c == b'\t' as W32;
    let mut start = 0;
    while start < len - 1 && ws(s[start]) {
        start += 1;
    }
    let mut end = len - 1;
    while end > start && ws(s[end]) {
        end -= 1;
    }
    s.truncate(end + 1);
    s.drain(..start);
}

fn sorted_unique(src: &[W32]) -> BTreeSet<W32> {
    src.iter().copied().collect()
}

/// `logFactorial()` (Stirling's approximation).
fn log_factorial(n: i32) -> f64 {
    if n < 2 {
        return 0.0;
    }
    let two_pi = 2.0 * (-1.0f64).acos();
    let d = n as f64;
    d * crate::math::dinkum_log(d) - d + 0.5 * crate::math::dinkum_log(two_pi * d) + 1.0 / (12.0 * d)
}

/// The password generator.
pub struct PasswordGenerator {
    custom_char_set: W32String,
    custom_char_set_freq: Option<CharSetFreq>,
    custom_char_set_type: CharSetType,
    custom_char_set_unique_size: i32,
    custom_char_set_entropy: f64,
    custom_char_set_non_lc: bool,
    char_set_codes: Vec<W32String>,
    char_set_decodes: [W32String; PASSWGEN_NUMCHARSETCODES],
    include_char_sets: [W32String; PASSWGEN_NUMINCLUDECHARSETS],
    custom_subsets: [W32String; PASSWGEN_NUMINCLUDECHARSETS],
    format_char_sets: [W32String; PASSWGEN_NUMFORMATCHARSETS],
    word_sep: W32String,
    word_char_sep: W32String,
    word_list: Vec<W32String>,
    word_list_size: i32,
    word_list_entropy: f64,
    ambig_char_set: W32String,
    ambig_groups: Vec<W32String>,
    phonetic_sigma: u32,
    phonetic_tris: Vec<u32>,
    phonetic_entropy: f64,
}

impl Default for PasswordGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl PasswordGenerator {
    /// Constructor: alphanumeric default set, built-in word list and trigrams.
    pub fn new() -> Self {
        let mut g = PasswordGenerator {
            custom_char_set: Vec::new(),
            custom_char_set_freq: None,
            custom_char_set_type: CharSetType::Standard,
            custom_char_set_unique_size: 0,
            custom_char_set_entropy: 0.0,
            custom_char_set_non_lc: false,
            char_set_codes: CHARSET_CODES.iter().map(|s| ascii_to_w32(s)).collect(),
            char_set_decodes: std::array::from_fn(|i| ascii_to_w32(CHARSET_DECODES[i])),
            include_char_sets: Default::default(),
            custom_subsets: Default::default(),
            format_char_sets: Default::default(),
            word_sep: Vec::new(),
            word_char_sep: Vec::new(),
            word_list: Vec::new(),
            word_list_size: WORDLIST_DEFAULT_SIZE,
            word_list_entropy: crate::math::delphi_log2(WORDLIST_DEFAULT_SIZE as f64),
            ambig_char_set: Vec::new(),
            ambig_groups: Vec::new(),
            phonetic_sigma: PHONETIC_SIGMA,
            phonetic_tris: Vec::new(),
            phonetic_entropy: PHONETIC_ENTROPY,
        };
        g.setup_char_sets(CHARSET_FORMAT[2], "", "", false, false, false);
        g
    }

    // ----------------------------------------------------------------------
    // character sets

    /// `MakeCharSetUnique()`: sorted set of the characters of `src`, minus
    /// ambiguous characters (by groups if there are at least two groups,
    /// otherwise by the plain list). Removed characters are appended to
    /// `removed` in the order they are tested.
    pub fn make_char_set_unique(
        src: &[W32],
        ambig_chars: Option<&[W32]>,
        ambig_groups: Option<&[W32String]>,
        mut removed: Option<&mut W32String>,
    ) -> W32String {
        let mut chset = sorted_unique(src);
        match (ambig_groups, ambig_chars) {
            (Some(groups), _) if groups.len() >= 2 => {
                for g in groups {
                    let matches = g.iter().filter(|c| chset.contains(c)).count();
                    if matches >= 2 {
                        for c in g {
                            if chset.remove(c) {
                                if let Some(r) = removed.as_deref_mut() {
                                    r.push(*c);
                                }
                            }
                        }
                    }
                }
            }
            (_, Some(ambig)) => {
                for c in ambig {
                    if chset.remove(c) {
                        if let Some(r) = removed.as_deref_mut() {
                            r.push(*c);
                        }
                    }
                }
            }
            _ => {}
        }
        chset.into_iter().collect()
    }

    /// `CreateSetOfAmbiguousChars()`: the set of ambiguous characters; if the
    /// definition contains space-separated groups (at least two), they are
    /// returned in `groups`.
    pub fn create_set_of_ambiguous_chars(ambig: &[W32], groups_out: &mut Vec<W32String>) -> W32String {
        let mut chset = BTreeSet::new();
        let mut groups: Vec<W32String> = Vec::new();
        let sep_pos = ambig.iter().position(|&c| c == SPACE);
        if let Some(p) = sep_pos.filter(|&p| p >= 2 && p as isize <= ambig.len() as isize - 3) {
            let _ = p;
            let mut group = W32String::new();
            for i in 0..=ambig.len() {
                if i == ambig.len() || ambig[i] == SPACE {
                    if group.len() >= 2 {
                        groups.push(std::mem::take(&mut group));
                    }
                    group.clear();
                } else if chset.insert(ambig[i]) {
                    group.push(ambig[i]);
                }
            }
        } else {
            chset.extend(ambig.iter().copied());
        }
        if groups.len() >= 2 {
            *groups_out = groups;
        }
        chset.into_iter().collect()
    }

    /// `ParseCharSet()`. Returns the unique character set, its type and — if
    /// `want_freq` — the subset frequency list.
    fn parse_char_set(
        &self,
        input: &[W32],
        include_char_from_each_subset: bool,
        want_freq: bool,
    ) -> Option<(W32String, CharSetType, Option<CharSetFreq>)> {
        if input.len() < 2 {
            return None;
        }
        let mut input: W32String = input.to_vec();
        if input[0] == b'[' as W32 {
            if let Some(pos) = input.iter().position(|&c| c == b']' as W32) {
                if pos >= 2 {
                    input.drain(..=pos);
                }
            }
        }
        if input == ascii_to_w32(CHARSET_CODE_PHONETIC) {
            return Some((ascii_to_w32(CHARSET_DECODES[1]), CharSetType::Phonetic, None));
        }
        if input == ascii_to_w32(CHARSET_CODE_PHONETIC_UPPERCASE) {
            return Some((ascii_to_w32(CHARSET_DECODES[0]), CharSetType::PhoneticUpperCase, None));
        }
        if input == ascii_to_w32(CHARSET_CODE_PHONETIC_MIXEDCASE) {
            let mut s = ascii_to_w32(CHARSET_DECODES[0]);
            s.extend(ascii_to_w32(CHARSET_DECODES[1]));
            return Some((s, CharSetType::PhoneticMixedCase, None));
        }

        const LT: W32 = b'<' as W32;
        const GT: W32 = b'>' as W32;
        let n = input.len();
        let mut freq_map: BTreeMap<W32String, i32> = BTreeMap::new();
        let mut state = 0;
        let mut pot_code = W32String::new();
        let mut chars = W32String::new();
        let mut i = 0;
        while i < n {
            let c = input[i];
            if c == LT && state != 2 {
                if state != 0 {
                    chars.append(&mut pot_code);
                    chars.push(LT);
                }
                if i + 1 < n && input[i + 1] == LT {
                    state = 2;
                    i += 1;
                } else {
                    state = 1;
                }
                i += 1;
                continue;
            }
            if c == GT && state != 0 {
                let mut key: Option<W32String> = None;
                let mut valid = false;
                if state == 1 && pot_code.len() >= 2 {
                    for k in 0..CHARSET_CODES.len() {
                        if pot_code == self.char_set_codes[k] {
                            let decoded = self.char_set_decodes[CHARSET_CODES_MAP[k]].clone();
                            freq_map.entry(decoded.clone()).or_insert(0);
                            key = Some(decoded);
                            valid = true;
                            break;
                        }
                    }
                    if !valid && pot_code.len() == 2 {
                        let (c1, c2) = (pot_code[0], pot_code[1]);
                        if c2 > c1 && c2 < 0xd800 && c2 - c1 <= 256 {
                            let set: W32String = (c1..=c2).collect();
                            freq_map.entry(set.clone()).or_insert(0);
                            key = Some(set);
                            valid = true;
                        }
                    }
                } else if state == 2 && !pot_code.is_empty() {
                    if i + 1 < n && input[i + 1] == GT && !(i + 2 < n && input[i + 2] == GT) {
                        let chset = Self::make_char_set_unique(&pot_code, None, None, None);
                        if !chset.is_empty() {
                            freq_map.entry(chset.clone()).or_insert(0);
                            key = Some(chset);
                            i += 1;
                            valid = true;
                        }
                    } else {
                        pot_code.push(GT);
                        i += 1;
                        continue;
                    }
                }

                if let Some(k) = key {
                    if n - i >= 3 && input[i + 1] == b':' as W32 && is_ascii_digit(input[i + 2]) {
                        let mut s_freq = String::new();
                        let mut at_least = false;
                        let mut j = i + 2;
                        while j < n && s_freq.len() <= 5 {
                            if is_ascii_digit(input[j]) {
                                s_freq.push(char::from_u32(input[j]).unwrap());
                            } else {
                                if input[j] == b'+' as W32 {
                                    at_least = true;
                                }
                                break;
                            }
                            j += 1;
                        }
                        if !s_freq.is_empty() {
                            let mut f: i32 = s_freq.parse().unwrap();
                            if at_least {
                                f = -f;
                                i += 1;
                            }
                            *freq_map.get_mut(&k).unwrap() = f;
                            i += 1 + s_freq.len();
                        }
                    }
                    if include_char_from_each_subset && freq_map[&k] == 0 {
                        *freq_map.get_mut(&k).unwrap() = -1;
                    }
                } else if !valid {
                    chars.append(&mut pot_code);
                    chars.push(LT);
                    chars.push(GT);
                }
                state = 0;
                pot_code.clear();
            } else if state != 0 {
                pot_code.push(c);
            } else {
                chars.push(c);
            }
            i += 1;
        }

        if state != 0 {
            chars.append(&mut pot_code);
            chars.push(LT);
        }

        let mut full = chars.clone();
        let mut has_freq = false;
        for (k, v) in &freq_map {
            full.extend_from_slice(k);
            if want_freq && *v != 0 {
                has_freq = true;
            }
        }
        has_freq = has_freq && (freq_map.len() + usize::from(chars.len() >= 2)) >= 2;

        let mut removed = W32String::new();
        let full = Self::make_char_set_unique(
            &full,
            Some(&self.ambig_char_set),
            Some(&self.ambig_groups),
            if has_freq { Some(&mut removed) } else { None },
        );

        if full.len() < 2 {
            return None;
        }

        if has_freq {
            let mut unique_freq: BTreeMap<W32String, i32> = BTreeMap::new();
            if removed.is_empty() {
                unique_freq = freq_map.clone();
            } else {
                for (k, v) in &freq_map {
                    let cur = Self::make_char_set_unique(k, Some(&removed), None, None);
                    if !cur.is_empty() {
                        unique_freq.entry(cur).or_insert(*v);
                    }
                }
            }
            if !chars.is_empty() {
                let input_set = Self::make_char_set_unique(&chars, Some(&removed), None, None);
                if !input_set.is_empty() {
                    unique_freq.entry(input_set).or_insert(0);
                }
            }
            if unique_freq.len() >= 2 {
                let mut common = W32String::new();
                let mut freq_list: CharSetFreq = Vec::new();
                for (k, v) in &unique_freq {
                    let num = v.abs();
                    if num > 0 {
                        freq_list.push((k.clone(), num));
                    }
                    if *v <= 0 {
                        common.extend_from_slice(k);
                    }
                }
                if common.len() < 2 {
                    for k in unique_freq.keys() {
                        common.extend_from_slice(k);
                    }
                }
                let common = Self::make_char_set_unique(&common, Some(&removed), None, None);
                if common.len() >= 2 {
                    freq_list.sort_by(|a, b| b.1.cmp(&a.1));
                    freq_list.push((common, i32::MAX));
                    return Some((full, CharSetType::StandardWithFreq, Some(freq_list)));
                }
            }
        }

        Some((full, CharSetType::Standard, None))
    }

    /// `SetupCharSets()`: set the custom character set and the redefinable
    /// sets. Returns the resulting custom set as a string (empty if invalid).
    pub fn setup_char_sets(
        &mut self,
        custom_chars: &str,
        ambig_chars: &str,
        special_symbols: &str,
        include_char_from_each_subset: bool,
        exclude_ambig_chars: bool,
        determine_subsets: bool,
    ) -> String {
        let mut ambig_groups = Vec::new();
        let ambig_set = if !ambig_chars.is_empty() {
            Self::create_set_of_ambiguous_chars(&str_to_w32(ambig_chars), &mut ambig_groups)
        } else {
            ascii_to_w32(CHARSET_AMBIGUOUS)
        };

        if exclude_ambig_chars {
            self.ambig_char_set = ambig_set.clone();
            self.ambig_groups = ambig_groups;
        } else {
            self.ambig_char_set.clear();
            self.ambig_groups.clear();
        }

        let special_set = if !special_symbols.is_empty() {
            Self::make_char_set_unique(&str_to_w32(special_symbols), None, None, None)
        } else {
            ascii_to_w32(CHARSET_SYMBOLS)
        };

        let mut etr = ascii_to_w32(CHARSET_DECODES[CHARSET_CODES_AZ]);
        etr.extend(ascii_to_w32(CHARSET_DECODES[CHARSET_CODES_LC_AZ]));
        etr.extend(ascii_to_w32(CHARSET_DECODES[CHARSET_CODES_09]));
        self.char_set_decodes[CHARSET_CODES_EASYTOREAD] = Self::make_char_set_unique(&etr, Some(&ambig_set), None, None);
        self.char_set_decodes[CHARSET_CODES_SYMBOLS] = special_set.clone();

        if self.char_set_decodes[CHARSET_CODES_HIGHANSI].is_empty() {
            let bytes: Vec<u8> = (0..129u32).map(|i| (127 + i) as u8).collect();
            let s = crate::text_file::ansi_encoding().decode_without_bom_handling(&bytes).0;
            self.char_set_decodes[CHARSET_CODES_HIGHANSI] = str_to_w32(&s);
        }

        let result = self.parse_char_set(&str_to_w32(custom_chars), include_char_from_each_subset, true);

        self.custom_char_set_non_lc = false;
        if let Some((set, _, _)) = &result {
            self.custom_char_set_non_lc = set.iter().any(|&c| c < b'a' as W32 || c > b'z' as W32);
        }

        let custom_str = result.as_ref().map(|(s, _, _)| w32_to_string(s)).unwrap_or_default();
        self.custom_char_set_unique_size = 0;

        if let Some((set, ty, freq)) = result {
            self.custom_char_set = set;
            self.custom_char_set_type = ty;
            self.custom_char_set_freq = freq;
            match ty {
                CharSetType::Standard => {
                    self.custom_char_set_entropy = crate::math::delphi_log2(self.custom_char_set.len() as f64);
                    self.custom_char_set_unique_size = self.custom_char_set.len() as i32;
                }
                CharSetType::StandardWithFreq => {
                    self.custom_char_set_entropy = crate::math::delphi_log2(self.custom_char_set.len() as f64);
                    let mut chset = BTreeSet::new();
                    let mut unique = 0;
                    for (s, f) in self.custom_char_set_freq.as_ref().unwrap() {
                        let mut cur = 0;
                        for c in s {
                            if chset.insert(*c) {
                                cur += 1;
                                if cur == *f {
                                    break;
                                }
                            }
                        }
                        unique += cur;
                    }
                    self.custom_char_set_unique_size = unique;
                }
                CharSetType::Phonetic | CharSetType::PhoneticUpperCase => {
                    self.custom_char_set_entropy = self.phonetic_entropy;
                    self.custom_char_set_unique_size = i32::MAX;
                }
                CharSetType::PhoneticMixedCase => {
                    self.custom_char_set_entropy = self.phonetic_entropy + 1.0;
                    self.custom_char_set_unique_size = i32::MAX;
                }
            }
        }

        self.include_char_sets[CHARSET_INCLUDE_AZ] = self.char_set_decodes[CHARSET_CODES_AZ].clone();
        self.include_char_sets[CHARSET_INCLUDE_LC_AZ] = self.char_set_decodes[CHARSET_CODES_LC_AZ].clone();
        self.include_char_sets[CHARSET_INCLUDE_09] = self.char_set_decodes[CHARSET_CODES_09].clone();
        self.include_char_sets[CHARSET_INCLUDE_SPECIAL] = special_set.clone();

        if exclude_ambig_chars {
            for i in 0..PASSWGEN_NUMINCLUDECHARSETS {
                let s = Self::make_char_set_unique(&self.include_char_sets[i], Some(&ambig_set), None, None);
                if s.len() >= 2 {
                    self.include_char_sets[i] = s;
                }
            }
        }

        for i in 0..PASSWGEN_NUMFORMATCHARSETS {
            self.format_char_sets[i] = ascii_to_w32(CHARSET_FORMAT[i]);
        }
        self.format_char_sets[CHARSET_FORMAT_X] = self.custom_char_set.clone();
        self.format_char_sets[CHARSET_FORMAT_E] = self.char_set_decodes[CHARSET_CODES_EASYTOREAD].clone();
        self.format_char_sets[CHARSET_FORMAT_LC_S] = special_set.clone();
        let mut s_set = ascii_to_w32(CHARSET_FORMAT[CHARSET_FORMAT_S]);
        s_set.extend_from_slice(&special_set);
        self.format_char_sets[CHARSET_FORMAT_S] = Self::make_char_set_unique(&s_set, None, None, None);
        self.format_char_sets[CHARSET_FORMAT_Y] = self.char_set_decodes[CHARSET_CODES_HIGHANSI].clone();

        if exclude_ambig_chars {
            for idx in CHARSET_FORMAT_CONST {
                let s = Self::make_char_set_unique(&self.format_char_sets[idx], Some(&ambig_set), None, None);
                if s.len() >= 2 {
                    self.format_char_sets[idx] = s;
                }
            }
        }

        if determine_subsets {
            for i in 0..PASSWGEN_NUMINCLUDECHARSETS {
                let subset: W32String =
                    self.custom_char_set.iter().copied().filter(|c| self.include_char_sets[i].contains(c)).collect();
                self.custom_subsets[i] = subset;
            }
        }

        custom_str
    }

    // ----------------------------------------------------------------------
    // word lists

    /// `LoadWordListFile()`: load a word list (`path = None`: built-in list).
    /// Words are separated by newlines, tabs or spaces; duplicates are
    /// dropped. Returns the number of words.
    pub fn load_word_list_file(
        &mut self,
        path: Option<&Path>,
        min_word_len: i32,
        max_word_len: i32,
        convert_to_lc: bool,
        default_enc: CharEncoding,
    ) -> Result<i32, WordListError> {
        let mut num_words = WORDLIST_DEFAULT_SIZE;
        if let Some(path) = path {
            let (text, enc) = read_text_file(path, default_enc).map_err(WordListError::Io)?;
            let min = min_word_len.max(1);
            let max = max_word_len.max(min);
            let mut seen: HashSet<W32String> = HashSet::new();
            let mut list: Vec<W32String> = Vec::new();
            for token in split_tokens(&text, &['\n', '\t', ' ']) {
                if list.len() >= WORDLIST_MAX_SIZE {
                    break;
                }
                // ReadString() reads at most 1023 UTF-16 code units per token
                // (including the separator); longer tokens are an error for
                // 8-bit encodings and are truncated for UTF-16.
                let units = crate::unicode::utf16_len(token) + 1;
                let token: std::borrow::Cow<str> = if units > 1023 {
                    match enc {
                        CharEncoding::Ansi | CharEncoding::Utf8 => {
                            return Err(WordListError::Io(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Invalid ANSI or UTF-8 character encoding, or Unicode string too long",
                            )));
                        }
                        _ => {
                            let mut s = String::new();
                            let mut n = 0;
                            for ch in token.chars() {
                                n += ch.len_utf16();
                                if n > 1023 {
                                    break;
                                }
                                s.push(ch);
                            }
                            s.into()
                        }
                    }
                } else {
                    token.into()
                };
                let word = crate::unicode::vcl_trim(&token);
                if word.is_empty() {
                    continue;
                }
                let len = word.chars().count() as i32;
                if len < min || len > max {
                    continue;
                }
                let w32 = if convert_to_lc {
                    str_to_w32(&word.to_ascii_lowercase())
                } else {
                    str_to_w32(word)
                };
                if seen.insert(w32.clone()) {
                    list.push(w32);
                }
            }
            num_words = list.len() as i32;
            if num_words < 2 {
                return Err(WordListError::TooFewWords);
            }
            self.word_list = list;
        } else {
            self.word_list = Vec::new();
        }
        self.word_list_size = num_words;
        self.word_list_entropy = crate::math::delphi_log2(num_words as f64);
        Ok(num_words)
    }

    /// Word with the given index (`GetWord()`).
    pub fn get_word(&self, index: i32) -> String {
        w32_to_string(&self.word_w32(index))
    }

    fn word_w32(&self, index: i32) -> W32String {
        if self.word_list.is_empty() {
            ascii_to_w32(DICEWARE8K[(index & 0x1fff) as usize])
        } else {
            self.word_list[index as usize].clone()
        }
    }

    // ----------------------------------------------------------------------
    // generation

    /// `GetPassword()`: password of `length` characters from the custom set.
    pub fn get_password(&self, rng: &mut dyn RandomGenerator, length: i32, mut flags: u32) -> SecureW32String {
        let mut dest: SecureW32String = Zeroizing::new(Vec::new());
        if length < 1 {
            return dest;
        }
        let mut length = length as usize;
        let mut passw_char_set: Option<BTreeSet<W32>> = None;

        if let Some(freq) = &self.custom_char_set_freq {
            flags &= !PASSW_FLAG_EXCLUDEREPCHARS;
            let mut freq = freq.clone();
            let mut item_idx = 0;
            let mut stop = false;
            for p in 0..freq.len() {
                let count = freq[p].1;
                let mut i = 0;
                while i < count && !freq[p].0.is_empty() && dest.len() < length {
                    let set_len = freq[p].0.len() as u32;
                    let ch = freq[p].0[rng.get_num_range(set_len) as usize];
                    dest.push(ch);
                    if flags & PASSW_FLAG_EACHCHARONLYONCE != 0 && dest.len() < length {
                        for set in freq.iter_mut().skip(item_idx) {
                            if let Ok(pos) = set.0.binary_search(&ch) {
                                set.0.remove(pos);
                            }
                        }
                    }
                    i += 1;
                }
                if dest.len() >= length {
                    stop = true;
                }
                if stop {
                    break;
                }
                item_idx += 1;
            }
            length = dest.len();
            if length >= 2 {
                permute(rng, &mut dest[..]);
            }
        } else {
            if flags & PASSW_FLAG_EACHCHARONLYONCE != 0 && flags & PASSW_FLAG_CHECKDUPLICATESBYSET != 0 {
                passw_char_set = Some(BTreeSet::new());
            }
            let set_size = self.custom_char_set.len() as u32;
            while dest.len() < length {
                let ch = self.custom_char_set[rng.get_num_range(set_size) as usize];
                let i = dest.len();
                if i == 0
                    && self.custom_char_set_non_lc
                    && flags & PASSW_FLAG_FIRSTCHARNOTLC != 0
                    && (b'a' as W32..=b'z' as W32).contains(&ch)
                {
                    continue;
                }
                if flags & PASSW_FLAG_EACHCHARONLYONCE != 0 {
                    if let Some(set) = passw_char_set.as_mut() {
                        if !set.insert(ch) {
                            continue;
                        }
                    } else if i > 0 && dest.contains(&ch) {
                        continue;
                    }
                } else if i > 0 && flags & PASSW_FLAG_EXCLUDEREPCHARS != 0 && ch == dest[i - 1] {
                    continue;
                }
                dest.push(ch);
            }
        }

        if flags >= PASSW_FLAG_INCLUDEUPPERCASE {
            let mut rand_perm: [i64; PASSWGEN_NUMINCLUDECHARSETS] = [0; PASSWGEN_NUMINCLUDECHARSETS];
            let char_sets = if flags & PASSW_FLAG_INCLUDESUBSET != 0 {
                &self.custom_subsets
            } else {
                &self.include_char_sets
            };
            let mut j = 0usize;
            let mut i = 0usize;
            while i < PASSWGEN_NUMINCLUDECHARSETS && j < length {
                let flag_val = PASSW_FLAG_INCLUDEUPPERCASE << i;
                if flags & flag_val == 0 || char_sets[i].is_empty() {
                    i += 1;
                    continue;
                }
                let mut rand: i64;
                loop {
                    if flag_val == PASSW_FLAG_INCLUDELOWERCASE && flags & PASSW_FLAG_FIRSTCHARNOTLC != 0 {
                        if length - j >= 2 {
                            rand = 1 + rng.get_num_range(length as u32 - 1) as i64;
                        } else {
                            rand = -1;
                            break;
                        }
                    } else {
                        rand = rng.get_num_range(length as u32) as i64;
                    }
                    if !rand_perm[..j].contains(&rand) {
                        break;
                    }
                }
                if rand < 0 {
                    i += 1;
                    continue;
                }
                rand_perm[j] = rand;
                j += 1;
                let r = rand as usize;

                let set = &char_sets[i];
                if set.contains(&dest[r]) {
                    i += 1;
                    continue;
                }
                if flags & PASSW_FLAG_EACHCHARONLYONCE != 0 && set.iter().all(|c| dest.contains(c)) {
                    i += 1;
                    continue;
                }
                let set_size = set.len() as u32;
                let ch = loop {
                    let ch = set[rng.get_num_range(set_size) as usize];
                    if flags & PASSW_FLAG_EACHCHARONLYONCE != 0 {
                        if let Some(s) = passw_char_set.as_mut() {
                            if !s.insert(ch) {
                                continue;
                            }
                        } else if dest.contains(&ch) {
                            continue;
                        }
                    } else if flags & PASSW_FLAG_EXCLUDEREPCHARS != 0 && set_size >= 3 {
                        if r > 0 && ch == dest[r - 1] {
                            continue;
                        }
                        if r < length - 1 && ch == dest[r + 1] {
                            continue;
                        }
                    }
                    break ch;
                };
                dest[r] = ch;
                i += 1;
            }
        }

        if flags & PASSW_FLAG_REMOVEWHITESPACE != 0 {
            remove_whitespace(&mut dest);
        }
        dest
    }

    /// `GetPassphrase()`: `words` words, optionally combined with `chars`.
    /// Returns the passphrase and the number of characters belonging to the
    /// words (incl. separators between words).
    pub fn get_passphrase(
        &self,
        rng: &mut dyn RandomGenerator,
        words: i32,
        chars: &[W32],
        flags: u32,
    ) -> (SecureW32String, i32) {
        let mut dest: SecureW32String = Zeroizing::new(Vec::new());
        if words < 1 {
            return (dest, 0);
        }
        let chars_len = chars.len() as i32;
        let mut append_chars = false;
        let word_sep: &[W32] = if self.word_sep.is_empty() { &[SPACE] } else { &self.word_sep };
        let wch_sep: &[W32] = if self.word_char_sep.is_empty() { &[b'-' as W32] } else { &self.word_char_sep };

        if chars_len > 0 && flags & PASSPHR_FLAG_COMBINEWCH == 0 {
            if flags & PASSPHR_FLAG_REVERSEWCHORDER != 0 {
                append_chars = true;
            } else {
                dest.extend_from_slice(chars);
                dest.extend_from_slice(word_sep);
            }
        }

        let chars_per_word = if chars_len > 0 { chars_len / words } else { 0 };
        let chars_rest = if chars_len > 0 { chars_len % words } else { 0 };
        let mut chars_pos = 0i32;
        let mut unique_idx: Option<HashSet<u32>> =
            if flags & PASSPHR_FLAG_EACHWORDONLYONCE != 0 { Some(HashSet::new()) } else { None };
        let mut net_words_len = 0i32;

        let mut i = 0;
        while i < words {
            let rand = rng.get_num_range(self.word_list_size as u32);
            if let Some(set) = unique_idx.as_mut() {
                if !set.insert(rand) {
                    continue;
                }
            }
            let mut word = Zeroizing::new(self.word_w32(rand as i32));
            let word_len = word.len() as i32;
            if flags & PASSPHR_FLAG_CAPITALIZEWORDS != 0 && !word.is_empty() {
                word[0] = ascii_upper(word[0]);
            }

            if flags & PASSPHR_FLAG_COMBINEWCH != 0 && chars_pos < chars_len {
                let mut to_copy = chars_per_word;
                if i < chars_rest {
                    to_copy += 1;
                }
                let part = &chars[chars_pos as usize..(chars_pos + to_copy) as usize];
                if flags & PASSPHR_FLAG_REVERSEWCHORDER != 0 {
                    dest.extend_from_slice(part);
                    if flags & PASSPHR_FLAG_DONTSEPWCH == 0 {
                        dest.extend_from_slice(wch_sep);
                    }
                    dest.extend_from_slice(&word);
                } else {
                    dest.extend_from_slice(&word);
                    if flags & PASSPHR_FLAG_DONTSEPWCH == 0 {
                        dest.extend_from_slice(wch_sep);
                    }
                    dest.extend_from_slice(part);
                }
                chars_pos += to_copy;
            } else {
                dest.extend_from_slice(&word);
            }

            net_words_len += word_len;

            i += 1;
            if i < words && flags & PASSPHR_FLAG_DONTSEPWORDS == 0 {
                dest.extend_from_slice(word_sep);
                net_words_len += word_sep.len() as i32;
            }
        }

        if append_chars {
            dest.extend_from_slice(word_sep);
            dest.extend_from_slice(chars);
        }

        (dest, net_words_len)
    }

    /// `GetFormatPassw()`: password according to a format string.
    ///
    /// `max_len` is the maximum length of the result (the original derives it
    /// from the size of the caller's buffer). `passw` is the password that `P`
    /// inserts. If `security` is given, the estimated entropy is added to it.
    pub fn get_format_passw(
        &self,
        rng: &mut dyn RandomGenerator,
        format: &[W32],
        flags: u32,
        passw: Option<&[W32]>,
        max_len: usize,
        mut security: Option<&mut f64>,
    ) -> FormatPasswResult {
        let mut dest: SecureW32String = Zeroizing::new(Vec::new());
        let mut passw_used = if passw.is_some() { PASSFORMAT_PWUSED_NOSPECIFIER } else { 0 };
        let mut passw_used_set = false;
        let mut invalid_spec = W32String::new();

        if max_len < 1 || format.is_empty() {
            return FormatPasswResult { passw: dest, passw_used: 0, invalid_spec };
        }
        let max_dest_len = max_len.min(1_000_000_000);
        let format_len = format.len();
        let mut src_idx = 0usize;
        let mut verbatim = false;
        let mut unique = false;
        let mut second_num = false;
        let mut num_buf = String::new();
        let mut num_default = true;
        let mut num: i32 = 1;
        let mut repeat_idx = 0usize;
        let mut repeat_num = [0i32; FORMAT_REPEAT_MAXDEPTH];
        let mut repeat_start = [0usize; FORMAT_REPEAT_MAXDEPTH];
        let mut perm_num: i32 = 0;
        // Uninitialised in the original if '}' comes without '{'.
        let mut perm_start = 0usize;
        let mut user_char_set_num: i32 = 0;
        let mut user_char_set_start = 0usize;
        let mut perm_security = 0.0f64;

        if format[0] == b'[' as W32 {
            src_idx += 1;
            while src_idx < format_len && format[src_idx] != b']' as W32 {
                src_idx += 1;
            }
            src_idx += 1;
        }

        while src_idx < format_len && dest.len() < max_dest_len {
            let ch = format[src_idx];

            if user_char_set_num > 0 {
                let mut ends = false;
                while ch == b'>' as W32 && src_idx < format_len - 1 && format[src_idx + 1] == b'>' as W32 {
                    src_idx += 1;
                    ends = true;
                }
                if !ends {
                    src_idx += 1;
                    continue;
                }
            }

            if ch == b'"' as W32 {
                if src_idx < format_len - 1 && format[src_idx + 1] == b'"' as W32 {
                    src_idx += 1;
                } else {
                    verbatim = !verbatim;
                    src_idx += 1;
                    continue;
                }
            }

            if verbatim {
                dest.push(ch);
                src_idx += 1;
                continue;
            }

            if ch == b'*' as W32 {
                unique = true;
                src_idx += 1;
                continue;
            }

            if is_ascii_digit(ch) && num_buf.len() < 5 {
                num_buf.push(char::from_u32(ch).unwrap());
                src_idx += 1;
                continue;
            }

            let mut parsed = 1i32;
            if !num_buf.is_empty() {
                parsed = num_buf.parse().unwrap_or(0);
                if parsed > 0 {
                    num_default = false;
                } else {
                    parsed = 1;
                }
            }

            if second_num {
                if !num_default && num != parsed {
                    num = num.min(parsed) + rng.get_num_range((parsed - num).unsigned_abs() + 1) as i32;
                }
            } else {
                num = parsed;
                if !num_default && ch == b'-' as W32 {
                    second_num = true;
                    num_buf.clear();
                    src_idx += 1;
                    continue;
                }
            }

            let mut user_char_set: Option<W32String> = None;
            let mut use_custom = false;
            let mut format_set: Option<usize> = None;
            let mut char_set_type = CharSetType::Standard;

            match char::from_u32(ch).unwrap_or('\0') {
                '<' => {
                    if src_idx < format_len - 1 && format[src_idx + 1] == b'<' as W32 {
                        user_char_set_num = num;
                        src_idx += 1;
                        user_char_set_start = src_idx + 1;
                    }
                }
                '>' => {
                    if user_char_set_num > 0 {
                        let len = src_idx as isize - user_char_set_start as isize - 1;
                        if len >= 2 {
                            let sub = &format[user_char_set_start..user_char_set_start + len as usize];
                            if let Some((set, _, _)) = self.parse_char_set(sub, false, false) {
                                user_char_set = Some(set);
                                num = user_char_set_num;
                            }
                        }
                    }
                    user_char_set_num = 0;
                }
                'P' => {
                    if let Some(pw) = passw {
                        let to_copy = pw.len().min(max_dest_len - dest.len());
                        dest.extend_from_slice(&pw[..to_copy]);
                        if !passw_used_set {
                            passw_used = to_copy as i32;
                            passw_used_set = true;
                        }
                    } else if !passw_used_set {
                        passw_used = PASSFORMAT_PWUSED_EMPTYPASSW;
                        passw_used_set = true;
                    }
                }
                'q' => char_set_type = CharSetType::Phonetic,
                'Q' => char_set_type = CharSetType::PhoneticUpperCase,
                'r' => char_set_type = CharSetType::PhoneticMixedCase,
                'W' | 'w' => {
                    let mut unique_idx: Option<BTreeSet<u32>> = None;
                    if unique {
                        num = if num_default { self.word_list_size } else { num.min(self.word_list_size) };
                        unique_idx = Some(BTreeSet::new());
                    }
                    let mut i = 0;
                    while i < num && dest.len() < max_dest_len {
                        let rand = rng.get_num_range(self.word_list_size as u32);
                        let word = Zeroizing::new(self.word_w32(rand as i32));
                        if let Some(set) = unique_idx.as_mut() {
                            if !set.insert(rand) {
                                continue;
                            }
                        }
                        let to_copy = word.len().min(max_dest_len - dest.len());
                        dest.extend_from_slice(&word[..to_copy]);
                        if ch == b'w' as W32 && i < num - 1 && dest.len() < max_dest_len {
                            if self.word_sep.is_empty() {
                                dest.push(SPACE);
                            } else {
                                let to_copy = self.word_sep.len().min(max_dest_len - dest.len());
                                dest.extend_from_slice(&self.word_sep[..to_copy]);
                            }
                        }
                        i += 1;
                    }
                    if let Some(sec) = security.as_deref_mut() {
                        if unique {
                            *sec += Self::calc_perm_set_entropy(self.word_list_size, i);
                        } else {
                            *sec += self.word_list_entropy * i as f64;
                        }
                    }
                }
                '[' => {
                    if repeat_idx < FORMAT_REPEAT_MAXDEPTH {
                        repeat_num[repeat_idx] = num - 1;
                        repeat_start[repeat_idx] = src_idx;
                        repeat_idx += 1;
                    }
                }
                ']' => {
                    if repeat_idx > 0 {
                        let top = repeat_idx - 1;
                        if repeat_num[top] == 0 || src_idx - repeat_start[top] < 3 {
                            repeat_idx -= 1;
                        } else {
                            let was = repeat_num[top];
                            repeat_num[top] -= 1;
                            if was > 0 {
                                src_idx = repeat_start[top];
                            }
                        }
                    }
                }
                '{' => {
                    perm_num = if num_default { 0 } else { num };
                    perm_start = dest.len();
                    if let Some(sec) = security.as_deref() {
                        perm_security = *sec;
                    }
                }
                '}' => {
                    if perm_num >= 0 {
                        let perm_size = dest.len() as isize - perm_start as isize;
                        if perm_size >= 2 {
                            let perm_size = perm_size as usize;
                            permute(rng, &mut dest[perm_start..perm_start + perm_size]);
                            let to_use =
                                if perm_num == 0 { perm_size } else { (perm_num as usize).min(perm_size) };
                            dest.truncate(perm_start + to_use);
                            if let Some(sec) = security.as_deref_mut() {
                                if to_use < perm_size {
                                    *sec = perm_security
                                        + (*sec - perm_security) * to_use as f64 / perm_size as f64;
                                }
                            }
                        }
                        perm_num = -1;
                    }
                }
                _ => {
                    if is_ascii_alpha(ch) {
                        if let Some(p) = FORMAT_PLACEHOLDERS.bytes().position(|b| b as W32 == ch) {
                            if p == CHARSET_FORMAT_X {
                                use_custom = true;
                            } else {
                                format_set = Some(p);
                            }
                        } else {
                            invalid_spec.push(ch);
                        }
                    } else {
                        dest.push(ch);
                    }
                }
            }

            if matches!(
                char_set_type,
                CharSetType::Phonetic | CharSetType::PhoneticUpperCase | CharSetType::PhoneticMixedCase
            ) {
                let mut ph_flags = 0;
                if char_set_type == CharSetType::PhoneticUpperCase {
                    ph_flags |= PASSW_FLAG_PHONETICUPPERCASE;
                } else if char_set_type == CharSetType::PhoneticMixedCase {
                    ph_flags |= PASSW_FLAG_PHONETICMIXEDCASE;
                }
                let len = (num as i64).min(max_dest_len as i64 - dest.len() as i64) as i32;
                let ph = self.get_phonetic_passw(rng, len, ph_flags);
                dest.extend_from_slice(&ph);
                if let Some(sec) = security.as_deref_mut() {
                    *sec += (self.phonetic_entropy
                        + if char_set_type == CharSetType::PhoneticMixedCase { 1.0 } else { 0.0 })
                        * ph.len() as f64;
                }
            } else {
                let set: Option<&[W32]> = if let Some(s) = &user_char_set {
                    Some(s)
                } else if use_custom {
                    Some(&self.custom_char_set)
                } else {
                    format_set.map(|p| &self.format_char_sets[p][..])
                };
                if let Some(set) = set.filter(|s| s.len() >= 2) {
                    let set_size = set.len();
                    let start = dest.len();
                    if unique {
                        num = if num_default { set_size as i32 } else { num.min(set_size as i32) };
                    }
                    let mut i = 0;
                    while i < num && dest.len() < max_dest_len {
                        let c = set[rng.get_num_range(set_size as u32) as usize];
                        if unique && i > 0 {
                            if dest[start..start + i as usize].contains(&c) {
                                continue;
                            }
                        } else if flags & PASSFORMAT_FLAG_EXCLUDEREPCHARS != 0 && !dest.is_empty() && c == *dest.last().unwrap()
                        {
                            continue;
                        }
                        dest.push(c);
                        i += 1;
                    }
                    if let Some(sec) = security.as_deref_mut() {
                        if unique {
                            *sec += Self::calc_perm_set_entropy(set_size as i32, i);
                        } else {
                            *sec += crate::math::delphi_log2(set_size as f64) * i as f64;
                        }
                    }
                }
            }

            if user_char_set_num == 0 {
                unique = false;
                num_default = true;
            }
            second_num = false;
            num_buf.clear();
            src_idx += 1;
        }

        if flags & PASSFORMAT_FLAG_REMOVEWHITESPACE != 0 {
            remove_whitespace(&mut dest);
        }

        FormatPasswResult { passw: dest, passw_used, invalid_spec }
    }

    /// `GetPhoneticPassw()`: pronounceable password following trigram
    /// frequencies.
    pub fn get_phonetic_passw(&self, rng: &mut dyn RandomGenerator, length: i32, flags: u32) -> SecureW32String {
        let mut dest: SecureW32String = Zeroizing::new(Vec::new());
        if length < 1 {
            return dest;
        }
        let length = length as usize;
        let default_tris = self.phonetic_tris.is_empty();
        let tri = |i: usize| -> u32 { if default_tris { PHONETIC_TRIS[i] as u32 } else { self.phonetic_tris[i] } };
        let mixed_case = flags & PASSW_FLAG_PHONETICMIXEDCASE != 0;
        let base: u32 = if flags & PASSW_FLAG_PHONETICUPPERCASE != 0 { b'A' as u32 } else { b'a' as u32 };
        let get_letter = |rng: &mut dyn RandomGenerator, c: u32| -> W32 {
            if mixed_case {
                c + if rng.get_byte() & 1 != 0 { b'a' as u32 } else { b'A' as u32 }
            } else {
                c + base
            }
        };

        let rand = rng.get_num_range(self.phonetic_sigma);
        let mut sum = 0u32;
        let (mut ch1, mut ch2, mut ch3) = (0u32, 0u32, 0u32);
        for i in 0..PHONETIC_TRIS_NUM {
            sum = sum.wrapping_add(tri(i));
            if sum > rand {
                ch1 = (i / 676) as u32;
                ch2 = ((i / 26) % 26) as u32;
                ch3 = (i % 26) as u32;
                break;
            }
        }

        if length >= 1 {
            let l = get_letter(rng, ch1);
            dest.push(l);
        }
        if length >= 2 {
            let l = get_letter(rng, ch2);
            dest.push(l);
        }
        if length >= 3 {
            let l = get_letter(rng, ch3);
            dest.push(l);
        }

        while dest.len() < length {
            ch1 = ch2;
            ch2 = ch3;
            let base_idx = (676 * ch1 + 26 * ch2) as usize;
            let mut sum_freq = 0u32;
            for c in 0..26 {
                sum_freq = sum_freq.wrapping_add(tri(base_idx + c));
            }
            if sum_freq == 0 {
                const VOWELS: [u32; 5] = [0, 4, 8, 14, 20];
                ch3 = VOWELS[rng.get_num_range(5) as usize];
            } else {
                let r = rng.get_num_range(sum_freq);
                let mut s = 0u32;
                ch3 = 26;
                for c in 0..26u32 {
                    s = s.wrapping_add(tri(base_idx + c as usize));
                    if s > r {
                        ch3 = c;
                        break;
                    }
                }
            }
            let l = get_letter(rng, ch3);
            dest.push(l);
        }

        if flags >= PASSW_FLAG_INCLUDEUPPERCASE {
            let mut rand_perm: [usize; PASSWGEN_NUMINCLUDECHARSETS] = [0; PASSWGEN_NUMINCLUDECHARSETS];
            let mut j = 0usize;
            for i in 0..PASSWGEN_NUMINCLUDECHARSETS {
                if j >= length {
                    break;
                }
                let flag_val = PASSW_FLAG_INCLUDEUPPERCASE << i;
                if flags & flag_val == 0 {
                    continue;
                }
                if flag_val == PASSW_FLAG_INCLUDELOWERCASE
                    && flags & (PASSW_FLAG_PHONETICUPPERCASE | PASSW_FLAG_PHONETICMIXEDCASE) == 0
                {
                    continue;
                }
                if flag_val == PASSW_FLAG_INCLUDEUPPERCASE && flags & PASSW_FLAG_PHONETICUPPERCASE != 0 && !mixed_case {
                    continue;
                }
                let r = loop {
                    let r = rng.get_num_range(length as u32) as usize;
                    if !rand_perm[..j].contains(&r) {
                        break r;
                    }
                };
                rand_perm[j] = r;
                j += 1;
                if flag_val == PASSW_FLAG_INCLUDEUPPERCASE {
                    dest[r] = ascii_upper(dest[r]);
                } else if flag_val == PASSW_FLAG_INCLUDELOWERCASE {
                    dest[r] = ascii_lower(dest[r]);
                } else {
                    let set = &self.include_char_sets[i];
                    dest[r] = set[rng.get_num_range(set.len() as u32) as usize];
                }
            }
        }

        dest
    }

    // ----------------------------------------------------------------------
    // trigram files

    /// `CreateTrigramFile()`: count letter trigrams of a word list and write a
    /// trigram file. Returns (number of trigrams, entropy per letter).
    pub fn create_trigram_file(src: &Path, dest: &Path, default_enc: CharEncoding) -> Result<(u32, f64), TrigramError> {
        let (text, _) = read_text_file(src, default_enc)?;
        let mut tris = vec![0u32; PHONETIC_TRIS_NUM];
        let mut sigma: u32 = 0;
        for token in split_tokens(&text, &['\n', '\t', ' ']) {
            let word = crate::unicode::vcl_trim(token);
            if crate::unicode::utf16_len(word) < 3 {
                continue;
            }
            let (mut c1, mut c2): (i32, i32) = (-1, -1);
            for wch in word.encode_utf16() {
                if wch <= b'z' as u16 {
                    let c3 = (wch as u8).to_ascii_lowercase() as i32 - b'a' as i32;
                    if (0..=25).contains(&c3) {
                        if c1 >= 0 {
                            tris[(676 * c1 + 26 * c2 + c3) as usize] += 1;
                            sigma = sigma.wrapping_add(1);
                        }
                        c1 = c2;
                        c2 = c3;
                    }
                }
            }
        }
        let mut entropy = 0.0f64;
        if sigma != 0 {
            for &t in &tris {
                let p = t as f64 / sigma as f64;
                if p > 0.0 {
                    entropy += p * crate::math::delphi_log2(p);
                }
            }
            entropy = -entropy / 3.0;
        }
        if sigma == 0 || entropy < 1.0 {
            return Err(TrigramError::NotEnoughTrigrams);
        }
        let mut f = fs::File::create(dest)?;
        let mut buf = Vec::with_capacity(12 + 4 * PHONETIC_TRIS_NUM);
        buf.extend_from_slice(&sigma.to_le_bytes());
        buf.extend_from_slice(&entropy.to_le_bytes());
        for t in &tris {
            buf.extend_from_slice(&t.to_le_bytes());
        }
        f.write_all(&buf)?;
        Ok((sigma, entropy))
    }

    /// `LoadTrigramFile()`: `None` = built-in table. Returns `Ok(true)` on
    /// success, `Ok(false)` if the file structure is invalid.
    pub fn load_trigram_file(&mut self, path: Option<&Path>) -> io::Result<bool> {
        let mut tris = Vec::new();
        let mut sigma = PHONETIC_SIGMA;
        let mut entropy = PHONETIC_ENTROPY;
        if let Some(path) = path {
            let data = fs::read(path)?;
            if data.len() != PHONETIC_TRIS_NUM * 4 + 4 + 8 {
                return Ok(false);
            }
            sigma = u32::from_le_bytes(data[0..4].try_into().unwrap());
            entropy = f64::from_le_bytes(data[4..12].try_into().unwrap());
            tris = data[12..].chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
            if sigma == 0 || !(entropy >= 1.0) || entropy > crate::math::delphi_log2(26f64) {
                return Ok(false);
            }
            let check = tris.iter().fold(0u32, |a, &t| a.wrapping_add(t));
            if check != sigma {
                return Ok(false);
            }
        }
        self.phonetic_tris = tris;
        self.phonetic_sigma = sigma;
        self.phonetic_entropy = entropy;
        Ok(true)
    }

    // ----------------------------------------------------------------------
    // entropy

    /// `CalcPermSetEntropy()`: log2(N! / (N-M)!).
    pub fn calc_perm_set_entropy(set_size: i32, num_samples: i32) -> f64 {
        (log_factorial(set_size) - log_factorial(set_size - num_samples)) / crate::math::LOG2_CONST
    }

    /// `EstimatePasswSecurity()`: rough strength estimate in bits (algorithm of
    /// KeePass' `EstimatePasswordBits()`).
    pub fn estimate_passw_security(passw: &str) -> f64 {
        if passw.is_empty() {
            return 0.0;
        }
        let p = Zeroizing::new(str_to_w32(passw));
        let mut chcount: HashMap<W32, i32> = HashMap::new();
        let mut chdiff: HashMap<i32, i32> = HashMap::new();
        let (mut control, mut upper, mut lower, mut digits, mut special) = (false, false, false, false, false);
        let mut char_space = 0i32;
        let mut eff_len = 0.0f64;
        for i in 0..p.len() {
            let c = p[i];
            let mut other = false;
            let r = |a: u8, b: u8| (a as u32..=b as u32).contains(&c);
            if c > 0 && c < b' ' as u32 {
                control = true;
            } else if r(b'A', b'Z') {
                upper = true;
            } else if r(b'a', b'z') {
                lower = true;
            } else if r(b'0', b'9') {
                digits = true;
            } else if r(b' ', b'/') || r(b':', b'@') || r(b'[', b'`') || r(b'{', b'~') {
                special = true;
            } else {
                other = true;
            }
            let mut diff_factor = 1.0f64;
            if i > 0 {
                let diff = c.wrapping_sub(p[i - 1]) as i32;
                let e = chdiff.entry(diff).or_insert(0);
                *e += 1;
                diff_factor /= *e as f64;
            }
            let e = chcount.entry(c).or_insert(0);
            *e += 1;
            let count = *e;
            if count == 1 && other {
                char_space += 1;
            }
            eff_len += diff_factor / count as f64;
        }
        if control {
            char_space += 2;
        }
        if upper {
            char_space += 26;
        }
        if lower {
            char_space += 26;
        }
        if digits {
            char_space += 10;
        }
        if special {
            char_space += 33;
        }
        crate::math::delphi_log2(char_space as f64) * eff_len
    }

    // ----------------------------------------------------------------------
    // properties

    /// The custom character set as a string (`CustomCharSet`).
    pub fn custom_char_set(&self) -> String {
        w32_to_string(&self.custom_char_set)
    }
    pub fn custom_char_set_w32(&self) -> &[W32] {
        &self.custom_char_set
    }
    pub fn custom_char_set_type(&self) -> CharSetType {
        self.custom_char_set_type
    }
    pub fn custom_char_set_entropy(&self) -> f64 {
        self.custom_char_set_entropy
    }
    pub fn custom_char_set_unique_size(&self) -> i32 {
        self.custom_char_set_unique_size
    }
    pub fn custom_char_set_freq(&self) -> Option<&CharSetFreq> {
        self.custom_char_set_freq.as_ref()
    }
    pub fn word_list_entropy(&self) -> f64 {
        self.word_list_entropy
    }
    pub fn word_list_size(&self) -> i32 {
        self.word_list_size
    }
    pub fn phonetic_entropy(&self) -> f64 {
        self.phonetic_entropy
    }
    pub fn word_separator(&self) -> &[W32] {
        &self.word_sep
    }
    pub fn set_word_separator(&mut self, sep: &str) {
        self.word_sep = str_to_w32(sep);
    }
    pub fn word_char_separator(&self) -> &[W32] {
        &self.word_char_sep
    }
    pub fn set_word_char_separator(&mut self, sep: &str) {
        self.word_char_sep = str_to_w32(sep);
    }
    /// The four "include" sets (A–Z, a–z, 0–9, special).
    pub fn include_char_sets(&self) -> &[W32String; PASSWGEN_NUMINCLUDECHARSETS] {
        &self.include_char_sets
    }
    pub fn format_char_sets(&self) -> &[W32String; PASSWGEN_NUMFORMATCHARSETS] {
        &self.format_char_sets
    }
}

/// Convenience: a single character as [`W32`].
pub fn w32(c: char) -> W32 {
    char_to_w32(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::AesCtrPrng;
    use crate::unicode::w32_to_string as s;

    fn rng() -> AesCtrPrng {
        AesCtrPrng::from_seed(b"test")
    }

    #[test]
    fn default_char_set() {
        let g = PasswordGenerator::new();
        assert_eq!(g.custom_char_set(), "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz");
        assert_eq!(g.custom_char_set_type(), CharSetType::Standard);
        assert!((g.custom_char_set_entropy() - 62f64.log2()).abs() < 1e-12);
    }

    #[test]
    fn parse_codes_ranges_and_freq() {
        let mut g = PasswordGenerator::new();
        assert_eq!(g.setup_char_sets("<AZ><09>", "", "", false, false, false), "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        assert_eq!(g.setup_char_sets("<ae>xyz", "", "", false, false, false), "abcdexyz");
        assert_eq!(g.setup_char_sets("[comment]<<<>>>ab", "", "", false, false, false), "<>ab");
        g.setup_char_sets("<AZ>:2<az>:3<09>", "", "", false, false, false);
        assert_eq!(g.custom_char_set_type(), CharSetType::StandardWithFreq);
        let f = g.custom_char_set_freq().unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!((s(&f[0].0).len(), f[0].1), (26, 3));
        assert_eq!((s(&f[1].0).len(), f[1].1), (26, 2));
        assert_eq!(f[2].1, i32::MAX);
        let mut r = rng();
        let p = g.get_password(&mut r, 10, 0);
        assert_eq!(p.len(), 10);
        let pw = s(&p);
        assert!(pw.chars().filter(|c| c.is_ascii_uppercase()).count() >= 2);
        assert!(pw.chars().filter(|c| c.is_ascii_lowercase()).count() >= 3);
    }

    #[test]
    fn ambiguous_groups() {
        let mut groups = Vec::new();
        let set = PasswordGenerator::create_set_of_ambiguous_chars(&str_to_w32("B8 G6 Il1"), &mut groups);
        assert_eq!(s(&set), "168BGIl");
        assert_eq!(groups.len(), 3);
    }

    #[test]
    fn exclude_ambiguous() {
        let mut g = PasswordGenerator::new();
        let cs = g.setup_char_sets("<AZ><az><09>", "", "", false, true, false);
        for c in CHARSET_AMBIGUOUS.chars() {
            assert!(!cs.contains(c), "{c} should be excluded");
        }
    }

    #[test]
    fn password_flags() {
        let g = PasswordGenerator::new();
        let mut r = rng();
        for _ in 0..50 {
            let p = s(&g.get_password(&mut r, 20, PASSW_FLAG_EACHCHARONLYONCE | PASSW_FLAG_INCLUDEDIGIT));
            let set: HashSet<char> = p.chars().collect();
            assert_eq!(set.len(), 20);
            assert!(p.chars().any(|c| c.is_ascii_digit()));
            let p = s(&g.get_password(&mut r, 20, PASSW_FLAG_EXCLUDEREPCHARS | PASSW_FLAG_FIRSTCHARNOTLC));
            assert!(!p.starts_with(|c: char| c.is_ascii_lowercase()));
            assert!(p.as_bytes().windows(2).all(|w| w[0] != w[1]));
        }
    }

    #[test]
    fn passphrase_and_words() {
        let g = PasswordGenerator::new();
        let mut r = rng();
        let (p, net) = g.get_passphrase(&mut r, 4, &[], PASSPHR_FLAG_CAPITALIZEWORDS);
        let ps = s(&p);
        assert_eq!(ps.split(' ').count(), 4);
        assert_eq!(net as usize, ps.chars().count());
        let chars = str_to_w32("XYZ");
        let (p, _) = g.get_passphrase(&mut r, 2, &chars, PASSPHR_FLAG_COMBINEWCH);
        let ps = s(&p);
        assert!(ps.contains("-XY") && ps.ends_with("-Z"), "{ps}");
    }

    #[test]
    fn format_strings() {
        let g = PasswordGenerator::new();
        let mut r = rng();
        let mut sec = 0.0;
        let res = g.get_format_passw(&mut r, &str_to_w32("\"ID-\"5d-3[A]"), 0, None, 1000, Some(&mut sec));
        let p = s(&res.passw);
        assert!(p.starts_with("ID-"), "{p}");
        assert_eq!(p.len(), 3 + 5 + 1 + 1, "{p}");
        assert!((sec - (5.0 * 10f64.log2() + 62f64.log2())).abs() < 1e-9);
        // repetition needs at least 2 chars between the brackets
        let res = g.get_format_passw(&mut r, &str_to_w32("3[dd]"), 0, None, 1000, None);
        assert_eq!(res.passw.len(), 6);
        // P and invalid specifiers
        let pw = str_to_w32("secret");
        let res = g.get_format_passw(&mut r, &str_to_w32("P-k"), 0, Some(&pw), 1000, None);
        assert_eq!(s(&res.passw), "secret-");
        assert_eq!(res.passw_used, 6);
        assert_eq!(s(&res.invalid_spec), "k");
        // unique + permutation
        let res = g.get_format_passw(&mut r, &str_to_w32("{*10d}"), 0, None, 1000, None);
        let set: HashSet<W32> = res.passw.iter().copied().collect();
        assert_eq!(set.len(), 10);
        // user-defined set
        let res = g.get_format_passw(&mut r, &str_to_w32("8<<xy>>"), 0, None, 1000, None);
        let p = s(&res.passw);
        assert_eq!(p.len(), 8);
        assert!(p.chars().all(|c| c == 'x' || c == 'y'));
    }

    #[test]
    fn phonetic() {
        let g = PasswordGenerator::new();
        let mut r = rng();
        let p = s(&g.get_phonetic_passw(&mut r, 12, PASSW_FLAG_INCLUDEUPPERCASE | PASSW_FLAG_INCLUDEDIGIT));
        assert_eq!(p.len(), 12);
        assert!(p.chars().any(|c| c.is_ascii_uppercase()));
        assert!(p.chars().any(|c| c.is_ascii_digit()));
        let p = s(&g.get_phonetic_passw(&mut r, 12, PASSW_FLAG_PHONETICMIXEDCASE));
        assert!(p.chars().all(|c| c.is_ascii_alphabetic()));
    }

    #[test]
    fn estimate() {
        assert_eq!(PasswordGenerator::estimate_passw_security(""), 0.0);
        let e = PasswordGenerator::estimate_passw_security("abc");
        // lower only: log2(26) * (1 + 1 + 1/2)
        assert!((e - 26f64.log2() * 2.5).abs() < 1e-9, "{e}");
        assert!((PasswordGenerator::calc_perm_set_entropy(10, 10) - 21.791).abs() < 0.01);
    }

    #[test]
    fn trigram_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("pwtech-tri-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("words.txt");
        let words: Vec<&str> = DICEWARE8K.iter().copied().collect();
        fs::write(&src, words.join("\n")).unwrap();
        let dst = dir.join("tri.bin");
        let (sigma, ent) = PasswordGenerator::create_trigram_file(&src, &dst, CharEncoding::Utf8).unwrap();
        assert!(sigma > 1000 && ent > 1.0);
        let mut g = PasswordGenerator::new();
        assert!(g.load_trigram_file(Some(&dst)).unwrap());
        assert_eq!(g.phonetic_entropy(), ent);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn word_list_loading() {
        let dir = std::env::temp_dir().join(format!("pwtech-wl-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("words.txt");
        fs::write(&src, "Alpha beta\tGAMMA\r\nalpha\n\nverylongword\n").unwrap();
        let mut g = PasswordGenerator::new();
        let n = g.load_word_list_file(Some(&src), 1, 6, true, CharEncoding::Utf8).unwrap();
        assert_eq!(n, 3);
        assert_eq!(g.get_word(0), "alpha");
        assert_eq!(g.get_word(2), "gamma");
        assert!(matches!(
            g.load_word_list_file(Some(&src), 10, 20, false, CharEncoding::Utf8),
            Err(WordListError::TooFewWords)
        ));
        fs::remove_dir_all(&dir).unwrap();
    }
}
