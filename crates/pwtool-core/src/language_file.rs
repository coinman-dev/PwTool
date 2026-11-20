//! Translation files (`LanguageSupport` in `upstream/src/main/Language.cpp`):
//! gettext `.po` files and the legacy `.lng` format.
//!
//! Lookup keys are case-insensitive (the original hashes the upper-cased
//! ASCII characters). Old-style format specifiers in `.po` files (`%d`, `%s`,
//! `%.1f`, …) are converted to `%1`, `%2`, … (`convertFormatSpec`).

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::text_file::{CharEncoding, TextFileWriter, read_text_file};

#[derive(Debug, thiserror::Error)]
pub enum LanguageError {
    #[error("{0}")]
    Format(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}

fn err(s: impl Into<String>) -> LanguageError {
    LanguageError::Format(s.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageFormat {
    Po,
    Lng,
}

/// A loaded translation.
#[derive(Debug, Clone)]
pub struct LanguageFile {
    pub format: LanguageFormat,
    pub code: String,
    pub name: String,
    pub version: String,
    pub translator: String,
    pub help_file: String,
    map: HashMap<String, String>,
    /// All entries in file order (`.lng` only, for conversion to `.po`).
    full: Vec<(String, String)>,
}

const LNG_LANGUAGE_NAME: &str = "[LANGUAGENAME]";
const LNG_LANGUAGE_VERSION: &str = "[LANGUAGEVERSION]";
const LNG_LANGUAGE_AUTHOR: &str = "[LANGUAGEAUTHOR]";
const LNG_LANGUAGE_HELPFILE: &str = "[LANGUAGEHELPFILE]";
const UNKNOWN: &str = "(Unknown)";

/// Lookup key: upper-case ASCII, like `strihash()`.
pub fn key(s: &str) -> String {
    s.to_ascii_uppercase()
}

/// `convertFormatSpec()`: `%d`, `%s`, `%f`, `%.1f`, `%02d`… → `%1`, `%2`, …
pub fn convert_format_spec(s: &str) -> String {
    const SPEC: &str = ".012dfs";
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut arg = 0;
    let mut i = 0;
    while i < c.len() {
        let mut valid = false;
        if c[i] == '%' {
            let mut j = i + 1;
            let mut k = 0;
            while k < 4 && j < c.len() {
                if SPEC.contains(c[j]) {
                    if c[j].is_ascii_lowercase() {
                        valid = true;
                        i = j;
                        break;
                    }
                } else {
                    break;
                }
                k += 1;
                j += 1;
            }
        }
        if valid {
            arg += 1;
            out.push('%');
            out.push_str(&arg.to_string());
        } else {
            out.push(c[i]);
        }
        i += 1;
    }
    out
}

/// English and native name of a language code, as Windows returns them
/// (`LOCALE_SENGLISHLANGUAGENAME`, `LOCALE_SNATIVELANGUAGENAME`). Codes with
/// `_` are not valid Windows locale names; the caller then uses the file
/// name, as the original does.
pub fn language_names(code: &str) -> Option<(&'static str, &'static str)> {
    if code.contains('_') {
        return None;
    }
    let primary = code.split('-').next()?.to_ascii_lowercase();
    Some(match primary.as_str() {
        "en" => ("English", "English"),
        "de" => ("German", "Deutsch"),
        "fr" => ("French", "français"),
        "es" => ("Spanish", "español"),
        "it" => ("Italian", "italiano"),
        "pt" => ("Portuguese", "português"),
        "nl" => ("Dutch", "Nederlands"),
        "pl" => ("Polish", "polski"),
        "ru" => ("Russian", "русский"),
        "uk" => ("Ukrainian", "українська"),
        "be" => ("Belarusian", "беларуская"),
        "bg" => ("Bulgarian", "български"),
        "cs" => ("Czech", "čeština"),
        "sk" => ("Slovak", "slovenčina"),
        "hu" => ("Hungarian", "magyar"),
        "ro" => ("Romanian", "română"),
        "el" => ("Greek", "Ελληνικά"),
        "tr" => ("Turkish", "Türkçe"),
        "sv" => ("Swedish", "svenska"),
        "da" => ("Danish", "dansk"),
        "nb" | "no" => ("Norwegian", "norsk"),
        "fi" => ("Finnish", "suomi"),
        "ja" => ("Japanese", "日本語"),
        "zh" => ("Chinese", "中文"),
        "ko" => ("Korean", "한국어"),
        "ar" => ("Arabic", "العربية"),
        "he" => ("Hebrew", "עברית"),
        _ => return None,
    })
}

impl LanguageFile {
    /// Load a `.po` (by extension) or `.lng` file. With `header_only`, only
    /// name, version etc. are read.
    pub fn load(path: &Path, header_only: bool) -> Result<Self, LanguageError> {
        let raw = std::fs::read(path)?;
        let has_bom = raw.starts_with(&crate::text_file::UTF8_BOM)
            || raw.starts_with(&crate::text_file::UNICODE_BOM)
            || raw.starts_with(&crate::text_file::UNICODE_BOM_SWAPPED);
        let (text, enc) = read_text_file(path, CharEncoding::Utf8)?;
        let file_stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let is_po = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("po"));
        let mut lf = LanguageFile {
            format: if is_po { LanguageFormat::Po } else { LanguageFormat::Lng },
            code: String::new(),
            name: String::new(),
            version: String::new(),
            translator: String::new(),
            help_file: String::new(),
            map: HashMap::new(),
            full: Vec::new(),
        };
        if is_po {
            lf.parse_po(&text, header_only, enc == CharEncoding::Utf8 && !has_bom, &file_stem)?;
        } else {
            lf.parse_lng(&text, header_only)?;
        }
        if !header_only && lf.map.is_empty() {
            return Err(err("Language file does not contain any valid entries"));
        }
        Ok(lf)
    }

    fn parse_po(&mut self, text: &str, header_only: bool, check_charset: bool, file_stem: &str) -> Result<(), LanguageError> {
        #[derive(PartialEq, Clone, Copy)]
        enum Dest {
            None,
            Id,
            Str,
            Ctxt,
        }
        let (mut id, mut msgstr, mut ctxt) = (String::new(), String::new(), String::new());
        let mut dest = Dest::None;
        let mut header = false;
        let mut line_no = 0;

        for line in text.split('\n') {
            line_no += 1;
            let l: Vec<char> = line.chars().collect();
            let starts = |p: &str| line.starts_with(p);
            let prev = dest;
            let mut i = 0;
            if starts("msgid") {
                dest = Dest::Id;
                i = 5;
            } else if starts("msgstr") {
                dest = Dest::Str;
                i = 6;
            } else if starts("msgctxt") {
                dest = Dest::Ctxt;
                i = 7;
            } else if !starts("\"") {
                dest = Dest::None;
            }

            if prev == Dest::Str && dest != prev {
                if id.is_empty() && !header {
                    if msgstr.is_empty() {
                        break;
                    }
                    self.parse_po_header(&msgstr, check_charset, file_stem)?;
                    header = true;
                    id.clear();
                    msgstr.clear();
                    ctxt.clear();
                    dest = Dest::None;
                    if header_only {
                        return Ok(());
                    }
                } else if !header {
                    break;
                } else {
                    self.add_po_entry(&mut id, &mut msgstr, &mut ctxt, line_no)?;
                }
            }
            if dest == Dest::None {
                continue;
            }
            // text between the first pair of quotes, with escapes
            let mut parsed = String::new();
            let mut started = false;
            while i < l.len() {
                if started {
                    if l[i] == '"' {
                        break;
                    } else if l[i] == '\\' {
                        let next = l.get(i + 1).copied().unwrap_or('\0');
                        let c = match next {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            '\\' | '\'' | '"' => next,
                            _ => {
                                i += 1;
                                continue;
                            }
                        };
                        parsed.push(c);
                        i += 1;
                    } else {
                        parsed.push(l[i]);
                    }
                } else if l[i] == '"' {
                    started = true;
                }
                i += 1;
            }
            if !parsed.is_empty() {
                match dest {
                    Dest::Id => id.push_str(&parsed),
                    Dest::Str => msgstr.push_str(&parsed),
                    Dest::Ctxt => ctxt.push_str(&parsed),
                    Dest::None => {}
                }
            }
        }
        if !header {
            return Err(err("Missing header information"));
        }
        if !msgstr.is_empty() {
            self.add_po_entry(&mut id, &mut msgstr, &mut ctxt, line_no)?;
        }
        Ok(())
    }

    fn add_po_entry(&mut self, id: &mut String, msgstr: &mut String, ctxt: &mut String, line: usize) -> Result<(), LanguageError> {
        if msgstr.is_empty() {
            return Err(err(format!("Empty field \"msgstr\" (parsing line {line})")));
        }
        if id.is_empty() {
            return Err(err(format!("Empty field \"msgid\" (parsing line {line})")));
        }
        if !ctxt.is_empty() {
            id.push_str("||");
            id.push_str(ctxt);
        }
        self.map.entry(key(&convert_format_spec(id))).or_insert_with(|| convert_format_spec(msgstr));
        id.clear();
        msgstr.clear();
        ctxt.clear();
        Ok(())
    }

    fn parse_po_header(&mut self, header: &str, check_charset: bool, file_stem: &str) -> Result<(), LanguageError> {
        let mut kv: HashMap<String, String> = HashMap::new();
        for line in header.split('\n') {
            // ([a-zA-Z-]+) *: *(.*)$
            let start = line.find(|c: char| c.is_ascii_alphabetic() || c == '-');
            let Some(start) = start else { continue };
            let rest = &line[start..];
            let name_len = rest.find(|c: char| !(c.is_ascii_alphabetic() || c == '-')).unwrap_or(rest.len());
            let name = &rest[..name_len];
            let after = rest[name_len..].trim_start_matches(' ');
            if let Some(v) = after.strip_prefix(':') {
                let v = v.trim_start_matches(' ').trim_matches([' ', '\n', '\r', '\t']);
                if !v.is_empty() {
                    kv.entry(name.to_string()).or_insert_with(|| v.to_string());
                }
            }
        }
        let idv = kv.get("Project-Id-Version").ok_or_else(|| err("Missing \"Project-Id-Version\" information"))?;
        self.version = find_version(idv).ok_or_else(|| err("Missing or invalid language version number"))?;
        if check_charset {
            let ok = kv.get("Content-Type").is_some_and(|c| c.contains("charset=UTF-8") || c.contains("charset=utf-8"));
            if !ok {
                return Err(err("Unknown charset encoding"));
            }
        }
        if let Some(code) = kv.get("Language") {
            self.code = code.clone();
            if let Some((en, native)) = language_names(code) {
                self.name = format!("{en} ({native})");
            }
        }
        if self.name.is_empty() {
            self.name = file_stem.to_string();
        }
        self.translator = kv.get("Last-Translator").cloned().unwrap_or_else(|| UNKNOWN.to_string());
        if let Some(h) = kv.get("X-PasswordTech-Manual") {
            self.help_file = h.clone();
        }
        Ok(())
    }

    fn parse_lng(&mut self, text: &str, header_only: bool) -> Result<(), LanguageError> {
        let mut state = 0;
        let mut msg_id = String::new();
        let mut num = 0;
        for line in text.split('\n') {
            let l: Vec<char> = line.chars().collect();
            let mut parsed = String::new();
            let mut i = 0;
            while i < l.len() {
                if l[i] == '\\' {
                    let next = l.get(i + 1).copied().unwrap_or('\0');
                    let c = match next {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' | '\'' | '"' => next,
                        _ => {
                            i += 1;
                            continue;
                        }
                    };
                    parsed.push(c);
                    i += 2;
                } else if l[i] == '/' && l.get(i + 1) == Some(&'/') {
                    break;
                } else {
                    parsed.push(l[i]);
                    i += 1;
                }
            }
            let msg = parsed.trim_matches([' ', '\n', '\r', '\t']).to_string();
            if msg.is_empty() {
                if num == 0 {
                    return Err(err("Missing header information"));
                }
                state = 0;
                continue;
            }
            if state == 0 {
                msg_id = msg;
                state = 1;
            } else {
                state = 0;
                num += 1;
                if num == 1 {
                    if msg_id != LNG_LANGUAGE_NAME {
                        return Err(err("Missing language name information"));
                    }
                    self.name = msg.clone();
                    self.code = msg;
                    continue;
                } else if num == 2 {
                    if msg_id != LNG_LANGUAGE_VERSION {
                        return Err(err("Missing language version information"));
                    }
                    self.version = msg;
                    if header_only {
                        break;
                    }
                    continue;
                } else if msg_id == LNG_LANGUAGE_AUTHOR {
                    self.translator = msg;
                    continue;
                } else if msg_id == LNG_LANGUAGE_HELPFILE {
                    self.help_file = msg;
                    continue;
                }
                self.map.entry(key(&msg_id)).or_insert_with(|| msg.clone());
                self.full.push((msg_id.clone(), msg));
            }
        }
        if self.translator.is_empty() {
            self.translator = UNKNOWN.to_string();
        }
        Ok(())
    }

    /// Translate (`Translate()`); `None` if there is no translation.
    pub fn translate(&self, s: &str) -> Option<&str> {
        self.map.get(&key(s)).map(|s| s.as_str())
    }

    /// Translation with context (`TRL(str, ctxt)`): `"str||ctxt"` in PO,
    /// `"str ((ctxt))"` in LNG files; falls back to the untranslated string.
    pub fn translate_ctx(&self, s: &str, ctx: &str) -> String {
        let k = match self.format {
            LanguageFormat::Lng => format!("{s} (({ctx}))"),
            LanguageFormat::Po => format!("{s}||{ctx}"),
        };
        self.translate(&k).map(|t| t.to_string()).unwrap_or_else(|| s.to_string())
    }

    /// The translation table for [`crate::language::set_translations`]
    /// (keys upper-cased).
    pub fn into_map(self) -> HashMap<String, String> {
        self.map
    }

    pub fn map(&self) -> &HashMap<String, String> {
        &self.map
    }

    /// `SaveToPOFileFormat()`: convert a `.lng` translation to `.po`.
    pub fn save_to_po(&self, path: &Path, enc: CharEncoding, program_name: &str, program_version: &str) -> Result<(), LanguageError> {
        if self.format != LanguageFormat::Lng {
            return Err(err("Full translation table not available"));
        }
        let mut f = TextFileWriter::create(path, enc, true)?;
        let header = crate::unicode::format_w(
            "# \"%1\" translation of %2\n# Converted by %3 %4\n\n# NOTE: You have to provide the appropriate language code in the \"Language\" section of the following header.\n# If the language code is missing or invalid, PwTech will derive the language name from the file name.\n%5 \"\"\n%6 \"\"\n\"Project-Id-Version: %7 %8\\n\"\n\"Last-Translator: %9\\n\"\n\"Language: \\n\"\n\"X-PasswordTech-Manual: manual.pdf\\n\"\n\n",
            &[&self.name, program_name, program_name, program_version, "msgid", "msgstr", program_name, &self.version, &self.translator],
        );
        f.write_str(&header)?;
        let convert = |src: &str, ty: &str| -> String {
            let mut m = String::from("\"");
            let mut multi = false;
            for c in src.chars() {
                let esc = match c {
                    '\n' => Some('n'),
                    '\r' => Some('r'),
                    '\t' => Some('t'),
                    '"' => Some('"'),
                    _ => None,
                };
                if let Some(e) = esc {
                    m.push('\\');
                    m.push(e);
                    if e == 'n' {
                        m.push_str("\"\n\"");
                        multi = true;
                    }
                } else {
                    m.push(c);
                }
            }
            m.push('"');
            let mut d = format!("{ty} ");
            if multi {
                d.push_str("\"\"\n");
            }
            d.push_str(&m);
            d
        };
        for (id, s) in &self.full {
            let mut msg_id = id.clone();
            if let Some(p2) = msg_id.find("))").filter(|&p| p == msg_id.len() - 2) {
                if let Some(p1) = msg_id.find("((") {
                    if p2 > p1 + 2 {
                        let c = convert(&msg_id[p1 + 2..p2], "msgctxt") + "\n";
                        f.write_str(&c)?;
                    }
                    msg_id = msg_id[..p1].trim_matches([' ', '\n', '\r', '\t']).to_string();
                }
            }
            let entry = convert(&msg_id, "msgid") + "\n" + &convert(s, "msgstr") + "\n\n";
            f.write_str(&entry)?;
        }
        Ok(())
    }
}

/// First `N.N.N` in a string.
fn find_version(s: &str) -> Option<String> {
    let c: Vec<char> = s.chars().collect();
    for start in 0..c.len() {
        if !c[start].is_ascii_digit() || (start > 0 && c[start - 1].is_ascii_digit()) {
            continue;
        }
        let mut i = start;
        let mut parts = 0;
        loop {
            let b = i;
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            if i == b {
                break;
            }
            parts += 1;
            if parts == 3 {
                return Some(c[start..i].iter().collect());
            }
            if i < c.len() && c[i] == '.' {
                i += 1;
            } else {
                break;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_spec_conversion() {
        assert_eq!(convert_format_spec("%d characters / %.1f bits"), "%1 characters / %2 bits");
        assert_eq!(convert_format_spec("100% %s"), "100% %1");
        assert_eq!(convert_format_spec("%1 and %2"), "%1 and %2");
    }

    #[test]
    fn bundled_translations_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../upstream/languages");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut n = 0;
        for e in rd {
            let p = e.unwrap().path();
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase());
            if !matches!(ext.as_deref(), Some("po") | Some("lng")) {
                continue;
            }
            let lf = LanguageFile::load(&p, false).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(lf.map().len() > 300, "{}: {}", p.display(), lf.map().len());
            assert!(!lf.version.is_empty());
            n += 1;
        }
        assert!(n >= 13);
        let ru = LanguageFile::load(&dir.join("Russian.po"), false).unwrap();
        assert_eq!(ru.name, "Russian (русский)");
        assert_eq!(ru.code, "ru");
        assert!(ru.translate("Generate").is_some());
        assert!(ru.translate("generate").is_some(), "lookup is case-insensitive");
        let t = ru.translate("%1 characters / %2 bits per character").unwrap();
        assert!(t.contains("%1") && t.contains("%2"), "{t}");
        let cn = LanguageFile::load(&dir.join("Chinese-Simplified.po"), true).unwrap();
        assert_eq!(cn.name, "Chinese-Simplified");
    }

    #[test]
    fn lng_to_po_conversion() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../upstream/languages");
        let Ok(gr) = LanguageFile::load(&dir.join("Greek.lng"), false) else { return };
        let out = std::env::temp_dir().join(format!("pwtool-greek-{}.po", std::process::id()));
        gr.save_to_po(&out, CharEncoding::Utf8, "Password Tech", "3.6.1").unwrap();
        let back = LanguageFile::load(&out, false).unwrap();
        assert!(back.map().len() + 5 >= gr.map().len());
        std::fs::remove_file(out).unwrap();
    }
}
