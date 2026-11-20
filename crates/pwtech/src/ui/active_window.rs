//! Title of the active window of another application (`GetForegroundWindow()`
//! + `GetWindowText()` in the original). Wayland does not give this to
//! applications; on KDE Plasma a small KWin script reports the caption back
//! over D-Bus.

use std::cell::RefCell;

use gtk4 as gtk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

const OBJ_PATH: &str = "/io/github/pwtech/PasswordTech/ActiveWindow";
const IFACE: &str = "io.github.pwtech.ActiveWindow";
const PLUGIN: &str = "pwtech-active-window";
const XML: &str = r#"<node><interface name="io.github.pwtech.ActiveWindow">
<method name="Report"><arg type="s" direction="in"/></method></interface></node>"#;

thread_local! {
    static REG: RefCell<Option<(gio::RegistrationId, async_channel::Receiver<String>)>> = const { RefCell::new(None) };
}

fn receiver(conn: &gio::DBusConnection) -> Option<async_channel::Receiver<String>> {
    if let Some(rx) = REG.with(|r| r.borrow().as_ref().map(|(_, rx)| rx.clone())) {
        return Some(rx);
    }
    let node = gio::DBusNodeInfo::for_xml(XML).ok()?;
    let iface = node.lookup_interface(IFACE)?;
    let (tx, rx) = async_channel::unbounded::<String>();
    let id = conn
        .register_object(OBJ_PATH, &iface)
        .method_call(move |_, _, _, _, _, params, inv| {
            if let Some((caption,)) = params.get::<(String,)>() {
                let _ = tx.try_send(caption);
            }
            inv.return_value(None);
        })
        .build()
        .ok()?;
    REG.with(|r| *r.borrow_mut() = Some((id, rx.clone())));
    Some(rx)
}

async fn call(conn: &gio::DBusConnection, path: &str, iface: &str, method: &str, args: Option<&glib::Variant>) -> Option<glib::Variant> {
    conn.call_future(Some("org.kde.KWin"), path, iface, method, args, None, gio::DBusCallFlags::NONE, 3000)
        .await
        .ok()
}

/// Load and run a KWin script (`body` is JavaScript); returns whether KWin
/// accepted it. `wait_report` waits up to 2 s for a `Report` call of the
/// script and returns its argument.
async fn run_kwin_script(app: &gtk::Application, body: &str, wait_report: bool) -> Option<String> {
    let conn = app.dbus_connection()?;
    let rx = if wait_report {
        let rx = receiver(&conn)?;
        while rx.try_recv().is_ok() {}
        Some(rx)
    } else {
        None
    };
    let dir = glib::user_runtime_dir();
    let file = dir.join(format!("pwtech-kwin-{}.js", std::process::id()));
    std::fs::write(&file, body).ok()?;
    // unload a script left over from an earlier call
    let _ = call(&conn, "/Scripting", "org.kde.kwin.Scripting", "unloadScript", Some(&(PLUGIN,).to_variant())).await;
    let id = call(
        &conn,
        "/Scripting",
        "org.kde.kwin.Scripting",
        "loadScript",
        Some(&(file.to_string_lossy().to_string(), PLUGIN).to_variant()),
    )
    .await
    .and_then(|v| v.get::<(i32,)>())
    .map(|(i,)| i);
    let result = match id {
        Some(id) if id >= 0 => {
            let ran = call(&conn, &format!("/Scripting/Script{id}"), "org.kde.kwin.Script", "run", None).await.is_some()
                || call(&conn, &format!("/{id}"), "org.kde.kwin.Script", "run", None).await.is_some();
            match (ran, rx) {
                (false, _) => None,
                (true, None) => Some(String::new()),
                (true, Some(rx)) => {
                    let timeout = glib::timeout_future(std::time::Duration::from_millis(2000));
                    futures_util::pin_mut!(timeout);
                    let recv = rx.recv();
                    futures_util::pin_mut!(recv);
                    match futures_util::future::select(recv, timeout).await {
                        futures_util::future::Either::Left((Ok(c), _)) => Some(c),
                        _ => None,
                    }
                }
            }
        }
        _ => None,
    };
    let _ = call(&conn, "/Scripting", "org.kde.kwin.Scripting", "unloadScript", Some(&(PLUGIN,).to_variant())).await;
    let _ = std::fs::remove_file(&file);
    result
}

/// Caption of the active window, `None` if it cannot be determined (no
/// KWin, or the session bus is not available).
pub async fn active_window_title(app: &gtk::Application) -> Option<String> {
    let name = app.application_id()?;
    let script = format!(
        "var w = workspace.activeWindow || workspace.activeClient;\n\
         callDBus(\"{name}\", \"{OBJ_PATH}\", \"{IFACE}\", \"Report\", w ? String(w.caption) : \"\");\n"
    );
    run_kwin_script(app, &script, true).await
}

/// "Always on Top" for all windows of this process (KWin `keepAbove`).
/// Returns `false` if KWin is not available.
pub async fn set_keep_above(app: &gtk::Application, above: bool) -> bool {
    let pid = std::process::id();
    let script = format!(
        "var list = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (var i = 0; i < list.length; i++) {{ if (list[i].pid == {pid}) list[i].keepAbove = {above}; }}\n"
    );
    run_kwin_script(app, &script, false).await.is_some()
}
