//! Password history dialog (`upstream/src/main/PasswMngPwHistory.*`).

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use pwtech_core::config::IniFile;
use pwtech_core::language::trl;
use pwtech_core::passw::database::{MAX_PASSW_HISTORY_SIZE, PasswHistory};
use zeroize::Zeroizing;

use super::helpers::close_on_escape;
use super::table::Table;
use crate::ui::builder::{Built, build_form};
use crate::ui::dialogs::form_window;
use crate::ui::form::load_form;

const CONFIG_ID: &str = "PasswHistoryDlg";

/// Result of the dialog.
pub struct HistoryResult {
    pub active: bool,
    pub max_size: u32,
    /// The list was cleared with the "Clear" button.
    pub cleared: bool,
}

pub struct PwHistoryDlg {
    pub b: Built,
    pub win: gtk::Window,
    table: Rc<Table>,
}

impl PwHistoryDlg {
    pub fn new(parent: &gtk::Window, ini: &IniFile) -> Rc<Self> {
        let b = build_form(load_form("PasswMngPwHistory"), "PasswMngPwHistory");
        let win = form_window(&b, Some(parent), true);
        win.set_hide_on_close(true);
        if let Some(u) = b.updowns.get("HistorySizeSpinBtn") {
            u.set_min(1);
            u.set_max(MAX_PASSW_HISTORY_SIZE as i64);
        }
        let mut widths = vec![b.sc(150), b.sc(250)];
        let s = ini.read_string(CONFIG_ID, "ColWidths", "");
        if s.len() >= 3 {
            if let Some((a, c)) = s.split_once(';') {
                widths = vec![a.trim().parse().unwrap_or(20), c.trim().parse().unwrap_or(20)];
            }
        }
        let table = Table::new(&[trl("Set Date/Time"), trl("Password")], &widths);
        let sw: gtk::ScrolledWindow = b.get("HistoryView");
        sw.set_child(Some(&table.view));
        let h = ini.read_int(CONFIG_ID, "WindowHeight", -1);
        let w = ini.read_int(CONFIG_ID, "WindowWidth", -1);
        if w > 0 && h > 0 {
            win.set_default_size(w, h);
        }
        close_on_escape(&win);
        let d = Rc::new(PwHistoryDlg { b, win, table });
        let dd = d.clone();
        d.b.get::<gtk::CheckButton>("EnableHistoryCheck").connect_toggled(move |_| dd.enable_history_check());
        let dd = d.clone();
        d.b.get::<gtk::Button>("ClearBtn").connect_clicked(move |_| {
            dd.table.clear();
        });
        let dd = d.clone();
        d.b.get::<gtk::Button>("CopyBtn").connect_clicked(move |_| dd.copy());
        let key = gtk::EventControllerKey::new();
        let dd = d.clone();
        key.connect_key_pressed(move |_, k, _, m| {
            if m.contains(gtk::gdk::ModifierType::CONTROL_MASK) && (k == gtk::gdk::Key::c || k == gtk::gdk::Key::C) {
                dd.copy();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        d.table.view.add_controller(key);
        d
    }

    pub fn save_config(&self, ini: &mut IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.width() as i64);
        let cw = self.table.column_widths();
        ini.write_string(CONFIG_ID, "ColWidths", &format!("{};{}", cw[0], cw[1]));
    }

    /// `EnableHistoryCheckClick()`.
    fn enable_history_check(&self) {
        let on = self.b.get::<gtk::CheckButton>("EnableHistoryCheck").is_active();
        self.b.w("HistorySizeBox").set_sensitive(on);
        if let Some(u) = self.b.updowns.get("HistorySizeSpinBtn") {
            u.set_sensitive(on);
        }
    }

    /// `CopyBtnClick()`: selected rows (or all) as "time<TAB>password".
    fn copy(&self) {
        let n = self.table.len();
        if n == 0 {
            return;
        }
        let all = self.table.selected_count() == 0;
        let mut s = Zeroizing::new(String::new());
        for (i, r) in self.table.rows().iter().enumerate() {
            if all || self.table.is_selected(i) {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&r.cells[0]);
                s.push('\t');
                s.push_str(&r.cells[1]);
            }
        }
        crate::ui::clipboard::set_data(&s);
    }

    /// Show the history; `None` if cancelled.
    pub async fn execute(self: &Rc<Self>, hist: &PasswHistory) -> Option<HistoryResult> {
        self.b.get::<gtk::CheckButton>("EnableHistoryCheck").set_active(hist.active());
        self.enable_history_check();
        if let Some(u) = self.b.updowns.get("HistorySizeSpinBtn") {
            u.set_position(hist.max_size() as i64);
        }
        self.table.clear();
        for (t, p) in hist.iter() {
            let ts = if *t == 0 { "-".to_string() } else { pwtech_core::util::time_stamp_to_string(*t) };
            self.table.add_row(vec![ts, p.to_string()], false);
        }
        let has = self.table.len() > 0;
        self.b.w("ClearBtn").set_sensitive(has);
        self.b.w("CopyBtn").set_sensitive(has);
        let ok_pressed = Rc::new(Cell::new(false));
        let (tx, rx) = async_channel::bounded::<()>(1);
        let ok: gtk::Button = self.b.get("OKBtn");
        let cancel: gtk::Button = self.b.get("CancelBtn");
        let r = ok_pressed.clone();
        let w = self.win.clone();
        let h1 = ok.connect_clicked(move |_| {
            r.set(true);
            w.close();
        });
        let w = self.win.clone();
        let h2 = cancel.connect_clicked(move |_| w.close());
        let h3 = self.win.connect_close_request(move |_| {
            let _ = tx.try_send(());
            glib::Propagation::Proceed
        });
        self.win.set_default_widget(Some(&ok));
        self.win.present();
        let _ = rx.recv().await;
        ok.disconnect(h1);
        cancel.disconnect(h2);
        self.win.disconnect(h3);
        let cleared = has && self.table.len() == 0;
        self.table.clear();
        if !ok_pressed.get() {
            return None;
        }
        Some(HistoryResult {
            active: self.b.get::<gtk::CheckButton>("EnableHistoryCheck").is_active(),
            max_size: self.b.updowns.get("HistorySizeSpinBtn").map(|u| u.position()).unwrap_or(1).max(1) as u32,
            cleared,
        })
    }
}
