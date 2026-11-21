//! Password Tool: everything that does not depend on the GUI toolkit —
//! application state and configuration (`app`, `app_config`), console mode
//! (`cli`), the form descriptions of the original (`form`), and the
//! services of the operating system (`platform`: tray, global hot keys,
//! autotype, clipboard, single instance, autostart…).
//!
//! Used by the GTK interface (`pwtool`) and the egui interface.

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
pub mod rt;
pub mod shortcut;
