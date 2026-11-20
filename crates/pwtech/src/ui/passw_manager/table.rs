//! Simple report-style list (`TListView` with `vsReport`) for the smaller
//! dialogs: rows of strings, optional group header rows, multi-selection.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gio, glib, pango};

/// One row: cell texts; header rows span the row in bold.
#[derive(Clone, Debug, Default)]
pub struct TableRow {
    pub cells: Vec<String>,
    pub header: bool,
}

pub struct Table {
    pub view: gtk::ColumnView,
    pub store: gio::ListStore,
    pub selection: gtk::MultiSelection,
    pub columns: Vec<gtk::ColumnViewColumn>,
    rows: RefCell<Vec<TableRow>>,
}

impl Table {
    pub fn new(titles: &[String], widths: &[i32]) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::MultiSelection::new(Some(store.clone()));
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.set_reorderable(false);
        view.set_show_column_separators(true);
        let mut columns = Vec::new();
        for (ci, title) in titles.iter().enumerate() {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(|_, obj| {
                let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
                let l = gtk::Label::new(None);
                l.set_xalign(0.0);
                l.set_ellipsize(pango::EllipsizeMode::End);
                item.set_child(Some(&l));
            });
            factory.connect_bind(move |_, obj| {
                let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
                let Some(l) = item.child().and_downcast::<gtk::Label>() else { return };
                let Some(data) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
                let row = data.borrow::<TableRow>();
                if row.header {
                    let text = if ci == 0 { row.cells.first().cloned().unwrap_or_default() } else { String::new() };
                    l.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(&text)));
                } else {
                    l.set_text(row.cells.get(ci).map(|s| s.as_str()).unwrap_or(""));
                }
            });
            let col = gtk::ColumnViewColumn::new(Some(title), Some(factory));
            col.set_resizable(true);
            if let Some(&w) = widths.get(ci) {
                col.set_fixed_width(w.max(20));
            }
            if ci + 1 == titles.len() {
                col.set_expand(true);
            }
            view.append_column(&col);
            columns.push(col);
        }
        Rc::new(Table { view, store, selection, columns, rows: RefCell::new(Vec::new()) })
    }

    pub fn clear(&self) {
        self.rows.borrow_mut().clear();
        self.store.remove_all();
    }

    pub fn add_row(&self, cells: Vec<String>, header: bool) {
        let row = TableRow { cells, header };
        self.rows.borrow_mut().push(row.clone());
        self.store.append(&glib::BoxedAnyObject::new(row));
    }

    pub fn len(&self) -> usize {
        self.rows.borrow().len()
    }

    pub fn rows(&self) -> Vec<TableRow> {
        self.rows.borrow().clone()
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.selection.is_selected(i as u32)
    }

    pub fn selected_count(&self) -> usize {
        self.selection.selection().size() as usize
    }

    pub fn column_widths(&self) -> Vec<i32> {
        self.columns.iter().map(|c| c.fixed_width().max(20)).collect()
    }
}
