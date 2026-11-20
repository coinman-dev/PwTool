//! Start the program with the session (`LaunchSystemStartup`).

use crate::program::*;

/// Autostart entry (`LaunchSystemStartup`): `~/.config/autostart/
/// pwtool.desktop` starting the program with `--silent` (the original
/// writes the Run key of the registry).
#[cfg(not(windows))]
pub fn set_autostart(enable: bool) -> Result<(), String> {
    let dir = crate::app::config_home().ok_or_else(|| "~/.config/autostart".to_string())?.join("autostart");
    let file = dir.join("pwtool.desktop");
    if enable {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|_| dir.to_string_lossy().into_owned())?;
        let content = format!(
            "[Desktop Entry]\nType=Application\nName={PROGRAM_NAME}\nExec=\"{}\" --silent\nIcon={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=false\n",
            exe.to_string_lossy(),
            crate::program::APP_ID
        );
        std::fs::write(&file, content).map_err(|_| file.to_string_lossy().into_owned())
    } else if file.exists() {
        std::fs::remove_file(&file).map_err(|_| file.to_string_lossy().into_owned())
    } else {
        Ok(())
    }
}

/// Autostart entry (`LaunchSystemStartup`): value "PwTool" of the Run key
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` starting the
/// program with `--silent`, as in the original.
#[cfg(windows)]
pub fn set_autostart(enable: bool) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
    };
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key_name, value_name) = (wide(KEY), wide(PROGRAM_SHORT_NAME));
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated strings and output pointer.
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, KEY_SET_VALUE, &mut key) } != ERROR_SUCCESS {
        return Err(format!(r"HKCU\{KEY}"));
    }
    let rc = if enable {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = wide(&format!("\"{}\" --silent", exe.to_string_lossy()));
        // SAFETY: data is the UTF-16 command line including the terminating NUL.
        unsafe { RegSetValueExW(key, value_name.as_ptr(), 0, REG_SZ, cmd.as_ptr().cast(), (cmd.len() * 2) as u32) }
    } else {
        // SAFETY: valid key and value name.
        match unsafe { RegDeleteValueW(key, value_name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            rc => rc,
        }
    };
    // SAFETY: key was opened above.
    unsafe { RegCloseKey(key) };
    if rc == ERROR_SUCCESS { Ok(()) } else { Err(format!(r"HKCU\{KEY}\{PROGRAM_SHORT_NAME}")) }
}
