//! Drawing a [`Form`] with egui: every control at its laid-out rectangle,
//! the main menu, popup menus, and the keyboard handling of a VCL form
//! (menu shortcuts, Alt+letter mnemonics, Enter for the default button,
//! Esc for the cancel button). Drawing only records [`Event`]s.

use std::cell::RefCell;
use std::collections::HashMap;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, FontFamily, FontId, Key, Modifiers, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextStyle,
    TextureHandle, Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::fonts::{BOLD, ITALIC};
use super::layout;
use super::model::{Align, Control, Event, FontStyle, Form, FormData, Icon, MenuItemState, MenuNode, State};

// ------------------------------------------------------------------- helpers

/// Caption without the `&` mnemonic markers (`&&` → `&`).
pub fn strip_amp(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '&' {
            if it.peek() == Some(&'&') {
                it.next();
                out.push('&');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Mnemonic letter of a caption (`"&File"` → `'f'`).
pub fn mnemonic_of(s: &str) -> Option<char> {
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '&' {
            match it.next() {
                Some('&') => {}
                Some(m) => return Some(m.to_lowercase().next().unwrap_or(m)),
                None => return None,
            }
        }
    }
    None
}

fn body_size(ui: &Ui) -> f32 {
    ui.style().text_styles.get(&TextStyle::Body).map_or(12.0, |f| f.size)
}

fn font_id(f: &FontStyle, body: f32) -> FontId {
    if let Some(fam) = f.family {
        let size = match (fam == super::fonts::PASSW).then(super::fonts::passw_size).flatten() {
            Some(s) => s,
            None => body * if f.scale > 0.0 { f.scale } else { 1.0 },
        };
        return FontId::new(size, FontFamily::Name(fam.into()));
    }
    let family = if f.mono {
        FontFamily::Monospace
    } else if f.bold {
        FontFamily::Name(BOLD.into())
    } else if f.italic {
        FontFamily::Name(ITALIC.into())
    } else {
        FontFamily::Proportional
    };
    FontId::new(body * if f.scale > 0.0 { f.scale } else { 1.0 }, family)
}

fn rich(text: &str, f: &FontStyle, body: f32) -> RichText {
    let mut r = RichText::new(strip_amp(text)).font(font_id(f, body));
    if let Some(c) = f.color {
        r = r.color(c);
    }
    if f.underline {
        r = r.underline();
    }
    r
}

/// VCL virtual key → egui key.
pub fn vk_to_key(vk: u16) -> Option<Key> {
    Some(match vk {
        0x30..=0x39 => [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9]
            [(vk - 0x30) as usize],
        0x41..=0x5a => Key::from_name(&((vk as u8) as char).to_string())?,
        0x60..=0x69 => [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9]
            [(vk - 0x60) as usize],
        0x70..=0x87 => Key::from_name(&format!("F{}", vk - 0x6f))?,
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x0d => Key::Enter,
        0x1b => Key::Escape,
        0x20 => Key::Space,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::ArrowLeft,
        0x26 => Key::ArrowUp,
        0x27 => Key::ArrowRight,
        0x28 => Key::ArrowDown,
        0x2d => Key::Insert,
        0x2e => Key::Delete,
        _ => return None,
    })
}

/// egui key → Windows virtual key code (digits and letters as on the
/// main keyboard).
pub fn key_to_vk(key: Key) -> Option<u16> {
    (0u16..=0xff).find(|&vk| vk_to_key(vk) == Some(key))
}

/// VCL `TShortCut` → egui modifiers and key.
pub fn shortcut_key(code: u16) -> Option<(Modifiers, Key)> {
    let mut m = Modifiers::NONE;
    if code & 0x2000 != 0 {
        m |= Modifiers::SHIFT;
    }
    if code & 0x4000 != 0 {
        m |= Modifiers::CTRL;
    }
    if code & 0x8000 != 0 {
        m |= Modifiers::ALT;
    }
    Some((m, vk_to_key(code & 0xff)?))
}

thread_local! {
    static TEXTURES: RefCell<HashMap<String, Option<TextureHandle>>> = RefCell::new(HashMap::new());
}

/// Texture of an icon of the original forms.
pub fn icon_texture(ctx: &egui::Context, icon: &Icon) -> Option<TextureHandle> {
    let key = format!("icon:{}/{}/{}", icon.form, icon.collection, icon.name);
    TEXTURES.with(|t| {
        t.borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| {
                let png = pwtool_app::icons::form_icon(&icon.form, &icon.collection, &icon.name)?;
                png_texture(ctx, &key, png)
            })
            .clone()
    })
}

/// Load a PNG as a texture.
pub fn png_texture(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<TextureHandle> {
    let img = pwtool_app::icons::decode_png(png)?;
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    Some(ctx.load_texture(name, ci, egui::TextureOptions::LINEAR))
}

fn image_texture(ctx: &egui::Context, key: &str, png: &[u8]) -> Option<TextureHandle> {
    TEXTURES.with(|t| t.borrow_mut().entry(key.to_string()).or_insert_with(|| png_texture(ctx, key, png)).clone())
}

/// A child `Ui` limited to `rect`.
fn at<R>(ui: &mut Ui, rect: Rect, enabled: bool, f: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        if !enabled {
            ui.disable();
        }
        ui.set_clip_rect(rect.expand(2.0).intersect(ui.clip_rect()));
        f(ui)
    })
    .inner
}

fn text_color(ui: &Ui, f: &FontStyle, enabled: bool) -> Color32 {
    if !enabled {
        ui.visuals().weak_text_color()
    } else {
        f.color.unwrap_or_else(|| ui.visuals().text_color())
    }
}

/// Frame of a read-only edit.
fn readonly_frame(ui: &Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(2.0)
        .inner_margin(egui::Margin::symmetric(4, 2))
}

/// Frame line of group boxes, panels and lists.
fn frame_stroke(ui: &Ui) -> Stroke {
    ui.visuals().widgets.noninteractive.bg_stroke
}

// ------------------------------------------------------------------- menus

