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
<method name="Report"><arg type="s" direction="in"/></method>
<method name="Minimized"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
</interface></node>"#;

type MinimizedFn = Box<dyn Fn(&str, bool)>;

thread_local! {
    static REG: RefCell<Option<(gio::RegistrationId, async_channel::Receiver<String>)>> = const { RefCell::new(None) };
    static ON_MINIMIZED: RefCell<Option<MinimizedFn>> = const { RefCell::new(None) };
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
        .method_call(move |_, _, _, _, method, params, inv| {
            match method {
                "Report" => {
                    if let Some((caption,)) = params.get::<(String,)>() {
                        let _ = tx.try_send(caption);
                    }
                }
                "Minimized" => {
                    if let Some((caption, min)) = params.get::<(String, bool)>() {
                        ON_MINIMIZED.with(|f| {
                            if let Some(f) = f.borrow().as_ref() {
                                f(&caption, min);
                            }
                        });
                    }
                }
                _ => {}
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
    run_kwin_script_named(app, body, wait_report, PLUGIN, true).await
}

async fn run_kwin_script_named(
    app: &gtk::Application,
    body: &str,
    wait_report: bool,
    plugin: &str,
    unload: bool,
) -> Option<String> {
    let conn = app.dbus_connection()?;
    let rx = if wait_report {
        let rx = receiver(&conn)?;
        while rx.try_recv().is_ok() {}
        Some(rx)
    } else {
        None
    };
    let dir = glib::user_runtime_dir();
    let file = dir.join(format!("{plugin}-{}.js", std::process::id()));
    std::fs::write(&file, body).ok()?;
    // unload a script left over from an earlier call
    let _ = call(&conn, "/Scripting", "org.kde.kwin.Scripting", "unloadScript", Some(&(plugin,).to_variant())).await;
    let id = call(
        &conn,
        "/Scripting",
        "org.kde.kwin.Scripting",
        "loadScript",
        Some(&(file.to_string_lossy().to_string(), plugin).to_variant()),
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
    if unload {
        let _ = call(&conn, "/Scripting", "org.kde.kwin.Scripting", "unloadScript", Some(&(plugin,).to_variant())).await;
    }
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

fn watch_plugin() -> String {
    format!("pwtech-watch-{}", std::process::id())
}

/// Report minimizing/restoring of the windows of this process (KWin script
/// staying loaded while the program runs): `f(caption, minimized)`.
/// Returns `false` without KWin.
pub async fn watch_minimized(app: &gtk::Application, f: impl Fn(&str, bool) + 'static) -> bool {
    let Some(conn) = app.dbus_connection() else { return false };
    let Some(name) = app.application_id() else { return false };
    if receiver(&conn).is_none() {
        return false;
    }
    ON_MINIMIZED.with(|x| *x.borrow_mut() = Some(Box::new(f)));
    let pid = std::process::id();
    let script = format!(
        "function pwHook(w) {{\n\
           if (!w || w.pid != {pid}) return;\n\
           var report = function() {{ callDBus(\"{name}\", \"{OBJ_PATH}\", \"{IFACE}\", \"Minimized\", String(w.caption), w.minimized); }};\n\
           if (w.minimizedChanged) w.minimizedChanged.connect(report);\n\
         }}\n\
         var list = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (var i = 0; i < list.length; i++) pwHook(list[i]);\n\
         if (workspace.windowAdded) workspace.windowAdded.connect(pwHook); else workspace.clientAdded.connect(pwHook);\n\
         if (!workspace.windowList && workspace.clientMinimized) {{\n\
           workspace.clientMinimized.connect(function(w) {{ if (w.pid == {pid}) callDBus(\"{name}\", \"{OBJ_PATH}\", \"{IFACE}\", \"Minimized\", String(w.caption), true); }});\n\
           workspace.clientUnminimized.connect(function(w) {{ if (w.pid == {pid}) callDBus(\"{name}\", \"{OBJ_PATH}\", \"{IFACE}\", \"Minimized\", String(w.caption), false); }});\n\
         }}\n"
    );
    run_kwin_script_named(app, &script, false, &watch_plugin(), false).await.is_some()
}

/// Unload the watcher script (program exit).
pub fn stop_watching(app: &gtk::Application) {
    if let Some(conn) = app.dbus_connection() {
        let plugin = watch_plugin();
        let _ = conn.call_sync(
            Some("org.kde.KWin"),
            "/Scripting",
            "org.kde.kwin.Scripting",
            "unloadScript",
            Some(&(plugin,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        );
    }
}

/// `TrayMenu_ResetWindowPosClick()`: move all windows of this process to
/// the top left corner of the work area.
pub async fn reset_window_positions(app: &gtk::Application) -> bool {
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
    run_kwin_script(app, &script, false).await.is_some()
}
