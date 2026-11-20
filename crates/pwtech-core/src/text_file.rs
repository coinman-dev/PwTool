//! Text files in the encodings supported by the original
//! (`upstream/src/util/StringFileStreamW.*`).
//!
//! Reading: a byte-order mark selects UTF-16 LE/BE or UTF-8; without a BOM the
//! file is read as UTF-8 if that is the configured encoding, otherwise as
//! "ANSI". Writing: optional BOM, then the text in the chosen encoding.
//!
//! "ANSI" is the Windows code page of the system (`CP_ACP`). Linux has no such
//! notion; the port uses Windows-1251 for Cyrillic locales and Windows-1252
//! otherwise, and it can be changed with [`set_ansi_encoding`].

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::RwLock;

use encoding_rs::{Encoding, UTF_8, WINDOWS_1251, WINDOWS_1252};
use zeroize::Zeroizing;

pub const UNICODE_BOM: [u8; 2] = [0xFF, 0xFE];
pub const UNICODE_BOM_SWAPPED: [u8; 2] = [0xFE, 0xFF];
pub const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Character encoding of text files (`CharacterEncoding`; the numeric values
/// are those stored in `PwTech.ini`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharEncoding {
    Ansi = 0,
    Utf16 = 1,
    Utf16BigEndian = 2,
    Utf8 = 3,
}

impl CharEncoding {
    pub fn from_index(i: i32) -> Option<Self> {
        match i {
            0 => Some(Self::Ansi),
            1 => Some(Self::Utf16),
            2 => Some(Self::Utf16BigEndian),
            3 => Some(Self::Utf8),
            _ => None,
        }
    }
}

static ANSI: RwLock<Option<&'static Encoding>> = RwLock::new(None);

/// The encoding used for "ANSI" text.
pub fn ansi_encoding() -> &'static Encoding {
    if let Some(e) = *ANSI.read().unwrap() {
        return e;
    }
    let lang = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    let cyrillic = ["ru", "uk", "be", "bg", "sr", "mk"].iter().any(|p| lang.starts_with(p));
    if cyrillic { WINDOWS_1251 } else { WINDOWS_1252 }
}

/// Override the "ANSI" encoding (e.g. from the configuration).
pub fn set_ansi_encoding(enc: &'static Encoding) {
    *ANSI.write().unwrap() = Some(enc);
}

/// Decode bytes of a file, detecting a BOM. Returns the text and the encoding
/// actually used. Invalid sequences become U+FFFD, as with
/// `MultiByteToWideChar` without `MB_ERR_INVALID_CHARS`.
pub fn decode_bytes(bytes: &[u8], default_enc: CharEncoding) -> (Zeroizing<String>, CharEncoding) {
    let (enc, bom_len) = if bytes.len() >= 2 && bytes[..2] == UNICODE_BOM {
        (CharEncoding::Utf16, 2)
    } else if bytes.len() >= 2 && bytes[..2] == UNICODE_BOM_SWAPPED {
        (CharEncoding::Utf16BigEndian, 2)
    } else if bytes.len() >= 3 && bytes[..3] == UTF8_BOM {
        (CharEncoding::Utf8, 3)
    } else if default_enc == CharEncoding::Utf8 {
        (CharEncoding::Utf8, 0)
    } else {
        (CharEncoding::Ansi, 0)
    };
    let data = &bytes[bom_len..];
    let text = match enc {
        CharEncoding::Utf8 => UTF_8.decode_without_bom_handling(data).0.into_owned(),
        CharEncoding::Ansi => ansi_encoding().decode_without_bom_handling(data).0.into_owned(),
        CharEncoding::Utf16 | CharEncoding::Utf16BigEndian => {
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| {
                    if enc == CharEncoding::Utf16 {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    }
                })
                .collect();
            char::decode_utf16(units).map(|r| r.unwrap_or('\u{fffd}')).collect()
        }
    };
    (Zeroizing::new(text), enc)
}

/// Read and decode a whole text file.
pub fn read_text_file(path: &Path, default_enc: CharEncoding) -> io::Result<(Zeroizing<String>, CharEncoding)> {
    let mut bytes = Zeroizing::new(Vec::new());
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() % 2 == 1 && (bytes.starts_with(&UNICODE_BOM) || bytes.starts_with(&UNICODE_BOM_SWAPPED)) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-16 character encoding"));
    }
    Ok(decode_bytes(&bytes, default_enc))
}

