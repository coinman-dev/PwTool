//! The entry list (`DbView`, a report-style `TListView`) and the tag list
//! (`TagView`, small icons) of the password manager.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};

use super::helpers::DynCss;

/// Identity of a list row: the "<New Entry>" row or a database entry (id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    New,
    Entry(u32),
}

/// Data of one row.
#[derive(Clone, Debug)]
pub struct RowData {
    pub key: RowKey,
    /// Texts of the visible columns.
    pub cols: Vec<String>,
    /// Image index: 0 = expired, 1 = expires soon, -1 = none.
    pub image: i32,
}

pub struct DbList {
    pub view: gtk::ColumnView,
    pub store: gio::ListStore,
    pub selection: gtk::MultiSelection,
    columns: RefCell<Vec<(gtk::ColumnViewColumn, usize)>>,
    keys: RefCell<Vec<RowKey>>,
    images: [Option<gdk::Texture>; 2],
    pub css: DynCss,
    on_context: RefCell<Option<Rc<dyn Fn(f64, f64)>>>,
    on_sort: RefCell<Option<Rc<dyn Fn(usize, bool)>>>,
    can_drag: RefCell<Option<Rc<dyn Fn() -> bool>>>,
    on_drop: RefCell<Option<Rc<dyn Fn(u32)>>>,
    suppress_sort: Cell<bool>,
    icon_size: i32,
}

