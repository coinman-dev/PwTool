//! Test automation of the password manager for headless runs
//! (`PWTECH_AUTOMATE` commands starting with `pm`, see `ui::automate`).
//!
//! - `pmshow` — create/show the manager window
//! - `pmact NAME` — activate a menu action (`MainMenu_File_New` …),
//!   `pmactp NAME VALUE` with an int or string parameter
//! - `pmclick NAME` / `pmtoggle NAME` — click a button / toggle a button
//! - `pmtext NAME VALUE` — set the text of an entry, combo box or memo
//! - `pmsel I` — select row I of the entry list (`pmsel -` = clear)
//! - `pmtag I` — select tag list item I
//! - `pmenter` — press the default button of the top-most modal dialog
//! - `pmdlgtext NAME VALUE` — text of an entry in the top-most dialog
//!   (password dialog: `PasswBox`, `ConfirmPasswBox`)
//! - `pmescape` — close the top-most modal dialog
//! - `pmopen PATH` / `pmsaveas PATH` — open/save a database without the
//!   file chooser
//! - `pmprint NAME` / `pmtitle` / `pmrows` / `pmtags` / `pmstate` — print to
//!   stdout
//! - `pmsavecfg` — save the configuration and print the INI file
//! - `pmgen` — right-click on the password toggle button (generate)
//! - `pmdrop I` — drop the selected entry on row I (drag & drop)
//! - `pmsnap PATH` — picture of the manager window, `pmsnapdlg PATH` of the
//!   top-most dialog

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;

use super::list::{RowData, TagKind};
use super::*;

thread_local! {
    static PM: RefCell<Option<Rc<PasswManager>>> = const { RefCell::new(None) };
}

/// The manager instance used by the automation (created on first use).
pub fn instance(main: &Rc<MainWindow>) -> Rc<PasswManager> {
    main.passw_manager()
}

/// Top-most visible modal window (not the parent of another visible
/// window).
fn top_dialog() -> Option<gtk::Window> {
    let wins: Vec<gtk::Window> = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Window>().ok())
        .filter(|w| w.is_visible())
        .collect();
    let modal: Vec<&gtk::Window> = wins.iter().filter(|w| w.is_modal()).collect();
    modal
        .iter()
        .find(|m| !wins.iter().any(|o| o.transient_for().as_ref() == Some(**m) && o.is_visible()))
        .map(|w| (*w).clone())
}

fn find_named(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut c = root.first_child();
    while let Some(w) = c {
        if let Some(f) = find_named(&w, name) {
            return Some(f);
        }
        c = w.next_sibling();
    }
    None
}

fn set_text(w: &gtk::Widget, value: &str) {
    if let Some(e) = w.downcast_ref::<gtk::Entry>() {
        e.set_text(value);
    } else if let Some(cb) = w.downcast_ref::<gtk::ComboBoxText>() {
        crate::ui::util::set_combo_text(cb, value);
    } else if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
        if let Some(tv) = sw.child().and_downcast::<gtk::TextView>() {
            tv.buffer().set_text(&value.replace("\\n", "\n"));
        }
    }
}

