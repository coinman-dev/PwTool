//! Password Tool: everything that does not depend on the GUI toolkit —
//! application state and configuration (`app`, `app_config`), console mode
//! (`cli`), the form descriptions of the original (`form`), and the
//! services of the operating system (`platform`: tray, global hot keys,
//! autotype, clipboard, single instance, autostart…).
//!
//! Used by the interface (`pwtool`).

pub mod app;
pub mod app_config;
pub mod autotype;
pub mod char_set_builder;
pub mod cli;
pub mod form;
pub mod icons;
pub mod platform;
pub mod program;
pub mod quality;
pub mod resources;
pub mod rt;
pub mod shortcut;

/// Diagnostic message on stderr, shown only when `PWTOOL_DEBUG` is set:
/// for things that are normal on many systems (no tray host, no portal)
/// and should not clutter the console at every start.
pub fn diag(msg: std::fmt::Arguments) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("PWTOOL_DEBUG").is_some()) {
        eprintln!("pwtool: {msg}");
    }
}