/// Split text into tokens at any of the separator characters, like
/// `TStringFileStreamW::ReadString()` with the given separators. Tokens are
/// returned without the separator; empty tokens are kept (callers trim and
/// skip them, as the original does).
pub fn split_tokens<'a>(text: &'a str, seps: &'a [char]) -> impl Iterator<Item = &'a str> + 'a {
    text.split(move |c: char| seps.contains(&c))
}

/// Tokens as returned by `TStringFileStreamW::ReadString()` with a buffer of
/// `buf_size` UTF-16 code units (including the terminating zero): a token
/// and its separator (if one follows) must fit into `buf_size - 1` units;
/// otherwise reading fails for 8-bit encodings ("Invalid ANSI or UTF-8
/// character encoding, or Unicode string too long") and the token is
/// truncated for UTF-16. The separator itself is not part of the tokens.
pub fn read_string_tokens<'a>(
    text: &'a str,
    seps: &[char],
    enc: CharEncoding,
    buf_size: usize,
) -> io::Result<Vec<std::borrow::Cow<'a, str>>> {
    let max = buf_size.saturating_sub(1);
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (token, has_sep, next) = match rest.char_indices().find(|(_, c)| seps.contains(c)) {
            Some((i, c)) => (&rest[..i], true, &rest[i + c.len_utf8()..]),
            None => (rest, false, ""),
        };
        let units = crate::unicode::utf16_len(token) + has_sep as usize;
        if units > max {
            match enc {
                CharEncoding::Ansi | CharEncoding::Utf8 => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid ANSI or UTF-8 character encoding, or Unicode string too long",
                    ));
                }
                _ => {
                    let mut n = 0;
                    let mut end = 0;
                    for (i, ch) in token.char_indices() {
                        n += ch.len_utf16();
                        if n > max {
                            break;
                        }
                        end = i + ch.len_utf8();
                    }
                    out.push(std::borrow::Cow::Borrowed(&token[..end]));
                }
            }
        } else {
            out.push(std::borrow::Cow::Borrowed(token));
        }
        rest = next;
    }
    Ok(out)
}

/// Encode text for writing in the given encoding (without BOM).
pub fn encode_text(text: &str, enc: CharEncoding) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(match enc {
        CharEncoding::Utf8 => text.as_bytes().to_vec(),
        CharEncoding::Ansi => ansi_encoding().encode(text).0.into_owned(),
        CharEncoding::Utf16 => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        CharEncoding::Utf16BigEndian => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    })
}