/// Run one automation command; returns `false` if unknown.
pub fn automate(main: &Rc<MainWindow>, cmd: &str, rest: &str) -> bool {
    let pm = instance(main);
    match cmd {
        "pmshow" => pm.show(),
        "pmact" => {
            if !pm.activate_action(rest) {
                eprintln!("automate: action {rest} not available");
            }
        }
        "pmactp" => {
            let (name, val) = rest.split_once(' ').unwrap_or((rest, ""));
            let v = match val.parse::<i32>() {
                Ok(i) => i.to_variant(),
                Err(_) => val.to_variant(),
            };
            gtk::gio::prelude::ActionGroupExt::activate_action(&pm.b.actions, name, Some(&v));
        }
        "pmclick" => match pm.b.try_get::<gtk::Button>(rest) {
            Some(b) => b.emit_clicked(),
            None => eprintln!("automate: no button {rest}"),
        },
        "pmtoggle" => {
            if let Some(b) = pm.b.try_get::<gtk::ToggleButton>(rest) {
                b.set_active(!b.is_active());
            } else if let Some(b) = pm.b.try_get::<gtk::CheckButton>(rest) {
                b.set_active(!b.is_active());
            }
        }
        "pmtext" => {
            let (name, val) = rest.split_once(' ').unwrap_or((rest, ""));
            match pm.b.widgets.get(name) {
                Some(w) => set_text(w, val),
                None => eprintln!("automate: no widget {name}"),
            }
        }
        "pmsel" => {
            if rest == "-" {
                pm.list.clear_selection();
            } else if let Ok(i) = rest.parse::<u32>() {
                pm.list.select_only(i);
            }
        }
        "pmtag" => {
            if let Ok(i) = rest.parse::<usize>() {
                pm.tag_view.select_display_index(i);
            }
        }
        "pmenter" => match top_dialog() {
            Some(d) => {
                d.activate_default();
            }
            None => eprintln!("automate: no dialog"),
        },
        "pmescape" => {
            if let Some(d) = top_dialog() {
                d.close();
            }
        }
        "pmdlgtext" => {
            let (name, val) = rest.split_once(' ').unwrap_or((rest, ""));
            match top_dialog().and_then(|d| find_named(d.upcast_ref(), name)) {
                Some(w) => set_text(&w, val),
                None => eprintln!("automate: no dialog widget {name}"),
            }
        }
        "pmopen" => {
            let pm = pm.clone();
            let path = rest.to_string();
            glib::spawn_future_local(async move {
                pm.open_database(DB_OPEN_FLAG_EXISTING, &path).await;
            });
        }
        "pmsaveas" => {
            if pm.save_database(rest) {
                *pm.db_file_name.borrow_mut() = rest.to_string();
                pm.db_read_only.set(false);
                pm.set_db_changed(false, false);
                pm.change_caption();
                pm.reset_db_open_controls();
            }
        }
        "pmprint" => {
            if let Some(w) = pm.b.widgets.get(rest) {
                if let Some(l) = w.downcast_ref::<gtk::Label>() {
                    println!("{rest}={}", l.text());
                } else if let Some(e) = w.downcast_ref::<gtk::Entry>() {
                    println!("{rest}={}", e.text());
                } else if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
                    if let Some(tv) = sw.child().and_downcast::<gtk::TextView>() {
                        println!("{rest}={:?}", helpers::text_view_text(&tv).as_str());
                    }
                }
            }
        }
        "pmsavecfg" => {
            let mut core = main.core.borrow_mut();
            pm.save_config(&mut core.ini);
            for line in core.ini.to_text().lines() {
                println!("ini: {line}");
            }
        }
        "pmgen" => pm.toggle_passw_btn_right_click(),
        "pmdrop" => {
            if let Ok(i) = rest.parse::<u32>() {
                pm.db_view_drag_drop(i);
            }
        }
        "pmtitle" => println!("title={}", pm.win.title().unwrap_or_default()),
        "pmrows" => {
            for i in 0..pm.list.len() {
                if let Some(o) = pm.list.store.item(i).and_downcast::<glib::BoxedAnyObject>() {
                    let r = o.borrow::<RowData>();
                    println!(
                        "row{i}{}={}{}",
                        if pm.list.is_selected(i) { "*" } else { "" },
                        r.cols.join(" | "),
                        if r.image >= 0 { format!(" [img{}]", r.image) } else { String::new() }
                    );
                }
            }
        }
        "pmtags" => {
            let sel = pm.tag_view.selected();
            let names: Vec<String> = sel
                .iter()
                .map(|k| match k {
                    TagKind::All => "All".to_string(),
                    TagKind::Tag(t) => t.clone(),
                    TagKind::Untagged => "Untagged".to_string(),
                })
                .collect();
            println!("tags: {} items, selected {:?}, menu {}", pm.tag_view.len(), names, pm.tag_menu.n_items());
        }
        "pmstate" => println!(
            "open={} locked={} changed={} item_changed={} selected={:?} file={} search_mode={}",
            pm.is_db_open(),
            pm.locked.get(),
            pm.db_changed.get(),
            pm.item_changed.get(),
            pm.selected.get(),
            pm.db_file_name.borrow(),
            pm.search_mode.get()
        ),
        "pmsnap" => {
            crate::ui::util::snapshot_window(pm.win.upcast_ref(), rest);
        }
        "pmsnapdlg" => {
            if let Some(d) = top_dialog() {
                crate::ui::util::snapshot_window(&d, rest);
            }
        }
        _ => return false,
    }
    true
}
