//! Test automation (`PWTOOL_AUTOMATE`): commands separated by `;` —
//! `click NAME`, `act NAME` (menu item), `text NAME VALUE`, `wait MS`,
//! `print NAME` (text of a control to stdout), `snap PATH` (picture of the
//! main window), `quit`. The same scripts work with the GTK version.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use egui::{UserData, ViewportCommand};

use super::main_window::MainWindow;
use crate::vcl::Event;

thread_local! {
    static SNAP_DONE: RefCell<Option<async_channel::Sender<()>>> = const { RefCell::new(None) };
}

pub fn start(w: Rc<MainWindow>, script: String) {
    crate::runtime::spawn(async move {
        pwtool_app::rt::sleep(Duration::from_millis(500)).await;
        for c in script.split(';').map(str::trim).filter(|c| !c.is_empty()) {
            let (cmd, rest) = c.split_once(' ').unwrap_or((c, ""));
            let f = &w.form;
            match cmd {
                "click" | "toggle" => {
                    if cmd == "toggle" {
                        f.set_checked(rest, !f.checked(rest));
                    }
                    f.push_event(Event::Click(rest.to_string()));
                }
                "act" => f.push_event(Event::Menu(rest.to_string())),
                "text" => {
                    let (name, val) = rest.split_once(' ').unwrap_or((rest, ""));
                    f.set_text(name, val);
                    f.push_event(Event::Change(name.to_string()));
                }
                "wait" => pwtool_app::rt::sleep(Duration::from_millis(rest.parse().unwrap_or(100))).await,
                "print" => println!("{rest}={}", f.text(rest)),
                "minimize" => crate::windows::ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
                "state" => {
                    let ctx = crate::windows::ctx();
                    let (min, vis, foc) = ctx.input(|i| (i.viewport().minimized, i.viewport().inner_rect.is_some(), i.viewport().focused));
                    println!("hidden={} hotkeys={} minimized={min:?} rect={vis} focused={foc:?}", w.is_hidden(), w.hot_key_count());
                }
                "snap" => {
                    let (tx, rx) = async_channel::bounded(1);
                    SNAP_DONE.with(|s| *s.borrow_mut() = Some(tx));
                    crate::windows::ctx().send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(rest.to_string())));
                    let _ = rx.recv().await;
                }
                "quit" => {
                    crate::windows::ctx().send_viewport_cmd(ViewportCommand::Close);
                    return;
                }
                _ => eprintln!("automate: unknown command {cmd}"),
            }
            pwtool_app::rt::sleep(Duration::from_millis(50)).await;
        }
    });
}

/// Save screenshots requested by `snap` (called every frame).
pub fn save_screenshots(ctx: &egui::Context) {
    let shots: Vec<(String, std::sync::Arc<egui::ColorImage>)> = ctx.input(|i| {
        i.raw
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Screenshot { user_data, image, .. } => {
                    user_data.data.as_ref().and_then(|d| d.downcast_ref::<String>()).map(|p| (p.clone(), image.clone()))
                }
                _ => None,
            })
            .collect()
    });
    for (path, img) in shots {
        if let Err(e) = crate::vcl::snapshot::save_png(&img, std::path::Path::new(&path)) {
            eprintln!("automate: {path}: {e}");
        }
        SNAP_DONE.with(|s| {
            if let Some(tx) = s.borrow_mut().take() {
                let _ = tx.try_send(());
            }
        });
    }
}
