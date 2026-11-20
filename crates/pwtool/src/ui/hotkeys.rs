//! Global hot keys (`ActivateHotKeys()` / `RegisterHotKey()` of the
//! original) through the GlobalShortcuts portal. The desktop shows a dialog
//! to confirm the shortcuts when they are bound.

use std::cell::RefCell;
use std::rc::Rc;

use ashpd::WindowIdentifier;
use ashpd::desktop::Session;
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use gtk4 as gtk;
use gtk::glib;

struct Bound {
    session: Session<GlobalShortcuts>,
}

thread_local! {
    static BOUND: RefCell<Option<Bound>> = const { RefCell::new(None) };
    static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Shortcut trigger in the format of the XDG shortcuts specification
/// ("CTRL+ALT+p") for a VCL `TShortCut`.
pub fn trigger_for_shortcut(code: u16) -> Option<String> {
    let accel = super::builder::shortcut_to_accel(code as i64)?;
    let mut s = String::new();
    if code & 0x4000 != 0 {
        s.push_str("CTRL+");
    }
    if code & 0x8000 != 0 {
        s.push_str("ALT+");
    }
    if code & 0x2000 != 0 {
        s.push_str("SHIFT+");
    }
    let key = accel.rsplit('>').next().unwrap_or("").to_string();
    s.push_str(&key);
    Some(s)
}

/// Unbind all hot keys (`DeactivateHotKeys()`).
pub fn deactivate() {
    GENERATION.with(|g| g.set(g.get() + 1));
    if let Some(b) = BOUND.with(|b| b.borrow_mut().take()) {
        glib::spawn_future_local(async move {
            let _ = b.session.close().await;
        });
    }
}

/// Bind hot keys: `keys` = (shortcut code, description). `on_activated` is
/// called with the shortcut code. Returns the number of bound shortcuts.
pub async fn activate(
    parent: Option<&gtk::Window>,
    keys: Vec<(u16, String)>,
    on_activated: Rc<dyn Fn(u16)>,
) -> Result<usize, String> {
    deactivate();
    if keys.is_empty() {
        return Ok(0);
    }
    let generation = GENERATION.with(|g| g.get());
    super::portal::ensure_registered().await;
    let proxy = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
    let session = proxy.create_session(Default::default()).await.map_err(|e| e.to_string())?;
    let shortcuts: Vec<NewShortcut> = keys
        .iter()
        .map(|(code, desc)| {
            let t = trigger_for_shortcut(*code);
            NewShortcut::new(format!("hotkey-{code}"), desc.clone()).preferred_trigger(t.as_deref())
        })
        .collect();
    let ident = match parent {
        Some(w) => WindowIdentifier::from_native(w).await,
        None => None,
    };
    let bound = proxy
        .bind_shortcuts(&session, &shortcuts, ident.as_ref(), Default::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let n = bound.shortcuts().len();
    let mut stream = proxy.receive_activated().await.map_err(|e| e.to_string())?;
    if GENERATION.with(|g| g.get()) != generation {
        // superseded while waiting for the portal
        let _ = session.close().await;
        return Ok(0);
    }
    BOUND.with(|b| *b.borrow_mut() = Some(Bound { session }));
    glib::spawn_future_local(async move {
        let _proxy = proxy;
        while let Some(a) = stream.next().await {
            if GENERATION.with(|g| g.get()) != generation {
                break;
            }
            if let Some(code) = a.shortcut_id().strip_prefix("hotkey-").and_then(|c| c.parse::<u16>().ok()) {
                on_activated(code);
            }
        }
    });
    Ok(n)
}
