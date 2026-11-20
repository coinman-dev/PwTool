//! Password Tool (Linux port of Password Tech) — entry point.

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
        std::process::exit(cli::run(cmd));
    }
    let rc = ui::run(cmd);
    if app::restart_requested() {
        use std::os::unix::process::CommandExt;
        if let Ok(exe) = std::env::current_exe() {
            let err = std::process::Command::new(exe).exec();
            eprintln!("pwtool: restart failed: {err}");
        }
    }
    std::process::exit(rc);
}
