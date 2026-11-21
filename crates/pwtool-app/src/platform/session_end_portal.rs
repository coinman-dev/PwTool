//! Saving before the session ends (`OnQueryEndSession()`/`OnEndSession()`
//! of the original): the session monitor of the XDG Inhibit portal reports
//! the "query end" state of the session; the handler runs, then the portal
//! is told that the program is ready.

/// Call `on_end` on the GUI thread when the user session ends (log off,
/// shut down). Call once.
pub fn watch(on_end: impl Fn() + 'static) {
    crate::rt::spawn_local(async move {
        if let Err(e) = monitor(&on_end).await {
            eprintln!("pwtool: session monitor: {e}");
        }
    });
}

async fn monitor(on_end: &dyn Fn()) -> ashpd::Result<()> {
    use ashpd::desktop::inhibit::{InhibitProxy, SessionState};
    use futures_util::StreamExt;

    super::portal::ensure_registered().await;
    let proxy = InhibitProxy::new().await?;
    let mut states = proxy.receive_state_changed().await?;
    let session = proxy.create_monitor(None, Default::default()).await?;
    let mut done = false;
    while let Some(st) = states.next().await {
        match st.session_state() {
            SessionState::QueryEnd => {
                if !done {
                    on_end();
                    done = true;
                }
                proxy.query_end_response(&session).await?;
            }
            // the log out was cancelled
            SessionState::Running => done = false,
            SessionState::Ending => {}
        }
    }
    Ok(())
}
