//! Global hot keys on Windows: `RegisterHotKey()` as in the original
//! (`ActivateHotKeys()`). The keys are registered in a separate thread with
//! its own message loop; `WM_HOTKEY` is forwarded to the GUI thread.

use std::cell::RefCell;
use std::rc::Rc;

use crate::platform::Parent;
use crate::rt;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT, WM_USER,
};

thread_local! {
    /// Thread id of the message loop holding the current registrations.
    static ACTIVE: RefCell<Option<u32>> = const { RefCell::new(None) };
}

/// Modifiers and virtual key of a VCL `TShortCut` (low byte: virtual key;
/// 0x2000 Shift, 0x4000 Ctrl, 0x8000 Alt).
fn split_shortcut(code: u16) -> (u32, u32) {
    let mut mods = MOD_NOREPEAT;
    if code & 0x2000 != 0 {
        mods |= MOD_SHIFT;
    }
    if code & 0x4000 != 0 {
        mods |= MOD_CONTROL;
    }
    if code & 0x8000 != 0 {
        mods |= MOD_ALT;
    }
    (mods, (code & 0xff) as u32)
}

/// Unregister all hot keys (`DeactivateHotKeys()`).
pub fn deactivate() {
    if let Some(tid) = ACTIVE.with(|a| a.borrow_mut().take()) {
        // SAFETY: posting to a thread id; harmless if the thread has ended.
        unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
    }
}

/// Register hot keys: `keys` = (shortcut code, description). `on_activated`
/// is called with the shortcut code. Returns the number of registered keys;
/// an error if some key could not be registered (used by another program).
pub async fn activate(
    _parent: Option<&Parent>,
    keys: Vec<(u16, String)>,
    on_activated: Rc<dyn Fn(u16)>,
) -> Result<usize, String> {
    deactivate();
    if keys.is_empty() {
        return Ok(0);
    }
    let codes: Vec<u16> = keys.iter().map(|(c, _)| *c).collect();
    let (tx, rx) = async_channel::unbounded::<u16>();
    let (rtx, rrx) = std::sync::mpsc::channel::<(u32, Vec<u16>)>();
    std::thread::Builder::new()
        .name("pwtool-hotkeys".into())
        .spawn(move || {
            // SAFETY: Win32 message loop of this thread; all pointers are local.
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                // create the message queue before the id is published
                PeekMessageW(&mut msg, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
                let mut failed = Vec::new();
                for (i, &code) in codes.iter().enumerate() {
                    let (mods, vk) = split_shortcut(code);
                    if vk == 0 || RegisterHotKey(std::ptr::null_mut(), i as i32 + 1, mods, vk) == 0 {
                        failed.push(code);
                    }
                }
                let _ = rtx.send((GetCurrentThreadId(), failed));
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if msg.message == WM_HOTKEY {
                        if let Some(&code) = codes.get((msg.wParam as usize).wrapping_sub(1)) {
                            let _ = tx.send_blocking(code);
                        }
                    }
                }
                for i in 0..codes.len() {
                    UnregisterHotKey(std::ptr::null_mut(), i as i32 + 1);
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let (tid, failed) = rrx.recv().map_err(|e| e.to_string())?;
    ACTIVE.with(|a| *a.borrow_mut() = Some(tid));
    rt::spawn_local(async move {
        while let Ok(code) = rx.recv().await {
            on_activated(code);
        }
    });
    if failed.is_empty() {
        Ok(keys.len())
    } else {
        let names: Vec<String> = failed.iter().map(|c| crate::shortcut::shortcut_to_text(*c)).collect();
        Err(format!("RegisterHotKey failed: {}", names.join(", ")))
    }
}
