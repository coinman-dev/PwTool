//! VCL keyboard shortcuts (`TShortCut`: low byte virtual key, 0x2000 Shift,
//! 0x4000 Ctrl, 0x8000 Alt) as text and as accelerator strings.

/// VCL `ShortCut` value → accelerator string in GTK syntax ("<Control><Alt>p"),
/// also used for the XDG GlobalShortcuts portal.
pub fn shortcut_to_accel(code: i64) -> Option<String> {
    let mut s = String::new();
    if code & 0x4000 != 0 {
        s.push_str("<Control>");
    }
    if code & 0x8000 != 0 {
        s.push_str("<Alt>");
    }
    if code & 0x2000 != 0 {
        s.push_str("<Shift>");
    }
    let key = (code & 0xff) as u32;
    let name = match key {
        0x70..=0x87 => format!("F{}", key - 0x6f),
        0x30..=0x39 => ((key as u8) as char).to_string(),
        0x41..=0x5a => ((key as u8 + 32) as char).to_string(),
        0x08 => "BackSpace".into(),
        0x09 => "Tab".into(),
        0x0d => "Return".into(),
        0x1b => "Escape".into(),
        0x20 => "space".into(),
        0x21 => "Page_Up".into(),
        0x22 => "Page_Down".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2d => "Insert".into(),
        0x2e => "Delete".into(),
        0x13 => "Pause".into(),
        0x60..=0x69 => format!("KP_{}", key - 0x60),
        _ => return None,
    };
    s.push_str(&name);
    Some(s)
}

/// `ShortCutToText()`: e.g. "Ctrl+Alt+P".
pub fn shortcut_to_text(code: u16) -> String {
    let mut s = String::new();
    if code & 0x2000 != 0 {
        s.push_str("Shift+");
    }
    if code & 0x4000 != 0 {
        s.push_str("Ctrl+");
    }
    if code & 0x8000 != 0 {
        s.push_str("Alt+");
    }
    let key = code & 0xff;
    let name = match key {
        0x30..=0x39 | 0x41..=0x5a => ((key as u8) as char).to_string(),
        0x60..=0x69 => format!("Num {}", key - 0x60),
        0x70..=0x87 => format!("F{}", key - 0x6f),
        0x08 => "BkSp".into(),
        0x09 => "Tab".into(),
        0x0d => "Enter".into(),
        0x13 => "Pause".into(),
        0x1b => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2d => "Ins".into(),
        0x2e => "Del".into(),
        _ => format!("#{key}"),
    };
    s.push_str(&name);
    s
}
