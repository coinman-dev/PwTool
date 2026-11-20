//! Autotype backend for Windows: `SendInput()` as in the original
//! (`SendKeys.cpp`). Characters are typed as Unicode input
//! (`KEYEVENTF_UNICODE`), independent of the keyboard layout; keys combined
//! with modifiers and special keys are sent as virtual keys.

use std::rc::Rc;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VkKeyScanW,
};

use super::{INIT_DELAY, KeyStroke};
use crate::platform::Parent;
use crate::rt;

/// Virtual key of a keysym of a special key: (VK, extended key).
fn special_vk(keysym: u32) -> Option<(u16, bool)> {
    Some(match keysym {
        0xff08 => (0x08, false), // Backspace
        0xff09 => (0x09, false), // Tab
        0xff0b => (0x0c, false), // Clear
        0xff0d => (0x0d, false), // Return
        0xff13 => (0x13, false), // Pause
        0xff1b => (0x1b, false), // Escape
        0xff50 => (0x24, true),  // Home
        0xff51 => (0x25, true),  // Left
        0xff52 => (0x26, true),  // Up
        0xff53 => (0x27, true),  // Right
        0xff54 => (0x28, true),  // Down
        0xff55 => (0x21, true),  // Page Up
        0xff56 => (0x22, true),  // Page Down
        0xff57 => (0x23, true),  // End
        0xff60 => (0x29, false), // Select
        0xff61 => (0x2c, false), // Print Screen
        0xff62 => (0x2b, false), // Execute
        0xff63 => (0x2d, true),  // Insert
        0xff6a => (0x2f, false), // Help
        0xffe1 => (0x10, false), // Shift
        0xffe3 => (0x11, false), // Ctrl
        0xffe5 => (0x14, false), // Caps Lock
        0xffe9 => (0x12, false), // Alt
        0xffff => (0x2e, true),  // Delete
        _ => return None,
    })
}

/// Character of a keysym (inverse of `char_keysym`).
fn keysym_char(keysym: u32) -> Option<char> {
    if keysym & 0xff00_0000 == 0x0100_0000 {
        char::from_u32(keysym & 0x00ff_ffff)
    } else if keysym < 0x100 {
        char::from_u32(keysym)
    } else {
        None
    }
}

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn vk_input(vk: u16, extended: bool, up: bool) -> INPUT {
    // SAFETY: pure function.
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    let mut flags = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    key_input(vk, scan, flags)
}

/// Inputs for one stroke: all keys pressed in order, released in reverse.
fn stroke_inputs(st: &KeyStroke) -> Vec<INPUT> {
    let combo = st.keysyms.len() > 1;
    let mut down = Vec::new();
    let mut up = Vec::new();
    for &k in &st.keysyms {
        if let Some((vk, ext)) = special_vk(k) {
            down.push(vk_input(vk, ext, false));
            up.push(vk_input(vk, ext, true));
            continue;
        }
        let Some(c) = keysym_char(k) else { continue };
        if combo {
            // with modifiers: the virtual key of the character ("Ctrl+A")
            let mut buf = [0u16; 2];
            if c.encode_utf16(&mut buf).len() == 1 {
                // SAFETY: pure function.
                let r = unsafe { VkKeyScanW(buf[0]) };
                if r != -1 {
                    let vk = (r & 0xff) as u16;
                    down.push(vk_input(vk, false, false));
                    up.push(vk_input(vk, false, true));
                    continue;
                }
            }
        }
        let mut buf = [0u16; 2];
        for &unit in c.encode_utf16(&mut buf).iter() {
            down.push(key_input(0, unit, KEYEVENTF_UNICODE));
            up.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
    }
    up.reverse();
    down.extend(up);
    down
}

/// Nothing to close on Windows.
pub fn close_session() {}

/// No permission is needed on Windows.
pub async fn prepare(_parent: Option<&Parent>, _token: &str) -> Result<Option<String>, String> {
    Ok(None)
}

/// Type a sequence into the foreground window.
pub async fn send(
    _parent: Option<&Parent>,
    _token: &str,
    strokes: Vec<KeyStroke>,
    abort: Rc<std::cell::Cell<bool>>,
) -> Result<Option<String>, String> {
    rt::sleep(std::time::Duration::from_millis(INIT_DELAY as u64)).await;
    for st in &strokes {
        if abort.get() {
            break;
        }
        let mut inputs = stroke_inputs(st);
        if !inputs.is_empty() {
            // SAFETY: valid array of INPUT structures.
            let n = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
            let sent_all = n as usize == inputs.len();
            // wipe the characters (INPUT is plain data)
            // SAFETY: overwriting initialised plain-data elements with zeros.
            unsafe { std::ptr::write_bytes(inputs.as_mut_ptr(), 0, inputs.len()) };
            if !sent_all {
                // e.g. the foreground window belongs to an elevated process (UIPI)
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        if st.delay_ms > 0 {
            rt::sleep(std::time::Duration::from_millis(st.delay_ms as u64)).await;
        }
    }
    Ok(None)
}
