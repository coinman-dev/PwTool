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

/// Encode text for writing in the given encoding (without BOM).
pub fn encode_text(text: &str, enc: CharEncoding) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(match enc {
        CharEncoding::Utf8 => text.as_bytes().to_vec(),
        CharEncoding::Ansi => ansi_encoding().encode(text).0.into_owned(),
        CharEncoding::Utf16 => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        CharEncoding::Utf16BigEndian => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    })
}

/// Encode text in the ANSI code page, replacing unmappable characters with
/// `?` (`WideCharToMultiByte(CP_ACP, 0, …)` without a default char).
pub fn ansi_encode_lossy(text: &str) -> Zeroizing<Vec<u8>> {
    let enc = ansi_encoding();
    let mut out = Zeroizing::new(Vec::with_capacity(text.len()));
    let mut buf = [0u8; 4];
    for c in text.chars() {
        let (bytes, _, had_errors) = enc.encode(c.encode_utf8(&mut buf));
        if had_errors {
            out.push(b'?');
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
        let v: Vec<&str> = split_tokens("a b\tc\r\n\nd", &['\n', '\t', ' ']).collect();
        assert_eq!(v, ["a", "b", "c\r", "", "d"]);
    }
}
