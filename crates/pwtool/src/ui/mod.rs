//! GTK 4 user interface.

pub mod active_window;
pub mod autotype;
pub mod builder;
pub mod clipboard;
pub mod dialogs;
pub mod form;
pub mod hotkeys;
pub mod icons;
pub mod layout;
pub mod main_window;
pub mod passw_enter;
pub mod passw_manager;
pub mod portal;
pub mod quality_bar;
pub mod tray;
pub mod update_check;
pub mod util;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;

use crate::app::{AppCore, CmdLineOptions};

/// Application id (D-Bus name, desktop file).
pub const APP_ID: &str = "io.github.coinman_dev.PwTool";

/// Run the GUI. Returns the exit code.
pub fn run(cmd: CmdLineOptions) -> i32 {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        // one instance (the original refuses a second start); a second
        // start shows the window of the running instance
        .flags(gtk::gio::ApplicationFlags::empty())
        .register_session(true)
        .build();
    let main: Rc<RefCell<Option<Rc<main_window::MainWindow>>>> = Rc::new(RefCell::new(None));
    let m = main.clone();
    app.connect_activate(move |app| {
        if let Some(w) = m.borrow().as_ref() {
            w.restore_from_tray();
            return;
        }
        let core = AppCore::new(cmd.clone());
        // window icon (X11; on Wayland the compositor takes it from the
        // desktop file): the installed hicolor icon, or the bundled PNGs
        // when running from the build tree
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            if !theme.has_icon(APP_ID) {
                let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/dev-icons");
                for s in [16, 32, 48, 64, 128, 256] {
                    let dir = dev.join(format!("hicolor/{s}x{s}/apps"));
                    let src = core.paths.resources.join(format!("appicons/pwtool_default_{s}.png"));
                    if src.is_file() && std::fs::create_dir_all(&dir).is_ok() {
                        let _ = std::fs::copy(&src, dir.join(format!("{APP_ID}.png")));
                    }
                }
                theme.add_search_path(&dev);
            }
        }
        gtk::Window::set_default_icon_name(APP_ID);
        let silent = core.cmd.silent;
        let w = main_window::MainWindow::new(app, core);
        let w_mng = w.clone();
        glib::idle_add_local_once(move || w_mng.startup_open_manager());
        if !silent {
            w.window.present();
            let w2 = w.clone();
            glib::idle_add_local_once(move || w2.show_startup_errors());
        } else {
            w.start_hidden();
        }
        if let Ok(script) = std::env::var("PWTOOL_AUTOMATE") {
            automate(w.clone(), script);
        }
        if let Ok(path) = std::env::var("PWTOOL_SNAPSHOT") {
            let w3 = w.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                util::snapshot_window(w3.window.upcast_ref(), &path);
                if std::env::var("PWTOOL_SNAPSHOT_QUIT").is_ok() {
                    w3.window.application().unwrap().quit();
                }
            });
        }
        // session end (`OnQueryEndSession()` / `OnEndSession()`)
        let w4 = Rc::downgrade(&w);
        app.connect_query_end(move |_| {
            if let Some(w) = w4.upgrade() {
                w.on_end_session();
            }
        });
        *m.borrow_mut() = Some(w);
    });
    let code = app.run_with_args::<&str>(&[]);
    code.into()
}

