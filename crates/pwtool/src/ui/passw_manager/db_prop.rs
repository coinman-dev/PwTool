//! Database properties dialog (`upstream/src/main/PasswMngDbProp.*`).

use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use pwtool_core::config::IniFile;
use pwtool_core::language::trl;

use super::table::Table;
use crate::ui::builder::{Built, build_form};
use crate::ui::dialogs::{form_window, wait_closed};
use crate::ui::form::load_form;

const CONFIG_ID: &str = "PasswMngDbProp";

/// `DbProperty`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbProperty {
    FileName,
    FilePath,
    FileSize,
    CreationTime,
    ModificationTime,
    Description,
    FormatVersion,
    RecoveryKeySet,
    NumEntries,
    NumExpiredEntries,
    NumTags,
}

/// Captions of the properties (items of `PropView`) and their group.
const PROPS: [(&str, usize); 11] = [
    ("Name", 0),
    ("Location", 0),
    ("Size", 0),
    ("Creation time", 0),
    ("Last modification", 0),
    ("Description", 1),
    ("Format version", 1),
    ("Recovery password set", 1),
    ("Number of entries", 1),
    ("Number of expired entries", 1),
    ("Number of tags", 1),
];
const GROUPS: [&str; 2] = ["File", "Database"];

pub struct DbPropDlg {
    pub b: Built,
    pub win: gtk::Window,
    table: Rc<Table>,
    values: std::cell::RefCell<[String; 11]>,
}

impl DbPropDlg {
    pub fn new(parent: &gtk::Window, ini: &IniFile) -> Rc<Self> {
        let b = build_form(load_form("PasswMngDbProp"), "PasswMngDbProp");
        let win = form_window(&b, Some(parent), true);
        win.set_hide_on_close(true);
        let mut widths = vec![188, 300];
        let cw = ini.read_string(CONFIG_ID, "ColWidths", "");
        if cw.len() >= 3 {
            if let Some((a, c)) = cw.split_once(';') {
                widths = vec![a.trim().parse().unwrap_or(20), c.trim().parse().unwrap_or(20)];
            }
        }
        let table = Table::new(&[trl("Property"), trl("Value")], &widths);
        let sw: gtk::ScrolledWindow = b.get("PropView");
        sw.set_child(Some(&table.view));
        let d = Rc::new(DbPropDlg { b, win, table, values: Default::default() });
        let w = d.win.clone();
        d.b.get::<gtk::Button>("CloseBtn").connect_clicked(move |_| w.close());
        d
    }

    pub fn save_config(&self, ini: &mut IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.width() as i64);
        let cw = self.table.column_widths();
        ini.write_string(CONFIG_ID, "ColWidths", &format!("{};{}", cw[0], cw[1]));
    }

    /// `SetProperty()` (empty values are ignored).
    pub fn set_property(&self, prop: DbProperty, val: &str) {
        if !val.is_empty() {
            self.values.borrow_mut()[prop as usize] = val.to_string();
        }
    }

    /// `ShowModal()`; the values are cleared afterwards (`FormClose`).
    pub async fn show_modal(&self) {
        self.table.clear();
        let vals = self.values.borrow().clone();
        for (g, gname) in GROUPS.iter().enumerate() {
            self.table.add_row(vec![trl(gname)], true);
            for (i, (name, grp)) in PROPS.iter().enumerate() {
                if *grp == g {
                    self.table.add_row(vec![trl(name), vals[i].clone()], false);
                }
            }
        }
        self.win.present();
        wait_closed(&self.win).await;
        *self.values.borrow_mut() = Default::default();
    }
}