/// Draw menu items; chosen items are added to `events`.
pub fn menu_items(ui: &mut Ui, nodes: &[MenuNode], items: &HashMap<String, MenuItemState>, events: &mut Vec<Event>) {
    let mut last_sep = true;
    let visible: Vec<&MenuNode> =
        nodes.iter().filter(|n| n.separator || items.get(&n.name).is_some_and(|s| s.visible)).collect();
    for (i, n) in visible.iter().enumerate() {
        if n.separator {
            // no separator at the start, the end or twice
            if !last_sep && visible[i + 1..].iter().any(|x| !x.separator) {
                ui.separator();
            }
            last_sep = true;
            continue;
        }
        last_sep = false;
        let Some(st) = items.get(&n.name) else { continue };
        let caption = strip_amp(&st.caption);
        let label = match (st.checked, st.radio) {
            (Some(true), true) => format!("●  {caption}"),
            (Some(true), false) => format!("✓  {caption}"),
            (Some(false), _) => format!("     {caption}"),
            (None, _) => caption,
        };
        let text = if st.default { RichText::new(label).strong() } else { RichText::new(label) };
        if n.children.is_empty() {
            let mut b = egui::Button::new(text);
            if let Some(sc) = st.shortcut.filter(|&s| s != 0) {
                b = b.shortcut_text(pwtool_app::shortcut::shortcut_to_text(sc));
            }
            if ui.add_enabled(st.enabled, b).clicked() {
                events.push(Event::Menu(n.name.clone()));
                ui.close();
            }
        } else {
            ui.add_enabled_ui(st.enabled, |ui| {
                ui.menu_button(text, |ui| menu_items(ui, &n.children, items, events));
            });
        }
    }
}

/// Height of the main menu bar.
pub fn menu_bar_height(ctx: &egui::Context) -> f32 {
    ctx.global_style().spacing.interact_size.y + 4.0
}

// ------------------------------------------------------------------- drawing

enum Deferred {
    Radio(usize),
    UpDown(usize, i64),
    Speed(usize),
    Splitter(usize, f32),
}

struct Frame<'a> {
    events: &'a mut Vec<Event>,
    deferred: Vec<Deferred>,
    body: f32,
    scale: f32,
    text_height: f32,
    form: &'a str,
}

fn tooltip(resp: Response, hint: &str) -> Response {
    if hint.is_empty() { resp } else { resp.on_hover_text(strip_amp(hint)) }
}

fn draw_label(ui: &mut Ui, c: &Control, enabled: bool, f: &mut Frame) {
    let State::Label { halign, wrap, vcenter, .. } = &c.state else { return };
    let text = strip_amp(&c.caption);
    let color = text_color(ui, &c.font, enabled);
    let fid = font_id(&c.font, f.body);
    let lines = text.lines().count().max(1);
    let mut job = LayoutJob::simple(text, fid, color, if *wrap { c.rect.width() } else { f32::INFINITY });
    if !*wrap && c.autosize.is_none() {
        job.wrap = TextWrapping::truncate_at_width(c.rect.width().max(1.0));
        // explicit line breaks are kept (VCL draws every line)
        job.wrap.max_rows = lines;
    }
    job.halign = egui::Align::Min;
    let galley = ui.painter().layout_job(job);
    let x = c.rect.min.x + (c.rect.width() - galley.size().x).max(0.0) * halign;
    let y = if *vcenter { c.rect.center().y - galley.size().y / 2.0 } else { c.rect.min.y };
    ui.painter().with_clip_rect(c.rect.expand(1.0)).galley(pos2(x, y), galley, color);
    let resp = ui.interact(c.rect, ui.id().with(("label", &c.name)), Sense::click());
    if resp.clicked() && enabled {
        f.events.push(Event::Click(c.name.clone()));
    }
    tooltip(resp, &c.hint);
}

/// Visible text of a `TLinkLabel` caption (without the `<a>` tags).
fn link_text(caption: &str) -> String {
    let mut out = String::new();
    let mut rest = caption;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let tag = &rest[lt..];
        if tag.starts_with("<a") || tag.starts_with("</a") {
            match tag.find('>') {
                Some(gt) => rest = &tag[gt + 1..],
                None => {
                    rest = "";
                }
            }
        } else {
            out.push('<');
            rest = &tag[1..];
        }
    }
    out.push_str(rest);
    out
}

/// `TLinkLabel`: text with `<a href="…">…</a>` links.
fn draw_link(ui: &mut Ui, c: &Control, enabled: bool, f: &mut Frame) {
    let mut parts: Vec<(String, Option<String>)> = Vec::new();
    let mut rest = c.caption.as_str();
    while let Some(a) = rest.find("<a") {
        if a > 0 {
            parts.push((rest[..a].to_string(), None));
        }
        let Some(gt) = rest[a..].find('>') else { break };
        let tag = &rest[a..a + gt];
        let href = tag.split("href=\"").nth(1).and_then(|h| h.split('"').next()).unwrap_or("").to_string();
        let after = &rest[a + gt + 1..];
        let end = after.find("</a>").unwrap_or(after.len());
        parts.push((after[..end].to_string(), Some(href)));
        rest = after.get(end + 4..).unwrap_or("");
    }
    if !rest.is_empty() {
        parts.push((rest.to_string(), None));
    }
    let fid = font_id(&c.font, f.body);
    at(ui, c.rect, enabled, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (text, href) in parts {
            match href {
                Some(h) => {
                    if ui.link(RichText::new(&text).font(fid.clone())).clicked() {
                        f.events.push(Event::Link(c.name.clone(), if h.is_empty() { text.clone() } else { h }));
                    }
                }
                None => {
                    ui.label(RichText::new(&text).font(fid.clone()));
                }
            }
        }
    });
}

fn draw_frame_box(ui: &mut Ui, c: &Control, enabled: bool, f: &mut Frame) {
    let stroke = frame_stroke(ui);
    let painter = ui.painter();
    // windowed controls cover the non-windowed ones below them (VCL z-order)
    painter.rect_filled(c.rect, 0.0, ui.visuals().panel_fill);
    match &c.state {
        State::Group | State::RadioGroup { .. } => {
            let top = c.rect.min.y + f.text_height / 2.0;
            let r = Rect::from_min_max(pos2(c.rect.min.x, top), c.rect.max);
            painter.rect_stroke(r, 3.0, stroke, StrokeKind::Inside);
            let text = strip_amp(&c.caption);
            if !text.is_empty() {
                let fid = font_id(&c.font, f.body);
                let galley = painter.layout_no_wrap(text, fid, text_color(ui, &c.font, enabled));
                let pos = pos2(c.rect.min.x + 8.0 * f.scale, c.rect.min.y);
                let bg = Rect::from_min_size(pos - vec2(3.0, 0.0), galley.size() + vec2(6.0, 0.0));
                painter.rect_filled(bg, 0.0, ui.visuals().panel_fill);
                painter.galley(pos, galley, Color32::PLACEHOLDER);
            }
        }
        State::Panel { bevel } => {
            if *bevel {
                painter.rect_stroke(c.rect, 2.0, stroke, StrokeKind::Inside);
            }
            let text = strip_amp(&c.caption);
            if !text.is_empty() && c.children.is_empty() {
                painter.text(c.rect.center(), Align2::CENTER_CENTER, text, font_id(&c.font, f.body), text_color(ui, &c.font, enabled));
            }
        }
        _ => {}
    }
}

