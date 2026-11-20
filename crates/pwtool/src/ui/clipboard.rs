//! Clipboard for sensitive data (`upstream/src/util/SecureClipboard.*`).
//!
//! Text is put on the clipboard together with the KDE hint
//! `x-kde-passwordManagerHint: secret`, so that Klipper does not keep it in
//! its history. On Windows the formats `ExcludeClipboardContentFromMonitorProcessing`,
//! `CanIncludeInClipboardHistory` and `CanUploadToCloudClipboard` keep it out of
//! the clipboard history and cloud clipboard. A SHA-256 digest of the text is kept; automatic clearing only
//! clears the clipboard if it still holds that text.

use std::cell::RefCell;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, glib};
use zeroize::Zeroizing;

thread_local! {
    static DIGEST: RefCell<Option<[u8; 32]>> = const { RefCell::new(None) };
    static ON_SET: RefCell<Vec<Box<dyn Fn()>>> = RefCell::new(Vec::new());
}

fn digest(text: &str) -> [u8; 32] {
    let units: Zeroizing<Vec<u8>> = Zeroizing::new(text.encode_utf16().flat_map(u16::to_le_bytes).collect());
    pwtool_core::crypto::sha256(&units[..units.len().min(1_000_000)])
}

fn clipboard() -> Option<gdk::Clipboard> {
    gdk::Display::default().map(|d| d.clipboard())
}

/// Register a function called after sensitive data was put on the clipboard
/// (`RegisterOnSetDataFun`; the main window starts the auto-clear timer).
pub fn on_set_data(f: impl Fn() + 'static) {
    ON_SET.with(|v| v.borrow_mut().push(Box::new(f)));
}

/// Put sensitive text on the clipboard (`SecureClipboard::SetData`).
pub fn set_data(text: &str) {
    if text.is_empty() {
        return;
    }
    set_secret(text);
    DIGEST.with(|d| *d.borrow_mut() = Some(digest(text)));
    ON_SET.with(|v| {
        for f in v.borrow().iter() {
            f();
        }
    });
}

/// Put sensitive text on the clipboard with the password-manager hint, but
/// without automatic clearing (decrypted text: the original uses the plain
/// clipboard here).
pub fn set_hidden(text: &str) {
    set_secret(text);
}

/// Put text on the clipboard marked as secret.
fn set_secret(text: &str) {
    #[cfg(windows)]
    if win::set_secret_text(text) {
        return;
    }
    let Some(cb) = clipboard() else { return };
    let providers = [
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &glib::Bytes::from(text.as_bytes())),
        gdk::ContentProvider::for_value(&text.to_value()),
        gdk::ContentProvider::for_bytes("x-kde-passwordManagerHint", &glib::Bytes::from_static(b"secret")),
    ];
    let _ = cb.set_content(Some(&gdk::ContentProvider::new_union(&providers)));
}

/// Put ordinary (non-sensitive) text on the clipboard.
pub fn set_plain(text: &str) {
    if let Some(cb) = clipboard() {
        cb.set_text(text);
    }
}

/// Clear the clipboard (`ClearData`): with `force` always, otherwise only if
/// it still holds the text set by [`set_data`]. The callback receives
/// whether the operation succeeded.
pub fn clear_data(force: bool, done: impl FnOnce(bool) + 'static) {
    let has_digest = DIGEST.with(|d| d.borrow().is_some());
    if !has_digest && !force {
        done(true);
        return;
    }
    let Some(cb) = clipboard() else {
        done(false);
        return;
    };
    let cb2 = cb.clone();
    cb.read_text_async(gtk::gio::Cancellable::NONE, move |r| {
        let current = r.ok().flatten().map(|s| Zeroizing::new(s.to_string()));
        let clear = match &current {
            Some(t) if !t.is_empty() => force || DIGEST.with(|d| *d.borrow() == Some(digest(t))),
            _ => false,
        };
        if clear {
            let _ = cb2.set_content(None::<&gdk::ContentProvider>);
            cb2.set_text("");
        }
        DIGEST.with(|d| *d.borrow_mut() = None);
        done(true);
    });
}

/// Read the clipboard text.
pub fn read_text(done: impl FnOnce(Option<Zeroizing<String>>) + 'static) {
    let Some(cb) = clipboard() else {
        done(None);
        return;
    };
    cb.read_text_async(gtk::gio::Cancellable::NONE, move |r| {
        done(r.ok().flatten().map(|s| Zeroizing::new(s.to_string())));
    });
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::Foundation::{GlobalFree, HWND};
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, HWND_MESSAGE};
    use zeroize::Zeroize;

    const CF_UNICODETEXT: u32 = 13;

    thread_local! {
        static OWNER: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    }

    /// Message-only window owning the clipboard data (with a NULL owner
    /// `SetClipboardData` fails after `EmptyClipboard`). Created on the GTK
    /// thread, whose message loop serves it.
    fn owner_window() -> HWND {
        OWNER.with(|o| {
            if o.get() == 0 {
                let class: Vec<u16> = "STATIC".encode_utf16().chain(std::iter::once(0)).collect();
                // SAFETY: predefined window class; message-only parent.
                let hwnd = unsafe {
                    CreateWindowExW(
                        0,
                        class.as_ptr(),
                        std::ptr::null(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        HWND_MESSAGE,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    )
                };
                o.set(hwnd as isize);
            }
            o.get() as HWND
        })
    }

    /// Copy bytes into a movable global memory block for `SetClipboardData`.
    unsafe fn global_copy(bytes: &[u8]) -> *mut core::ffi::c_void {
        // SAFETY: allocation checked; the copy stays within the block.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
            if h.is_null() {
                return h;
            }
            let p = GlobalLock(h);
            if p.is_null() {
                GlobalFree(h);
                return std::ptr::null_mut();
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
            GlobalUnlock(h);
            h
        }
    }

    unsafe fn set_format(name: &str, bytes: &[u8]) {
        let w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: valid NUL-terminated name; the clipboard is open.
        unsafe {
            let fmt = RegisterClipboardFormatW(w.as_ptr());
            if fmt != 0 {
                let h = global_copy(bytes);
                if !h.is_null() && SetClipboardData(fmt, h).is_null() {
                    GlobalFree(h);
                }
            }
        }
    }

    /// `SecureClipboard::SetData` of the original: Unicode text plus the
    /// formats that exclude it from clipboard history, cloud clipboard and
    /// clipboard monitors. Returns `false` if the clipboard is not available.
    pub fn set_secret_text(text: &str) -> bool {
        let owner = owner_window();
        if owner.is_null() {
            return false;
        }
        let mut units: Vec<u8> = text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
        // SAFETY: clipboard calls between OpenClipboard and CloseClipboard.
        let ok = unsafe {
            let mut opened = false;
            for _ in 0..10 {
                if OpenClipboard(owner) != 0 {
                    opened = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if !opened {
                false
            } else {
                EmptyClipboard();
                let h = global_copy(&units);
                let ok = !h.is_null() && !SetClipboardData(CF_UNICODETEXT, h).is_null();
                if !ok && !h.is_null() {
                    GlobalFree(h);
                }
                set_format("ExcludeClipboardContentFromMonitorProcessing", &[0]);
                set_format("CanIncludeInClipboardHistory", &0u32.to_le_bytes());
                set_format("CanUploadToCloudClipboard", &0u32.to_le_bytes());
                CloseClipboard();
                ok
            }
        };
        units.zeroize();
        ok
    }
}
