//! Key-value list editor (`upstream/src/main/PasswMngKeyValEdit.*`): a
//! two-column grid with the (translated) key names and editable values.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use pwtech_core::config::IniFile;
use pwtech_core::language::trl;
use zeroize::Zeroizing;

use super::helpers::close_on_escape;
use crate::ui::builder::{Built, build_form};
use crate::ui::dialogs::form_window;
use crate::ui::form::load_form;

const CONFIG_ID: &str = "PasswMngKeyValEdit";

pub struct KeyValDlg {
    pub b: Built,
    pub win: gtk::Window,
    grid: gtk::Grid,
    key_labels: Vec<gtk::Label>,
    value_boxes: Vec<gtk::Entry>,
    col_widths: Cell<(i32, i32)>,
}

impl KeyValDlg {
    /// `names`: translated key names in the order of the rows.
    pub fn new(parent: &gtk::Window, ini: &IniFile, names: &[String]) -> Rc<Self> {
        let b = build_form(load_form("PasswMngKeyValEdit"), "PasswMngKeyValEdit");
        let win = form_window(&b, Some(parent), true);
        win.set_hide_on_close(true);
        let mut cw = (b.sc(150), b.sc(300));
        let s = ini.read_string(CONFIG_ID, "ColWidths", "");
        if s.len() >= 3 {
            if let Some((a, c)) = s.split_once(';') {
                cw = (a.trim().parse().unwrap_or(20), c.trim().parse().unwrap_or(20));
            }
        }
        let grid = gtk::Grid::new();
        grid.set_column_spacing(1);
        grid.set_row_spacing(1);
        grid.add_css_class("view");
        let hk = gtk::Label::new(Some(&trl("Key")));
        let hv = gtk::Label::new(Some(&trl("Value")));
        for (i, h) in [&hk, &hv].into_iter().enumerate() {
            h.set_xalign(0.0);
            h.set_margin_start(4);
            h.add_css_class("heading");
            grid.attach(h, i as i32, 0, 1, 1);
        }
        hk.set_size_request(cw.0, -1);
        let mut key_labels = Vec::new();
        let mut value_boxes = Vec::new();
        for (r, n) in names.iter().enumerate() {
            let l = gtk::Label::new(Some(n));
            l.set_xalign(0.0);
            l.set_margin_start(4);
            l.set_size_request(cw.0, -1);
            let e = gtk::Entry::new();
            e.set_hexpand(true);
            grid.attach(&l, 0, r as i32 + 1, 1, 1);
            grid.attach(&e, 1, r as i32 + 1, 1, 1);
            key_labels.push(l);
            value_boxes.push(e);
        }
        let sw: gtk::ScrolledWindow = b.get("KeyValueGrid");
        sw.set_child(Some(&grid));
        let h = ini.read_int(CONFIG_ID, "WindowHeight", -1);
        let w = ini.read_int(CONFIG_ID, "WindowWidth", -1);
        if w > 0 && h > 0 {
            win.set_default_size(w, h);
        }
        close_on_escape(&win);
        let okb: gtk::Button = b.get("OKBtn");
        for e in &value_boxes {
            let okb = okb.clone();
            e.connect_activate(move |_| okb.emit_clicked());
        }
        Rc::new(KeyValDlg { b, win, grid, key_labels, value_boxes, col_widths: Cell::new(cw) })
    }

    pub fn save_config(&self, ini: &mut IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.width() as i64);
        let w0 = self.key_labels.first().map(|l| l.width()).filter(|&w| w > 0).unwrap_or(self.col_widths.get().0);
        let w1 = self.value_boxes.first().map(|l| l.width()).filter(|&w| w > 0).unwrap_or(self.col_widths.get().1);
        ini.write_string(CONFIG_ID, "ColWidths", &format!("{w0};{w1}"));
    }

    /// Show the dialog with the given values (one per key row); returns the
    /// edited values if OK was pressed. The grid is cleared afterwards.
    pub async fn execute(self: &Rc<Self>, values: &[Zeroizing<String>]) -> Option<Vec<Zeroizing<String>>> {
        for (e, v) in self.value_boxes.iter().zip(values.iter()) {
            e.set_text(v);
        }
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
        if let Some(e) = self.value_boxes.first() {
            e.grab_focus();
        }
        let _ = rx.recv().await;
        ok.disconnect(h1);
        cancel.disconnect(h2);
        self.win.disconnect(h3);
        let result = if ok_pressed.get() {
            Some(self.value_boxes.iter().map(|e| Zeroizing::new(e.text().to_string())).collect())
        } else {
            None
        };
        // `Clear()`
        for e in &self.value_boxes {
            e.set_text("");
        }
        let _ = &self.grid;
        result
    }
}
