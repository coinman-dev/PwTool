//! Test automation of the password manager (`PWTOOL_AUTOMATE` commands
//! starting with `pm`, see `ui::automate`); the same commands as in the GTK
//! version where possible. Dialogs are driven with the `f…` commands
//! (`fclick PasswEnter OKBtn`, `ftext PasswEnter PasswBox …`).
//!
//! - `pmshow` — create/show the manager window
//! - `pmact NAME` — activate a menu item (`MainMenu_File_New` …)
//! - `pmclick NAME` / `pmtoggle NAME` — click / toggle a button
//! - `pmtext NAME VALUE` — set the text of an edit, combo box or memo
//!   (as typed: a change event follows)
//! - `pmsel I` — select row I of the entry list (`pmsel -` = clear)
//! - `pmtag I` — select tag list item I
//! - `pmopen PATH` / `pmsaveas PATH` — open/save without the file dialog
//! - `pmprint NAME` / `pmtitle` / `pmrows` / `pmtags` / `pmstate` — print
//! - `pmsavecfg` — save the configuration and print the INI file
//! - `pmgen` — right click on the password toggle button (generate)
//! - `pmdrop I` — drop the selected entry on row I (drag & drop)

use std::rc::Rc;

use super::*;

/// The manager instance used by the automation (created on first use).
pub fn instance(main: &Rc<MainWindow>) -> Rc<PasswManager> {
    main.passw_manager()
}

pub fn automate(main: &Rc<MainWindow>, cmd: &str, rest: &str) -> bool {
    let pm = instance(main);
    let f = pm.form.clone();
    match cmd {
        "pmshow" => pm.show(),
        "pmact" => {
            if !pm.activate(rest) {
                eprintln!("automate: menu item {rest} not available");
            }
        }
        "pmclick" => f.push_event(Event::Click(rest.to_string())),
        "pmtoggle" => {
            f.set_checked(rest, !f.checked(rest));
            f.push_event(Event::Click(rest.to_string()));
        }
        "pmtext" => {
            let (name, val) = rest.split_once(' ').unwrap_or((rest, ""));
            f.set_text(name, val);
            f.push_event(Event::Change(name.to_string()));
        }
        "pmsel" => {
            if rest == "-" {
                pm.list.clear_selection();
            } else if let Ok(i) = rest.parse::<usize>() {
                pm.list.select_only(i);
            }
            pm.trigger_apply();
        }
        "pmtag" => {
            if let Ok(i) = rest.parse::<usize>() {
                pm.tag_view.select_display_index(i);
                pm.tag_apply_pending.set(true);
            }
        }
        "pmopen" => {
            pm.open_window();
            let pm = pm.clone();
            let path = rest.to_string();
            crate::runtime::spawn(async move {
                pm.open_database(DB_OPEN_FLAG_EXISTING, &path).await;
            });
        }
        "pmsaveas" => {
            let pm = pm.clone();
            let path = rest.to_string();
            crate::runtime::spawn(async move {
                pm.save_as(&path).await;
            });
        }
        "pmprint" => println!("{rest}={:?}", f.text(rest)),
        "pmsavecfg" => {
            let mut core = main.core.borrow_mut();
            pm.save_config(&mut core.ini);
            for line in core.ini.to_text().lines() {
                println!("ini: {line}");
            }
        }
        "pmgen" => pm.toggle_passw_btn_right_click(),
        // `pmfill N`: add N entries (titles "Entry n", tags) to test long lists
        "pmfill" => {
            let n: usize = rest.parse().unwrap_or(1000);
            let t = std::time::Instant::now();
            if let Some(db) = pm.db.borrow_mut().as_mut() {
                for i in 0..n {
                    let idx = db.new_entry();
                    db.set_entry_passw(idx, &format!("passw{i:05}"));
                    let e = db.entry_mut(idx);
                    e.set_string(FieldType::Title, &format!("Entry {i:05}"));
                    e.set_string(FieldType::UserName, &format!("user{}", i % 97));
                    e.add_tag(["web", "mail", "bank", "work"][i % 4]);
                }
            }
            pm.set_db_changed(true, false);
            pm.reset_list_view(RELOAD_TAGS);
            println!("pmfill: {n} entries in {} ms", t.elapsed().as_millis());
        }
        // `pmsearch TEXT`: search and print the time
        "pmsearch" => {
            let t = std::time::Instant::now();
            pm.search_database(rest, pm.search_flags.get(), false, false);
            println!("pmsearch: {} rows in {} ms", pm.list.len(), t.elapsed().as_millis());
        }
        "pmdrop" => {
            if let Ok(i) = rest.parse::<usize>() {
                pm.db_view_drag_drop(i);
            }
        }
        "pmtitle" => println!("title={}", pm.window.borrow().as_ref().map(|w| w.title.borrow().clone()).unwrap_or_default()),
        "pmrows" => {
            for (i, r) in pm.list.rows().iter().enumerate() {
                println!(
                    "row{i}{}={}{}",
                    if pm.list.is_selected(i) { "*" } else { "" },
                    r.cells.join(" | "),
                    r.icon.as_ref().map(|ic| format!(" [{}]", ic.name)).unwrap_or_default()
                );
            }
        }
        "pmtags" => {
            let names: Vec<String> = pm.tag_view.selected().iter().map(tag_name).collect();
            println!("tags: {} items, selected {:?}, menu {}", pm.tag_view.len(), names, pm.tag_menu_items.borrow().len());
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
        _ => return false,
    }
    windows::ctx().request_repaint();
    true
}

use crate::vcl::Event;
