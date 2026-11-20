//! Configuration file `PwTech.ini` (Delphi `TMemIniFile`), Delphi list
//! syntax (`TStringList::CommaText`), generator settings of the main window
//! and profiles (`Main.cpp`: `LoadConfig`, `SaveConfig`, `CreateProfile`,
//! `LoadProfile`).

use std::fs;
use std::io;
use std::path::Path;

use crate::generate::PasswOptions;
use crate::passw::generator::WORDLIST_MAX_WORDLEN;
use crate::text_file::{CharEncoding, UTF8_BOM, decode_bytes};

/// Section of the main window settings.
pub const CONFIG_ID: &str = "Main";
/// Prefix of the profile sections (`PWGenProfile0`, …).
pub const CONFIG_PROFILE_ID: &str = "PWGenProfile";
pub const PROFILES_MAX_NUM: usize = 50;
pub const LISTS_MAX_ENTRIES: usize = 50;
/// Entry of the word list box meaning the built-in list.
pub const WLFNLIST_DEFAULT: &str = "<default>";

/// Default entries of the character set list.
pub const CHARSETLIST_DEFAULTENTRIES: [&str; 12] = [
    "<AZ><az><09>",
    "<AZ><az><09><symbols>",
    "<AZ>:2+<az><09>:3+<symbols>:1+",
    "<AZ><az><09><symbols><high>",
    "<AZ>:1<az>:1+<09>:1+<symbols>:1<high>:1",
    "<easytoread>",
    "<Hex>",
    "<hex>",
    "<base64>",
    "<phonetic>",
    "<phoneticu>",
    "<phoneticx>",
];

/// Default entries of the format list.
pub const FORMATLIST_DEFAULTENTRIES: [&str; 12] = [
    "10qs4d", "12q{sLd}", "{4u4l2ds}", "{6ALd}", "3[8q ]", "*d", "*10-20A", "6q2ds6q2ds", "5[w-2d ]", "U9A", "32h",
    "5[2h-]2h",
];

/// INI file compatible with Delphi's `TMemIniFile` (case-insensitive
/// sections and keys, order preserved, UTF-8).
#[derive(Clone, Debug, Default)]
pub struct IniFile {
    sections: Vec<(String, Vec<(String, String)>)>,
}

/// `StrToIntDef()`: decimal, or hexadecimal with `$`/`0x` prefix.
pub fn str_to_int_def(s: &str, default: i64) -> i64 {
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let v = if let Some(h) = t.strip_prefix('$').or_else(|| t.strip_prefix("0x")).or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()
    } else {
        t.parse::<i64>().ok()
    };
    match v {
        Some(v) => {
            if neg {
                -v
            } else {
                v
            }
        }
        None => default,
    }
}

impl IniFile {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse INI text.
    pub fn parse(text: &str) -> Self {
        let mut ini = IniFile::new();
        let mut cur: Option<usize> = None;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for line in text.lines() {
            let s = line.trim();
            if s.is_empty() || s.starts_with(';') {
                continue;
            }
            if s.starts_with('[') && s.ends_with(']') {
                let name = s[1..s.len() - 1].trim().to_string();
                cur = Some(ini.section_index_or_add(&name));
            } else if let Some(i) = cur {
                if let Some(eq) = s.find('=') {
                    let k = s[..eq].trim().to_string();
                    let v = s[eq + 1..].trim().to_string();
                    let sec = &mut ini.sections[i].1;
                    if let Some(e) = sec.iter_mut().find(|(kk, _)| kk.eq_ignore_ascii_case(&k)) {
                        e.1 = v;
                    } else {
                        sec.push((k, v));
                    }
                }
            }
        }
        ini
    }

    /// Load a file (BOM detection; UTF-8 without BOM).
    pub fn load(path: &Path) -> io::Result<Self> {
        let data = fs::read(path)?;
        let (text, _) = decode_bytes(&data, CharEncoding::Utf8);
        Ok(Self::parse(&text))
    }

