//! Autotype (`upstream/src/util/SendKeys.*`): typing text and key sequences
//! into the window of another application.
//!
//! Key sequences are built here as X keysyms; the backend types them: the
//! RemoteDesktop portal on Linux (`autotype_portal.rs`), `SendInput()` on
//! Windows (`autotype_win.rs`).

use pwtool_core::passw::database::{FIELD_NAMES, NUM_STRING_FIELDS};
use zeroize::{Zeroize, Zeroizing};

/// Delay after virtual keys like Tab or Enter (`VIRTUAL_KEY_DELAY`).
pub const VIRTUAL_KEY_DELAY: u32 = 200;
/// Delay before typing starts (`INIT_DELAY`).
pub const INIT_DELAY: u32 = 250;

const USERNAME: usize = 1;
const PASSWORD: usize = 2;

/// Keysyms of modifier keys.
const XK_SHIFT_L: u32 = 0xffe1;
const XK_CONTROL_L: u32 = 0xffe3;
const XK_ALT_L: u32 = 0xffe9;

/// One step: keys pressed together (modifiers first), then a delay.
pub struct KeyStroke {
    pub keysyms: Vec<u32>,
    pub delay_ms: u32,
}

impl Drop for KeyStroke {
    fn drop(&mut self) {
        self.keysyms.zeroize();
    }
}

/// Keysym of a character.
pub fn char_keysym(c: char) -> u32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        c if (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c) => c as u32,
        c => 0x0100_0000 | c as u32,
    }
}

/// Keysym of a key placeholder (`s_keyPlaceholders` with `VK_*`).
fn placeholder_keysym(name: &str) -> Option<u32> {
    Some(match name {
        "tab" => 0xff09,
        "return" | "enter" => 0xff0d,
        "ctrl" => XK_CONTROL_L,
        "backspace" => 0xff08,
        "clear" => 0xff0b,
        "shift" => XK_SHIFT_L,
        "alt" => XK_ALT_L,
        "pause" => 0xff13,
        "capslock" => 0xffe5,
        "escape" => 0xff1b,
        "space" => 0x20,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "end" => 0xff57,
        "home" => 0xff50,
        "left" => 0xff51,
        "right" => 0xff53,
        "down" => 0xff54,
        "up" => 0xff52,
        "select" => 0xff60,
        "print" | "snapshot" => 0xff61,
        "execute" => 0xff62,
        "insert" => 0xff63,
        "delete" => 0xffff,
        "help" => 0xff6a,
        _ => return None,
    })
}

/// Parse leading signed 32-bit decimal integer (matches C++ `std::stoi`).
fn parse_stoi(s: &str) -> Option<i32> {
    let s = s.trim_start();
    let mut end = 0;
    let bytes = s.as_bytes();
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let digits_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        return None;
    }
    s[..end].parse::<i32>().ok()
}

/// `SendKeys::AddString()`: one stroke per character (CRLF produces one Return).
pub fn string_sequence(s: &str, delay: u32, dest: &mut Vec<KeyStroke>) {
    let mut prev_cr = false;
    for c in s.chars() {
        if c == '\n' && prev_cr {
            prev_cr = false;
            continue;
        }
        prev_cr = c == '\r';
        dest.push(KeyStroke { keysyms: vec![char_keysym(c)], delay_ms: delay });
    }
}

