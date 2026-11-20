//! Window functions on Windows for the GTK interface (same interface as the
//! KWin version in `active_window.rs`); see
//! `pwtool_app::platform::active_window`.

use gtk4 as gtk;
use pwtool_app::platform::active_window as aw;

pub async fn active_window_title(_app: &gtk::Application) -> Option<String> {
    aw::active_window_title().await
}

pub async fn set_keep_above(_app: &gtk::Application, above: bool) -> bool {
    aw::set_keep_above(above).await
}

pub async fn watch_minimized(_app: &gtk::Application, f: impl Fn(&str, bool) + 'static) -> bool {
    aw::watch_minimized(f).await
}

pub fn stop_watching(_app: &gtk::Application) {
    aw::stop_watching()
}

pub async fn reset_window_positions(_app: &gtk::Application) -> bool {
    aw::reset_window_positions().await
}