    /// Text of the file (`[Section]` blocks, `key=value` lines, CRLF).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, (name, keys)) in self.sections.iter().enumerate() {
            if i > 0 {
                out.push_str("\r\n");
            }
            out.push('[');
            out.push_str(name);
            out.push_str("]\r\n");
            for (k, v) in keys {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push_str("\r\n");
            }
        }
        out
    }

    /// Save as UTF-8 with BOM (`UpdateFile()` with `TEncoding::UTF8`).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut data = UTF8_BOM.to_vec();
        data.extend_from_slice(self.to_text().as_bytes());
        let tmp = path.with_extension("ini.tmp");
        fs::write(&tmp, &data)?;
        fs::rename(&tmp, path)
    }

    fn section_index(&self, section: &str) -> Option<usize> {
        self.sections.iter().position(|(n, _)| n.eq_ignore_ascii_case(section))
    }

    fn section_index_or_add(&mut self, section: &str) -> usize {
        match self.section_index(section) {
            Some(i) => i,
            None => {
                self.sections.push((section.to_string(), Vec::new()));
                self.sections.len() - 1
            }
        }
    }

    pub fn section_exists(&self, section: &str) -> bool {
        self.section_index(section).is_some()
    }

    pub fn erase_section(&mut self, section: &str) {
        if let Some(i) = self.section_index(section) {
            self.sections.remove(i);
        }
    }

    pub fn clear(&mut self) {
        self.sections.clear();
    }

    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(n, _)| n.as_str())
    }

    pub fn value(&self, section: &str, key: &str) -> Option<&str> {
        let i = self.section_index(section)?;
        self.sections[i].1.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    pub fn read_string(&self, section: &str, key: &str, default: &str) -> String {
        self.value(section, key).unwrap_or(default).to_string()
    }

    pub fn read_integer(&self, section: &str, key: &str, default: i64) -> i64 {
        match self.value(section, key) {
            Some(v) => str_to_int_def(v, default),
            None => default,
        }
    }

    pub fn read_int(&self, section: &str, key: &str, default: i32) -> i32 {
        self.read_integer(section, key, default as i64) as i32
    }

    pub fn read_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.read_integer(section, key, default as i64) != 0
    }

    pub fn write_string(&mut self, section: &str, key: &str, value: &str) {
        let i = self.section_index_or_add(section);
        let sec = &mut self.sections[i].1;
        if let Some(e) = sec.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            e.1 = value.to_string();
        } else {
            sec.push((key.to_string(), value.to_string()));
        }
    }

    pub fn write_integer(&mut self, section: &str, key: &str, value: i64) {
        self.write_string(section, key, &value.to_string());
    }

    pub fn write_bool(&mut self, section: &str, key: &str, value: bool) {
        self.write_string(section, key, if value { "1" } else { "0" });
    }
}

