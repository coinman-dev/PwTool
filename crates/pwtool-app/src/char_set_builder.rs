//! Character Set Builder logic (`TCharSetBuilderForm::CharSetParamChange()`,
//! `upstream/src/main/CharSetBuilder.cpp`), independent of the GUI toolkit.

use std::collections::BTreeSet;

use pwtool_core::passw::generator::CHARSET_SYMBOLS;
use pwtool_core::unicode::{W32, str_to_w32, w32_to_string};

pub const NUM_PARAM: usize = 6;
pub const CHARSET_USED: [&str; NUM_PARAM] = [
    "LowerCaseCheck",
    "UpperCaseCheck",
    "DigitsCheck",
    "SpecialSymbolsCheck",
    "CharactersRangeCheck",
    "AdditionalCharsCheck",
];
const CODES: [&str; 4] = ["<az>", "<AZ>", "<09>", "<symbols>"];

/// Input of the builder (`CharSetParamChange()`).
#[derive(Debug, Default, Clone)]
pub struct BuilderParams {
    pub used: [bool; NUM_PARAM],
    pub num_chars: [i64; NUM_PARAM],
    pub at_least: [bool; NUM_PARAM],
    pub from: String,
    pub to: String,
    pub additional: String,
    pub exclude: String,
    pub tag: String,
}

/// Error of [`build_char_set`].
#[derive(Debug, PartialEq, Eq)]
pub enum BuildError {
    InvalidRange,
}

fn default_set(i: usize) -> BTreeSet<W32> {
    match i {
        0 => (b'a'..=b'z').map(W32::from).collect(),
        1 => (b'A'..=b'Z').map(W32::from).collect(),
        2 => (b'0'..=b'9').map(W32::from).collect(),
        _ => str_to_w32(CHARSET_SYMBOLS).into_iter().collect(),
    }
}

fn set_to_string(s: &BTreeSet<W32>) -> String {
    w32_to_string(&s.iter().copied().collect::<Vec<_>>())
}

/// `CharSetParamChange()`: character set definition for the parameters
/// (empty if it would contain fewer than 2 characters).
pub fn build_char_set(p: &BuilderParams) -> Result<String, BuildError> {
    let mut result = String::new();
    let mut size: u64 = 0;
    let exclude: BTreeSet<W32> = str_to_w32(&p.exclude).into_iter().collect();
    let has_exclude = !exclude.is_empty();
    for i in 0..NUM_PARAM {
        if !p.used[i] {
            continue;
        }
        match i {
            4 => {
                let from = str_to_w32(&p.from);
                let to = str_to_w32(&p.to);
                if from.len() != 1 || to.len() != 1 {
                    continue;
                }
                let (first, last) = (from[0], to[0]);
                if last <= first {
                    return Err(BuildError::InvalidRange);
                }
                if has_exclude && exclude.range(first..=last).next().is_some() {
                    continue;
                }
                result.push('<');
                result.push_str(&w32_to_string(&[first, last]));
                result.push('>');
                size += (last - first) as u64 + 1;
            }
            5 => {
                let mut set: BTreeSet<W32> = str_to_w32(&p.additional).into_iter().collect();
                if set.is_empty() {
                    continue;
                }
                if has_exclude {
                    for c in &exclude {
                        set.remove(c);
                    }
                    if set.is_empty() {
                        continue;
                    }
                }
                size += set.len() as u64;
                result.push_str("<<");
                result.push_str(&set_to_string(&set));
                result.push_str(">>");
            }
            _ => {
                let full = default_set(i);
                let mut use_default = true;
                if has_exclude {
                    let set: BTreeSet<W32> = full.difference(&exclude).copied().collect();
                    if set.is_empty() {
                        continue;
                    }
                    if set.len() < full.len() {
                        result.push_str("<<");
                        result.push_str(&set_to_string(&set));
                        result.push_str(">>");
                        size += set.len() as u64;
                        use_default = false;
                    }
                }
                if use_default {
                    result.push_str(CODES[i]);
                    size += full.len() as u64;
                }
            }
        }
        let n = p.num_chars[i];
        if n > 0 {
            result.push_str(&format!(":{n}"));
            if p.at_least[i] {
                result.push('+');
            }
        }
    }
    if size >= 2 {
        if !p.tag.is_empty() {
            result = format!("[{}]{result}", p.tag);
        }
        Ok(result)
    } else {
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> BuilderParams {
        BuilderParams::default()
    }

    #[test]
    fn basic_sets() {
        let mut x = p();
        x.used[0] = true;
        x.used[2] = true;
        x.num_chars[2] = 2;
        x.at_least[2] = true;
        assert_eq!(build_char_set(&x).unwrap(), "<az><09>:2+");
        x.exclude = "aeiou0".into();
        assert_eq!(build_char_set(&x).unwrap(), "<<bcdfghjklmnpqrstvwxyz>><<123456789>>:2+");
        x.tag = "t".into();
        assert_eq!(build_char_set(&x).unwrap(), "[t]<<bcdfghjklmnpqrstvwxyz>><<123456789>>:2+");
    }

    #[test]
    fn range_and_additional() {
        let mut x = p();
        x.used[4] = true;
        x.from = "a".into();
        x.to = "a".into();
        assert_eq!(build_char_set(&x), Err(BuildError::InvalidRange));
        x.to = "f".into();
        assert_eq!(build_char_set(&x).unwrap(), "<af>");
        x.exclude = "c".into();
        assert_eq!(build_char_set(&x).unwrap(), "");
        x.used[5] = true;
        x.additional = "zyxc".into();
        assert_eq!(build_char_set(&x).unwrap(), "<<xyz>>");
        // a single character is not a set
        x.additional = "zc".into();
        assert_eq!(build_char_set(&x).unwrap(), "");
    }
}
