//! Global hot keys for the GTK interface (`pwtool_app::platform::hotkeys`:
//! GlobalShortcuts portal on Linux, `RegisterHotKey` on Windows).

use std::rc::Rc;

use gtk4 as gtk;
pub use pwtool_app::platform::hotkeys::deactivate;

/// Bind hot keys: `keys` = (shortcut code, description). `on_activated` is
/// called with the shortcut code. Returns the number of bound shortcuts.
pub async fn activate(
    parent: Option<&gtk::Window>,
    keys: Vec<(u16, String)>,
    on_activated: Rc<dyn Fn(u16)>,
) -> Result<usize, String> {
    let p = super::platform_parent(parent).await;
    pwtool_app::platform::hotkeys::activate(p.as_ref(), keys, on_activated).await
}