/// Accented Latin letters (U+00C0..U+017F) and their base letters (from the
/// canonical decomposition), as mapped by Windows' best-fit tables.
const BEST_FIT_LATIN: &[(char, u8)] = &[
    ('\u{00c0}', b'A'),
    ('\u{00c1}', b'A'),
    ('\u{00c2}', b'A'),
    ('\u{00c3}', b'A'),
    ('\u{00c4}', b'A'),
    ('\u{00c5}', b'A'),
    ('\u{00c7}', b'C'),
    ('\u{00c8}', b'E'),
    ('\u{00c9}', b'E'),
    ('\u{00ca}', b'E'),
    ('\u{00cb}', b'E'),
    ('\u{00cc}', b'I'),
    ('\u{00cd}', b'I'),
    ('\u{00ce}', b'I'),
    ('\u{00cf}', b'I'),
    ('\u{00d1}', b'N'),
    ('\u{00d2}', b'O'),
    ('\u{00d3}', b'O'),
    ('\u{00d4}', b'O'),
    ('\u{00d5}', b'O'),
    ('\u{00d6}', b'O'),
    ('\u{00d9}', b'U'),
    ('\u{00da}', b'U'),
    ('\u{00db}', b'U'),
    ('\u{00dc}', b'U'),
    ('\u{00dd}', b'Y'),
    ('\u{00e0}', b'a'),
    ('\u{00e1}', b'a'),
    ('\u{00e2}', b'a'),
    ('\u{00e3}', b'a'),
    ('\u{00e4}', b'a'),
    ('\u{00e5}', b'a'),
    ('\u{00e7}', b'c'),
    ('\u{00e8}', b'e'),
    ('\u{00e9}', b'e'),
    ('\u{00ea}', b'e'),
    ('\u{00eb}', b'e'),
    ('\u{00ec}', b'i'),
    ('\u{00ed}', b'i'),
    ('\u{00ee}', b'i'),
    ('\u{00ef}', b'i'),
    ('\u{00f1}', b'n'),
    ('\u{00f2}', b'o'),
    ('\u{00f3}', b'o'),
    ('\u{00f4}', b'o'),
    ('\u{00f5}', b'o'),
    ('\u{00f6}', b'o'),
    ('\u{00f9}', b'u'),
    ('\u{00fa}', b'u'),
    ('\u{00fb}', b'u'),
    ('\u{00fc}', b'u'),
    ('\u{00fd}', b'y'),
    ('\u{00ff}', b'y'),
    ('\u{0100}', b'A'),
    ('\u{0101}', b'a'),
    ('\u{0102}', b'A'),
    ('\u{0103}', b'a'),
    ('\u{0104}', b'A'),
    ('\u{0105}', b'a'),
    ('\u{0106}', b'C'),
    ('\u{0107}', b'c'),
    ('\u{0108}', b'C'),
    ('\u{0109}', b'c'),
    ('\u{010a}', b'C'),
    ('\u{010b}', b'c'),
    ('\u{010c}', b'C'),
    ('\u{010d}', b'c'),
    ('\u{010e}', b'D'),
    ('\u{010f}', b'd'),
    ('\u{0112}', b'E'),
    ('\u{0113}', b'e'),
    ('\u{0114}', b'E'),
    ('\u{0115}', b'e'),
    ('\u{0116}', b'E'),
    ('\u{0117}', b'e'),
    ('\u{0118}', b'E'),
    ('\u{0119}', b'e'),
    ('\u{011a}', b'E'),
    ('\u{011b}', b'e'),
    ('\u{011c}', b'G'),
    ('\u{011d}', b'g'),
    ('\u{011e}', b'G'),
    ('\u{011f}', b'g'),
    ('\u{0120}', b'G'),
    ('\u{0121}', b'g'),
    ('\u{0122}', b'G'),
    ('\u{0123}', b'g'),
    ('\u{0124}', b'H'),
    ('\u{0125}', b'h'),
    ('\u{0128}', b'I'),
    ('\u{0129}', b'i'),
    ('\u{012a}', b'I'),
    ('\u{012b}', b'i'),
    ('\u{012c}', b'I'),
    ('\u{012d}', b'i'),
    ('\u{012e}', b'I'),
    ('\u{012f}', b'i'),
    ('\u{0130}', b'I'),
    ('\u{0134}', b'J'),
    ('\u{0135}', b'j'),
    ('\u{0136}', b'K'),
    ('\u{0137}', b'k'),
    ('\u{0139}', b'L'),
    ('\u{013a}', b'l'),
    ('\u{013b}', b'L'),
    ('\u{013c}', b'l'),
    ('\u{013d}', b'L'),
    ('\u{013e}', b'l'),
    ('\u{0143}', b'N'),
    ('\u{0144}', b'n'),
    ('\u{0145}', b'N'),
    ('\u{0146}', b'n'),
    ('\u{0147}', b'N'),
    ('\u{0148}', b'n'),
    ('\u{014c}', b'O'),
    ('\u{014d}', b'o'),
    ('\u{014e}', b'O'),
    ('\u{014f}', b'o'),
    ('\u{0150}', b'O'),
    ('\u{0151}', b'o'),
    ('\u{0154}', b'R'),
    ('\u{0155}', b'r'),
    ('\u{0156}', b'R'),
    ('\u{0157}', b'r'),
    ('\u{0158}', b'R'),
    ('\u{0159}', b'r'),
    ('\u{015a}', b'S'),
    ('\u{015b}', b's'),
    ('\u{015c}', b'S'),
    ('\u{015d}', b's'),
    ('\u{015e}', b'S'),
    ('\u{015f}', b's'),
    ('\u{0160}', b'S'),
    ('\u{0161}', b's'),
    ('\u{0162}', b'T'),
    ('\u{0163}', b't'),
    ('\u{0164}', b'T'),
    ('\u{0165}', b't'),
    ('\u{0168}', b'U'),
    ('\u{0169}', b'u'),
    ('\u{016a}', b'U'),
    ('\u{016b}', b'u'),
    ('\u{016c}', b'U'),
    ('\u{016d}', b'u'),
    ('\u{016e}', b'U'),
    ('\u{016f}', b'u'),
    ('\u{0170}', b'U'),
    ('\u{0171}', b'u'),
    ('\u{0172}', b'U'),
    ('\u{0173}', b'u'),
    ('\u{0174}', b'W'),
    ('\u{0175}', b'w'),
    ('\u{0176}', b'Y'),
    ('\u{0177}', b'y'),
    ('\u{0178}', b'Y'),
    ('\u{0179}', b'Z'),
    ('\u{017a}', b'z'),
    ('\u{017b}', b'Z'),
    ('\u{017c}', b'z'),
    ('\u{017d}', b'Z'),
    ('\u{017e}', b'z'),
];

