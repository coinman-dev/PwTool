//! Autotype for the GTK interface: key sequences and the backends
//! (RemoteDesktop portal on Linux, `SendInput` on Windows) come from
//! `pwtool_app::autotype`; here GTK windows become portal parents.

use std::rc::Rc;

use gtk4 as gtk;
pub use pwtool_app::autotype::*;

/// Make sure the session exists (asks for permission the first time).
pub async fn prepare(parent: Option<&gtk::Window>, token: &str) -> Result<Option<String>, String> {
    let p = super::platform_parent(parent).await;
    pwtool_app::autotype::prepare(p.as_ref(), token).await
}

/// Type a sequence. Returns a new restore token if one was issued.
pub async fn send(
    parent: Option<&gtk::Window>,
    token: &str,
    strokes: Vec<KeyStroke>,
    abort: Rc<std::cell::Cell<bool>>,
) -> Result<Option<String>, String> {
    let p = super::platform_parent(parent).await;
    pwtool_app::autotype::send(p.as_ref(), token, strokes, abort).await
}
