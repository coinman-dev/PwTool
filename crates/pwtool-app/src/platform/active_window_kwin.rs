//! Window functions on Linux through KWin (KDE Plasma): title of the active
//! window of another program (`GetForegroundWindow()` + `GetWindowText()` in
//! the original), "Always on Top" and resetting the window positions.
//! Wayland gives none of this to applications; a short KWin script loaded
//! over D-Bus does it and reports results back with `callDBus` to this
//! process. Without KWin the functions return `None`/`false`.

use std::cell::RefCell;
use std::time::Duration;

use ashpd::zbus;
use futures_util::StreamExt;

const OBJ_PATH: &str = "/io/github/coinman_dev/PwTool/ActiveWindow";
const IFACE: &str = "io.github.coinman_dev.PwTool.ActiveWindow";
const PLUGIN: &str = "pwtool-active-window";

thread_local! {
    /// Session bus connection and the captions reported by the script.
    static BUS: RefCell<Option<(zbus::Connection, async_channel::Receiver<String>)>> = const { RefCell::new(None) };
}

/// The session bus connection; the first call starts serving the `Report`
/// calls of the scripts.
async fn bus() -> Option<(zbus::Connection, async_channel::Receiver<String>)> {
    if let Some(b) = BUS.with(|b| b.borrow().clone()) {
        return Some(b);
    }
    let conn = zbus::Connection::session().await.ok()?;
    let (tx, rx) = async_channel::unbounded::<String>();
    let mut stream = zbus::MessageStream::from(&conn);
    let c = conn.clone();
    crate::rt::spawn_local(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let h = msg.header();
            if h.message_type() != zbus::message::Type::MethodCall
                || h.path().is_none_or(|p| p.as_str() != OBJ_PATH)
                || h.interface().is_none_or(|i| i.as_str() != IFACE)
            {
                continue;
            }
            if h.member().is_some_and(|m| m.as_str() == "Report") {
                if let Ok(caption) = msg.body().deserialize::<String>() {
                    let _ = tx.try_send(caption);
                }
            }
            let _ = c.reply(&h, &()).await;
        }
    });
    BUS.with(|b| *b.borrow_mut() = Some((conn.clone(), rx.clone())));
    Some((conn, rx))
}

/// `fut`, or `None` after `ms` milliseconds.
async fn with_timeout<T>(ms: u64, fut: impl std::future::Future<Output = Option<T>>) -> Option<T> {
    let timeout = crate::rt::sleep(Duration::from_millis(ms));
    futures_util::pin_mut!(fut, timeout);
    match futures_util::future::select(fut, timeout).await {
        futures_util::future::Either::Left((r, _)) => r,
        futures_util::future::Either::Right(_) => None,
    }
}

async fn call<B>(conn: &zbus::Connection, path: &str, iface: &str, method: &str, body: &B) -> Option<zbus::Message>
where
    B: serde::ser::Serialize + zbus::zvariant::DynamicType,
{
    with_timeout(3000, async { conn.call_method(Some("org.kde.KWin"), path, Some(iface), method, body).await.ok() }).await
}

/// Load and run a KWin script (`body` is JavaScript). `wait_report`: wait up
/// to 2 s for its `Report` call and return the argument; else `Some("")`
/// once KWin has run it. `None` without KWin.
async fn run_kwin_script(body: &str, wait_report: bool) -> Option<String> {
    let (conn, rx) = bus().await?;
    while rx.try_recv().is_ok() {}
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let file = dir.join(format!("{PLUGIN}-{}.js", std::process::id()));
    std::fs::write(&file, body).ok()?;
    // a script left over from an earlier call
    let _ = call(&conn, "/Scripting", "org.kde.kwin.Scripting", "unloadScript", &(PLUGIN,)).await;
    let id = call(&conn, "/Scripting", "org.kde.kwin.Scripting", "loadScript", &(file.to_string_lossy().to_string(), PLUGIN))
        .await
        .and_then(|m| m.body().deserialize::<i32>().ok());
    let result = match id {
        Some(id) if id >= 0 => {
            // the object path of a script differs between Plasma versions
            let ran = call(&conn, &format!("/Scripting/Script{id}"), "org.kde.kwin.Script", "run", &()).await.is_some()
                || call(&conn, &format!("/{id}"), "org.kde.kwin.Script", "run", &()).await.is_some();
            match (ran, wait_report) {
                (false, _) => None,
                (true, false) => Some(String::new()),
                (true, true) => with_timeout(2000, async { rx.recv().await.ok() }).await,
            }
        }
        _ => None,
    };
    let _ = call(&conn, "/Scripting", "org.kde.kwin.Scripting", "unloadScript", &(PLUGIN,)).await;
    let _ = std::fs::remove_file(&file);
    result
}

/// Unique name of this process on the session bus (target of `callDBus`).
async fn own_name() -> Option<String> {
    let (conn, _) = bus().await?;
    conn.unique_name().map(|n| n.to_string())
}

/// Caption of the active window, `None` if it cannot be determined (no
/// KWin or no session bus).
pub async fn active_window_title() -> Option<String> {
    let name = own_name().await?;
    let script = format!(
        "var w = workspace.activeWindow || workspace.activeClient;\n\
         callDBus(\"{name}\", \"{OBJ_PATH}\", \"{IFACE}\", \"Report\", w ? String(w.caption) : \"\");\n"
    );
    run_kwin_script(&script, true).await
}

/// "Always on Top" for all windows of this process (KWin `keepAbove`).
/// `false` without KWin.
pub async fn set_keep_above(above: bool) -> bool {
    let pid = std::process::id();
    let script = format!(
        "var list = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (var i = 0; i < list.length; i++) {{ if (list[i].pid == {pid}) list[i].keepAbove = {above}; }}\n"
    );
    run_kwin_script(&script, false).await.is_some()
}

/// `TrayMenu_ResetWindowPosClick()`: move all windows of this process to
/// the top left corner of the work area. `false` without KWin.
pub async fn reset_window_positions() -> bool {
    let pid = std::process::id();
    let script = format!(
        "var list = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (var i = 0; i < list.length; i++) {{\n\
           var w = list[i]; if (w.pid != {pid}) continue;\n\
           var a = workspace.clientArea(KWin.MaximizeArea, w);\n\
           var g = w.frameGeometry;\n\
           w.frameGeometry = {{x: a.x, y: a.y, width: g.width, height: g.height}};\n\
         }}\n"
    );
    run_kwin_script(&script, false).await.is_some()
}