/// Encode text in the ANSI code page, replacing unmappable characters with
/// `?` (`WideCharToMultiByte(CP_ACP, 0, …)` without a default char).
pub fn ansi_encode_lossy(text: &str) -> Zeroizing<Vec<u8>> {
    let enc = ansi_encoding();
    let mut out = Zeroizing::new(Vec::with_capacity(text.len()));
    let mut buf = [0u8; 4];
    for c in text.chars() {
        let (bytes, _, had_errors) = enc.encode(c.encode_utf8(&mut buf));
        if had_errors {
            // Windows' "best fit" mapping: accented Latin letters become the
            // base letter (e.g. 'ä' -> 'a' in code page 1251)
            match BEST_FIT_LATIN.binary_search_by_key(&c, |&(k, _)| k) {
                Ok(i) => out.push(BEST_FIT_LATIN[i].1),
                Err(_) => out.push(b'?'),
            }
        } else {
            out.extend_from_slice(&bytes);
        }
    }
    out
}

/// BOM written for an encoding (`TStringFileStreamW` with
/// `blWriteOrAutodetectBOM`); none for ANSI.
pub fn bom(enc: CharEncoding) -> &'static [u8] {
    match enc {
        CharEncoding::Ansi => &[],
        CharEncoding::Utf16 => &UNICODE_BOM,
        CharEncoding::Utf16BigEndian => &UNICODE_BOM_SWAPPED,
        CharEncoding::Utf8 => &UTF8_BOM,
    }
}

/// Writer for text files in a given encoding (write side of
/// `TStringFileStreamW`).
pub struct TextFileWriter {
    file: File,
    enc: CharEncoding,
}

impl TextFileWriter {
    /// Create (truncate) a file; writes a BOM if `write_bom`.
    pub fn create(path: &Path, enc: CharEncoding, write_bom: bool) -> io::Result<Self> {
        let mut file = File::create(path)?;
        if write_bom {
            file.write_all(bom(enc))?;
        }
        Ok(TextFileWriter { file, enc })
    }

    pub fn write_str(&mut self, s: &str) -> io::Result<()> {
        if s.is_empty() {
            return Ok(());
        }
        self.file.write_all(&encode_text(s, self.enc))
    }

    pub fn encoding(&self) -> CharEncoding {
        self.enc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_detection() {
        let (t, e) = decode_bytes(&[0xFF, 0xFE, b'a', 0, b'b', 0], CharEncoding::Ansi);
        assert_eq!((t.as_str(), e), ("ab", CharEncoding::Utf16));
        let (t, e) = decode_bytes(&[0xFE, 0xFF, 0, b'a'], CharEncoding::Utf8);
        assert_eq!((t.as_str(), e), ("a", CharEncoding::Utf16BigEndian));
        let (t, e) = decode_bytes(&[0xEF, 0xBB, 0xBF, 0xC3, 0xA4], CharEncoding::Ansi);
        assert_eq!((t.as_str(), e), ("ä", CharEncoding::Utf8));
        let (t, e) = decode_bytes(&[0xE4], CharEncoding::Utf16);
        assert_eq!(e, CharEncoding::Ansi, "without BOM non-UTF-8 defaults fall back to ANSI");
        assert!(t.as_str() == "ä" || t.as_str() == "д");
    }

    #[test]
    fn tokens() {
        let t = read_string_tokens("aa bb x", &[' '], CharEncoding::Utf8, 4).unwrap();
        assert_eq!(t, vec!["aa", "bb", "x"]);
        // the last token without separator may use the whole buffer
        assert!(read_string_tokens("aaa", &[' '], CharEncoding::Utf8, 4).is_ok());
        assert!(read_string_tokens("aaa b", &[' '], CharEncoding::Utf8, 4).is_err());
        assert_eq!(read_string_tokens("aaaa b", &[' '], CharEncoding::Utf16, 4).unwrap(), vec!["aaa", "b"]);
        let v: Vec<&str> = split_tokens("a b\tc\r\n\nd", &['\n', '\t', ' ']).collect();
        assert_eq!(v, ["a", "b", "c\r", "", "d"]);
    }
}
