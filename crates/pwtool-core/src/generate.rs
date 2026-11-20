//! The password generation loop of the main window
//! (`TMainForm::GeneratePassw()` in `upstream/src/main/Main.cpp`), without
//! the user interface: option flags are translated into generator flags,
//! characters, words and format are combined, entropy is estimated, generated
//! passwords are checked against the list of common passwords, duplicates are
//! removed, and password lists are assembled with header and appendix.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use zeroize::Zeroizing;

use crate::language::{trl, trl_format};
use crate::passw::generator::*;
use crate::random::RandomGenerator;
use crate::random::pool::MAX_ENTROPY;
use crate::text_file::{CharEncoding, read_text_file};
use crate::unicode::{W32String, str_to_w32, utf16_len, w32_to_string_secure};
use crate::util::{floor_entropy_bits, format_float_1};

// Advanced password options (`PasswOptions.h`), bit values as in PwTech.ini.
pub const PASSWOPTIONS_NUM: usize = 19;
pub const PASSWOPTION_EXCLUDEAMBIG: u32 = 0x00001;
pub const PASSWOPTION_FIRSTCHARNOTLC: u32 = 0x00002;
pub const PASSWOPTION_DONTSEPWORDS: u32 = 0x00004;
pub const PASSWOPTION_DONTSEPWORDSCHARS: u32 = 0x00008;
pub const PASSWOPTION_REVERSEWCHORDER: u32 = 0x00010;
pub const PASSWOPTION_INCLUDEUPPERCASE: u32 = 0x00020;
pub const PASSWOPTION_INCLUDELOWERCASE: u32 = 0x00040;
pub const PASSWOPTION_INCLUDEDIGIT: u32 = 0x00080;
pub const PASSWOPTION_INCLUDESYMBOL: u32 = 0x00100;
pub const PASSWOPTION_INCLUDESUBSET: u32 = 0x00200;
pub const PASSWOPTION_EXCLUDEREPCHARS: u32 = 0x00400;
pub const PASSWOPTION_EXCLUDEDUPLICATES: u32 = 0x00800;
pub const PASSWOPTION_LOWERCASEWORDS: u32 = 0x01000;
pub const PASSWOPTION_EACHCHARONLYONCE: u32 = 0x02000;
pub const PASSWOPTION_EACHWORDONLYONCE: u32 = 0x04000;
pub const PASSWOPTION_CAPITALIZEWORDS: u32 = 0x08000;
pub const PASSWOPTION_CHECKEACHPASSW: u32 = 0x10000;
pub const PASSWOPTION_INCLUDE_CHAR_FROM_EACH_SUBSET: u32 = 0x20000;
pub const PASSWOPTION_REMOVEWHITESPACE: u32 = 0x40000;

/// List position (in the options dialog) of each bit (`OPTION_INDEX_TO_BIT`).
pub const OPTION_INDEX_TO_BIT: [usize; PASSWOPTIONS_NUM] = [0, 1, 13, 10, 18, 2, 3, 4, 15, 12, 14, 5, 6, 7, 8, 17, 9, 11, 16];

/// Options marked with "*" (might reduce security), indexed by bit.
pub const PASSWOPTIONS_STARRED: [bool; PASSWOPTIONS_NUM] = [
    true, true, false, false, false, false, false, false, false, false, true, true, true, true, true, false, false, false,
    false,
];

/// Bit field of all starred options.
pub const fn passwoptions_starred_bitfield() -> u32 {
    let mut f = 0;
    let mut i = 0;
    while i < PASSWOPTIONS_NUM {
        if PASSWOPTIONS_STARRED[i] {
            f |= 1 << i;
        }
        i += 1;
    }
    f
}

pub const PASSW_MAX_CHARS: i32 = 10000;
pub const PASSW_MAX_WORDS: i32 = 100;
pub const PASSWFORMAT_MAX_CHARS: usize = 16000;
pub const PASSWSCRIPT_MAX_CHARS: usize = 16000;
pub const PASSW_MAX_NUM: u64 = 1_000_000_000_000;
pub const PASSWLIST_MAX_BYTES: u64 = 4_000_000_000;