/// `SendKeys::SendComplexString()` with a destination sequence. `field`
/// returns the string fields of a database entry (index as in
/// [`FIELD_NAMES`]); without an entry `{parameter}`/`{username}` and
/// `{password}` use `param` and `passw`.
pub fn build_sequence(
    seq: &str,
    field: Option<&dyn Fn(usize) -> Option<Zeroizing<String>>>,
    param: Option<&str>,
    passw: Option<&str>,
    delay: u32,
) -> Vec<KeyStroke> {
    let mut dest: Vec<KeyStroke> = Vec::new();
    let mut buffered: Vec<u32> = Vec::new();
    let chars: Vec<char> = seq.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let mut found = false;
        if chars[i] == '{' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '}').map(|p| p + i + 1) {
                if end != i + 1 {
                    let name: String = chars[i + 1..end].iter().collect::<String>().to_lowercase();
                    if name.len() > 5 && name.starts_with("wait:") {
                        found = true;
                        i = end + 1;
                        if let Some(t) = parse_stoi(&name[5..]) {
                            dest.push(KeyStroke { keysyms: Vec::new(), delay_ms: t.clamp(10, 10000) as u32 });
                        }
                    } else if let Some(idx) = FIELD_NAMES[..NUM_STRING_FIELDS]
                        .iter()
                        .position(|f| f.eq_ignore_ascii_case(&name))
                        .or_else(|| (name == "parameter").then_some(USERNAME))
                    {
                        found = true;
                        i = end + 1;
                        match field {
                            Some(f) => {
                                if let Some(v) = f(idx) {
                                    string_sequence(&v, delay, &mut dest);
                                }
                            }
                            None => {
                                if idx == USERNAME {
                                    if let Some(p) = param {
                                        string_sequence(p, delay, &mut dest);
                                    }
                                } else if idx == PASSWORD {
                                    if let Some(p) = passw {
                                        string_sequence(p, delay, &mut dest);
                                    }
                                }
                            }
                        }
                    } else if let Some(ks) = placeholder_keysym(&name) {
                        found = true;
                        i = end + 1;
                        match ks {
                            XK_CONTROL_L | XK_SHIFT_L | XK_ALT_L => {
                                if buffered.len() < 3 {
                                    buffered.push(ks);
                                }
                            }
                            _ => {
                                let d = if ks == 0xff09 || ks == 0xff0d { delay.max(VIRTUAL_KEY_DELAY) } else { delay };
                                let mut keys = std::mem::take(&mut buffered);
                                keys.push(ks);
                                if let Some(last) = dest.last_mut() {
                                    last.delay_ms = last.delay_ms.max(VIRTUAL_KEY_DELAY);
                                }
                                dest.push(KeyStroke { keysyms: keys, delay_ms: d });
                            }
                        }
                    }
                }
            }
        }
        if !found {
            let c = chars[i];
            i += 1;
            if c == '\r' && i < chars.len() && chars[i] == '\n' {
                i += 1;
            }
            let mut keys = std::mem::take(&mut buffered);
            // with modifiers the key itself is sent (lower case)
            let c = if keys.is_empty() { c } else { c.to_ascii_lowercase() };
            keys.push(char_keysym(c));
            dest.push(KeyStroke { keysyms: keys, delay_ms: delay });
        }
    }
    dest
}

#[cfg_attr(unix, path = "platform/autotype_portal.rs")]
#[cfg_attr(windows, path = "platform/autotype_win.rs")]
mod backend;
pub use backend::{close_session, prepare, send};

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(v: &[KeyStroke]) -> Vec<(Vec<u32>, u32)> {
        v.iter().map(|k| (k.keysyms.clone(), k.delay_ms)).collect()
    }

    #[test]
    fn sequences() {
        let s = build_sequence("{parameter}{tab}{password}{enter}", None, Some("ab"), Some("Ц1"), 50);
        assert_eq!(
            ks(&s),
            vec![
                (vec![0x61], 50),
                (vec![0x62], 200),
                (vec![0xff09], 200),
                (vec![0x0100_0426], 50),
                (vec![0x31], 200),
                (vec![0xff0d], 200),
            ]
        );
        let s = build_sequence("{ctrl}{shift}A{wait:5}{WAIT:x}{unknown}", None, None, None, 10);
        assert_eq!(ks(&s)[0], (vec![XK_CONTROL_L, XK_SHIFT_L, 0x61], 10));
        assert_eq!(ks(&s)[1], (vec![], 10));
        // "{unknown}" is typed literally
        assert_eq!(s.len(), 2 + "{unknown}".len());
        let s = build_sequence("{title}", Some(&|i| Some(Zeroizing::new(format!("f{i}")))), None, None, 1);
        assert_eq!(ks(&s), vec![(vec![0x66], 1), (vec![0x30], 1)]);

        // CRLF produces a single Return keystroke, not two
        let mut crlf = Vec::new();
        string_sequence("a\r\nb", 10, &mut crlf);
        assert_eq!(ks(&crlf), vec![(vec![0x61], 10), (vec![0xff0d], 10), (vec![0x62], 10)]);

        // {wait:500ms} parses leading integer like std::stoi
        let w = build_sequence("{wait:500ms}", None, None, None, 10);
        assert_eq!(ks(&w), vec![(vec![], 500)]);
    }
}
