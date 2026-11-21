//! Clipboard for secrets on Windows (`SecureClipboard::SetData` of the
//! original): text that clipboard history, cloud clipboard and clipboard
//! monitors leave alone.

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
/// `SetClipboardData` fails after `EmptyClipboard`). Created on the GUI
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