/// Advanced password options (`struct PasswOptions`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PasswOptions {
    pub flags: u32,
    pub ambig_chars: String,
    pub special_symbols: String,
    pub min_word_len: i32,
    pub max_word_len: i32,
    pub word_separator: String,
    pub word_char_separator: String,
    pub trigram_file_name: String,
}

/// Where generated passwords go (`GeneratePasswDest`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenDest {
    /// Single password in the edit box of the main window.
    GuiSingle,
    /// Password list in the "Password List" window.
    GuiList,
    /// Password list to the clipboard.
    ClipboardList,
    /// Password list to a file.
    FileList,
    /// Single password to the clipboard (hot key).
    Clipboard,
    /// Single password shown in a message box (hot key).
    MsgBox,
    /// Output on the console (`-gen`).
    Console,
    /// Single password typed into another window (hot key).
    Autotype,
}

impl GenDest {
    pub fn is_list(self) -> bool {
        matches!(self, GenDest::GuiList | GenDest::ClipboardList | GenDest::FileList)
    }
}

/// List of common passwords (`common_passwords.txt`).
#[derive(Default)]
pub struct CommonPasswords {
    set: HashSet<String>,
    entropy: f64,
}

impl CommonPasswords {
    /// Load the list: one password per line, ANSI unless a BOM says
    /// otherwise; at most 1,000,000 entries.
    pub fn load(path: &Path) -> io::Result<Self> {
        let (text, enc) = read_text_file(path, CharEncoding::Ansi)?;
        let mut set = HashSet::new();
        let mut rest = text.as_str();
        while !rest.is_empty() && set.len() < 1_000_000 {
            let (line, has_sep, next) = match rest.find('\n') {
                Some(i) => (&rest[..i], true, &rest[i + 1..]),
                None => (rest, false, ""),
            };
            let units = utf16_len(line) + usize::from(has_sep);
            let item = if units > 255 {
                match enc {
                    CharEncoding::Ansi | CharEncoding::Utf8 => {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "Unicode string too long"));
                    }
                    _ => {
                        let mut n = 0;
                        let mut end = 0;
                        for (i, ch) in line.char_indices() {
                            n += ch.len_utf16();
                            if n > 255 {
                                break;
                            }
                            end = i + ch.len_utf8();
                        }
                        &line[..end]
                    }
                }
            } else {
                line
            };
            set.insert(item.trim_matches([' ', '\n', '\r', '\t']).to_string());
            rest = next;
        }
        let entropy = if set.is_empty() { 0.0 } else { crate::math::delphi_log2(set.len() as f64) };
        Ok(CommonPasswords { set, entropy })
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn contains(&self, passw: &str) -> bool {
        self.set.contains(passw)
    }

    /// Entropy assigned to a matching password (log2 of the list size).
    pub fn entropy(&self) -> f64 {
        self.entropy
    }
}

/// Length requirement for passphrases ("Specify length" box).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassphraseLength {
    pub min: i32,
    pub max: i32,
    /// `*` given: the length refers to all characters, not only the words.
    pub all_chars: bool,
}

/// Parse the "Specify length" box: `N`, `N-M`, `>N`, `>=N`, `<N`, `<=N`,
/// optionally with `*`. `Ok(None)` = empty box.
pub fn parse_passphrase_length(text: &str) -> Result<Option<PassphraseLength>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    let mut t: Vec<char> = text.chars().collect();
    let mut all_chars = false;
    if let Some(p) = t.iter().position(|&c| c == '*') {
        all_chars = true;
        t.remove(p);
    }
    let s: String = t.iter().collect();
    let to_int = |s: &str, def: i32| s.trim().parse::<i32>().unwrap_or(def);
    let n = t.len();
    let (mut min, mut max) = (-1, -1);
    if n >= 2 && t[0] == '>' {
        if n >= 3 && t[1] == '=' {
            min = to_int(&s[2..], -1);
        } else {
            min = to_int(&s[1..], -2) + 1;
        }
        max = i32::MAX;
    } else if n >= 2 && t[0] == '<' {
        if n >= 3 && t[1] == '=' {
            max = to_int(&s[2..], -1);
        } else {
            max = to_int(&s[1..], -1) - 1;
        }
        min = 0;
    } else if let Some(sep) = t.iter().position(|&c| c == '-').filter(|&p| p >= 1) {
        let a: String = t[..sep].iter().collect();
        let b: String = t[sep + 1..].iter().collect();
        min = to_int(&a, -1);
        max = to_int(&b, -1);
    } else if n > 0 {
        min = to_int(&s, -1);
        max = min;
    }
    if max < min {
        std::mem::swap(&mut min, &mut max);
    }
    if min < 0 || max < 1 {
        return Err(trl("Passphrase: Invalid length range."));
    }
    Ok(Some(PassphraseLength { min, max, all_chars }))
}

