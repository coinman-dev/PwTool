//! `TListView` in report style for custom controls (`Form::set_painter`):
//! columns with headers (click = sort, drag = width), rows with an icon in
//! the first column, single or multiple selection with mouse and keyboard,
//! context menu position, drag & drop of a row onto another row. Only the
//! visible rows are drawn, so long lists stay fast.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Align2, Color32, CursorIcon, FontId, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::Icon;

/// One row: cell texts, icon of the first column, bold header row.
#[derive(Clone, Debug)]
pub struct LvRow<K> {
    pub key: K,
    pub cells: Vec<String>,
    pub icon: Option<Icon>,
    /// Group header (bold, first cell only).
    pub header: bool,
}

impl<K> LvRow<K> {
    pub fn new(key: K, cells: Vec<String>) -> Self {
        LvRow { key, cells, icon: None, header: false }
    }
}

/// What happened in the list (taken by the window logic with
/// [`ListView::take_events`]).
#[derive(Clone, Debug, PartialEq)]
pub enum LvEvent {
    /// The selection changed.
    Selection,
    DblClick(usize),
    /// Right click at a position (the row under it is selected first).
    Context(Pos2),
    /// Click on the header of column `n` (index among the shown columns).
    HeaderClick(usize),
    /// The selected row was dragged onto row `n`.
    Drop(usize),
}

struct State<K> {
    columns: Vec<(String, f32)>,
    rows: Vec<LvRow<K>>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    cursor: Option<usize>,
    multi: bool,
    show_header: bool,
    sort: Option<(usize, bool)>,
    scroll_to: Option<usize>,
    font: Option<FontId>,
    color: Option<Color32>,
    events: Vec<LvEvent>,
    can_drag: bool,
    drag_from: Option<usize>,
    enabled: bool,
    focus_request: bool,
    has_focus: bool,
    /// Width of a single column filling the list (no header).
    fill_width: bool,
}

pub struct ListView<K> {
    st: RefCell<State<K>>,
    id: egui::Id,
}

