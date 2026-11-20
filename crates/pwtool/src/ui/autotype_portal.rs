//! Autotype backend for Linux: the RemoteDesktop portal (Wayland does not
//! allow applications to inject input directly; the desktop asks once for
//! permission, which is kept with a restore token). Key presses are sent as
//! X keysyms.

use std::cell::RefCell;
use std::rc::Rc;

use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, Session};
use ashpd::WindowIdentifier;
use gtk4 as gtk;
use gtk::glib;

use super::{INIT_DELAY, KeyStroke};

struct RdSession {
    proxy: RemoteDesktop,
    session: Session<RemoteDesktop>,
}

thread_local! {
    static SESSION: RefCell<Option<Rc<RdSession>>> = const { RefCell::new(None) };
}

/// Get (or start) the RemoteDesktop session. Returns the session and a new
/// restore token to be stored, if any.
async fn session(parent: Option<&gtk::Window>, token: &str) -> Result<(Rc<RdSession>, Option<String>), String> {
    if let Some(s) = SESSION.with(|s| s.borrow().clone()) {
        return Ok((s, None));
    }
    crate::ui::portal::ensure_registered().await;
    let proxy = RemoteDesktop::new().await.map_err(|e| e.to_string())?;
    let session = proxy.create_session(Default::default()).await.map_err(|e| e.to_string())?;
    proxy
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::KEYBOARD)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(if token.is_empty() { None } else { Some(token) }),
        )
        .await
        .map_err(|e| e.to_string())?;
    let ident = match parent {
        Some(w) => WindowIdentifier::from_native(w).await,
        None => None,
    };
    let resp = proxy
        .start(&session, ident.as_ref(), Default::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let new_token = resp.restore_token().map(String::from);
    let s = Rc::new(RdSession { proxy, session });
    SESSION.with(|x| *x.borrow_mut() = Some(s.clone()));
    Ok((s, new_token))
}

/// Close the session (e.g. at program exit).
pub fn close_session() {
    if let Some(s) = SESSION.with(|s| s.borrow_mut().take()) {
        glib::spawn_future_local(async move {
            let _ = s.session.close().await;
        });
    }
}

/// Make sure the session exists (asks for permission the first time).
pub async fn prepare(parent: Option<&gtk::Window>, token: &str) -> Result<Option<String>, String> {
    session(parent, token).await.map(|(_, t)| t)
}

/// Type a sequence. Returns a new restore token if one was issued.
pub async fn send(
    parent: Option<&gtk::Window>,
    token: &str,
    strokes: Vec<KeyStroke>,
    abort: Rc<std::cell::Cell<bool>>,
) -> Result<Option<String>, String> {
    let (s, new_token) = session(parent, token).await?;
    glib::timeout_future(std::time::Duration::from_millis(INIT_DELAY as u64)).await;
    for st in &strokes {
        if abort.get() {
            break;
        }
        let mut failed = None;
        for &k in &st.keysyms {
            if let Err(e) = s.proxy.notify_keyboard_keysym(&s.session, k as i32, KeyState::Pressed, Default::default()).await {
                failed = Some(e.to_string());
                break;
            }
        }
        for &k in st.keysyms.iter().rev() {
            let _ = s.proxy.notify_keyboard_keysym(&s.session, k as i32, KeyState::Released, Default::default()).await;
        }
        if let Some(e) = failed {
            // the session may have been closed by the desktop
            SESSION.with(|x| x.borrow_mut().take());
            return Err(e);
        }
        if st.delay_ms > 0 {
            glib::timeout_future(std::time::Duration::from_millis(st.delay_ms as u64)).await;
        }
    }
    Ok(new_token)
}
