//! Font selection dialog (the original uses the Windows `TFontDialog`):
//! family list with a filter, size, bold/italic and a sample.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Key, Ui, ViewportId, vec2};
use pwtool_core::language::trl;

use crate::vcl::fonts::{self, FontSpec};
use crate::windows::{self, Content, Window};

const SIZES: [i32; 16] = [6, 7, 8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36];
const SAMPLE: &str = "AaBbYyZz 0123456789\nIl1| O0Q !@#$%&*";

struct State {
    families: Vec<(String, bool)>,
    filter: String,
    family: String,
    size: i32,
    bold: bool,
    italic: bool,
    mono_only: bool,
    result: Option<Option<FontSpec>>,
    scroll_to_selection: bool,
}

struct FontDialog {
    id: ViewportId,
    state: RefCell<State>,
    done: async_channel::Sender<()>,
}

impl FontDialog {
    fn finish(&self, r: Option<FontSpec>) {
        self.state.borrow_mut().result = Some(r);
        windows::close(self.id);
        let _ = self.done.try_send(());
    }
}

impl Content for FontDialog {
    fn ui(&self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut ok = false;
        let mut cancel = false;
        {
            let mut st = self.state.borrow_mut();
            let st = &mut *st;
            egui::Frame::central_panel(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(trl("Filter:"));
                    ui.add(egui::TextEdit::singleline(&mut st.filter).desired_width(160.0));
                    ui.checkbox(&mut st.mono_only, trl("Monospaced fonts only"));
                });
                ui.add_space(4.0);
                let list_h = ui.available_height() - 150.0;
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            egui::ScrollArea::vertical().id_salt("families").max_height(list_h.max(80.0)).auto_shrink(false).show(ui, |ui| {
                                let filter = st.filter.to_lowercase();
                                for (name, mono) in &st.families {
                                    if (st.mono_only && !mono) || (!filter.is_empty() && !name.to_lowercase().contains(&filter)) {
                                        continue;
                                    }
                                    let sel = *name == st.family;
                                    let r = ui.selectable_label(sel, name);
                                    if sel && st.scroll_to_selection {
                                        r.scroll_to_me(Some(egui::Align::Center));
                                    }
                                    if r.clicked() {
                                        st.family = name.clone();
                                    }
                                    if r.double_clicked() {
                                        ok = true;
                                    }
                                }
                                st.scroll_to_selection = false;
                            });
                        });
                    });
                    ui.vertical(|ui| {
                        ui.label(trl("Size:"));
                        egui::ComboBox::from_id_salt("size").selected_text(st.size.to_string()).width(70.0).show_ui(ui, |ui| {
                            for s in SIZES {
                                ui.selectable_value(&mut st.size, s, s.to_string());
                            }
                        });
                        ui.add(egui::DragValue::new(&mut st.size).range(4..=72).suffix(" pt"));
                        ui.add_space(8.0);
                        ui.checkbox(&mut st.bold, trl("Bold"));
                        ui.checkbox(&mut st.italic, trl("Italic"));
                    });
                });
                ui.add_space(6.0);
                ui.label(trl("Sample"));
                fonts::set_preview_font(&ctx, (!st.family.is_empty()).then_some((st.family.as_str(), st.bold, st.italic)));
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_size(vec2(ui.available_width(), 70.0));
                    let pt = st.size.max(4) as f32 * 4.0 / 3.0;
                    ui.label(egui::RichText::new(SAMPLE).font(egui::FontId::new(pt.min(40.0), egui::FontFamily::Name(fonts::PREVIEW.into()))));
                });
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(format!("  {}  ", trl("Cancel"))).clicked() {
                        cancel = true;
                    }
                    if ui.add_enabled(!st.family.is_empty(), egui::Button::new(format!("  {}  ", trl("OK")))).clicked() {
                        ok = true;
                    }
                });
            });
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
            if ui.input(|i| i.key_pressed(Key::Enter)) && !st.family.is_empty() {
                ok = true;
            }
        }
        if ok {
            let st = self.state.borrow();
            let style = i32::from(st.bold) | (i32::from(st.italic) << 1);
            let spec = FontSpec { name: st.family.clone(), size: st.size, style, color: "clWindowText".into() };
            drop(st);
            self.finish(Some(spec));
        } else if cancel {
            self.finish(None);
        }
    }
}

/// Let the user choose a font; `mono_only` preselects monospaced fonts.
pub async fn choose_font(parent: ViewportId, current: Option<&FontSpec>, mono_only: bool) -> Option<FontSpec> {
    let (tx, rx) = async_channel::bounded(1);
    let id = ViewportId::from_hash_of("FontDialog");
    let size = (crate::vcl::fonts::DEFAULT_SIZE * 0.75).round() as i32;
    let st = State {
        families: fonts::families(),
        filter: String::new(),
        family: current.map(|c| c.name.clone()).unwrap_or_default(),
        size: current.map(|c| c.size.abs()).unwrap_or(size),
        bold: current.is_some_and(|c| c.style & 1 != 0),
        italic: current.is_some_and(|c| c.style & 2 != 0),
        mono_only: mono_only && current.is_none(),
        result: None,
        scroll_to_selection: true,
    };
    let dlg = Rc::new(FontDialog { id, state: RefCell::new(st), done: tx.clone() });
    let w = Window::new("FontDialog", &trl("Font"), dlg.clone(), parent, vec2(480.0, 470.0)).modal(true).resizable(true, vec2(400.0, 380.0));
    *w.on_close.borrow_mut() = Some(Box::new(move || {
        let _ = tx.try_send(());
        true
    }));
    windows::open(Rc::new(w));
    let _ = rx.recv().await;
    fonts::set_preview_font(&windows::ctx(), None);
    dlg.state.borrow_mut().result.take().flatten()
}
