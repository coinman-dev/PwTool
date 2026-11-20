//! Window functions on Windows, done with the Win32 API as in the original:
//! title of the foreground window (`GetForegroundWindow()` +
//! `GetWindowText()`), "Always on Top" (`TopMostManager`) and resetting the
//! window positions. Minimizing is reported by GDK itself, so no watcher is
//! needed.

use gtk4 as gtk;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST, IsWindowVisible, SPI_GETWORKAREA, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, SystemParametersInfoW,
};

/// Visible top-level windows of this process.
fn own_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: lparam is the &mut Vec passed below; hwnd comes from EnumWindows.
        unsafe {
            let v = &mut *(lparam as *mut Vec<HWND>);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
                v.push(hwnd);
            }
        }
        1
    }
    let mut v: Vec<HWND> = Vec::new();
    // SAFETY: the callback only uses the vector for the duration of the call.
    unsafe { EnumWindows(Some(collect), &mut v as *mut Vec<HWND> as LPARAM) };
    v
}

/// Caption of the foreground window.
pub async fn active_window_title(_app: &gtk::Application) -> Option<String> {
    // SAFETY: plain Win32 calls with a local buffer.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; len.max(0) as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        Some(String::from_utf16_lossy(&buf[..n.max(0) as usize]))
    }
}

/// "Always on Top" for all windows of this process (`HWND_TOPMOST`).
pub async fn set_keep_above(_app: &gtk::Application, above: bool) -> bool {
    let after = if above { HWND_TOPMOST } else { HWND_NOTOPMOST };
    for hwnd in own_windows() {
        // SAFETY: hwnd is a window of this process.
        unsafe { SetWindowPos(hwnd, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    }
    true
}

/// Not needed on Windows (GDK reports the minimized state); returns `false`.
pub async fn watch_minimized(_app: &gtk::Application, _f: impl Fn(&str, bool) + 'static) -> bool {
    false
}

/// Nothing to stop on Windows.
pub fn stop_watching(_app: &gtk::Application) {}

/// `TrayMenu_ResetWindowPosClick()`: move all windows of this process to
/// the top left corner of the work area of the primary monitor.
pub async fn reset_window_positions(_app: &gtk::Application) -> bool {
    // SAFETY: plain Win32 calls with local structs.
    unsafe {
        let mut area: RECT = std::mem::zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut area as *mut RECT).cast(), 0) == 0 {
            return false;
        }
        for hwnd in own_windows() {
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut r) != 0 {
                SetWindowPos(hwnd, std::ptr::null_mut(), area.left, area.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }
    true
}