fn draw_radio_group(ui: &mut Ui, c: &mut Control, enabled: bool, f: &mut Frame) {
    draw_frame_box(ui, c, enabled, f);
    let (name, rect, hint) = (c.name.clone(), c.rect, c.hint.clone());
    let State::RadioGroup { items, index, columns } = &mut c.state else { return };
    let s = f.scale;
    let n = items.len();
    let cols = (*columns).max(1);
    let rows = n.div_ceil(cols).max(1);
    let top = f.text_height + 6.0 * s;
    let rh = (rect.height() - top - 6.0 * s) / rows as f32;
    let cw = (rect.width() - 16.0 * s) / cols as f32;
    let fid = font_id(&c.font, f.body);
    for (i, item) in items.iter().enumerate() {
        let (row, col) = (i % rows, i / rows);
        let r = Rect::from_min_size(rect.min + vec2(8.0 * s + col as f32 * cw, top + row as f32 * rh), vec2(cw, rh.max(18.0 * s)));
        let resp = at(ui, r, enabled, |ui| ui.radio(*index == Some(i), RichText::new(strip_amp(item)).font(fid.clone())));
        if resp.clicked() && *index != Some(i) {
            *index = Some(i);
            f.events.push(Event::Click(name.clone()));
            f.events.push(Event::Change(name.clone()));
        }
        tooltip(resp, &hint);
    }
}

fn draw_pages(ui: &mut Ui, c: &mut Control, sheets: &[(String, bool)], enabled: bool, f: &mut Frame) {
    let th = layout::tab_height(f.text_height, f.scale);
    let client = layout::client_rect(c, f.text_height, f.scale);
    let stroke = frame_stroke(ui);
    ui.painter().rect_filled(c.rect, 0.0, ui.visuals().panel_fill);
    ui.painter().rect_stroke(Rect::from_min_max(pos2(c.rect.min.x, client.min.y - 2.0), c.rect.max), 2.0, stroke, StrokeKind::Inside);
    let State::Pages { active } = &mut c.state else { return };
    let mut x = c.rect.min.x + 2.0;
    let fid = font_id(&c.font, f.body);
    // tab widths: text plus padding; the padding shrinks if the tabs do not fit
    let widths: Vec<f32> = sheets
        .iter()
        .map(|(cap, v)| if *v { ui.painter().layout_no_wrap(strip_amp(cap), fid.clone(), Color32::WHITE).size().x } else { 0.0 })
        .collect();
    let n = sheets.iter().filter(|s| s.1).count().max(1) as f32;
    let avail = c.rect.width() - 4.0 - 2.0 * n;
    let text_sum: f32 = widths.iter().sum();
    let pad = ((avail - text_sum) / n).clamp(4.0, 16.0 * f.scale);
    for (i, (caption, visible)) in sheets.iter().enumerate() {
        if !visible {
            continue;
        }
        let text = strip_amp(caption);
        let w = widths[i] + pad;
        let r = Rect::from_min_size(pos2(x, c.rect.min.y), vec2(w, th));
        let sel = *active == i;
        let resp = at(ui, r, enabled, |ui| {
            ui.add_sized(r.size(), egui::Button::selectable(sel, RichText::new(text).font(fid.clone())))
        });
        if resp.clicked() && !sel {
            *active = i;
            f.events.push(Event::Change(c.name.clone()));
        }
        x += w + 2.0;
    }
}

fn draw_combo(ui: &mut Ui, c: &mut Control, enabled: bool, f: &mut Frame) {
    let id = ui.id().with(("combo", &c.name));
    let name = c.name.clone();
    let rect = c.rect;
    let fid = font_id(&c.font, f.body);
    let focus = std::mem::take(&mut c.focus_request);
    let State::Combo { items, index, text, editable } = &mut c.state else { return };
    let resp = at(ui, rect, enabled, |ui| {
        if *editable {
            let button_w = rect.height().min(24.0);
            let mut changed = false;
            let te = ui.add_sized(
                vec2(rect.width() - button_w, rect.height()),
                egui::TextEdit::singleline(text).id(id).font(fid.clone()).vertical_align(egui::Align::Center),
            );
            if te.changed() {
                *index = items.iter().position(|i| i == text.as_str());
                changed = true;
            }
            if focus {
                te.request_focus();
            }
            let b = ui.add_sized(vec2(button_w, rect.height()), egui::Button::new("⏷"));
            let popup = egui::Popup::menu(&b).width(rect.width()).align(egui::RectAlign::BOTTOM_END);
            popup.show(|ui| {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (i, it) in items.iter().enumerate() {
                        if ui.selectable_label(*index == Some(i), it.as_str()).clicked() {
                            *index = Some(i);
                            text.set(it);
                            changed = true;
                            ui.close();
                        }
                    }
                });
            });
            if te.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    f.events.push(Event::Enter(name.clone()));
                }
                f.events.push(Event::Exit(name.clone()));
            }
            if changed {
                f.events.push(Event::Change(name.clone()));
            }
            te
        } else {
            let mut changed = false;
            let ir = egui::ComboBox::from_id_salt(id)
                .width(rect.width() - ui.spacing().icon_width - 8.0)
                .selected_text(RichText::new(text.as_str()).font(fid.clone()))
                .show_ui(ui, |ui| {
                    for (i, it) in items.iter().enumerate() {
                        if ui.selectable_label(*index == Some(i), RichText::new(it.as_str()).font(fid.clone())).clicked() && *index != Some(i)
                        {
                            *index = Some(i);
                            changed = true;
                        }
                    }
                });
            if changed {
                text.set(index.map(|i| items[i].as_str()).unwrap_or(""));
                f.events.push(Event::Change(name.clone()));
            }
            ir.response
        }
    });
    c.has_focus = resp.has_focus();
    tooltip(resp, &c.hint);
}

