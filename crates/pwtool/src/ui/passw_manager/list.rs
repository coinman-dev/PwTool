//! Lists of the password manager: the keys of the entry list (`DbView`)
//! and the tag list (`TagView`), a flowing row of selectable tags.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Rect, Sense, Ui, vec2};

use crate::vcl::Icon;

/// Key of a row of the entry list: the "<New Entry>" row or an entry id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    New,
    Entry(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagKind {
    All,
    Tag(String),
    Untagged,
}

struct TagItem {
    kind: TagKind,
    caption: String,
    count: u32,
    selected: bool,
}

struct TagState {
    items: Vec<TagItem>,
    icons: [Option<Icon>; 3],
    sort: (i32, i32),
    changed: bool,
    enabled: bool,
    anchor: Option<usize>,
}

pub struct TagList {
    st: RefCell<TagState>,
}

impl TagList {
    /// `icons`: tag, "All", "Untagged".
    pub fn new(icons: [Option<Icon>; 3]) -> Rc<Self> {
        Rc::new(TagList { st: RefCell::new(TagState { items: Vec::new(), icons, sort: (0, 1), changed: false, enabled: true, anchor: None }) })
    }

    fn compare(sort: (i32, i32), a: &TagItem, b: &TagItem) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let (by, factor) = sort;
        match (&a.kind, &b.kind) {
            (TagKind::All, _) => Less,
            (_, TagKind::All) => Greater,
            (TagKind::Untagged, _) => Greater,
            (_, TagKind::Untagged) => Less,
            (TagKind::Tag(x), TagKind::Tag(y)) => {
                let o = if by == 0 { super::helpers::wcsicmp(x, y) } else { a.count.cmp(&b.count) };
                if factor < 0 { o.reverse() } else { o }
            }
        }
    }

    fn resort(st: &mut TagState) {
        let sort = st.sort;
        st.items.sort_by(|a, b| Self::compare(sort, a, b));
    }

    pub fn set_sort(&self, by: i32, factor: i32) {
        let mut st = self.st.borrow_mut();
        st.sort = (by, factor);
        Self::resort(&mut st);
    }

    pub fn set_enabled(&self, on: bool) {
        self.st.borrow_mut().enabled = on;
    }

    pub fn clear(&self) {
        let mut st = self.st.borrow_mut();
        st.items.clear();
        st.anchor = None;
    }

    pub fn add(&self, kind: TagKind, caption: &str, count: u32, selected: bool) {
        let mut st = self.st.borrow_mut();
        st.items.push(TagItem { kind, caption: caption.to_string(), count, selected });
        Self::resort(&mut st);
    }

    pub fn len(&self) -> usize {
        self.st.borrow().items.len()
    }

    /// Selected items in display order.
    pub fn selected(&self) -> Vec<TagKind> {
        self.st.borrow().items.iter().filter(|i| i.selected).map(|i| i.kind.clone()).collect()
    }

    pub fn selected_display_indices(&self) -> Vec<usize> {
        self.st.borrow().items.iter().enumerate().filter(|(_, i)| i.selected).map(|(n, _)| n).collect()
    }

    pub fn select_display_index(&self, idx: usize) {
        if let Some(i) = self.st.borrow_mut().items.get_mut(idx) {
            i.selected = true;
        }
    }

    /// Whether the user changed the selection since the last call.
    pub fn take_changed(&self) -> bool {
        std::mem::take(&mut self.st.borrow_mut().changed)
    }

    pub fn paint(&self, ui: &mut Ui, rect: Rect) {
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let v = ui.visuals().clone();
        ui.painter().rect_filled(rect, 2.0, v.extreme_bg_color);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(3.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        if !st.enabled {
            child.disable();
        }
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        egui::ScrollArea::vertical().id_salt("tagview").auto_shrink(false).show(&mut child, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 2.0);
                for (n, it) in st.items.iter().enumerate() {
                    let icon = match it.kind {
                        TagKind::Tag(_) => &st.icons[0],
                        TagKind::All => &st.icons[1],
                        TagKind::Untagged => &st.icons[2],
                    };
                    let resp = ui
                        .scope(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            let tex = icon.as_ref().and_then(|i| crate::vcl::render::icon_texture(ui.ctx(), i).map(|t| (t, i.size)));
                            let mut b = match tex {
                                Some((t, s)) => egui::Button::image_and_text(egui::Image::new(&t).fit_to_exact_size(vec2(s, s)), &it.caption),
                                None => egui::Button::new(&it.caption),
                            };
                            b = b.selected(it.selected).frame_when_inactive(false);
                            ui.add(b)
                        })
                        .inner;
                    if resp.clicked() {
                        clicked = Some((n, ui.input(|i| i.modifiers)));
                    }
                }
            });
        });
        if let Some((n, mods)) = clicked {
            if mods.command {
                st.items[n].selected = !st.items[n].selected;
                st.anchor = Some(n);
            } else if let (true, Some(a)) = (mods.shift, st.anchor) {
                let (lo, hi) = (a.min(n), a.max(n));
                for (j, it) in st.items.iter_mut().enumerate() {
                    it.selected = (lo..=hi).contains(&j);
                }
            } else {
                for (j, it) in st.items.iter_mut().enumerate() {
                    it.selected = j == n;
                }
                st.anchor = Some(n);
            }
            st.changed = true;
        }
        let _ = ui.interact(rect, ui.id().with("tagview-bg"), Sense::hover());
        ui.painter().rect_stroke(rect, 2.0, v.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    }
}
