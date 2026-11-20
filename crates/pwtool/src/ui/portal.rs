//! Common helpers for XDG desktop portals.

use std::cell::Cell;

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// Register the program with its application id at the portal host registry
/// (needed by non-sandboxed programs before the first portal request, so
/// that permissions and shortcuts are stored for this application).
pub async fn ensure_registered() {
    if REGISTERED.with(|r| r.replace(true)) {
        return;
    }
    if let Ok(id) = super::APP_ID.parse::<ashpd::AppID>() {
        if let Err(e) = ashpd::register_host_app(id).await {
            // older portals do not have the registry; not fatal
            eprintln!("pwtool: portal registration: {e}");
        }
    }
}