fn draw_edit(ui: &mut Ui, c: &mut Control, enabled: bool, f: &mut Frame) {
    let id = ui.id().with(("edit", &c.name));
    let name = c.name.clone();
    let rect = c.rect;
    let fid = font_id(&c.font, f.body);
    let focus = std::mem::take(&mut c.focus_request);
    let ctx = ui.ctx().clone();
    let (resp, secret) = match &mut c.state {
        State::Edit { text, password, readonly, max_len, halign } => {
            let secret = text.is_secret() || *password;
            let align = match *halign {
                h if h > 0.7 => egui::Align::Max,
                h if h > 0.3 => egui::Align::Center,
                _ => egui::Align::Min,
            };
            let resp = at(ui, rect, enabled, |ui| {
                let mut ro: &str = text.as_str();
                let te = if *readonly {
                    // a read-only edit keeps its frame (egui draws none)
                    egui::TextEdit::singleline(&mut ro).frame(readonly_frame(ui))
                } else {
                    egui::TextEdit::singleline(text)
                };
                let te = te
                    .id(id)
                    .password(*password)
                    .font(fid.clone())
                    .horizontal_align(align)
                    .vertical_align(egui::Align::Center)
                    .char_limit(if *max_len > 0 { *max_len } else { usize::MAX });
                ui.add_sized(rect.size(), te)
            });
            (resp, secret)
        }
        State::Memo { text, readonly, wrap } => {
            let secret = text.is_secret();
            let resp = at(ui, rect, enabled, |ui| {
                let stroke = frame_stroke(ui);
                ui.painter().rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
                let sa = if *wrap { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() };
                sa.id_salt(id.with("scroll"))
                    .max_height(rect.height())
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        let mut ro: &str = text.as_str();
                        let te = if *readonly { egui::TextEdit::multiline(&mut ro) } else { egui::TextEdit::multiline(text) };
                        let te = te.id(id).font(fid.clone()).frame(egui::Frame::NONE).desired_width(if *wrap { rect.width() - 16.0 } else { f32::INFINITY });
                        ui.add_sized(vec2(rect.width() - 14.0, rect.height() - 4.0), te)
                    })
                    .inner
            });
            (resp, secret)
        }
        _ => return,
    };
    if focus {
        resp.request_focus();
    }
    c.has_focus = resp.has_focus();
    // selection: apply requested changes, remember the current one
    if let Some(mut st) = egui::text_edit::TextEditState::load(&ctx, id) {
        use egui::text::{CCursor, CCursorRange};
        let mut changed_state = false;
        for op in std::mem::take(&mut c.edit_ops) {
            let len = match &c.state {
                State::Edit { text, .. } | State::Memo { text, .. } => text.as_str().chars().count(),
                _ => 0,
            };
            match op {
                super::model::EditOp::SelectAll => {
                    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(len))))
                }
                super::model::EditOp::Cursor(n) => st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(n.min(len))))),
                super::model::EditOp::Undo => {
                    if let State::Edit { text, .. } | State::Memo { text, .. } = &mut c.state {
                        let cur = (st.cursor.char_range().unwrap_or_default(), text.as_str().to_string());
                        let prev = st.undoer().undo(&cur).cloned();
                        if let Some((range, t)) = prev {
                            text.set(&t);
                            st.cursor.set_char_range(Some(range));
                            f.events.push(Event::Change(name.clone()));
                        }
                    }
                }
            }
            changed_state = true;
        }
        if let Some(r) = st.cursor.char_range() {
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            c.selection = Some((a.min(b), a.max(b)));
        }
        if changed_state {
            st.store(&ctx, id);
        }
    }
    if resp.changed() {
        f.events.push(Event::Change(name.clone()));
    }
    if resp.lost_focus() {
        if matches!(c.state, State::Edit { .. }) && ui.input(|i| i.key_pressed(Key::Enter)) {
            f.events.push(Event::Enter(name.clone()));
        }
        f.events.push(Event::Exit(name.clone()));
    }
    if secret {
        // the undo history keeps plain copies of the text
        if let Some(mut st) = egui::text_edit::TextEditState::load(&ctx, id) {
            st.clear_undoer();
            st.store(&ctx, id);
        }
    }
    tooltip(resp, &c.hint);
}

fn draw_list(ui: &mut Ui, c: &mut Control, enabled: bool, f: &mut Frame) {
    let id = ui.id().with(("list", &c.name));
    let name = c.name.clone();
    let rect = c.rect;
    let fid = font_id(&c.font, f.body);
    let stroke = frame_stroke(ui);
    ui.painter().rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
    at(ui, rect.shrink(2.0), enabled, |ui| {
        egui::ScrollArea::vertical().id_salt(id).auto_shrink(false).show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| match &mut c.state {
                State::ListBox { items, sel, marks } => {
                    for (i, it) in items.iter().enumerate() {
                        let on = match marks {
                            Some(m) => m[i],
                            None => *sel == Some(i),
                        };
                        let r = ui.selectable_label(on, RichText::new(it.as_str()).font(fid.clone()));
                        if r.clicked() {
                            match marks {
                                // Ctrl+click toggles, Shift+click selects a range
                                Some(m) => {
                                    let mods = ui.input(|i| i.modifiers);
                                    if mods.command {
                                        m[i] = !m[i];
                                    } else if let (true, Some(a)) = (mods.shift, *sel) {
                                        let (lo, hi) = (a.min(i), a.max(i));
                                        for (j, x) in m.iter_mut().enumerate() {
                                            *x = (lo..=hi).contains(&j);
                                        }
                                    } else {
                                        m.iter_mut().for_each(|x| *x = false);
                                        m[i] = true;
                                    }
                                    if !mods.shift {
                                        *sel = Some(i);
                                    }
                                    f.events.push(Event::Change(name.clone()));
                                }
                                None if *sel != Some(i) => {
                                    *sel = Some(i);
                                    f.events.push(Event::Change(name.clone()));
                                }
                                None => {}
                            }
                        }
                        if r.double_clicked() {
                            f.events.push(Event::DblClick(name.clone()));
                        }
                    }
                }
                State::CheckList { items, checked, sel } => {
                    for (i, it) in items.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut checked[i], "").changed() {
                                f.events.push(Event::Click(name.clone()));
                            }
                            let r = ui.selectable_label(*sel == Some(i), RichText::new(it.as_str()).font(fid.clone()));
                            if r.clicked() && *sel != Some(i) {
                                *sel = Some(i);
                                f.events.push(Event::Change(name.clone()));
                            }
                        });
                    }
                }
                _ => {}
            });
        });
    });
}

