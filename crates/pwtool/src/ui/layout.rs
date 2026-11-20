//! `VclBox`: a container that positions its children like the VCL does —
//! absolute design coordinates, `Anchors` (left/top/right/bottom) and
//! `Align` (top/bottom/left/right/client) — so that the windows keep the
//! layout of the original forms when they are resized.

use std::cell::{Cell, RefCell};

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

pub const AK_LEFT: u8 = 1;
pub const AK_TOP: u8 = 2;
pub const AK_RIGHT: u8 = 4;
pub const AK_BOTTOM: u8 = 8;

/// `TAlign`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Top,
    Bottom,
    Left,
    Right,
    Client,
}

#[derive(Clone)]
pub struct Item {
    pub widget: gtk::Widget,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub anchors: u8,
    pub align: Align,
    /// `TLabel.AutoSize`: width follows the text; the value is the
    /// horizontal alignment (0 left, 0.5 centre, 1 right edge kept).
    pub autosize: Option<f32>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VclBox {
        pub items: RefCell<Vec<Item>>,
        pub design_w: Cell<i32>,
        pub design_h: Cell<i32>,
        pub min_w: Cell<i32>,
        pub min_h: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VclBox {
        const NAME: &'static str = "PwToolVclBox";
        type Type = super::VclBox;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for VclBox {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for VclBox {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                (self.min_w.get(), self.design_w.get().max(self.min_w.get()), -1, -1)
            } else {
                (self.min_h.get(), self.design_h.get().max(self.min_h.get()), -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let dx = width - self.design_w.get();
            let dy = height - self.design_h.get();
            let items = self.items.borrow();
            // client rectangle left after aligned controls
            let (mut cx, mut cy, mut cw, mut ch) = (0, 0, width, height);
            let alloc = |it: &Item, x: i32, y: i32, w: i32, h: i32| {
                let (mw, _, _, _) = it.widget.measure(gtk::Orientation::Horizontal, -1);
                let w = w.max(mw).max(0);
                let (mh, _, _, _) = it.widget.measure(gtk::Orientation::Vertical, w);
                let h = h.max(mh).max(0);
                it.widget.size_allocate(&gtk::Allocation::new(x, y, w, h), -1);
            };
            for kind in [Align::Top, Align::Bottom, Align::Left, Align::Right, Align::Client] {
                for it in items.iter().filter(|i| i.align == kind && i.widget.is_visible()) {
                    match kind {
                        Align::Top => {
                            alloc(it, cx, cy, cw, it.h);
                            cy += it.h;
                            ch -= it.h;
                        }
                        Align::Bottom => {
                            alloc(it, cx, cy + ch - it.h, cw, it.h);
                            ch -= it.h;
                        }
                        Align::Left => {
                            alloc(it, cx, cy, it.w, ch);
                            cx += it.w;
                            cw -= it.w;
                        }
                        Align::Right => {
                            alloc(it, cx + cw - it.w, cy, it.w, ch);
                            cw -= it.w;
                        }
                        Align::Client => alloc(it, cx, cy, cw, ch),
                        Align::None => {}
                    }
                }
            }
            for it in items.iter().filter(|i| i.align == Align::None) {
                if !it.widget.is_visible() {
                    continue;
                }
                let (mut x, mut w) = (it.x, it.w);
                match (it.anchors & AK_LEFT != 0, it.anchors & AK_RIGHT != 0) {
                    (true, true) => w += dx,
                    (false, true) => x += dx,
                    (false, false) => x += dx / 2,
                    _ => {}
                }
                let (mut y, mut h) = (it.y, it.h);
                match (it.anchors & AK_TOP != 0, it.anchors & AK_BOTTOM != 0) {
                    (true, true) => h += dy,
                    (false, true) => y += dy,
                    (false, false) => y += dy / 2,
                    _ => {}
                }
                if let Some(xa) = it.autosize {
                    let (_, nat, _, _) = it.widget.measure(gtk::Orientation::Horizontal, -1);
                    if nat > w || xa != 0.0 {
                        x += ((w - nat) as f32 * xa) as i32;
                        w = nat;
                    } else {
                        w = w.max(nat);
                    }
                }
                alloc(it, x, y, w, h);
            }
        }
    }
}

glib::wrapper! {
    pub struct VclBox(ObjectSubclass<imp::VclBox>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VclBox {
    /// New container with the given design client size (already scaled).
    pub fn new(design_w: i32, design_h: i32) -> Self {
        let b: VclBox = glib::Object::new();
        b.imp().design_w.set(design_w);
        b.imp().design_h.set(design_h);
        b
    }

    /// Minimum size (form `Constraints`), scaled.
    pub fn set_min_size(&self, w: i32, h: i32) {
        self.imp().min_w.set(w);
        self.imp().min_h.set(h);
        self.queue_resize();
    }

    pub fn design_size(&self) -> (i32, i32) {
        (self.imp().design_w.get(), self.imp().design_h.get())
    }

    /// Add a child at a design position.
    pub fn put(&self, widget: &impl IsA<gtk::Widget>, x: i32, y: i32, w: i32, h: i32, anchors: u8, align: Align) {
        let widget = widget.clone().upcast::<gtk::Widget>();
        widget.set_parent(self);
        self.imp().items.borrow_mut().push(Item { widget, x, y, w, h, anchors, align, autosize: None });
        self.queue_resize();
    }

    /// Add an auto-sizing label (see [`Item::autosize`]).
    pub fn put_autosize(&self, widget: &impl IsA<gtk::Widget>, x: i32, y: i32, w: i32, h: i32, anchors: u8, xalign: f32) {
        let widget = widget.clone().upcast::<gtk::Widget>();
        widget.set_parent(self);
        self.imp().items.borrow_mut().push(Item { widget, x, y, w, h, anchors, align: Align::None, autosize: Some(xalign) });
        self.queue_resize();
    }

    /// Change the design rectangle of a child (e.g. after hiding a control
    /// the original moves others).
    pub fn move_child(&self, widget: &impl IsA<gtk::Widget>, x: i32, y: i32, w: i32, h: i32) {
        let widget = widget.as_ref();
        for it in self.imp().items.borrow_mut().iter_mut() {
            if &it.widget == widget {
                it.x = x;
                it.y = y;
                it.w = w;
                it.h = h;
            }
        }
        self.queue_resize();
    }

    /// Remove a child (e.g. to move it into another container).
    pub fn remove(&self, widget: &impl IsA<gtk::Widget>) {
        let widget = widget.as_ref();
        self.imp().items.borrow_mut().retain(|i| &i.widget != widget);
        if widget.parent().as_ref() == Some(self.upcast_ref()) {
            widget.unparent();
        }
        self.queue_resize();
    }

    pub fn child_rect(&self, widget: &impl IsA<gtk::Widget>) -> Option<(i32, i32, i32, i32)> {
        let widget = widget.as_ref();
        self.imp().items.borrow().iter().find(|i| &i.widget == widget).map(|i| (i.x, i.y, i.w, i.h))
    }
}
