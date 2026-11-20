//! Small operating system functions: opening files and links, free disk
//! space, the app mode of Windows, the console of the console mode.

use std::path::Path;

/// Last resort for opening a file or URI: `xdg-open` on Linux,
/// `ShellExecuteW("open")` on Windows.
#[cfg(not(windows))]
pub fn system_open(target: &std::ffi::OsStr) -> bool {
    std::process::Command::new("xdg-open").arg(target).spawn().is_ok()
}

#[cfg(windows)]
pub fn system_open(target: &std::ffi::OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let file: Vec<u16> = target.encode_wide().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    // SAFETY: valid NUL-terminated strings.
    let rc = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    rc as isize > 32
}

/// "Choose your app mode" of the Windows settings (`AppsUseLightTheme`):
/// `Some(true)` for dark.
#[cfg(windows)]
pub fn windows_apps_use_dark_theme() -> Option<bool> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    let mut data: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: valid NUL-terminated strings and output buffer of `size` bytes.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    (rc == 0).then_some(data == 0)
}

/// Free space on the file system of `path` (or its directory).
#[cfg(windows)]
pub fn free_disk_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let dir = if path.is_dir() { path } else { path.parent().filter(|p| !p.as_os_str().is_empty())? };
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut avail = 0u64;
    // SAFETY: valid NUL-terminated path and output pointer
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut avail,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(avail)
}

/// Free space on the file system of `path` (or its directory).
#[cfg(unix)]
pub fn free_disk_space(path: &Path) -> Option<u64> {
    let dir = if path.is_dir() { path } else { path.parent().filter(|p| !p.as_os_str().is_empty())? };
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid C string and output struct
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Console mode of the GUI executable: write to the console of the parent
/// process (cmd, PowerShell) unless the output is redirected.
#[cfg(windows)]
pub fn attach_parent_console() {
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    // SAFETY: plain Win32 calls; handles are checked before use.
    unsafe {
        let redirected = |id| {
            let h = GetStdHandle(id);
            !h.is_null() && h != INVALID_HANDLE_VALUE
        };
        let (out, err) = (redirected(STD_OUTPUT_HANDLE), redirected(STD_ERROR_HANDLE));
        if (out && err) || AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let h = CreateFileW(
            name.as_ptr(),
            FILE_GENERIC_READ | FILE_GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return;
        }
        if !out {
            SetStdHandle(STD_OUTPUT_HANDLE, h);
        }
        if !err {
            SetStdHandle(STD_ERROR_HANDLE, h);
        }
        // the prompt of the shell was already printed: start on a new line
        println!();
    }
}