/// Parse a Delphi `TStringList::CommaText` (quote char `"`, delimiter `,`,
/// non-strict: unquoted items also end at spaces).
pub fn parse_comma_text(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let ws = |ch: char| ('\u{1}'..=' ').contains(&ch);
    while i < c.len() && ws(c[i]) {
        i += 1;
    }
    while i < c.len() {
        let item;
        if c[i] == '"' {
            let mut s = String::new();
            i += 1;
            while i < c.len() {
                if c[i] == '"' {
                    if i + 1 < c.len() && c[i + 1] == '"' {
                        s.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                s.push(c[i]);
                i += 1;
            }
            item = s;
        } else {
            let start = i;
            while i < c.len() && c[i] > ' ' && c[i] != ',' {
                i += 1;
            }
            item = c[start..i].iter().collect();
        }
        out.push(item);
        while i < c.len() && ws(c[i]) {
            i += 1;
        }
        if i < c.len() && c[i] == ',' {
            if i + 1 == c.len() {
                out.push(String::new());
            }
            loop {
                i += 1;
                if !(i < c.len() && ws(c[i])) {
                    break;
                }
            }
        }
    }
    out
}

/// Build a `CommaText`: items containing control characters, spaces,
/// quotes or commas (and empty items) are quoted.
pub fn to_comma_text(items: &[String]) -> String {
    let mut out = String::new();
    for (n, s) in items.iter().enumerate() {
        if n > 0 {
            out.push(',');
        }
        if s.is_empty() || s.chars().any(|c| c <= ' ' || c == '"' || c == ',') {
            out.push('"');
            out.push_str(&s.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(s);
        }
    }
    out
}

/// `LimitStringLen()` (length in UTF-16 units, as `WString::Length`).
pub fn limit_string_len(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in s.chars() {
        n += c.len_utf16();
        if n > max {
            break;
        }
        out.push(c);
    }
    out
}

/// `RemoveCommentAndTrim()`: drop a leading `[comment]` and trim.
pub fn remove_comment_and_trim(s: &str) -> String {
    let mut t = s;
    if t.starts_with('[') {
        if let Some(p) = t.find(']') {
            t = &t[p + 1..];
        }
    }
    crate::unicode::vcl_trim(t).to_string()
}

/// A profile (`PWGenProfile`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PwGenProfile {
    pub profile_name: String,
    pub include_chars: bool,
    pub chars_length: i32,
    pub char_set: String,
    pub include_words: bool,
    pub words_num: i32,
    pub word_list_file_name: String,
    pub combine_words_chars: bool,
    pub specify_length: bool,
    pub specify_length_string: String,
    pub format_passw: bool,
    pub format_string: String,
    pub run_script: bool,
    pub script_file_name: String,
    pub num_passw: String,
    pub include_header: bool,
    pub advanced_passw_options: Option<PasswOptions>,
}

/// Find a profile by name (case-insensitive, `FindPWGenProfile`).
pub fn find_profile(list: &[PwGenProfile], name: &str) -> Option<usize> {
    list.iter().position(|p| p.profile_name.to_lowercase() == name.to_lowercase())
}

fn read_passw_options(ini: &IniFile, section: &str, flags_str: &str) -> PasswOptions {
    let min = ini.read_int(section, "MinWordLen", 1).clamp(1, WORDLIST_MAX_WORDLEN);
    let max = ini.read_int(section, "MaxWordLen", WORDLIST_MAX_WORDLEN).max(min).min(WORDLIST_MAX_WORDLEN);
    PasswOptions {
        flags: str_to_int_def(flags_str, 0) as u32,
        ambig_chars: ini.read_string(section, "AmbigChars", ""),
        special_symbols: ini.read_string(section, "SpecialSym", ""),
        min_word_len: min,
        max_word_len: max,
        word_separator: limit_string_len(&ini.read_string(section, "WordSeparator", ""), 10),
        word_char_separator: limit_string_len(&ini.read_string(section, "WordCharSeparator", ""), 10),
        trigram_file_name: ini.read_string(section, "TrigramFile", ""),
    }
}

fn write_passw_options(ini: &mut IniFile, section: &str, o: &PasswOptions) {
    ini.write_integer(section, "AdvancedPasswOptions", o.flags as i64);
    ini.write_string(section, "AmbigChars", &o.ambig_chars);
    ini.write_string(section, "SpecialSym", &o.special_symbols);
    ini.write_integer(section, "MinWordLen", o.min_word_len as i64);
    ini.write_integer(section, "MaxWordLen", o.max_word_len as i64);
    ini.write_string(section, "WordSeparator", &o.word_separator);
    ini.write_string(section, "WordCharSeparator", &o.word_char_separator);
    ini.write_string(section, "TrigramFile", &o.trigram_file_name);
}

/// Read the profiles (`PWGenProfile0`… until a section is missing).
pub fn load_profiles(ini: &IniFile) -> Vec<PwGenProfile> {
    let mut list: Vec<PwGenProfile> = Vec::new();
    let mut i = 0;
    while list.len() < PROFILES_MAX_NUM {
        let id = format!("{CONFIG_PROFILE_ID}{i}");
        i += 1;
        if !ini.section_exists(&id) {
            break;
        }
        let name = ini.read_string(&id, "ProfileName", "");
        if name.is_empty() || find_profile(&list, &name).is_some() {
            continue;
        }
        let opts = ini.read_string(&id, "AdvancedPasswOptions", "");
        list.push(PwGenProfile {
            profile_name: name,
            include_chars: ini.read_bool(&id, "IncludeChars", false),
            chars_length: ini.read_int(&id, "CharsLength", 1),
            char_set: ini.read_string(&id, "CharSet", ""),
            include_words: ini.read_bool(&id, "IncludeWords", false),
            words_num: ini.read_int(&id, "WordsNum", 1),
            word_list_file_name: ini.read_string(&id, "WordListFileName", ""),
            combine_words_chars: ini.read_bool(&id, "CombineWordsChars", false),
            specify_length: ini.read_bool(&id, "PassphrSpecLen", false),
            specify_length_string: ini.read_string(&id, "PassphrSpecLenString", ""),
            format_passw: ini.read_bool(&id, "FormatPassw", false),
            format_string: ini.read_string(&id, "FormatString", ""),
            run_script: ini.read_bool(&id, "RunScript", false),
            script_file_name: ini.read_string(&id, "ScriptFileName", ""),
            num_passw: ini.read_string(&id, "NumPassw", "2"),
            include_header: ini.read_bool(&id, "PasswListIncludeHeader", true),
            advanced_passw_options: if opts.is_empty() { None } else { Some(read_passw_options(ini, &id, &opts)) },
        });
    }
    list
}

/// Write the profiles.
pub fn save_profiles(ini: &mut IniFile, list: &[PwGenProfile]) {
    for (i, p) in list.iter().enumerate() {
        let id = format!("{CONFIG_PROFILE_ID}{i}");
        ini.write_string(&id, "ProfileName", &p.profile_name);
        ini.write_bool(&id, "IncludeChars", p.include_chars);
        ini.write_integer(&id, "CharsLength", p.chars_length as i64);
        ini.write_string(&id, "CharSet", &p.char_set);
        ini.write_bool(&id, "IncludeWords", p.include_words);
        ini.write_integer(&id, "WordsNum", p.words_num as i64);
        ini.write_string(&id, "WordListFileName", &p.word_list_file_name);
        ini.write_bool(&id, "CombineWordsChars", p.combine_words_chars);
        ini.write_bool(&id, "PassphrSpecLen", p.specify_length);
        ini.write_string(&id, "PassphrSpecLenString", &p.specify_length_string);
        ini.write_bool(&id, "FormatPassw", p.format_passw);
        ini.write_string(&id, "FormatString", &p.format_string);
        ini.write_bool(&id, "RunScript", p.run_script);
        ini.write_string(&id, "ScriptFileName", &p.script_file_name);
        ini.write_string(&id, "NumPassw", &p.num_passw);
        ini.write_bool(&id, "PasswListIncludeHeader", p.include_header);
        if let Some(o) = &p.advanced_passw_options {
            write_passw_options(ini, &id, o);
        }
    }
}

/// Generator settings of the main window (the part of `LoadConfig` /
/// `SaveConfig` that defines how passwords are generated).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratorSettings {
    pub include_chars: bool,
    pub chars_length: i32,
    pub char_set_list: Vec<String>,
    pub char_set_text: String,
    pub include_words: bool,
    pub words_num: i32,
    pub wlfn_list: Vec<String>,
    pub wlfn_text: String,
    pub combine_words_chars: bool,
    pub passphr_spec_len: bool,
    pub passphr_spec_len_string: String,
    pub format_passw: bool,
    pub format_list: Vec<String>,
    pub format_text: String,
    pub run_script: bool,
    pub script_list: Vec<String>,
    pub script_text: String,
    pub num_passw: String,
    pub include_header: bool,
    pub passw_options: PasswOptions,
}

impl Default for GeneratorSettings {
    fn default() -> Self {
        Self::load(&IniFile::new())
    }
}

fn limit_list(mut v: Vec<String>) -> Vec<String> {
    v.truncate(LISTS_MAX_ENTRIES);
    v
}

impl GeneratorSettings {
    /// Read from the `[Main]` section with the original defaults.
    pub fn load(ini: &IniFile) -> Self {
        let s = CONFIG_ID;
        let opts = ini.read_string(s, "AdvancedPasswOptions", "");
        let mut passw_options = read_passw_options(ini, s, &opts);
        // In [Main] the word length limits are checked individually.
        let min = ini.read_int(s, "MinWordLen", 1);
        let min = if !(1..=WORDLIST_MAX_WORDLEN).contains(&min) { 1 } else { min };
        let max = ini.read_int(s, "MaxWordLen", WORDLIST_MAX_WORDLEN);
        let max = if !(1..=WORDLIST_MAX_WORDLEN).contains(&max) { WORDLIST_MAX_WORDLEN } else { max };
        passw_options.min_word_len = min;
        passw_options.max_word_len = max.max(min);

        let mut char_set_list = parse_comma_text(&ini.read_string(s, "CharSetListEntries", ""));
        if char_set_list.is_empty() {
            char_set_list = CHARSETLIST_DEFAULTENTRIES.iter().map(|x| x.to_string()).collect();
        }
        let mut wlfn_list = parse_comma_text(&ini.read_string(s, "WLFNListEntries", ""));
        if wlfn_list.is_empty() {
            wlfn_list.push(WLFNLIST_DEFAULT.to_string());
        }
        let mut format_list = parse_comma_text(&ini.read_string(s, "FormatListEntries", ""));
        if format_list.is_empty() {
            format_list = FORMATLIST_DEFAULTENTRIES.iter().map(|x| x.to_string()).collect();
        }
        let char_set_list = limit_list(char_set_list);
        let wlfn_list = limit_list(wlfn_list);
        let format_list = limit_list(format_list);
        let script_list = limit_list(parse_comma_text(&ini.read_string(s, "ScriptListEntries", "")));
        let char_set_text = char_set_list.first().cloned().unwrap_or_default();
        let wlfn_text = wlfn_list.first().cloned().unwrap_or_default();
        let format_text = format_list.first().cloned().unwrap_or_default();
        let script_text = script_list.first().cloned().unwrap_or_default();
        GeneratorSettings {
            include_chars: ini.read_bool(s, "IncludeChars", true),
            chars_length: ini.read_int(s, "CharsLength", 16) as i16 as i32,
            char_set_list,
            char_set_text,
            include_words: ini.read_bool(s, "IncludeWords", false),
            words_num: ini.read_int(s, "WordsNum", 8),
            wlfn_list,
            wlfn_text,
            combine_words_chars: ini.read_bool(s, "CombineWordsChars", false),
            passphr_spec_len: ini.read_bool(s, "PassphrSpecLen", false),
            passphr_spec_len_string: ini.read_string(s, "PassphrSpecLenString", ""),
            format_passw: ini.read_bool(s, "FormatPassw", false),
            format_list,
            format_text,
            run_script: ini.read_bool(s, "RunScript", false),
            script_list,
            script_text,
            num_passw: ini.read_string(s, "NumPassw", "100"),
            include_header: ini.read_bool(s, "PasswListIncludeHeader", true),
            passw_options,
        }
    }

    /// Write to the `[Main]` section.
    pub fn save(&self, ini: &mut IniFile) {
        let s = CONFIG_ID;
        ini.write_bool(s, "IncludeChars", self.include_chars);
        ini.write_integer(s, "CharsLength", self.chars_length as i64);
        ini.write_string(s, "CharSetListEntries", &to_comma_text(&self.char_set_list));
        ini.write_bool(s, "IncludeWords", self.include_words);
        ini.write_integer(s, "WordsNum", self.words_num as i64);
        ini.write_string(s, "WLFNListEntries", &to_comma_text(&self.wlfn_list));
        ini.write_bool(s, "CombineWordsChars", self.combine_words_chars);
        ini.write_bool(s, "PassphrSpecLen", self.passphr_spec_len);
        ini.write_string(s, "PassphrSpecLenString", &self.passphr_spec_len_string);
        ini.write_bool(s, "FormatPassw", self.format_passw);
        ini.write_string(s, "FormatListEntries", &to_comma_text(&self.format_list));
        ini.write_bool(s, "RunScript", self.run_script);
        ini.write_string(s, "ScriptListEntries", &to_comma_text(&self.script_list));
        ini.write_string(s, "NumPassw", &self.num_passw);
        ini.write_bool(s, "PasswListIncludeHeader", self.include_header);
        write_passw_options(ini, s, &self.passw_options);
    }

    /// `CreateProfile()`: current settings as a profile, using the current box
    /// texts (not mutating or falling back to history lists).
    pub fn to_profile(&self, name: &str, save_advanced_options: bool) -> PwGenProfile {
        PwGenProfile {
            profile_name: name.to_string(),
            include_chars: self.include_chars,
            chars_length: self.chars_length,
            char_set: self.char_set_text.clone(),
            include_words: self.include_words,
            words_num: self.words_num,
            word_list_file_name: self.wlfn_text.clone(),
            combine_words_chars: self.combine_words_chars,
            specify_length: self.passphr_spec_len,
            specify_length_string: self.passphr_spec_len_string.clone(),
            format_passw: self.format_passw,
            format_string: self.format_text.clone(),
            run_script: self.run_script,
            script_file_name: self.script_text.clone(),
            num_passw: self.num_passw.clone(),
            include_header: self.include_header,
            advanced_passw_options: save_advanced_options.then(|| self.passw_options.clone()),
        }
    }

    /// `LoadProfile()`: apply a profile to the current box texts without
    /// polluting the history lists (entries are only added to history on
    /// successful load/use, as in upstream `Main.cpp:1892-1903`).
    pub fn apply_profile(&mut self, p: &PwGenProfile) {
        self.include_chars = p.include_chars;
        self.chars_length = p.chars_length;
        self.char_set_text = p.char_set.clone();
        self.include_words = p.include_words;
        self.words_num = p.words_num;
        self.wlfn_text = p.word_list_file_name.clone();
        self.combine_words_chars = p.combine_words_chars;
        self.passphr_spec_len = p.specify_length;
        self.passphr_spec_len_string = p.specify_length_string.clone();
        self.format_passw = p.format_passw;
        self.format_text = p.format_string.clone();
        self.run_script = p.run_script;
        self.script_text = p.script_file_name.clone();
        self.num_passw = p.num_passw.clone();
        self.include_header = p.include_header;
        if let Some(o) = &p.advanced_passw_options {
            self.passw_options = o.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comma_text_roundtrip() {
        let items: Vec<String> =
            ["<AZ><az><09>", "a b", "x\"y", "", "c,d"].iter().map(|s| s.to_string()).collect();
        let t = to_comma_text(&items);
        assert_eq!(t, "<AZ><az><09>,\"a b\",\"x\"\"y\",\"\",\"c,d\"");
        assert_eq!(parse_comma_text(&t), items);
        assert_eq!(parse_comma_text("a b,c"), ["a", "b", "c"]);
        assert_eq!(parse_comma_text("a,"), ["a", ""]);
    }

    #[test]
    fn ini_roundtrip_and_defaults() {
        let mut ini = IniFile::parse("\u{feff}[Main]\r\nIncludeChars=0\r\nCharsLength=$20\r\n[pwgenprofile0]\r\nProfileName=Test\r\nAdvancedPasswOptions=3\r\n");
        assert!(!ini.read_bool("main", "includechars", true));
        let g = GeneratorSettings::load(&ini);
        assert_eq!(g.chars_length, 32);
        assert_eq!(g.char_set_list.len(), 12);
        assert_eq!(g.passw_options.max_word_len, WORDLIST_MAX_WORDLEN);
        let profiles = load_profiles(&ini);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].advanced_passw_options.as_ref().unwrap().flags, 3);
        ini.write_bool("Main", "IncludeChars", true);
        let text = ini.to_text();
        let back = IniFile::parse(&text);
        assert!(back.read_bool("Main", "IncludeChars", false));
    }

    #[test]
    fn helpers() {
        assert_eq!(remove_comment_and_trim("[my list] /path/x.txt "), "/path/x.txt");
        assert_eq!(limit_string_len("abcdefghijklm", 10), "abcdefghij");
        assert_eq!(str_to_int_def("0x1F", 0), 31);
        assert_eq!(str_to_int_def("abc", 7), 7);
    }

    #[test]
    fn ini_trims_values_and_profile_preserves_history() {
        let ini = IniFile::parse("[Main]\nKey =   value with spaces   \n");
        assert_eq!(ini.read_string("Main", "Key", ""), "value with spaces");

        let mut gs = GeneratorSettings::load(&ini);
        let orig_list = gs.char_set_list.clone();
        let mut prof = gs.to_profile("P1", false);
        prof.char_set = "<09>".to_string();
        gs.apply_profile(&prof);
        assert_eq!(gs.char_set_text, "<09>");
        assert_eq!(gs.char_set_list, orig_list, "applying a profile must not mutate history list");
    }
}