fn draw_tool_bar(ui: &mut Ui, controls: &mut [Control], idx: usize, enabled: bool, f: &mut Frame) {
    let rect = controls[idx].rect;
    let kids = controls[idx].children.clone();
    let h = rect.height();
    let mut x = rect.min.x + 2.0;
    for k in kids {
        let c = &mut controls[k];
        if !c.visible {
            continue;
        }
        match &mut c.state {
            State::ToolButton { icon, separator, down, toggle } => {
                if *separator {
                    let w = 8.0 * f.scale;
                    ui.painter().vline(x + w / 2.0, rect.y_range().shrink(4.0), frame_stroke(ui));
                    c.rect = Rect::from_min_size(pos2(x, rect.min.y), vec2(w, h));
                    x += w;
                    continue;
                }
                let r = Rect::from_min_size(pos2(x, rect.min.y), vec2(h, h));
                c.rect = r;
                x += h + 1.0;
                let tex = icon.as_ref().and_then(|i| icon_texture(ui.ctx(), i).map(|t| (t, i.size)));
                let on = enabled && c.enabled;
                let resp = at(ui, r, on, |ui| {
                    let b = match &tex {
                        Some((t, size)) => egui::Button::image(egui::Image::new(t).fit_to_exact_size(vec2(*size, *size))),
                        None => egui::Button::new(strip_amp(&c.caption)),
                    };
                    ui.add_sized(r.size(), b.frame_when_inactive(false).selected(*down))
                });
                if resp.clicked() {
                    if *toggle {
                        *down = !*down;
                    }
                    f.events.push(Event::Click(c.name.clone()));
                }
                tooltip(resp, &c.hint);
            }
            _ => {
                // other controls on a tool bar (search box…) keep their width
                let w = c.design.width();
                c.rect = Rect::from_min_size(pos2(x + 2.0, rect.min.y + 2.0), vec2(w, h - 4.0));
                x += w + 4.0;
            }
        }
    }
}

