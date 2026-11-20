//! Services of the operating system, independent of the GUI toolkit.
//!
//! Linux: StatusNotifierItem tray (`ksni`), XDG desktop portals for global
//! shortcuts and autotype (`ashpd`). Windows: Win32 API as in the original
//! (`*_win.rs`). Both variants of a module have the same interface.

#[cfg_attr(unix, path = "tray_sni.rs")]
#[cfg_attr(windows, path = "tray_win.rs")]
pub mod tray;

#[cfg_attr(unix, path = "hotkeys_portal.rs")]
#[cfg_attr(windows, path = "hotkeys_win.rs")]
pub mod hotkeys;

#[cfg(unix)]
pub mod portal;

#[cfg(windows)]
#[path = "active_window_win.rs"]
pub mod active_window;

#[cfg(windows)]
#[path = "clipboard_win.rs"]
pub mod clipboard;

#[cfg(windows)]
#[path = "single_instance_win.rs"]
pub mod single_instance;

pub mod autostart;
pub mod sys;

/// The window a portal request or an owned window belongs to.
#[derive(Default)]
pub struct Parent {
    /// Identifier of the window for XDG desktop portals.
    #[cfg(unix)]
    pub portal: Option<ashpd::WindowIdentifier>,
    /// Native window handle (`HWND`), 0 if unknown.
    #[cfg(windows)]
    pub hwnd: isize,
}

impl std::fmt::Debug for Parent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Parent");
        #[cfg(unix)]
        d.field("portal", &self.portal.is_some());
        #[cfg(windows)]
        d.field("hwnd", &self.hwnd);
        d.finish()
    }
}