/// Test automation for headless runs (`PWTOOL_AUTOMATE`): commands separated
/// by `;` — `click NAME`, `act NAME`, `text NAME VALUE`, `wait MS`,
/// `snap PATH` (main window), `snapall DIR` (all windows), `print NAME`
/// (text of a label/entry to stdout), `sizes` (sizes of the visible
/// windows), `quit`.
fn automate(w: Rc<main_window::MainWindow>, script: String) {
    let cmds: Vec<String> = script.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    glib::spawn_future_local(async move {
        glib::timeout_future(std::time::Duration::from_millis(500)).await;
        for c in cmds {
            let (cmd, rest) = c.split_once(' ').unwrap_or((c.as_str(), ""));
            match cmd {
                "click" => {
                    if let Some(b) = find_widget(&w, rest).and_then(|x| x.downcast::<gtk::Button>().ok()) {
                        b.emit_clicked();
                    } else {
                        eprintln!("automate: no button {rest}");
                    }
                }
                "toggle" => {
                    if let Some(b) = find_widget(&w, rest).and_then(|x| x.downcast::<gtk::CheckButton>().ok()) {
                        b.set_active(!b.is_active());
                    }
                }
                "act" => {
                    gtk::prelude::ActionGroupExt::activate_action(&w.b.actions, rest, None);
                }
                "text" => {
                    let (name, val) = rest.split_once(' ').unwrap_or((rest, ""));
                    match find_widget(&w, name) {
                        Some(x) => {
                            if let Some(e) = x.downcast_ref::<gtk::Entry>() {
                                e.set_text(val);
                            } else if let Some(cb) = {
                                #[allow(deprecated)]
                                x.downcast_ref::<gtk::ComboBoxText>()
                            } {
                                util::set_combo_text(cb, val);
                            } else if let Some(tv) = x.downcast_ref::<gtk::TextView>() {
                                tv.buffer().set_text(val);
                            }
                        }
                        None => eprintln!("automate: no widget {name}"),
                    }
                }
                "page" => {
                    let (name, n) = rest.split_once(' ').unwrap_or((rest, "0"));
                    if let Some(nb) = find_widget(&w, name).and_then(|x| x.downcast::<gtk::Notebook>().ok()) {
                        nb.set_current_page(n.parse().ok());
                    }
                }
                "minimize" => w.window.minimize(),
                "theme" => {
                    if let Some(st) = gtk::Settings::default() {
                        let scheme = st
                            .find_property("gtk-interface-color-scheme")
                            .map(|_| format!("{:?}", st.property_value("gtk-interface-color-scheme")))
                            .unwrap_or_default();
                        println!("prefer_dark={} theme={:?} scheme={scheme}", st.is_gtk_application_prefer_dark_theme(), st.gtk_theme_name());
                    }
                }
                "state" => {
                    if let Some(t) = w.window.surface().and_then(|s| s.downcast::<gtk::gdk::Toplevel>().ok()) {
                        println!("state={:?} visible={}", t.state(), w.window.is_visible());
                    }
                }
                "activetitle" => {
                    if let Some(app) = w.window.application() {
                        println!("activetitle={:?}", active_window::active_window_title(&app).await);
                    }
                }
                "clip" => {
                    clipboard::set_plain(rest);
                }
                "printclip" => {
                    let (tx, rx) = async_channel::bounded(1);
                    clipboard::read_text(move |t| {
                        let _ = tx.try_send(t.map(|t| t.to_string()).unwrap_or_default());
                    });
                    if let Ok(t) = rx.recv().await {
                        println!("clipboard={t}");
                    }
                }
                "wait" => {
                    let ms = rest.parse().unwrap_or(500);
                    glib::timeout_future(std::time::Duration::from_millis(ms)).await;
                }
                "snap" => {
                    util::snapshot_window(w.window.upcast_ref(), rest);
                }
                "snapall" => {
                    for (i, win) in gtk::Window::list_toplevels().iter().enumerate() {
                        if let Ok(win) = win.clone().downcast::<gtk::Window>() {
                            if win.is_visible() {
                                let title = win.title().map(|t| t.to_string()).unwrap_or_default();
                                let name: String = title.chars().filter(|c| c.is_alphanumeric()).collect();
                                util::snapshot_window(&win, &format!("{rest}/{i}-{name}.png"));
                            }
                        }
                    }
                }
                "print" => match find_widget(&w, rest) {
                    Some(x) => {
                        if let Some(l) = x.downcast_ref::<gtk::Label>() {
                            println!("{rest}={}", l.text());
                        } else if let Some(e) = x.downcast_ref::<gtk::Entry>() {
                            println!("{rest}={}", e.text());
                        } else if let Some(tv) = x.downcast_ref::<gtk::TextView>() {
                            let b = tv.buffer();
                            println!("{rest}={}", b.text(&b.start_iter(), &b.end_iter(), false));
                        }
                    }
                    None => eprintln!("automate: no widget {rest}"),
                },
                "sizes" => {
                    println!("scale={}", w.b.scale);
                    for win in gtk::Window::list_toplevels() {
                        if let Ok(win) = win.downcast::<gtk::Window>() {
                            if win.is_visible() {
                                println!(
                                    "size: {} {}x{} resizable={}",
                                    win.title().unwrap_or_default(),
                                    win.width(),
                                    win.height(),
                                    win.is_resizable()
                                );
                            }
                        }
                    }
                }
                "windows" => {
                    for win in gtk::Window::list_toplevels() {
                        if let Ok(win) = win.downcast::<gtk::Window>() {
                            if win.is_visible() {
                                println!("window: {} (active: {})", win.title().unwrap_or_default(), win.is_active());
                            }
                        }
                    }
                }
                "quit" => {
                    if let Some(app) = w.window.application() {
                        app.quit();
                    }
                }
                c if c.starts_with("pm") && passw_manager::automation::automate(&w, c, rest) => {}
                _ => eprintln!("automate: unknown command {cmd}"),
            }
            glib::timeout_future(std::time::Duration::from_millis(150)).await;
        }
    });
}

/// Find a widget by name (`TComponent.Name`): first in the active window,
/// then in the other visible windows, finally in the main window's form.
fn find_widget(w: &main_window::MainWindow, name: &str) -> Option<gtk::Widget> {
    fn search(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
        if root.widget_name() == name {
            return Some(root.clone());
        }
        let mut c = root.first_child();
        while let Some(ch) = c {
            if let Some(f) = search(&ch, name) {
                return Some(f);
            }
            c = ch.next_sibling();
        }
        None
    }
    let mut wins: Vec<gtk::Window> = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|x| x.downcast::<gtk::Window>().ok())
        .filter(|x| x.is_visible())
        .collect();
    wins.sort_by_key(|x| !x.is_active());
    for win in wins {
        if let Some(f) = search(win.upcast_ref(), name) {
            return Some(f);
        }
    }
    w.b.widgets.get(name).cloned()
}
