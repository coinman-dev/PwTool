//! Password Tool (port of Password Tech for Linux and Windows) — entry point.

// GUI program on Windows (no console window); the console mode attaches to
// the console of the calling process.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod app_config;
mod cli;
mod program;
mod ui;

fn main() {
    pwtool_core::secure_mem::harden_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = app::parse_cmdline(&args);
    if cmd.gen_num_passw > 0 || cmd.show_help {
        #[cfg(windows)]
        attach_parent_console();
        std::process::exit(cli::run(cmd));
    }
    let rc = ui::run(cmd);
    if app::restart_requested() {
        if let Ok(exe) = std::env::current_exe() {
            #[cfg(unix)]
            let err = {
                use std::os::unix::process::CommandExt;
                std::process::Command::new(exe).exec()
            };
            #[cfg(windows)]
            let err = match std::process::Command::new(exe).spawn() {
                Ok(_) => std::process::exit(rc),
                Err(e) => e,
            };
            eprintln!("pwtool: restart failed: {err}");
        }
    }
    std::process::exit(rc);
}

/// Console mode of the GUI executable: write to the console of the parent
/// process (cmd, PowerShell) unless the output is redirected.
#[cfg(windows)]
fn attach_parent_console() {
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