impl<K: Clone + PartialEq + 'static> ListView<K> {
    pub fn new(id: &str, multi: bool) -> Rc<Self> {
        Rc::new(ListView {
            st: RefCell::new(State {
                columns: Vec::new(),
                rows: Vec::new(),
                selected: Vec::new(),
                anchor: None,
                cursor: None,
                multi,
                show_header: true,
                sort: None,
                scroll_to: None,
                font: None,
                color: None,
                events: Vec::new(),
                can_drag: false,
                drag_from: None,
                enabled: true,
                focus_request: false,
                has_focus: false,
                fill_width: false,
            }),
            id: egui::Id::new(("listview", id.to_string())),
        })
    }

    // ------------------------------------------------------------ setup

    /// Columns: title and width (points).
    pub fn set_columns(&self, cols: Vec<(String, f32)>) {
        self.st.borrow_mut().columns = cols;
    }

    pub fn column_widths(&self) -> Vec<f32> {
        self.st.borrow().columns.iter().map(|c| c.1).collect()
    }

    pub fn set_show_header(&self, on: bool) {
        self.st.borrow_mut().show_header = on;
    }

    /// The last column takes the remaining width.
    pub fn set_fill_width(&self, on: bool) {
        self.st.borrow_mut().fill_width = on;
    }

    /// Sort indicator in the header: shown column index, ascending.
    pub fn set_sort_indicator(&self, col: Option<usize>, ascending: bool) {
        self.st.borrow_mut().sort = col.map(|c| (c, ascending));
    }

    pub fn set_font(&self, font: Option<FontId>) {
        self.st.borrow_mut().font = font;
    }

    pub fn set_text_color(&self, color: Option<Color32>) {
        self.st.borrow_mut().color = color;
    }

    /// Rows may be dragged (checked when a drag starts).
    pub fn set_can_drag(&self, on: bool) {
        self.st.borrow_mut().can_drag = on;
    }

    pub fn set_enabled(&self, on: bool) {
        self.st.borrow_mut().enabled = on;
    }

    /// The list had the keyboard focus in the last frame.
    pub fn has_focus(&self) -> bool {
        self.st.borrow().has_focus
    }

    pub fn focus(&self) {
        self.st.borrow_mut().focus_request = true;
    }

    // ------------------------------------------------------------- rows

    /// Replace all rows (the selection is cleared).
    pub fn set_rows(&self, rows: Vec<LvRow<K>>) {
        let mut st = self.st.borrow_mut();
        st.selected = vec![false; rows.len()];
        st.rows = rows;
        st.anchor = None;
        st.cursor = None;
    }

    /// Replace the rows keeping the selection of the keys.
    pub fn set_rows_keep_selection(&self, rows: Vec<LvRow<K>>) {
        let keys: Vec<K> = self.selected_keys();
        let cursor_key = {
            let st = self.st.borrow();
            st.cursor.and_then(|c| st.rows.get(c)).map(|r| r.key.clone())
        };
        let mut st = self.st.borrow_mut();
        st.selected = rows.iter().map(|r| keys.contains(&r.key)).collect();
        st.cursor = cursor_key.and_then(|k| rows.iter().position(|r| r.key == k));
        st.anchor = st.cursor;
        st.rows = rows;
    }

    pub fn replace_row(&self, i: usize, row: LvRow<K>) {
        let mut st = self.st.borrow_mut();
        if i < st.rows.len() {
            st.rows[i] = row;
        }
    }

    pub fn append_row(&self, row: LvRow<K>) {
        let mut st = self.st.borrow_mut();
        st.rows.push(row);
        st.selected.push(false);
    }

    pub fn clear(&self) {
        self.set_rows(Vec::new());
    }

    pub fn len(&self) -> usize {
        self.st.borrow().rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn rows(&self) -> Vec<LvRow<K>> {
        self.st.borrow().rows.clone()
    }

    pub fn key_at(&self, i: usize) -> Option<K> {
        self.st.borrow().rows.get(i).map(|r| r.key.clone())
    }

    pub fn index_of(&self, key: &K) -> Option<usize> {
        self.st.borrow().rows.iter().position(|r| r.key == *key)
    }

    // -------------------------------------------------------- selection

    pub fn sel_count(&self) -> usize {
        self.st.borrow().selected.iter().filter(|x| **x).count()
    }

    pub fn first_selected(&self) -> Option<usize> {
        self.st.borrow().selected.iter().position(|x| *x)
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.st.borrow().selected.get(i).copied().unwrap_or(false)
    }

    pub fn selected_indices(&self) -> Vec<usize> {
        self.st.borrow().selected.iter().enumerate().filter(|(_, x)| **x).map(|(i, _)| i).collect()
    }

    pub fn selected_keys(&self) -> Vec<K> {
        let st = self.st.borrow();
        st.rows.iter().zip(st.selected.iter()).filter(|(_, s)| **s).map(|(r, _)| r.key.clone()).collect()
    }

    /// Select only row `i` (no event: the logic changes the selection).
    pub fn select_only(&self, i: usize) {
        let mut st = self.st.borrow_mut();
        if i < st.rows.len() {
            st.selected.iter_mut().for_each(|x| *x = false);
            st.selected[i] = true;
            st.anchor = Some(i);
            st.cursor = Some(i);
        }
    }

    /// Add row `i` to the selection.
    pub fn select_add(&self, i: usize) {
        let mut st = self.st.borrow_mut();
        if i < st.rows.len() {
            st.selected[i] = true;
        }
    }

    pub fn clear_selection(&self) {
        let mut st = self.st.borrow_mut();
        st.selected.iter_mut().for_each(|x| *x = false);
    }

    pub fn select_all(&self) {
        let mut st = self.st.borrow_mut();
        if st.multi {
            st.selected.iter_mut().for_each(|x| *x = true);
        }
    }

    pub fn scroll_to(&self, i: usize) {
        self.st.borrow_mut().scroll_to = Some(i);
    }

    pub fn take_events(&self) -> Vec<LvEvent> {
        std::mem::take(&mut self.st.borrow_mut().events)
    }

    // ------------------------------------------------------------ drawing

    fn click_row(st: &mut State<K>, i: usize, mods: egui::Modifiers) {
        let before = st.selected.clone();
        if st.multi && mods.command {
            st.selected[i] = !st.selected[i];
            st.anchor = Some(i);
        } else if let (true, true, Some(a)) = (st.multi, mods.shift, st.anchor) {
            let (lo, hi) = (a.min(i), a.max(i));
            for (j, s) in st.selected.iter_mut().enumerate() {
                *s = (lo..=hi).contains(&j);
            }
        } else {
            st.selected.iter_mut().for_each(|x| *x = false);
            st.selected[i] = true;
            st.anchor = Some(i);
        }
        st.cursor = Some(i);
        if st.selected != before {
            st.events.push(LvEvent::Selection);
        }
    }

    /// Keyboard navigation of the focused list.
    fn keys(st: &mut State<K>, ui: &Ui, page: usize) {
        let n = st.rows.len();
        if n == 0 {
            return;
        }
        let (mods, pressed) = ui.input(|i| {
            let keys = [Key::ArrowUp, Key::ArrowDown, Key::Home, Key::End, Key::PageUp, Key::PageDown, Key::A];
            (i.modifiers, keys.iter().copied().filter(|k| i.key_pressed(*k)).collect::<Vec<Key>>())
        });
        for k in pressed {
            if k == Key::A {
                if mods.command && st.multi {
                    let before = st.selected.clone();
                    st.selected.iter_mut().for_each(|x| *x = true);
                    if st.selected != before {
                        st.events.push(LvEvent::Selection);
                    }
                }
                continue;
            }
            let cur = st.cursor.or(st.selected.iter().position(|x| *x));
            let next = match (k, cur) {
                (Key::ArrowUp, Some(c)) => c.saturating_sub(1),
                (Key::ArrowDown, Some(c)) => (c + 1).min(n - 1),
                (Key::PageUp, Some(c)) => c.saturating_sub(page),
                (Key::PageDown, Some(c)) => (c + page).min(n - 1),
                (Key::Home, _) => 0,
                (Key::End, _) => n - 1,
                (_, None) => 0,
                _ => continue,
            };
            let m = egui::Modifiers { command: false, ctrl: false, mac_cmd: false, ..mods };
            Self::click_row(st, next, m);
            st.scroll_to = Some(next);
        }
    }

    /// Draw the list into `rect` (call from the painter of the control).
    pub fn paint(&self, ui: &mut Ui, rect: Rect) {
        let mut st = self.st.borrow_mut();
        let st = &mut *st;
        let enabled = st.enabled && ui.is_enabled();
        let visuals = ui.visuals().clone();
        let stroke = visuals.widgets.noninteractive.bg_stroke;
        ui.painter().rect_filled(rect, 2.0, visuals.extreme_bg_color);
        let font = st.font.clone().unwrap_or_else(|| egui::TextStyle::Body.resolve(ui.style()));
        let text_color = if enabled { st.color.unwrap_or(visuals.text_color()) } else { visuals.weak_text_color() };
        let row_h = ui.fonts_mut(|f| f.row_height(&font)).max(16.0) + 3.0;
        let inner = rect.shrink(1.0);
        let head_h = if st.show_header && !st.columns.is_empty() { ui.text_style_height(&egui::TextStyle::Body) + 6.0 } else { 0.0 };
        if st.fill_width {
            let fixed: f32 = st.columns.iter().rev().skip(1).map(|c| c.1).sum();
            if let Some(last) = st.columns.last_mut() {
                last.1 = (inner.width() - fixed - 2.0).max(30.0);
            }
        }
        let total_w: f32 = st.columns.iter().map(|c| c.1).sum::<f32>().max(inner.width());

        // keyboard focus
        let focus_resp = ui.interact(rect, self.id.with("focus"), Sense::focusable_noninteractive());
        if std::mem::take(&mut st.focus_request) {
            focus_resp.request_focus();
        }
        let has_focus = focus_resp.has_focus();
        st.has_focus = has_focus;
        if has_focus {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    focus_resp.id,
                    egui::EventFilter { vertical_arrows: true, horizontal_arrows: false, tab: false, escape: false },
                )
            });
            if enabled {
                let page = ((inner.height() - head_h) / row_h).max(1.0) as usize;
                Self::keys(st, ui, page);
            }
        }

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(inner.intersect(ui.clip_rect()));
        if !enabled {
            child.disable();
        }
        let scroll_to = st.scroll_to.take();
        let mut clicked_row: Option<(usize, egui::Modifiers)> = None;
        let mut context: Option<(Option<usize>, Pos2)> = None;
        let mut dbl: Option<usize> = None;
        let mut header_click: Option<usize> = None;
        let mut hover_row: Option<usize> = None;
        let mut drag_started: Option<usize> = None;
        egui::ScrollArea::horizontal().id_salt(self.id.with("h")).auto_shrink(false).show(&mut child, |ui| {
            ui.set_min_width(total_w);
            // header
            if head_h > 0.0 {
                let (hr, _) = ui.allocate_exact_size(vec2(total_w, head_h), Sense::hover());
                ui.painter().rect_filled(hr, 0.0, visuals.faint_bg_color);
                let mut x = hr.min.x;
                let hfont = egui::TextStyle::Body.resolve(ui.style());
                for ci in 0..st.columns.len() {
                    let w = st.columns[ci].1;
                    let cr = Rect::from_min_size(pos2(x, hr.min.y), vec2(w, head_h));
                    let resp = ui.interact(cr.shrink2(vec2(3.0, 0.0)), self.id.with(("head", ci)), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(cr, 0.0, visuals.widgets.hovered.weak_bg_fill);
                    }
                    if resp.clicked() {
                        header_click = Some(ci);
                    }
                    let mut title = st.columns[ci].0.clone();
                    if let Some((sc, asc)) = st.sort {
                        if sc == ci {
                            title.push_str(if asc { " ▲" } else { " ▼" });
                        }
                    }
                    ui.painter().with_clip_rect(cr.shrink(1.0)).text(
                        pos2(cr.min.x + 5.0, cr.center().y),
                        Align2::LEFT_CENTER,
                        title,
                        hfont.clone(),
                        visuals.text_color(),
                    );
                    ui.painter().vline(cr.max.x - 0.5, cr.y_range(), stroke);
                    // width by dragging the right edge
                    let edge = Rect::from_center_size(pos2(cr.max.x, cr.center().y), vec2(6.0, head_h));
                    let er = ui.interact(edge, self.id.with(("edge", ci)), Sense::drag());
                    if er.hovered() || er.dragged() {
                        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    }
                    if er.dragged() {
                        st.columns[ci].1 = (w + er.drag_delta().x).max(20.0);
                    }
                    x += w;
                }
                ui.painter().hline(hr.x_range(), hr.max.y - 0.5, stroke);
            }
            let body_h = (inner.height() - head_h).max(row_h);
            let mut va = egui::ScrollArea::vertical().id_salt(self.id.with("v")).auto_shrink(false).max_height(body_h);
            if let Some(i) = scroll_to {
                // keep the row visible
                let off = ui.ctx().data(|d| d.get_temp::<f32>(self.id.with("voff"))).unwrap_or(0.0);
                let top = i as f32 * row_h;
                if top < off {
                    va = va.vertical_scroll_offset(top);
                } else if top + row_h > off + body_h {
                    va = va.vertical_scroll_offset(top + row_h - body_h);
                }
            }
            let out = va.show_rows(ui, row_h, st.rows.len(), |ui, range| {
                let row_w = total_w.max(ui.available_width());
                for i in range {
                    let (rr, resp) = ui.allocate_exact_size(vec2(row_w, row_h), Sense::click_and_drag());
                    let sel = st.selected[i];
                    if sel {
                        let fill = if has_focus { visuals.selection.bg_fill } else { visuals.selection.bg_fill.gamma_multiply(0.6) };
                        ui.painter().rect_filled(rr, 0.0, fill);
                    } else if resp.hovered() && enabled {
                        ui.painter().rect_filled(rr, 0.0, visuals.widgets.hovered.weak_bg_fill);
                    }
                    if st.cursor == Some(i) && has_focus {
                        ui.painter().rect_stroke(rr.shrink(0.5), 0.0, Stroke::new(1.0, visuals.selection.stroke.color), StrokeKind::Inside);
                    }
                    let row = &st.rows[i];
                    let color = if sel { visuals.selection.stroke.color } else { text_color };
                    let mut x = rr.min.x;
                    for (ci, (_, w)) in st.columns.iter().enumerate() {
                        let cr = Rect::from_min_size(pos2(x, rr.min.y), vec2(*w, row_h));
                        let mut tx = cr.min.x + 5.0;
                        if ci == 0 {
                            if let Some(icon) = &row.icon {
                                if let Some(t) = super::render::icon_texture(ui.ctx(), icon) {
                                    let s = icon.size.min(row_h - 2.0);
                                    let ir = Rect::from_min_size(pos2(tx - 2.0, cr.center().y - s / 2.0), vec2(s, s));
                                    ui.painter().image(t.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                                    tx += s + 2.0;
                                }
                            }
                        }
                        let text = if row.header && ci > 0 { "" } else { row.cells.get(ci).map(|s| s.as_str()).unwrap_or("") };
                        if !text.is_empty() {
                            let f = if row.header {
                                FontId::new(font.size, egui::FontFamily::Name(super::fonts::BOLD.into()))
                            } else {
                                font.clone()
                            };
                            let galley = ui.painter().layout_job(egui::text::LayoutJob {
                                wrap: egui::text::TextWrapping::truncate_at_width((cr.max.x - tx - 3.0).max(1.0)),
                                ..egui::text::LayoutJob::simple_singleline(text.replace(['\r', '\n'], " "), f, color)
                            });
                            let y = cr.center().y - galley.size().y / 2.0;
                            ui.painter().with_clip_rect(cr).galley(pos2(tx, y), galley, color);
                        }
                        x += w;
                    }
                    if !enabled {
                        continue;
                    }
                    if resp.clicked() {
                        clicked_row = Some((i, ui.input(|inp| inp.modifiers)));
                    }
                    if resp.double_clicked() {
                        dbl = Some(i);
                    }
                    if resp.secondary_clicked() {
                        context = Some((Some(i), ui.input(|inp| inp.pointer.interact_pos()).unwrap_or(rr.center())));
                    }
                    if resp.drag_started() && st.can_drag && sel {
                        drag_started = Some(i);
                    }
                    if resp.contains_pointer() {
                        hover_row = Some(i);
                    }
                }
                // empty area below the rows: context menu, click clears
                let rest = ui.available_rect_before_wrap();
                if rest.height() > 0.0 && enabled {
                    let r = ui.interact(rest, self.id.with("empty"), Sense::click());
                    if r.secondary_clicked() {
                        context = Some((None, ui.input(|inp| inp.pointer.interact_pos()).unwrap_or(rest.center())));
                    }
                }
            });
            ui.ctx().data_mut(|d| d.insert_temp(self.id.with("voff"), out.state.offset.y));
        });
        if clicked_row.is_some() || context.is_some() || header_click.is_some() {
            focus_resp.request_focus();
        }
        if let Some((i, m)) = clicked_row {
            Self::click_row(st, i, m);
        }
        if let Some(i) = dbl {
            st.events.push(LvEvent::DblClick(i));
        }
        if let Some(c) = header_click {
            st.events.push(LvEvent::HeaderClick(c));
        }
        if let Some((row, pos)) = context {
            if let Some(i) = row {
                if !st.selected[i] {
                    Self::click_row(st, i, egui::Modifiers::NONE);
                }
            }
            st.events.push(LvEvent::Context(pos));
        }
        if let Some(i) = drag_started {
            st.drag_from = Some(i);
        }
        if let Some(from) = st.drag_from {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            if ui.input(|i| i.pointer.any_released()) {
                st.drag_from = None;
                if let Some(to) = hover_row.filter(|&t| t != from) {
                    st.events.push(LvEvent::Drop(to));
                }
            }
        }
        ui.painter().rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
        if has_focus {
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, visuals.selection.bg_fill), StrokeKind::Inside);
        }
    }
}
