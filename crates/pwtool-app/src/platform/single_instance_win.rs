//! One instance per user session on Windows. On Linux `GApplication` does
//! this over D-Bus; on Windows there is no session bus, so a named mutex
//! marks the running instance and a named event asks it to show its window
//! (the original refuses a second start in the same way).

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

const MUTEX_NAME: &str = r"Local\io.github.coinman_dev.PwTool.Instance";
const EVENT_NAME: &str = r"Local\io.github.coinman_dev.PwTool.Activate";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Held by the running instance.
pub struct Instance {
    mutex: HANDLE,
    event: HANDLE,
}

impl Instance {
    /// Become the running instance. If another instance runs, it is asked
    /// to show its window and `None` is returned.
    pub fn acquire() -> Option<Instance> {
        // SAFETY: named kernel objects with valid NUL-terminated names.
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, wide(MUTEX_NAME).as_ptr());
            if !mutex.is_null() && GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(mutex);
                let event = OpenEventW(EVENT_MODIFY_STATE, 0, wide(EVENT_NAME).as_ptr());
                if !event.is_null() {
                    // let the running instance bring its window to the front
                    AllowSetForegroundWindow(ASFW_ANY);
                    SetEvent(event);
                    CloseHandle(event);
                }
                return None;
            }
            let event = CreateEventW(std::ptr::null(), 0, 0, wide(EVENT_NAME).as_ptr());
            Some(Instance { mutex, event })
        }
    }

    /// Call `on_activate` (show the main window) on the GUI thread when
    /// another start is attempted.
    pub fn watch(&self, on_activate: impl Fn() + 'static) {
        if self.event.is_null() {
            return;
        }
        let event = self.event as isize;
        let (tx, rx) = async_channel::unbounded::<()>();
        let _ = std::thread::Builder::new().name("pwtool-instance".into()).spawn(move || {
            // SAFETY: the event handle stays open while the program runs.
            while unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } == 0 {
                if tx.send_blocking(()).is_err() {
                    break;
                }
            }
        });
        crate::rt::spawn_local(async move {
            while rx.recv().await.is_ok() {
                on_activate();
            }
        });
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: handles created in acquire(). The event stays open: the
        // watcher thread may still wait on it until the process exits.
        unsafe {
            if !self.mutex.is_null() {
                CloseHandle(self.mutex);
            }
        }
    }
}
