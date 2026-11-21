//! Saving before the session ends (`OnQueryEndSession()`/`OnEndSession()`
//! of the original). A hidden top-level window in its own thread receives
//! `WM_QUERYENDSESSION` and waits (at most 5 s) until the GUI thread has run
//! the handler; the session may then end.

use std::sync::OnceLock;
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, TranslateMessage, WM_ENDSESSION,
    WM_QUERYENDSESSION, WNDCLASSW,
};

type Reply = std::sync::mpsc::Sender<()>;

/// Requests to the GUI thread: run the handler, then answer.
static REQUESTS: OnceLock<async_channel::Sender<Reply>> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_QUERYENDSESSION => {
            if let Some(tx) = REQUESTS.get() {
                let (reply, done) = std::sync::mpsc::channel();
                if tx.send_blocking(reply).is_ok() {
                    let _ = done.recv_timeout(Duration::from_secs(5));
                }
            }
            1
        }
        WM_ENDSESSION => 0,
        // SAFETY: default handling of the other messages of this window.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Call `on_end` on the GUI thread when the user session ends (log off,
/// shut down). Call once.
pub fn watch(on_end: impl Fn() + 'static) {
    let (tx, rx) = async_channel::unbounded::<Reply>();
    if REQUESTS.set(tx).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("pwtool-session".into()).spawn(|| {
        // SAFETY: window class, window and message loop of this thread; the
        // strings live until the window is created.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide("PwToolSessionWindow");
            let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassW(&wc);
            // a top-level window (not message-only): only those get the
            // session messages
            let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
            if hwnd.is_null() {
                eprintln!("pwtool: session window: {}", std::io::Error::last_os_error());
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
    crate::rt::spawn_local(async move {
        while let Ok(reply) = rx.recv().await {
            on_end();
            let _ = reply.send(());
        }
    });
}