/// Parse the number of passwords with optional K/M/G/B suffix.
pub fn parse_num_passwords(text: &str) -> Option<u64> {
    let mut s = text.to_string();
    let mut factor = 1u64;
    if let Some(last) = s.chars().last() {
        match last.to_ascii_uppercase() {
            'K' => factor = 1000,
            'M' => factor = 1_000_000,
            'B' | 'G' => factor = 1_000_000_000,
            _ => {}
        }
        if factor > 1 {
            s.pop();
        }
    }
    let n: i64 = s.trim().parse().unwrap_or(0);
    let n = (n as u64).checked_mul(factor)?;
    if (2..=PASSW_MAX_NUM).contains(&n) { Some(n) } else { None }
}

/// Parameters of one "Generate" action.
pub struct GenRequest {
    pub dest: GenDest,
    pub num_passw: u64,
    /// Length of the character part (0 = "Include characters" off).
    pub chars_len: i32,
    /// Number of words (0 = "Include words" off).
    pub num_words: i32,
    pub combine_words_chars: bool,
    pub passphrase_length: Option<PassphraseLength>,
    /// Format string (empty = "Format password" off).
    pub format: String,
    /// `PASSWOPTION_*` flags.
    pub option_flags: u32,
    pub include_header: bool,
    /// Line ending for file lists (Configuration | Files).
    pub file_newline: String,
    pub test_common_passw: bool,
}

/// Hook for a Lua script (`TScriptingThread::CallGenerate`): receives the
/// password number (1-based), the password generated so far and its entropy;
/// returns the script's password and entropy.
pub type ScriptHook<'a> =
    dyn FnMut(&mut dyn RandomGenerator, u64, Option<&str>, f64) -> Result<(Zeroizing<String>, f64), String> + 'a;

/// One generated password as passed to the output.
pub struct GeneratedPassword<'a> {
    pub passw: &'a str,
    /// Length in characters as shown in the main window.
    pub len: usize,
    pub security: f64,
    pub common_match: bool,
    /// Text to append in lists (newline or "  [bits]" + newline).
    pub appendix: &'a str,
}

/// What the output wants next.
pub enum SinkAction {
    Continue,
    /// Do not count this password, generate a new one (message box "No").
    Regenerate,
    /// Stop generating.
    Stop,
}

/// Result of a generation run.
#[derive(Default)]
pub struct GenSummary {
    pub count: u64,
    pub first_security: f64,
    pub total_security: f64,
    /// Info/error text for the format box of the main window (first password).
    pub format_info: String,
    pub last_len: usize,
    pub last_security: f64,
    pub last_common_match: bool,
}

/// Errors that abort the generation before it starts.
#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error("{0}")]
    Message(String),
}

/// Warnings to show before generating (length or word count were reduced).
pub struct GenWarnings {
    pub chars_len_limited: Option<i32>,
    pub words_limited: Option<i32>,
}

/// Apply the limits of "each character/word only once" (shows warnings in
/// the original and changes the spin boxes).
pub fn apply_limits(pg: &PasswordGenerator, req: &mut GenRequest) -> GenWarnings {
    let mut w = GenWarnings { chars_len_limited: None, words_limited: None };
    if req.chars_len != 0
        && req.option_flags & PASSWOPTION_EACHCHARONLYONCE != 0
        && req.chars_len > pg.custom_char_set_unique_size()
    {
        req.chars_len = pg.custom_char_set_unique_size();
        w.chars_len_limited = Some(req.chars_len);
    }
    if req.num_words != 0 && req.option_flags & PASSWOPTION_EACHWORDONLYONCE != 0 && req.num_words > pg.word_list_size()
    {
        req.num_words = pg.word_list_size();
        w.words_limited = Some(req.num_words);
    }
    w
}

