//! Password Tool (port of Password Tech for Linux and Windows) — entry point.

// GUI program on Windows (no console window); the console mode attaches to
// the console of the calling process.
#![cfg_attr(windows, windows_subsystem = "windows")]

// Non-visual modules shared with the egui interface.
pub use pwtool_app::{app, app_config, cli, program};
mod ui;

fn main() {
    pwtool_core::secure_mem::harden_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = app::parse_cmdline(&args);
    if cmd.gen_num_passw > 0 || cmd.show_help {
        #[cfg(windows)]
        pwtool_app::platform::sys::attach_parent_console();
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