fn draw_one(ui: &mut Ui, controls: &mut [Control], idx: usize, enabled: bool, f: &mut Frame) {
    let body = f.body;
    match &controls[idx].state {
        State::ToolBar => return draw_tool_bar(ui, controls, idx, enabled, f),
        State::Pages { .. } => {
            let sheets: Vec<(String, bool)> =
                controls[idx].children.iter().map(|&k| (controls[k].caption.clone(), controls[k].visible)).collect();
            return draw_pages(ui, &mut controls[idx], &sheets, enabled, f);
        }
        _ => {}
    }
    let c = &mut controls[idx];
    let rect = c.rect;
    let name = c.name.clone();
    match &mut c.state {
        State::Label { .. } => draw_label(ui, c, enabled, f),
        State::Link => draw_link(ui, c, enabled, f),
        State::Button { .. } => {
            let text = rich(&c.caption, &c.font, body);
            let resp = at(ui, rect, enabled, |ui| ui.add_sized(rect.size(), egui::Button::new(text)));
            if std::mem::take(&mut c.focus_request) {
                resp.request_focus();
            }
            c.has_focus = resp.has_focus();
            if resp.clicked() {
                f.events.push(Event::Click(name));
            }
            tooltip(resp, &c.hint);
        }
        State::Speed { toggle, down, flat, icon, show_caption } => {
            let tex = icon.as_ref().and_then(|i| icon_texture(ui.ctx(), i).map(|t| (t, i.size)));
            let caption = if *show_caption { strip_amp(&c.caption) } else { String::new() };
            let resp = at(ui, rect, enabled, |ui| {
                let b = match (&tex, caption.is_empty()) {
                    (Some((t, size)), true) => egui::Button::image(egui::Image::new(t).fit_to_exact_size(vec2(*size, *size))),
                    (Some((t, size)), false) => {
                        egui::Button::image_and_text(egui::Image::new(t).fit_to_exact_size(vec2(*size, *size)), caption.clone())
                    }
                    (None, _) => egui::Button::new(rich(&c.caption, &c.font, body)),
                };
                ui.add_sized(rect.size(), b.frame_when_inactive(!*flat).selected(*down))
            });
            if resp.clicked() {
                if *toggle {
                    f.deferred.push(Deferred::Speed(idx));
                }
                f.events.push(Event::Click(name));
            }
            tooltip(resp, &c.hint);
        }
        State::Check { checked } => {
            let text = rich(&c.caption, &c.font, body);
            let resp = at(ui, rect, enabled, |ui| ui.checkbox(checked, text));
            if std::mem::take(&mut c.focus_request) {
                resp.request_focus();
            }
            c.has_focus = resp.has_focus();
            if resp.changed() {
                f.events.push(Event::Click(name));
            }
            tooltip(resp, &c.hint);
        }
        State::Radio { checked } => {
            let text = rich(&c.caption, &c.font, body);
            let on = *checked;
            let resp = at(ui, rect, enabled, |ui| ui.radio(on, text));
            if resp.clicked() && !on {
                f.deferred.push(Deferred::Radio(idx));
                f.events.push(Event::Click(name));
            }
            tooltip(resp, &c.hint);
        }
        State::Edit { .. } | State::Memo { .. } => draw_edit(ui, c, enabled, f),
        State::Combo { .. } => draw_combo(ui, c, enabled, f),
        State::UpDown { .. } => {
            let half = Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() / 2.0));
            let lower = half.translate(vec2(0.0, rect.height() / 2.0));
            for (r, d, sym) in [(half, 1, "⏶"), (lower, -1, "⏷")] {
                let resp = at(ui, r, enabled, |ui| {
                    ui.add_sized(r.size(), egui::Button::new(RichText::new(sym).size(r.height().clamp(6.0, 12.0))).frame_when_inactive(true))
                });
                if resp.clicked() {
                    f.deferred.push(Deferred::UpDown(idx, d));
                }
            }
        }
        State::Group | State::Panel { .. } => draw_frame_box(ui, c, enabled, f),
        State::RadioGroup { .. } => draw_radio_group(ui, c, enabled, f),
        State::Sheet => {}
        State::ListBox { .. } | State::CheckList { .. } => draw_list(ui, c, enabled, f),
        State::Status { text } => {
            ui.painter().hline(rect.x_range(), rect.min.y, frame_stroke(ui));
            let col = ui.visuals().text_color();
            ui.painter().with_clip_rect(rect).text(
                pos2(rect.min.x + 4.0, rect.center().y),
                Align2::LEFT_CENTER,
                text.as_str(),
                font_id(&c.font, body),
                col,
            );
        }
        State::Track { min, max, pos } => {
            let (lo, hi) = (*min, *max);
            let resp = at(ui, rect, enabled, |ui| {
                ui.spacing_mut().slider_width = rect.width() - 8.0;
                ui.add(egui::Slider::new(pos, lo..=hi).show_value(false))
            });
            if resp.changed() {
                f.events.push(Event::Change(name));
            }
            tooltip(resp, &c.hint);
        }
        State::Progress { pos, max } => {
            let frac = if *max > 0 { (*pos as f32 / *max as f32).clamp(0.0, 1.0) } else { 0.0 };
            at(ui, rect, enabled, |ui| ui.add_sized(rect.size(), egui::ProgressBar::new(frac)));
        }
        State::Date { ymd, checked, show_check } => {
            let resp = at(ui, rect, enabled, |ui| {
                let mut changed = false;
                if *show_check && ui.checkbox(checked, "").changed() {
                    changed = true;
                }
                let mut date = jiff::civil::Date::new(ymd.0 as i16, ymd.1 as i8, ymd.2 as i8).unwrap_or_default();
                let r = ui.add_enabled(!*show_check || *checked, egui_extras::DatePickerButton::new(&mut date).id_salt(&name));
                if r.changed() {
                    *ymd = (date.year() as i32, date.month() as u32, date.day() as u32);
                    changed = true;
                }
                (r, changed)
            });
            if resp.1 {
                f.events.push(Event::Change(name));
            }
            tooltip(resp.0, &c.hint);
        }
        State::Splitter => {
            let vertical = matches!(c.align, Align::Left | Align::Right);
            let resp = ui.interact(rect, ui.id().with(("split", &name)), Sense::drag());
            let resp = resp.on_hover_cursor(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
            let d = resp.drag_delta();
            let delta = if vertical { d.x } else { d.y };
            if delta != 0.0 {
                f.deferred.push(Deferred::Splitter(idx, delta));
            }
        }
        State::Bevel { shape } => {
            let s = frame_stroke(ui);
            let p = ui.painter();
            match shape.as_str() {
                "bsTopLine" => {
                    p.hline(rect.x_range(), rect.min.y, s);
                }
                "bsBottomLine" => {
                    p.hline(rect.x_range(), rect.max.y - 1.0, s);
                }
                "bsLeftLine" => {
                    p.vline(rect.min.x, rect.y_range(), s);
                }
                "bsRightLine" => {
                    p.vline(rect.max.x - 1.0, rect.y_range(), s);
                }
                "bsSpacer" => {}
                _ => {
                    p.rect_stroke(rect, 0.0, s, StrokeKind::Inside);
                }
            }
        }
        State::Image { png, version, stretch, center } => {
            if let Some(png) = png {
                let key = format!("img:{}/{}/{}", f.form, name, version);
                if let Some(t) = image_texture(ui.ctx(), &key, png) {
                    let size = t.size_vec2() / ui.ctx().pixels_per_point().max(1.0);
                    let size = if *stretch { rect.size() } else { size * f.scale.max(0.5) };
                    let size = if !*stretch && (size.x > rect.width() || size.y > rect.height()) {
                        let k = (rect.width() / size.x).min(rect.height() / size.y);
                        size * k
                    } else {
                        size
                    };
                    let r = if *center { Rect::from_center_size(rect.center(), size) } else { Rect::from_min_size(rect.min, size) };
                    egui::Image::new(&t).paint_at(ui, r);
                }
            }
            let resp = ui.interact(rect, ui.id().with(("image", &name)), Sense::click());
            if resp.clicked() {
                f.events.push(Event::Click(name));
            }
            tooltip(resp, &c.hint);
        }
        State::Custom | State::ToolBar | State::Pages { .. } | State::ToolButton { .. } => {}
    }
}