impl DbList {
    pub fn new(images: [Option<gdk::Texture>; 2], icon_size: i32) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::MultiSelection::new(Some(store.clone()));
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.set_show_column_separators(true);
        view.set_reorderable(false);
        let css = DynCss::new(&view);
        let l = Rc::new(DbList {
            view,
            store,
            selection,
            columns: RefCell::new(Vec::new()),
            keys: RefCell::new(Vec::new()),
            images,
            css,
            on_context: RefCell::new(None),
            on_sort: RefCell::new(None),
            can_drag: RefCell::new(None),
            on_drop: RefCell::new(None),
            suppress_sort: Cell::new(false),
            icon_size,
        });
        // header clicks: the list is sorted by the manager itself
        let ll = Rc::downgrade(&l);
        if let Some(sorter) = l.view.sorter() {
            sorter.connect_changed(move |s, _| {
                let Some(l) = ll.upgrade() else { return };
                if l.suppress_sort.get() {
                    return;
                }
                let Some(cs) = s.downcast_ref::<gtk::ColumnViewSorter>() else { return };
                let Some(col) = cs.primary_sort_column() else { return };
                let field = l.columns.borrow().iter().find(|(c, _)| *c == col).map(|(_, f)| *f);
                if let (Some(f), Some(cb)) = (field, l.on_sort.borrow().clone()) {
                    cb(f, cs.primary_sort_order() == gtk::SortType::Ascending);
                }
            });
        }
        // context menu on empty space
        let g = gtk::GestureClick::new();
        g.set_button(gdk::BUTTON_SECONDARY);
        let ll = Rc::downgrade(&l);
        g.connect_pressed(move |g, _, x, y| {
            let Some(l) = ll.upgrade() else { return };
            if let Some(cb) = l.on_context.borrow().clone() {
                g.set_state(gtk::EventSequenceState::Claimed);
                cb(x, y);
            }
        });
        l.view.add_controller(g);
        l
    }

    /// Function called on right-click (coordinates relative to the view);
    /// a click on an unselected row selects it first.
    pub fn connect_context(&self, f: impl Fn(f64, f64) + 'static) {
        *self.on_context.borrow_mut() = Some(Rc::new(f));
    }

    /// Drag & drop of a row onto another (`DbViewDragDrop`): `can_drag`
    /// decides whether dragging may start, `on_drop` receives the target
    /// row.
    pub fn connect_drag_drop(&self, can_drag: impl Fn() -> bool + 'static, on_drop: impl Fn(u32) + 'static) {
        *self.can_drag.borrow_mut() = Some(Rc::new(can_drag));
        *self.on_drop.borrow_mut() = Some(Rc::new(on_drop));
    }

    /// Function called when a column header was clicked: field index,
    /// ascending.
    pub fn connect_sort(&self, f: impl Fn(usize, bool) + 'static) {
        *self.on_sort.borrow_mut() = Some(Rc::new(f));
    }

    /// Recreate the columns: `(field index, title, width)`.
    pub fn set_columns(self: &Rc<Self>, cols: &[(usize, String, i32)]) {
        for (c, _) in self.columns.borrow_mut().drain(..) {
            self.view.remove_column(&c);
        }
        for (ci, (field, title, width)) in cols.iter().enumerate() {
            let factory = gtk::SignalListItemFactory::new();
            let ll = Rc::downgrade(self);
            let first = ci == 0;
            let icon_size = self.icon_size;
            factory.connect_setup(move |_, obj| {
                let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
                let bx = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                if first {
                    let img = gtk::Image::new();
                    img.set_pixel_size(icon_size);
                    img.set_visible(false);
                    bx.append(&img);
                }
                let l = gtk::Label::new(None);
                l.set_xalign(0.0);
                l.set_ellipsize(pango::EllipsizeMode::End);
                l.set_hexpand(true);
                bx.append(&l);
                // right-click selects the row (like a Windows list view)
                let g = gtk::GestureClick::new();
                g.set_button(gdk::BUTTON_SECONDARY);
                let item_w = item.downgrade();
                let ll1 = ll.clone();
                let bxw = bx.downgrade();
                g.connect_pressed(move |g, _, x, y| {
                    let (Some(l), Some(item), Some(bx)) = (ll1.upgrade(), item_w.upgrade(), bxw.upgrade()) else { return };
                    let pos = item.position();
                    if pos != gtk::INVALID_LIST_POSITION && !l.selection.is_selected(pos) {
                        l.selection.select_item(pos, true);
                    }
                    g.set_state(gtk::EventSequenceState::Claimed);
                    if let Some(cb) = l.on_context.borrow().clone() {
                        let p = bx
                            .compute_point(&l.view, &gtk::graphene::Point::new(x as f32, y as f32))
                            .map(|p| (p.x() as f64, p.y() as f64))
                            .unwrap_or((x, y));
                        cb(p.0, p.1);
                    }
                });
                bx.add_controller(g);
                // drag & drop to rearrange entries
                let ds = gtk::DragSource::new();
                ds.set_actions(gdk::DragAction::MOVE);
                let ll2 = ll.clone();
                ds.connect_prepare(move |_, _, _| {
                    let l = ll2.upgrade()?;
                    let ok = l.can_drag.borrow().as_ref().is_some_and(|f| f());
                    if ok { Some(gdk::ContentProvider::for_value(&"pwtech-db-entry".to_value())) } else { None }
                });
                bx.add_controller(ds);
                let dt = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
                let item_w2 = item.downgrade();
                let ll3 = ll.clone();
                dt.connect_drop(move |_, v, _, _| {
                    let (Some(l), Some(item)) = (ll3.upgrade(), item_w2.upgrade()) else { return false };
                    if v.get::<String>().ok().as_deref() != Some("pwtech-db-entry") {
                        return false;
                    }
                    let pos = item.position();
                    if pos == gtk::INVALID_LIST_POSITION {
                        return false;
                    }
                    if let Some(f) = l.on_drop.borrow().clone() {
                        f(pos);
                    }
                    true
                });
                bx.add_controller(dt);
                item.set_child(Some(&bx));
            });
            let ll = Rc::downgrade(self);
            factory.connect_bind(move |_, obj| {
                let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
                let Some(bx) = item.child().and_downcast::<gtk::Box>() else { return };
                let Some(data) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
                let row = data.borrow::<RowData>();
                let label = bx.last_child().and_downcast::<gtk::Label>().unwrap();
                label.set_text(row.cols.get(ci).map(|s| s.as_str()).unwrap_or(""));
                if first {
                    if let Some(img) = bx.first_child().and_downcast::<gtk::Image>() {
                        let tex = ll
                            .upgrade()
                            .and_then(|l| if row.image >= 0 { l.images.get(row.image as usize).cloned().flatten() } else { None });
                        match tex {
                            Some(t) => {
                                img.set_paintable(Some(&t));
                                img.set_visible(true);
                            }
                            None => {
                                img.set_paintable(None::<&gdk::Paintable>);
                                img.set_visible(false);
                            }
                        }
                    }
                }
            });
            let col = gtk::ColumnViewColumn::new(Some(title), Some(factory));
            col.set_resizable(true);
            col.set_fixed_width((*width).max(10));
            // a sorter makes the header clickable (sorting is done by the manager)
            col.set_sorter(Some(&gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal)));
            self.view.append_column(&col);
            self.columns.borrow_mut().push((col, *field));
        }
    }

    /// Current column widths (in column order).
    pub fn column_widths(&self) -> Vec<i32> {
        self.columns.borrow().iter().map(|(c, _)| c.fixed_width().max(10)).collect()
    }

    /// Show the sort indicator on the column of `field` (`None` = unsorted).
    pub fn set_sort_indicator(&self, field: Option<usize>, ascending: bool) {
        self.suppress_sort.set(true);
        let col = field.and_then(|f| self.columns.borrow().iter().find(|(_, cf)| *cf == f).map(|(c, _)| c.clone()));
        self.view.sort_by_column(
            col.as_ref(),
            if ascending { gtk::SortType::Ascending } else { gtk::SortType::Descending },
        );
        self.suppress_sort.set(false);
    }

    pub fn set_rows(&self, rows: Vec<RowData>) {
        *self.keys.borrow_mut() = rows.iter().map(|r| r.key).collect();
        let objs: Vec<glib::Object> = rows.into_iter().map(|r| glib::BoxedAnyObject::new(r).upcast()).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }

    pub fn clear(&self) {
        self.keys.borrow_mut().clear();
        self.store.remove_all();
    }

    pub fn len(&self) -> u32 {
        self.store.n_items()
    }

    pub fn key_at(&self, i: u32) -> Option<RowKey> {
        self.keys.borrow().get(i as usize).copied()
    }

    pub fn index_of(&self, key: RowKey) -> Option<u32> {
        self.keys.borrow().iter().position(|k| *k == key).map(|i| i as u32)
    }

    pub fn sel_count(&self) -> u32 {
        self.selection.selection().size() as u32
    }

    /// `DbView->Selected`: first selected row.
    pub fn first_selected(&self) -> Option<u32> {
        let s = self.selection.selection();
        if s.is_empty() { None } else { Some(s.minimum()) }
    }

    pub fn is_selected(&self, i: u32) -> bool {
        self.selection.is_selected(i)
    }

    pub fn selected_indices(&self) -> Vec<u32> {
        (0..self.len()).filter(|&i| self.selection.is_selected(i)).collect()
    }

    pub fn select_only(&self, i: u32) {
        if i < self.len() {
            self.selection.select_item(i, true);
        }
    }

    pub fn clear_selection(&self) {
        self.selection.unselect_all();
    }

    pub fn select_all(&self) {
        self.selection.select_all();
    }

    /// `ScrollToItem()`: bring a row into view.
    pub fn scroll_to(&self, i: u32) {
        let mut c = self.view.first_child();
        while let Some(w) = c {
            if w.type_().name() == "GtkColumnListView" || w.type_().name() == "GtkListView" {
                let _ = w.activate_action("list.scroll-to-item", Some(&i.to_variant()));
                return;
            }
            c = w.next_sibling();
        }
    }
}

