//! GTK 4 user interface.

pub mod builder;
pub mod clipboard;
pub mod dialogs;
pub mod form;
pub mod icons;
pub mod layout;
pub mod main_window;
pub mod passw_enter;
pub mod quality_bar;
pub mod tray;
pub mod util;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;

use crate::app::{AppCore, CmdLineOptions};

/// Application id (D-Bus name, desktop file).
pub const APP_ID: &str = "io.github.pwtech.PasswordTech";

/// Run the GUI. Returns the exit code.
pub fn run(cmd: CmdLineOptions) -> i32 {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let main: Rc<RefCell<Option<Rc<main_window::MainWindow>>>> = Rc::new(RefCell::new(None));
    let m = main.clone();
    app.connect_activate(move |app| {
        if let Some(w) = m.borrow().as_ref() {
            w.window.present();
            return;
        }
        let core = AppCore::new(cmd.clone());
        let silent = core.cmd.silent;
        let w = main_window::MainWindow::new(app, core);
        if !silent {
            w.window.present();
        } else {
            w.start_hidden();
        }
        let w2 = w.clone();
        glib::idle_add_local_once(move || w2.show_startup_errors());
        if let Ok(script) = std::env::var("PWTECH_AUTOMATE") {
            automate(w.clone(), script);
        }
        if let Ok(path) = std::env::var("PWTECH_SNAPSHOT") {
            let w3 = w.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                util::snapshot_window(w3.window.upcast_ref(), &path);
                if std::env::var("PWTECH_SNAPSHOT_QUIT").is_ok() {
                    w3.window.application().unwrap().quit();
                }
            });
        }
        *m.borrow_mut() = Some(w);
    });
    let code = app.run_with_args::<&str>(&[]);
    code.into()
}

/// Test automation for headless runs (`PWTECH_AUTOMATE`): commands separated
/// by `;` — `click NAME`, `act NAME`, `text NAME VALUE`, `wait MS`,
/// `snap PATH` (main window), `snapall DIR` (all windows), `print NAME`
/// (text of a label/entry to stdout), `quit`.
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
                            } else if let Some(cb) = x.downcast_ref::<gtk::ComboBoxText>() {
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