/// String that is wiped when dropped and can be stored in hash sets.
#[derive(PartialEq, Eq, Hash)]
struct SecretStr(String);

impl Drop for SecretStr {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

/// Header line of password lists (`asPasswListHeader`).
pub const PASSW_LIST_HEADER: &str =
    "%1 passwords generated.\r\nEntropy of the first password: %2 bits.\r\nMaximum entropy of the entire list: %3 bits.";

/// Run the generation loop. `sink` receives each password; `cancel` and
/// `progress` allow a GUI to cancel and show progress; `script` is the
/// optional Lua hook (`standalone` = script generates the password itself).
#[allow(clippy::too_many_arguments)]
pub fn generate(
    pg: &PasswordGenerator,
    rng: &mut dyn RandomGenerator,
    req: &GenRequest,
    common: Option<&CommonPasswords>,
    mut script: Option<(&mut ScriptHook<'_>, bool)>,
    cancel: &AtomicBool,
    progress: &AtomicU64,
    sink: &mut dyn FnMut(&GeneratedPassword<'_>, &GenSummary) -> Result<SinkAction, String>,
) -> Result<GenSummary, String> {
    let mut chars_len = req.chars_len;
    let mut num_words = req.num_words;
    let mut format: W32String = str_to_w32(&req.format);
    let char_set_size = pg.custom_char_set_w32().len();
    let option_flags = req.option_flags;
    let exclude_duplicates = option_flags & PASSWOPTION_EXCLUDEDUPLICATES != 0;
    let first_char_not_lc = option_flags & PASSWOPTION_FIRSTCHARNOTLC != 0;
    let char_set_type = pg.custom_char_set_type();

    let mut passw_flags = 0u32;
    let mut passphr_flags = 0u32;
    let mut format_flags = 0u32;

    if chars_len != 0 {
        if first_char_not_lc {
            passw_flags |= PASSW_FLAG_FIRSTCHARNOTLC;
        }
        if option_flags & PASSWOPTION_EXCLUDEREPCHARS != 0 {
            passw_flags |= PASSW_FLAG_EXCLUDEREPCHARS;
        }
        if option_flags & PASSWOPTION_INCLUDESUBSET != 0 {
            passw_flags |= PASSW_FLAG_INCLUDESUBSET;
        }
        if option_flags & PASSWOPTION_EACHCHARONLYONCE != 0 {
            passw_flags |= PASSW_FLAG_EACHCHARONLYONCE;
        }
        if char_set_size >= 128 && chars_len >= 128 {
            passw_flags |= PASSW_FLAG_CHECKDUPLICATESBYSET;
        }
        if option_flags & PASSWOPTION_REMOVEWHITESPACE != 0 {
            passw_flags |= PASSW_FLAG_REMOVEWHITESPACE;
        }
        for i in 0..PASSWGEN_NUMINCLUDECHARSETS {
            if option_flags & (PASSWOPTION_INCLUDEUPPERCASE << i) != 0 {
                passw_flags |= PASSW_FLAG_INCLUDEUPPERCASE << i;
            }
        }
        if char_set_type == CharSetType::PhoneticUpperCase {
            passw_flags |= PASSW_FLAG_PHONETICUPPERCASE;
        } else if char_set_type == CharSetType::PhoneticMixedCase {
            passw_flags |= PASSW_FLAG_PHONETICMIXEDCASE;
        }
    }

    if num_words != 0 {
        if req.combine_words_chars {
            passphr_flags |= PASSPHR_FLAG_COMBINEWCH;
        }
        if option_flags & PASSWOPTION_CAPITALIZEWORDS != 0 {
            passphr_flags |= PASSPHR_FLAG_CAPITALIZEWORDS;
        }
        if option_flags & PASSWOPTION_DONTSEPWORDS != 0 {
            passphr_flags |= PASSPHR_FLAG_DONTSEPWORDS;
        }
        if option_flags & PASSWOPTION_DONTSEPWORDSCHARS != 0 {
            passphr_flags |= PASSPHR_FLAG_DONTSEPWCH;
        }
        if option_flags & PASSWOPTION_REVERSEWCHORDER != 0 {
            passphr_flags |= PASSPHR_FLAG_REVERSEWCHORDER;
        }
        if option_flags & PASSWOPTION_EACHWORDONLYONCE != 0 {
            passphr_flags |= PASSPHR_FLAG_EACHWORDONLYONCE;
        }
        if passphr_flags & PASSPHR_FLAG_COMBINEWCH != 0 {
            passw_flags &= !PASSW_FLAG_FIRSTCHARNOTLC;
        }
    }

    if !format.is_empty() {
        if option_flags & PASSWOPTION_EXCLUDEREPCHARS != 0 {
            format_flags |= PASSFORMAT_FLAG_EXCLUDEREPCHARS;
        }
        if option_flags & PASSWOPTION_REMOVEWHITESPACE != 0 {
            format_flags |= PASSFORMAT_FLAG_REMOVEWHITESPACE;
        }
    }

    if let Some((_, true)) = script {
        chars_len = 0;
        num_words = 0;
        format.clear();
    }

    let num_passw = req.num_passw;
    let dest = req.dest;
    let list_dest = matches!(dest, GenDest::GuiList | GenDest::ClipboardList);
    let check_each = num_passw > 1 && option_flags & PASSWOPTION_CHECKEACHPASSW != 0;
    let variable_len = chars_len != 0
        && passw_flags & PASSW_FLAG_REMOVEWHITESPACE != 0
        && matches!(char_set_type, CharSetType::Standard | CharSetType::StandardWithFreq)
        && pg.custom_char_set_w32().iter().any(|&c| c == ' ' as u32 || c == '\t' as u32);
    let newline = if list_dest { "\r\n".to_string() } else { req.file_newline.clone() };
    let mut appendix = newline.clone();

    let mut summary = GenSummary::default();
    let mut unique_list: HashSet<SecretStr> = HashSet::new();
    let mut first_gen = true;
    let mut keep_prev = false;
    let mut base_sec = 0.0f64;
    let mut passw_sec = 0.0f64;
    let mut chars: Zeroizing<Vec<u32>> = Zeroizing::new(Vec::new());
    let mut from_script: Zeroizing<String> = Zeroizing::new(String::new());
    let mut common_match = false;
    let mut count: u64 = 0;

    while count < num_passw && !cancel.load(Ordering::Relaxed) {
        let mut gen_chars_len = 0usize;
        let mut passw_len: usize = 0;
        let mut cur: Option<Zeroizing<Vec<u32>>> = None;

        if chars_len != 0 {
            if keep_prev {
                gen_chars_len = chars.len();
            } else {
                chars = match char_set_type {
                    CharSetType::Phonetic | CharSetType::PhoneticUpperCase | CharSetType::PhoneticMixedCase => {
                        pg.get_phonetic_passw(rng, chars_len, passw_flags)
                    }
                    _ => pg.get_password(rng, chars_len, passw_flags),
                };
                gen_chars_len = chars.len();
                if first_gen || variable_len {
                    base_sec = if matches!(char_set_type, CharSetType::Standard | CharSetType::StandardWithFreq)
                        && option_flags & PASSWOPTION_EACHCHARONLYONCE != 0
                    {
                        PasswordGenerator::calc_perm_set_entropy(char_set_size as i32, gen_chars_len as i32)
                    } else {
                        pg.custom_char_set_entropy() * gen_chars_len as f64
                    };
                }
            }
            passw_len = gen_chars_len;
            cur = Some(chars.clone());
        }

        if num_words != 0 {
            let (words, net_words_len) = pg.get_passphrase(rng, num_words, &chars[..gen_chars_len], passphr_flags);
            passw_len = words.len();
            if let Some(l) = req.passphrase_length {
                let base_len = if l.all_chars { passw_len as i32 } else { net_words_len };
                if base_len < l.min || base_len > l.max {
                    keep_prev = true;
                    continue;
                }
            }
            if first_gen || variable_len {
                base_sec += if option_flags & PASSWOPTION_EACHWORDONLYONCE != 0 {
                    PasswordGenerator::calc_perm_set_entropy(pg.word_list_size(), num_words)
                } else {
                    pg.word_list_entropy() * num_words as f64
                };
            }
            cur = Some(words);
        }

        passw_sec = base_sec;
        keep_prev = false;

        if !format.is_empty() {
            let mut format_sec = 0.0;
            let res = pg.get_format_passw(
                rng,
                &format,
                format_flags,
                cur.as_ref().map(|v| &v[..]),
                PASSWFORMAT_MAX_CHARS,
                if first_gen || check_each { Some(&mut format_sec) } else { None },
            );
            if format_sec > 0.0 {
                if res.passw_used == PASSFORMAT_PWUSED_NOSPECIFIER {
                    passw_sec = format_sec;
                } else {
                    passw_sec += format_sec;
                }
            }
            if first_gen {
                let mut msg = String::new();
                if res.passw_used == PASSFORMAT_PWUSED_NOSPECIFIER {
                    chars = Zeroizing::new(Vec::new());
                    chars_len = 0;
                    num_words = 0;
                    base_sec = 0.0;
                    msg = trl("\"P\" is not specified");
                } else if res.passw_used == PASSFORMAT_PWUSED_EMPTYPASSW {
                    msg = format!("\"P\": {}", trl("Password not available"));
                } else if res.passw_used > 0 && (res.passw_used as usize) < passw_len {
                    msg = format!("\"P\": {}", trl("Password too long"));
                }
                if !res.invalid_spec.is_empty() {
                    let spec = crate::unicode::w32_to_string(&res.invalid_spec);
                    let spec_msg = if utf16_len(&spec) > 10 {
                        let mut s: String = spec.chars().take(10).collect();
                        s.push_str("...");
                        s
                    } else {
                        spec
                    };
                    if !msg.is_empty() {
                        msg.push_str(" | ");
                    }
                    msg.push_str(&trl_format(
                        "%1 invalid format specifier(s): %2",
                        &[&res.invalid_spec.len().to_string(), &spec_msg],
                    ));
                }
                summary.format_info = msg;
            }
            passw_len = res.passw.len();
            cur = Some(res.passw);
        }

        let mut passw: Zeroizing<String> = match &cur {
            Some(v) => w32_to_string_secure(v),
            None => Zeroizing::new(String::new()),
        };
        let mut has_passw = cur.is_some();
        if has_passw && first_char_not_lc && !passw.is_empty() {
            // toupper() on the first UTF-16 code unit: ASCII only
            let first = passw.chars().next().unwrap();
            if first.is_ascii_lowercase() {
                let mut s = Zeroizing::new(String::with_capacity(passw.len()));
                s.push(first.to_ascii_uppercase());
                s.push_str(&passw[1..]);
                passw = s;
            }
        }
        let mut passw_len_wchars = utf16_len(&passw);

        if let Some((hook, _)) = script.as_mut() {
            let (res, sec) = hook(rng, count + 1, if has_passw { Some(&passw) } else { None }, passw_sec)?;
            if !res.is_empty() {
                let mut s = Zeroizing::new(String::new());
                let mut n = 0;
                for c in res.chars() {
                    n += c.len_utf16();
                    if n > PASSWSCRIPT_MAX_CHARS {
                        break;
                    }
                    s.push(c);
                }
                from_script = s;
                passw = from_script.clone();
                has_passw = true;
                passw_len = passw.chars().count();
                passw_len_wchars = utf16_len(&passw);
            }
            passw_sec = (20.0 * utf16_len(&from_script) as f64).min(sec.max(0.0));
        }
        let _ = has_passw;

        if num_passw > 1 && passw_len_wchars > 0 && exclude_duplicates && !unique_list.insert(SecretStr(passw.to_string()))
        {
            continue;
        }

        if first_gen || check_each {
            passw_sec = passw_sec.min(MAX_ENTROPY as f64);
        }

        if num_passw == 1 || check_each {
            if let Some(c) = common.filter(|c| req.test_common_passw && !c.is_empty()) {
                common_match = c.contains(&passw);
                if common_match {
                    passw_sec = passw_sec.min(c.entropy());
                }
            }
        }

        if check_each {
            summary.total_security += passw_sec;
            appendix = String::from("  [");
            if common_match {
                appendix.push('*');
            }
            appendix.push_str(&floor_entropy_bits(passw_sec).to_string());
            appendix.push(']');
            appendix.push_str(&newline);
        }

        if first_gen {
            first_gen = false;
            if list_dest {
                summary.first_security = passw_sec;
                if !check_each {
                    summary.total_security = (passw_sec * num_passw as f64).min(MAX_ENTROPY as f64);
                }
            }
        }

        summary.last_len = passw_len;
        summary.last_security = passw_sec;
        summary.last_common_match = common_match;

        let gp = GeneratedPassword {
            passw: &passw,
            len: passw_len,
            security: passw_sec,
            common_match,
            appendix: &appendix,
        };
        match sink(&gp, &summary)? {
            SinkAction::Continue => {}
            SinkAction::Regenerate => continue,
            SinkAction::Stop => break,
        }

        count += 1;
        summary.count = count;
        progress.store(count, Ordering::Relaxed);
    }

    if summary.total_security == 0.0 {
        summary.total_security = passw_sec * count as f64;
    }
    summary.total_security = summary.total_security.min(MAX_ENTROPY as f64);
    Ok(summary)
}

/// Header for password lists (translated, with CRLF CRLF).
pub fn passw_list_header(count: u64, first_sec: f64, total_sec: f64) -> String {
    let mut s = trl_format(
        PASSW_LIST_HEADER,
        &[&count.to_string(), &format_float_1(first_sec), &format_float_1(total_sec)],
    );
    s.push_str("\r\n\r\n");
    s
}

/// Generate a password list as one string, like the GUI list / clipboard
/// list destinations: optional header, passwords with appendix, the final
/// line break removed.
#[allow(clippy::too_many_arguments)]
pub fn generate_list(
    pg: &PasswordGenerator,
    rng: &mut dyn RandomGenerator,
    req: &GenRequest,
    common: Option<&CommonPasswords>,
    script: Option<(&mut ScriptHook<'_>, bool)>,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<(Zeroizing<String>, GenSummary), String> {
    let mut list = Zeroizing::new(String::new());
    let mut header_len = 0;
    let check_each = req.num_passw > 1 && req.option_flags & PASSWOPTION_CHECKEACHPASSW != 0;
    let summary = generate(pg, rng, req, common, script, cancel, progress, &mut |p, s| {
        if list.is_empty() && req.include_header {
            let total = if check_each { MAX_ENTROPY as f64 } else { s.total_security };
            let h = passw_list_header(req.num_passw, s.first_security, total);
            header_len = h.len();
            list.push_str(&h);
        }
        let est = (list.len() + (p.passw.len() + p.appendix.len())) as u64 * 2;
        if est > PASSWLIST_MAX_BYTES {
            return Err(trl(
                "The estimated list size will probably exceed\nthe internal memory limit.\nPlease reduce the number of passwords",
            ));
        }
        list.push_str(p.passw);
        list.push_str(p.appendix);
        Ok(SinkAction::Continue)
    })?;
    if summary.count != 0 {
        if req.include_header
            && (summary.count < req.num_passw || (check_each && summary.total_security < MAX_ENTROPY as f64))
        {
            let h = passw_list_header(summary.count, summary.first_security, summary.total_security);
            if h.len() <= header_len {
                // The original overwrites the end of the old header area and
                // starts the list at the offset; the result is the new header.
                let mut s = Zeroizing::new(String::with_capacity(list.len()));
                s.push_str(&h);
                s.push_str(&list[header_len..]);
                list = s;
            }
        }
        // remove the last line break (2 characters)
        let mut cut = 0;
        for _ in 0..2 {
            if let Some(c) = list[..list.len() - cut].chars().last() {
                cut += c.len_utf8();
            }
        }
        let new_len = list.len() - cut;
        list.truncate(new_len);
    }
    Ok((list, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::AesCtrPrng;

    fn req(dest: GenDest, n: u64) -> GenRequest {
        GenRequest {
            dest,
            num_passw: n,
            chars_len: 16,
            num_words: 0,
            combine_words_chars: false,
            passphrase_length: None,
            format: String::new(),
            option_flags: 0,
            include_header: false,
            file_newline: "\n".into(),
            test_common_passw: true,
        }
    }

    #[test]
    fn single_password() {
        let g = PasswordGenerator::new();
        let mut rng = AesCtrPrng::from_seed(b"x");
        let mut out = String::new();
        let s = generate(&g, &mut rng, &req(GenDest::GuiSingle, 1), None, None, &AtomicBool::new(false), &AtomicU64::new(0), &mut |p, _| {
            out = p.passw.to_string();
            Ok(SinkAction::Continue)
        })
        .unwrap();
        assert_eq!(out.len(), 16);
        assert_eq!(s.count, 1);
        assert!((s.last_security - 16.0 * 62f64.log2()).abs() < 1e-9);
    }

    #[test]
    fn list_with_header_and_check() {
        let g = PasswordGenerator::new();
        let mut rng = AesCtrPrng::from_seed(b"x");
        let mut r = req(GenDest::GuiList, 5);
        r.include_header = true;
        r.option_flags = PASSWOPTION_CHECKEACHPASSW;
        let (list, s) =
            generate_list(&g, &mut rng, &r, None, None, &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
        assert_eq!(s.count, 5);
        let lines: Vec<&str> = list.split("\r\n").collect();
        assert!(lines[0].starts_with("5 passwords generated."));
        assert_eq!(lines.len(), 3 + 1 + 5);
        assert!(lines[4].ends_with("  [95]"), "{}", lines[4]);
    }

    #[test]
    fn passphrase_with_length() {
        let g = PasswordGenerator::new();
        let mut rng = AesCtrPrng::from_seed(b"x");
        let mut r = req(GenDest::GuiSingle, 1);
        r.chars_len = 0;
        r.num_words = 3;
        r.passphrase_length = parse_passphrase_length("10-12").unwrap();
        let mut out = String::new();
        generate(&g, &mut rng, &r, None, None, &AtomicBool::new(false), &AtomicU64::new(0), &mut |p, _| {
            out = p.passw.to_string();
            Ok(SinkAction::Continue)
        })
        .unwrap();
        let n = out.split(' ').map(|w| w.chars().count()).sum::<usize>() + 2;
        assert!((10..=12).contains(&n), "{out}");
    }

    #[test]
    fn length_parsing() {
        assert_eq!(parse_passphrase_length(">=8").unwrap(), Some(PassphraseLength { min: 8, max: i32::MAX, all_chars: false }));
        assert_eq!(parse_passphrase_length(">8").unwrap(), Some(PassphraseLength { min: 9, max: i32::MAX, all_chars: false }));
        assert_eq!(parse_passphrase_length("<8*").unwrap(), Some(PassphraseLength { min: 0, max: 7, all_chars: true }));
        assert_eq!(parse_passphrase_length("20-10").unwrap(), Some(PassphraseLength { min: 10, max: 20, all_chars: false }));
        assert!(parse_passphrase_length("abc").is_err());
        assert_eq!(parse_num_passwords("5k"), Some(5000));
        assert_eq!(parse_num_passwords("1"), None);
    }

    #[test]
    fn format_without_p() {
        let g = PasswordGenerator::new();
        let mut rng = AesCtrPrng::from_seed(b"x");
        let mut r = req(GenDest::GuiSingle, 1);
        r.format = "4d".into();
        let mut out = String::new();
        let s = generate(&g, &mut rng, &r, None, None, &AtomicBool::new(false), &AtomicU64::new(0), &mut |p, _| {
            out = p.passw.to_string();
            Ok(SinkAction::Continue)
        })
        .unwrap();
        assert_eq!(out.len(), 4);
        assert_eq!(s.format_info, "\"P\" is not specified");
        assert!((s.last_security - 4.0 * 10f64.log2()).abs() < 1e-9);
    }
}