/// Kind of a tag list item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagKind {
    All,
    Tag(String),
    Untagged,
}

struct TagItem {
    kind: TagKind,
    count: u32,
    child: gtk::FlowBoxChild,
}

pub struct TagList {
    pub flow: gtk::FlowBox,
    items: RefCell<Vec<TagItem>>,
    pub suppress: Cell<bool>,
    images: [Option<gdk::Texture>; 3],
    /// Sort by name (0) or frequency (1), order factor.
    sort: Cell<(i32, i32)>,
    icon_size: i32,
}

impl TagList {
    /// `images`: tag, "all", "untagged".
    pub fn new(images: [Option<gdk::Texture>; 3], icon_size: i32) -> Rc<Self> {
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::Multiple);
        flow.set_valign(gtk::Align::Start);
        flow.set_homogeneous(false);
        flow.set_max_children_per_line(1000);
        flow.set_activate_on_single_click(false);
        flow.set_row_spacing(0);
        flow.set_column_spacing(8);
        let t = Rc::new(TagList {
            flow,
            items: RefCell::new(Vec::new()),
            suppress: Cell::new(false),
            images,
            sort: Cell::new((0, 1)),
            icon_size,
        });
        let tw = Rc::downgrade(&t);
        t.flow.set_sort_func(move |a, b| {
            let Some(t) = tw.upgrade() else { return gtk::Ordering::Equal };
            let items = t.items.borrow();
            let find = |c: &gtk::FlowBoxChild| items.iter().find(|i| &i.child == c).map(|i| (i.kind.clone(), i.count));
            match (find(a), find(b)) {
                (Some(x), Some(y)) => t.compare(&x, &y).into(),
                _ => gtk::Ordering::Equal,
            }
        });
        t
    }

    /// `TagViewCompare()`.
    fn compare(&self, a: &(TagKind, u32), b: &(TagKind, u32)) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let (by, factor) = self.sort.get();
        match (&a.0, &b.0) {
            (TagKind::All, _) => Less,
            (_, TagKind::All) => Greater,
            (TagKind::Untagged, _) => Greater,
            (_, TagKind::Untagged) => Less,
            (TagKind::Tag(x), TagKind::Tag(y)) => {
                let o = if by == 0 { super::helpers::wcsicmp(x, y) } else { a.1.cmp(&b.1) };
                if factor < 0 { o.reverse() } else { o }
            }
        }
    }

    pub fn set_sort(&self, by: i32, factor: i32) {
        self.sort.set((by, factor));
        self.flow.invalidate_sort();
    }

    pub fn clear(&self) {
        self.suppress.set(true);
        for it in self.items.borrow_mut().drain(..) {
            self.flow.remove(&it.child);
        }
        self.suppress.set(false);
    }

    /// Add an item; returns its index.
    pub fn add(&self, kind: TagKind, caption: &str, count: u32, selected: bool) -> usize {
        let bx = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let img_idx = match kind {
            TagKind::Tag(_) => 0,
            TagKind::All => 1,
            TagKind::Untagged => 2,
        };
        if let Some(t) = &self.images[img_idx] {
            let img = gtk::Image::from_paintable(Some(t));
            img.set_pixel_size(self.icon_size);
            bx.append(&img);
        }
        let l = gtk::Label::new(Some(caption));
        bx.append(&l);
        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&bx));
        self.suppress.set(true);
        self.items.borrow_mut().push(TagItem { kind, count, child: child.clone() });
        self.flow.insert(&child, -1);
        if selected {
            self.flow.select_child(&child);
        }
        self.suppress.set(false);
        self.items.borrow().len() - 1
    }

    pub fn len(&self) -> usize {
        self.items.borrow().len()
    }

    /// Selected items in display order.
    pub fn selected(&self) -> Vec<TagKind> {
        let mut v: Vec<(i32, TagKind)> = self
            .items
            .borrow()
            .iter()
            .filter(|i| i.child.is_selected())
            .map(|i| (i.child.index(), i.kind.clone()))
            .collect();
        v.sort_by_key(|x| x.0);
        v.into_iter().map(|x| x.1).collect()
    }

    /// Display indices (excluding the "All" item at 0) of the selected items.
    pub fn selected_display_indices(&self) -> Vec<usize> {
        self.items.borrow().iter().filter(|i| i.child.is_selected()).map(|i| i.child.index().max(0) as usize).collect()
    }

    pub fn select_display_index(&self, idx: usize) {
        if let Some(c) = self.flow.child_at_index(idx as i32) {
            self.flow.select_child(&c);
        }
    }
}