/// Order in which the controls are drawn (parents before children, the
/// active sheet of page controls only), with their effective enabled state.
fn draw_order(d: &FormData) -> Vec<(usize, bool)> {
    fn walk(d: &FormData, list: &[usize], enabled: bool, out: &mut Vec<(usize, bool)>) {
        for &i in list {
            let c = &d.controls[i];
            if !c.visible {
                continue;
            }
            let on = enabled && c.enabled;
            out.push((i, on));
            match &c.state {
                State::Pages { active } => {
                    if let Some(&sheet) = c.children.get(*active) {
                        if d.controls[sheet].visible {
                            out.push((sheet, on && d.controls[sheet].enabled));
                            walk(d, &d.controls[sheet].children, on && d.controls[sheet].enabled, out);
                        }
                    }
                }
                State::ToolBar => {
                    // tool buttons are drawn by the tool bar; other controls after it
                    let others: Vec<usize> =
                        c.children.iter().copied().filter(|&k| !matches!(d.controls[k].state, State::ToolButton { .. })).collect();
                    walk(d, &others, on, out);
                }
                _ => walk(d, &c.children, on, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(d, &d.roots, true, &mut out);
    out
}

impl Form {
    /// Lay out the controls for the client rectangle `area`.
    fn layout(&self, ctx: &egui::Context, area: Rect, body: f32) {
        let mut d = self.data.borrow_mut();
        let mut measure = |c: &Control| {
            let fid = font_id(&c.font, body);
            let text = if c.class == "TLinkLabel" { link_text(&c.caption) } else { strip_amp(&c.caption) };
            ctx.fonts_mut(|f| f.layout_no_wrap(text, fid, Color32::WHITE).size().x.ceil() + 1.0)
        };
        layout::layout(&mut d, area, &mut measure);
    }

    /// Keyboard handling of the form (before drawing).
    fn keys(&self, ui: &mut Ui) {
        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let mut d = self.data.borrow_mut();
        let order = draw_order(&d);
        let FormData { controls, items, events, menus, main_menu, .. } = &mut *d;
        // menu shortcuts: the main menu always, a popup menu while a control
        // using it has the focus (VCL `IsShortCut`)
        fn collect<'a>(nodes: &'a [MenuNode], out: &mut Vec<&'a str>) {
            for n in nodes {
                out.push(&n.name);
                collect(&n.children, out);
            }
        }
        let mut active: Vec<&str> = Vec::new();
        for (m, nodes) in menus.iter() {
            let focused = controls.iter().any(|c| c.has_focus && c.popup_menu.as_deref() == Some(m.as_str()));
            if main_menu.as_deref() == Some(m.as_str()) || focused {
                collect(nodes, &mut active);
            }
        }
        let mut shortcuts: Vec<(String, u16)> = active
            .iter()
            .filter_map(|n| items.get(*n).map(|s| (n, s)))
            .filter(|(_, s)| s.enabled && s.visible)
            .filter_map(|(n, s)| s.shortcut.filter(|&x| x != 0).map(|x| (n.to_string(), x)))
            .collect();
        shortcuts.sort();
        shortcuts.dedup();
        for (name, sc) in shortcuts {
            if let Some((m, k)) = shortcut_key(sc) {
                if ui.input_mut(|i| i.consume_key(m, k)) {
                    events.push(Event::Menu(name));
                }
            }
        }
        if popup_open {
            return;
        }
        let enabled: HashMap<usize, bool> = order.iter().copied().collect();
        let usable = |i: usize| enabled.get(&i).copied().unwrap_or(false);
        // Esc: cancel button; Enter: default button (not in multi-line text)
        let esc = ui.input(|i| i.key_pressed(Key::Escape));
        let memo_focus = controls.iter().any(|c| c.has_focus && matches!(c.state, State::Memo { .. }));
        let enter = !memo_focus && ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.is_none());
        for (i, c) in controls.iter().enumerate() {
            if let State::Button { default, cancel } = c.state {
                if usable(i) && ((esc && cancel) || (enter && default)) {
                    events.push(Event::Click(c.name.clone()));
                    return;
                }
            }
        }
        // Alt+letter: control with that mnemonic
        let pressed: Option<char> = ui.input(|i| {
            if !i.modifiers.alt || i.modifiers.ctrl {
                return None;
            }
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, .. } => {
                    let n = key.name();
                    (n.len() == 1).then(|| n.chars().next().unwrap().to_ascii_lowercase())
                }
                _ => None,
            })
        });
        let Some(ch) = pressed else { return };
        let mut focus_target: Option<String> = None;
        for &(i, on) in &order {
            let c = &mut controls[i];
            if !on || mnemonic_of(&c.caption) != Some(ch) {
                continue;
            }
            match &mut c.state {
                State::Button { .. } | State::Speed { .. } => events.push(Event::Click(c.name.clone())),
                State::Check { checked } => {
                    *checked = !*checked;
                    c.focus_request = true;
                    events.push(Event::Click(c.name.clone()));
                }
                State::Radio { .. } => {
                    events.push(Event::Click(c.name.clone()));
                }
                State::Label { focus_control, .. } => focus_target = focus_control.clone(),
                _ => continue,
            }
            break;
        }
        if let Some(t) = focus_target {
            if let Some(c) = controls.iter_mut().find(|c| c.name == t) {
                c.focus_request = true;
            }
        }
    }

    /// Draw the form into `ui`: main menu at the top, controls below.
    pub fn show(&self, ui: &mut Ui) {
        let full = ui.max_rect();
        let body = body_size(ui);
        let mut area = full;
        let main_menu = self.data.borrow().main_menu.clone();
        if let Some(menu) = main_menu {
            let h = menu_bar_height(ui.ctx());
            let bar = Rect::from_min_size(full.min, vec2(full.width(), h));
            ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    let mut d = self.data.borrow_mut();
                    let FormData { menus, items, events, .. } = &mut *d;
                    let Some(nodes) = menus.get(&menu) else { return };
                    for n in nodes {
                        let Some(st) = items.get(&n.name) else { continue };
                        if !st.visible {
                            continue;
                        }
                        let caption = strip_amp(&st.caption);
                        if n.children.is_empty() {
                            if ui.add_enabled(st.enabled, egui::Button::new(caption).frame_when_inactive(false)).clicked() {
                                events.push(Event::Menu(n.name.clone()));
                            }
                        } else {
                            ui.add_enabled_ui(st.enabled, |ui| {
                                ui.menu_button(caption, |ui| menu_items(ui, &n.children, items, events));
                            });
                        }
                    }
                });
            });
            ui.painter().hline(full.x_range(), bar.max.y - 0.5, frame_stroke(ui));
            area.min.y += h;
        }
        self.keys(ui);
        self.layout(ui.ctx(), area, body);

        let (order, scale, text_height) = {
            let d = self.data.borrow();
            (draw_order(&d), d.scale, d.text_height)
        };
        let mut events = Vec::new();
        let mut deferred = Vec::new();
        for (i, enabled) in order {
            let custom = {
                let mut d = self.data.borrow_mut();
                if matches!(d.controls[i].state, State::Custom) {
                    Some((d.controls[i].name.clone(), d.controls[i].rect, d.controls[i].hint.clone(), d.controls[i].popup_menu.clone()))
                } else {
                    let mut f = Frame { events: &mut events, deferred: Vec::new(), body, scale, text_height, form: &self.name };
                    draw_one(ui, &mut d.controls, i, enabled, &mut f);
                    deferred.append(&mut f.deferred);
                    // context menu of the control
                    if let Some(menu) = d.controls[i].popup_menu.clone() {
                        let rect = d.controls[i].rect;
                        let cname = d.controls[i].name.clone();
                        let FormData { menus, items, popup_owner, .. } = &mut *d;
                        if let Some(nodes) = menus.get(&menu) {
                            let resp = ui.interact(rect, ui.id().with(("ctx", i)), Sense::click());
                            if resp.secondary_clicked() {
                                *popup_owner = Some(cname);
                            }
                            resp.context_menu(|ui| menu_items(ui, nodes, items, &mut events));
                        }
                    }
                    None
                }
            };
            if let Some((name, rect, hint, menu)) = custom {
                let painter = self.painters.borrow_mut().remove(&name);
                match painter {
                    Some(mut p) => {
                        at(ui, rect, enabled, |ui| p(ui, rect));
                        self.painters.borrow_mut().entry(name.clone()).or_insert(p);
                    }
                    None => {
                        ui.painter().rect_stroke(rect, 2.0, frame_stroke(ui), StrokeKind::Inside);
                    }
                }
                let resp = ui.interact(rect, ui.id().with(("custom", &name)), Sense::hover());
                tooltip(resp, &hint);
                if let Some(menu) = menu {
                    let mut d = self.data.borrow_mut();
                    let FormData { menus, items, popup_owner, .. } = &mut *d;
                    if let Some(nodes) = menus.get(&menu) {
                        let resp = ui.interact(rect, ui.id().with(("ctxc", &name)), Sense::click());
                        if resp.secondary_clicked() {
                            *popup_owner = Some(name.clone());
                        }
                        resp.context_menu(|ui| menu_items(ui, nodes, items, &mut events));
                    }
                }
            }
        }
        self.apply_deferred(deferred, &mut events);
        self.show_open_popup(ui, &mut events);
        self.data.borrow_mut().events.append(&mut events);
    }

    fn apply_deferred(&self, deferred: Vec<Deferred>, events: &mut Vec<Event>) {
        let mut d = self.data.borrow_mut();
        for x in deferred {
            match x {
                Deferred::Radio(i) => {
                    let parent = d.controls[i].parent;
                    for c in d.controls.iter_mut().filter(|c| c.parent == parent) {
                        if let State::Radio { checked } = &mut c.state {
                            *checked = false;
                        }
                    }
                    if let State::Radio { checked } = &mut d.controls[i].state {
                        *checked = true;
                    }
                }
                Deferred::Speed(i) => {
                    if let State::Speed { down, .. } = &mut d.controls[i].state {
                        *down = !*down;
                    }
                }
                Deferred::UpDown(i, dir) => {
                    let (assoc, min, max, inc, pos) = match &d.controls[i].state {
                        State::UpDown { associate, min, max, inc, pos } => (associate.clone(), *min, *max, *inc, *pos),
                        _ => continue,
                    };
                    let cur = assoc
                        .as_ref()
                        .and_then(|a| d.by_name.get(a).copied())
                        .and_then(|k| match &d.controls[k].state {
                            State::Edit { text, .. } => text.as_str().trim().parse::<i64>().ok(),
                            _ => None,
                        })
                        .unwrap_or(pos);
                    let new = (cur + dir * inc).clamp(min, max);
                    if let State::UpDown { pos, .. } = &mut d.controls[i].state {
                        *pos = new;
                    }
                    if let Some(k) = assoc.as_ref().and_then(|a| d.by_name.get(a).copied()) {
                        if let State::Edit { text, .. } = &mut d.controls[k].state {
                            text.set(&new.to_string());
                        }
                        events.push(Event::Change(d.controls[k].name.clone()));
                    }
                    events.push(Event::Click(d.controls[i].name.clone()));
                }
                Deferred::Splitter(i, delta) => {
                    // resize the control the splitter is attached to
                    let (align, rect, parent) = (d.controls[i].align, d.controls[i].rect, d.controls[i].parent);
                    let target = d.controls.iter().position(|c| {
                        c.parent == parent
                            && c.visible
                            && c.align == align
                            && match align {
                                Align::Left => (c.rect.max.x - rect.min.x).abs() < 1.0,
                                Align::Right => (c.rect.min.x - rect.max.x).abs() < 1.0,
                                Align::Top => (c.rect.max.y - rect.min.y).abs() < 1.0,
                                Align::Bottom => (c.rect.min.y - rect.max.y).abs() < 1.0,
                                _ => false,
                            }
                    });
                    if let Some(t) = target {
                        let c = &mut d.controls[t];
                        let min = 20.0;
                        match align {
                            Align::Left => c.design.max.x = (c.design.max.x + delta).max(c.design.min.x + min),
                            Align::Right => c.design.min.x = (c.design.min.x - delta).min(c.design.max.x - min),
                            Align::Top => c.design.max.y = (c.design.max.y + delta).max(c.design.min.y + min),
                            Align::Bottom => c.design.min.y = (c.design.min.y - delta).min(c.design.max.y - min),
                            _ => {}
                        }
                        events.push(Event::Change(d.controls[i].name.clone()));
                    }
                }
            }
        }
    }

    /// A popup menu opened by the logic (`popup_menu`).
    fn show_open_popup(&self, ui: &mut Ui, events: &mut Vec<Event>) {
        let Some((menu, pos)) = self.data.borrow().open_popup.clone() else { return };
        let ctx = ui.ctx().clone();
        let pos = pos.or_else(|| ctx.pointer_latest_pos()).unwrap_or(ui.max_rect().center());
        let id = ui.id().with(("popup", &menu, &self.name));
        let mut d = self.data.borrow_mut();
        let FormData { menus, items, open_popup, .. } = &mut *d;
        let Some(nodes) = menus.get(&menu) else {
            *open_popup = None;
            return;
        };
        let n_before = events.len();
        let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(&ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(160.0);
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| menu_items(ui, nodes, items, events));
            })
        });
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx.pointer_interact_pos().is_some_and(|p| !area.response.rect.contains(p))
            && !egui::Popup::is_any_open(&ctx);
        if events.len() > n_before || clicked_outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
            *open_popup = None;
        }
    }
}

/// Size of a form's window: client area plus the main menu.
pub fn window_size(form: &Form, ctx: &egui::Context) -> Vec2 {
    let mut s = form.client_size();
    if form.has_main_menu() {
        s.y += menu_bar_height(ctx);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonics() {
        assert_eq!(strip_amp("&File && Edit"), "File & Edit");
        assert_eq!(mnemonic_of("&File"), Some('f'));
        assert_eq!(mnemonic_of("A && B"), None);
        assert_eq!(mnemonic_of("Save &As"), Some('a'));
    }

    #[test]
    fn shortcuts() {
        assert_eq!(shortcut_key(0x4000 | 0x43), Some((Modifiers::CTRL, Key::C)));
        assert_eq!(shortcut_key(0x70), Some((Modifiers::NONE, Key::F1)));
        assert_eq!(shortcut_key(0x2000 | 0x8000 | 0x2e), Some((Modifiers::SHIFT | Modifiers::ALT, Key::Delete)));
    }
}
