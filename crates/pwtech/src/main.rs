//! Password Tech for Linux — entry point.

mod app;
mod cli;
mod program;

fn main() {
    pwtech_core::secure_mem::harden_process();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = app::parse_cmdline(&args);
    if cmd.gen_num_passw > 0 || cmd.show_help {
        std::process::exit(cli::run(cmd));
    }
    eprintln!("The graphical user interface is not available yet. Use --gen [number] or --help.");
    std::process::exit(2);
}
