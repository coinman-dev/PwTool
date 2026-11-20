//! Column / field selection dialog (`upstream/src/main/PasswMngColSelect.*`).

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use pwtool_core::language::trl;
use pwtool_core::passw::database::{NUM_FIELDS, NUM_STRING_FIELDS};

use crate::ui::builder::{Built, build_form};
use crate::ui::dialogs::form_window;
use crate::ui::form::load_form;
use crate::ui::util::{MsgButtons, MsgIcon, msg_box};

pub struct ColSelectDlg {
    pub b: Built,
    pub win: gtk::Window,
}

impl ColSelectDlg {
    pub fn new(parent: &gtk::Window) -> Rc<Self> {
        let b = build_form(load_form("PasswMngColSelect"), "PasswMngColSelect");
        let win = form_window(&b, Some(parent), true);
        win.set_hide_on_close(true);
        super::helpers::close_on_escape(&win);
        Rc::new(ColSelectDlg { b, win })
    }

    /// `Execute()`: returns the new bit mask, or `None` if cancelled.
    pub async fn execute(self: &Rc<Self>, col_names: &[String; NUM_FIELDS], flags: u32, strings_only: bool) -> Option<u32> {
        let n = if strings_only { NUM_STRING_FIELDS } else { NUM_FIELDS };
        let lb: gtk::ListBox = self.b.get("OptionsList.list");
        while let Some(c) = lb.first_child() {
            lb.remove(&c);
        }
        let mut checks = Vec::new();
        for (i, name) in col_names.iter().enumerate().take(n) {
            let c = gtk::CheckButton::with_label(name);
            c.set_active(flags & (1 << i) != 0);
            lb.append(&c);
            checks.push(c);
        }
        let result = Rc::new(Cell::new(None::<u32>));
        let (tx, rx) = async_channel::bounded::<()>(1);
        let ok: gtk::Button = self.b.get("OKBtn");
        let cancel: gtk::Button = self.b.get("CancelBtn");
        let r = result.clone();
        let w = self.win.clone();
        let checks2 = checks.clone();
        let h1 = ok.connect_clicked(move |_| {
            let mut f = 0u32;
            for (i, c) in checks2.iter().enumerate() {
                if c.is_active() {
                    f |= 1 << i;
                }
            }
            if f == 0 {
                let w = w.clone();
                glib::spawn_future_local(async move {
                    msg_box(Some(&w), &trl("Select at least one column."), MsgIcon::Warning, MsgButtons::Ok, false).await;
                });
                return;
            }
            r.set(Some(f));
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
        result.get()
    }
}
