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
                    let _ = futures_util::future::select(Box::pin(rx.recv()), Box::pin(pwtool_app::rt::sleep(Duration::from_secs(5)))).await;
                }
                // picture of the form of another window, drawn in software
                // (eframe takes screenshots of the main window only)
                "snapw" => {
                    let (id, path) = rest.split_once(' ').unwrap_or((rest, ""));
                    match super::dialogs::form_of_window(id) {
                        Some(form) => {
                            let size = crate::windows::last_size(egui::ViewportId::from_hash_of(id))
                                .unwrap_or_else(|| crate::vcl::window_size(&form, &crate::windows::ctx()));
                            if let Err(e) = snap_form(&form, size, std::path::Path::new(path)) {
                                eprintln!("automate: {path}: {e}");
                            }
                        }
                        None => eprintln!("automate: no window {id}"),
                    }
                }
                // commands for the form of another window: `fclick WIN NAME`,
                // `ftoggle WIN NAME`, `ftext WIN NAME VALUE`, `fprint WIN NAME`,
                // `fact WIN MENU`, `fsel WIN NAME INDEX`
                "fclick" | "ftoggle" | "ftext" | "fprint" | "fact" | "fsel" => {
                    let mut it = rest.splitn(3, ' ');
                    let (win, name, val) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
                    let Some(f) = super::dialogs::form_of_window(win) else {
                        eprintln!("automate: no window {win}");
                        continue;
                    };
                    match cmd {
                        "fclick" => f.push_event(Event::Click(name.to_string())),
                        "ftoggle" => {
                            f.set_checked(name, !f.checked(name));
                            f.push_event(Event::Click(name.to_string()));
                        }
                        "ftext" => {
                            f.set_text(name, val);
                            f.push_event(Event::Change(name.to_string()));
                        }
                        "fsel" => {
                            f.set_item_index(name, val.parse().ok());
                            f.push_event(Event::Change(name.to_string()));
                        }
                        "fact" => f.push_event(Event::Menu(name.to_string())),
                        _ => println!("{win}.{name}={}", f.text(name)),
                    }
                    crate::windows::ctx().request_repaint();
                }
                "windows" => println!("open={}", crate::windows::open_ids().join(",")),
                // `msgauto yes|no|ok|cancel`: answer of the next message box
                "msgauto" => {
                    use crate::vcl::dialogs::MsgResult::*;
                    crate::vcl::dialogs::set_auto_answer(match rest {
                        "yes" => Some(Yes),
                        "no" => Some(No),
                        "cancel" => Some(Cancel),
                        _ => Some(Ok),
                    });
                }
                c if c.starts_with("pm") && super::passw_manager::automation::automate(&w, c, rest) => {}
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

thread_local! {
    static SNAPSHOT: RefCell<Option<crate::vcl::snapshot::Snapshot>> = const { RefCell::new(None) };
}

/// Draw a form in a context of its own and save the picture.
fn snap_form(form: &crate::vcl::Form, size: egui::Vec2, path: &std::path::Path) -> std::io::Result<()> {
    let dark = crate::windows::ctx().global_style().visuals.dark_mode;
    let img = SNAPSHOT.with(|s| {
        let mut s = s.borrow_mut();
        let snap = s.get_or_insert_with(|| {
            let ctx = egui::Context::default();
            crate::vcl::fonts::install(&ctx, &crate::vcl::fonts::user_font());
            let mut snap = crate::vcl::snapshot::Snapshot::new(ctx);
            snap.render(egui::vec2(100.0, 100.0), 1.0, 1, |_| {});
            snap
        });
        snap.ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
        snap.render(size, 1.0, 3, |ui| {
            let fill = ui.visuals().panel_fill;
            ui.painter().rect_filled(ui.max_rect(), 0.0, fill);
            form.show(ui);
        })
    });
    crate::vcl::snapshot::save_png(&img, path)
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
