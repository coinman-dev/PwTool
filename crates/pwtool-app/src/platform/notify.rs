//! Desktop notifications (`ShowTrayInfo()` of the original): the balloon
//! of the tray icon on Windows, the Notification portal on Linux.

/// Show a notification. Returns `false` if none could be shown (then the
/// caller shows a message box).
pub async fn notify(text: &str, error: bool) -> bool {
    #[cfg(windows)]
    {
        super::tray::notify(text, error)
    }
    #[cfg(unix)]
    {
        use ashpd::desktop::notification::{Notification, NotificationProxy, Priority};
        super::portal::ensure_registered().await;
        let Ok(proxy) = NotificationProxy::new().await else { return false };
        let n = Notification::new(crate::program::PROGRAM_NAME)
            .body(text)
            .priority(if error { Priority::High } else { Priority::Normal });
        proxy.add_notification("pwtool-info", n).await.is_ok()
    }
}
