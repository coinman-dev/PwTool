//! Builds GTK widget trees from the original forms: every VCL control is
//! mapped to a GTK widget and placed in a [`VclBox`] at its design position,
//! scaled from the form's `PixelsPerInch` and the size of the system font.
//! Captions and hints are translated with [`trl`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};
use pwtech_core::language::trl;

use super::form::FormObj;
use super::icons;
use super::layout::{AK_BOTTOM, AK_LEFT, AK_RIGHT, AK_TOP, Align, VclBox};

/// VCL caption with `&` mnemonic → GTK mnemonic label.
pub fn mnemonic(caption: &str) -> String {
    let mut out = String::with_capacity(caption.len());
    let mut it = caption.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '&' => {
                if it.peek() == Some(&'&') {
                    it.next();
                    out.push('&');
                } else {
                    out.push('_');
                }
            }
            '_' => out.push_str("__"),
            c => out.push(c),
        }
    }
    out
}

/// VCL caption without mnemonic markers.
pub fn strip_mnemonic(caption: &str) -> String {
    let mut out = String::new();
    let mut it = caption.chars().peekable();
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

/// Translate a caption, keeping its mnemonic (the original translates the
/// caption including `&`).
pub fn tr_caption(caption: &str) -> String {
    trl(caption)
}

thread_local! {
    static FONT_FACTOR: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

/// Height of a line of text in the default GTK font, in pixels.
fn default_text_height() -> f64 {
    let label = gtk::Label::new(Some("Wg"));
    let layout = label.create_pango_layout(Some("Wg"));
    let (_, h) = layout.pixel_size();
    h.max(10) as f64
}

/// Scale from design pixels of a form to GTK pixels: the form's text height
/// (`TextHeight` at `PixelsPerInch`) is mapped to the height of the GTK
/// default font, like `TForm.Scaled` adapts forms to the system font.
pub fn form_scale(form: &FormObj) -> f64 {
    let mut f = FONT_FACTOR.with(|c| c.get());
    if f == 0.0 {
        f = default_text_height();
        FONT_FACTOR.with(|c| c.set(f));
    }
    let text_h = form.int_or("TextHeight", 17) as f64;
    // VCL's text height includes leading; GTK's layout height is similar.
    (f / text_h).clamp(0.6, 2.0)
}

/// Result of building a form.
pub struct Built {
    pub form: FormObj,
    pub root: VclBox,
    pub widgets: HashMap<String, gtk::Widget>,
    pub boxes: HashMap<String, VclBox>,
    pub scale: f64,
    /// Scaled client size and minimum size of the form.
    pub client: (i32, i32),
    pub min: (i32, i32),
    pub caption: String,
    /// Menu models of `TMainMenu` / `TPopupMenu` components.
    pub menus: HashMap<String, gio::Menu>,
    /// Actions of all menu items (name of the `TMenuItem`).
    pub actions: gio::SimpleActionGroup,
    /// Accelerators of menu items (action name → GTK accelerator).
    pub accels: Vec<(String, String)>,
    /// Images (list name → texture per index).
    pub image_lists: HashMap<String, Vec<Option<gdk::Texture>>>,
    /// Up-down buttons with their associated edit (`TUpDown.Associate`).
    pub updowns: HashMap<String, Rc<UpDown>>,
    /// Submenu models of menu items with children (item name → menu).
    pub submenus: HashMap<String, gio::Menu>,
    /// Position of every menu item: (section menu, index, item).
    pub menu_items: RefCell<HashMap<String, (gio::Menu, i32, gio::MenuItem)>>,
}

impl Built {
    pub fn w(&self, name: &str) -> gtk::Widget {
        self.widgets.get(name).cloned().unwrap_or_else(|| panic!("no widget {name}"))
    }

    pub fn get<T: IsA<gtk::Widget>>(&self, name: &str) -> T {
        self.w(name).downcast::<T>().unwrap_or_else(|w| panic!("widget {name} is a {}", w.type_().name()))
    }

    pub fn try_get<T: IsA<gtk::Widget>>(&self, name: &str) -> Option<T> {
        self.widgets.get(name).and_then(|w| w.clone().downcast::<T>().ok())
    }

    pub fn action(&self, name: &str) -> gio::SimpleAction {
        self.actions
            .lookup_action(name)
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
            .unwrap_or_else(|| panic!("no action {name}"))
    }

    /// Turn a menu item into a check item (items whose `Checked` state is
    /// only set in code are not marked in the form).
    pub fn make_checkable(&self, name: &str, checked: bool) {
        if let Some(a) = self.actions.lookup_action(name) {
            if a.state().is_some() {
                return;
            }
            let enabled = a.is_enabled();
            self.actions.remove_action(name);
            let na = gio::SimpleAction::new_stateful(name, None, &checked.to_variant());
            na.set_enabled(enabled);
            self.actions.add_action(&na);
        }
    }

    /// Change the caption of a menu item (`TMenuItem.Caption = ...`).
    pub fn set_menu_label(&self, name: &str, label: &str) {
        let items = self.menu_items.borrow();
        let Some((section, idx, item)) = items.get(name) else { return };
        item.set_label(Some(label));
        section.remove(*idx);
        section.insert_item(*idx, item);
    }

    pub fn has_action(&self, name: &str) -> bool {
        self.actions.lookup_action(name).is_some()
    }

    pub fn obj(&self, name: &str) -> Option<&FormObj> {
        self.form.find(name)
    }

    /// Scale a design value.
    pub fn sc(&self, v: i64) -> i32 {
        (v as f64 * self.scale).round() as i32
    }

    /// Texture of an image list entry.
    pub fn image(&self, list: &str, index: i64) -> Option<gdk::Texture> {
        self.image_lists.get(list)?.get(index as usize)?.clone()
    }

    pub fn image_size(&self, list: &str) -> i32 {
        self.form.find(list).map(|o| self.sc(o.int_or("Width", 16))).unwrap_or(16)
    }
}

/// `TUpDown`: two small arrow buttons that change the integer in the
/// associated edit box.
pub struct UpDown {
    pub widget: gtk::Box,
    pub min: RefCell<i64>,
    pub max: RefCell<i64>,
    pub increment: i64,
    pub entry: RefCell<Option<gtk::Entry>>,
    pub position: RefCell<i64>,
}

impl UpDown {
    pub fn position(&self) -> i64 {
        if let Some(e) = self.entry.borrow().as_ref() {
            if let Ok(v) = e.text().trim().parse::<i64>() {
                return v.clamp(*self.min.borrow(), *self.max.borrow());
            }
        }
        *self.position.borrow()
    }

    pub fn set_position(&self, v: i64) {
        let v = v.clamp(*self.min.borrow(), *self.max.borrow());
        *self.position.borrow_mut() = v;
        if let Some(e) = self.entry.borrow().as_ref() {
            e.set_text(&v.to_string());
        }
    }

    pub fn set_max(&self, v: i64) {
        *self.max.borrow_mut() = v;
    }

    pub fn set_min(&self, v: i64) {
        *self.min.borrow_mut() = v;
    }

    pub fn set_sensitive(&self, on: bool) {
        self.widget.set_sensitive(on);
    }

    fn step(&self, d: i64) {
        let p = self.position();
        self.set_position(p + d * self.increment);
    }
}

fn anchors_of(o: &FormObj, use_tag: bool) -> u8 {
    if use_tag {
        if let Some(t) = o.int("Tag") {
            if (1..16).contains(&t) {
                return t as u8;
            }
        }
    }
    match o.set("Anchors") {
        Some(set) => {
            let mut a = 0;
            for s in set {
                a |= match s.as_str() {
                    "akLeft" => AK_LEFT,
                    "akTop" => AK_TOP,
                    "akRight" => AK_RIGHT,
                    "akBottom" => AK_BOTTOM,
                    _ => 0,
                };
            }
            a
        }
        None => AK_LEFT | AK_TOP,
    }
}

fn align_of(o: &FormObj) -> Align {
    match o.ident("Align") {
        Some("alTop") => Align::Top,
        Some("alBottom") => Align::Bottom,
        Some("alLeft") => Align::Left,
        Some("alRight") => Align::Right,
        Some("alClient") => Align::Client,
        _ => Align::None,
    }
}

/// Apply font style/size of a control as Pango attributes or CSS.
fn apply_font(w: &gtk::Widget, o: &FormObj, form_font_height: i64) {
    let bold = o.set("Font.Style").is_some_and(|s| s.iter().any(|x| x == "fsBold"));
    let italic = o.set("Font.Style").is_some_and(|s| s.iter().any(|x| x == "fsItalic"));
    let height = o.int("Font.Height");
    let mut css = String::new();
    if bold {
        css.push_str("font-weight: bold;");
    }
    if italic {
        css.push_str("font-style: italic;");
    }
    if let Some(h) = height {
        if h != form_font_height && form_font_height != 0 {
            let pct = (h as f64 / form_font_height as f64 * 100.0).round();
            css.push_str(&format!("font-size: {pct}%;"));
        }
    }
    if let Some(name) = o.str("Font.Name") {
        if name.contains("Courier") || name.contains("Consolas") || name.contains("Lucida Console") {
            css.push_str("font-family: monospace;");
        }
    }
    if !css.is_empty() {
        add_css(w, &css);
    }
}

thread_local! {
    static CSS_COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Attach a CSS snippet to a single widget.
pub fn add_css(w: &impl IsA<gtk::Widget>, decls: &str) {
    let n = CSS_COUNTER.with(|c| {
        let v = c.get() + 1;
        c.set(v);
        v
    });
    let class = format!("pwc{n}");
    let provider = gtk::CssProvider::new();
    provider.load_from_data(&format!(".{class} {{ {decls} }}"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    w.add_css_class(&class);
}

struct Ctx {
    widgets: HashMap<String, gtk::Widget>,
    boxes: HashMap<String, VclBox>,
    scale: f64,
    use_tag: bool,
    form_font_height: i64,
    form_text_height: i64,
    image_lists: HashMap<String, Vec<Option<gdk::Texture>>>,
    image_sizes: HashMap<String, i32>,
    updowns: HashMap<String, Rc<UpDown>>,
    /// Client size of the tab sheets of the page control being built.
    sheet_size: (i64, i64),
}

/// `DesignSize` of a control (tab sheets have no `Width`/`Height`).
fn design_size(o: &FormObj) -> Option<(i64, i64)> {
    let l = o.props.get("DesignSize")?.get("list")?.as_array()?;
    Some((l.first()?.as_i64()?, l.get(1)?.as_i64()?))
}

impl Ctx {
    fn sc(&self, v: i64) -> i32 {
        (v as f64 * self.scale).round() as i32
    }

    fn icon(&self, o: &FormObj, list_prop: &str) -> Option<(gdk::Texture, i32)> {
        let list = o.ident(list_prop)?;
        let idx = o.int("ImageIndex")?;
        let tex = self.image_lists.get(list)?.get(idx as usize)?.clone()?;
        Some((tex, *self.image_sizes.get(list).unwrap_or(&16)))
    }
}

fn image_widget(tex: &gdk::Texture, size: i32) -> gtk::Image {
    let img = gtk::Image::from_paintable(Some(tex));
    img.set_pixel_size(size);
    img
}

/// Build the widget for one control and its children. Returns `None` for
/// non-visual components.
fn create(o: &FormObj, ctx: &mut Ctx, form_name: &str) -> Option<gtk::Widget> {
    let caption = o.str("Caption").map(tr_caption).unwrap_or_default();
    let w: gtk::Widget = match o.class.as_str() {
        "TLabel" => {
            let l = gtk::Label::new(Some(&strip_mnemonic(&caption)));
            if caption.contains('&') {
                l.set_text_with_mnemonic(&mnemonic(&caption));
            }
            let xalign = match o.ident("Alignment") {
                Some("taCenter") => 0.5,
                Some("taRightJustify") => 1.0,
                _ => 0.0,
            };
            l.set_xalign(xalign);
            l.set_yalign(if o.ident("Layout") == Some("tlCenter") { 0.5 } else { 0.0 });
            if o.bool_or("WordWrap", false) {
                l.set_wrap(true);
                l.set_wrap_mode(pango::WrapMode::WordChar);
            } else if !o.bool_or("AutoSize", true) {
                l.set_ellipsize(pango::EllipsizeMode::End);
            }
            l.upcast()
        }
        "TLinkLabel" => {
            let l = gtk::Label::new(None);
            l.set_markup(&caption);
            l.set_xalign(0.0);
            l.upcast()
        }
        "TButton" => {
            let b = gtk::Button::with_mnemonic(&mnemonic(&caption));
            b.upcast()
        }
        "TSpeedButton" | "TToolButton" => {
            let toggle = o.int_or("GroupIndex", 0) != 0 || o.bool_or("AllowAllUp", false);
            let b: gtk::Button = if toggle {
                let t = gtk::ToggleButton::new();
                t.set_active(o.bool_or("Down", false));
                t.upcast()
            } else {
                gtk::Button::new()
            };
            let icon = ctx.icon(o, "Images");
            match (icon, caption.is_empty() || o.class == "TToolButton") {
                (Some((tex, size)), _) => b.set_child(Some(&image_widget(&tex, size))),
                (None, false) => b.set_label(&strip_mnemonic(&caption)),
                _ => {}
            }
            if o.bool_or("Flat", false) || o.class == "TToolButton" {
                b.add_css_class("flat");
            }
            b.upcast()
        }
        "TCheckBox" => {
            let c = gtk::CheckButton::with_mnemonic(&mnemonic(&caption));
            c.set_active(o.bool_or("Checked", false) || o.ident("State") == Some("cbChecked"));
            c.upcast()
        }
        "TRadioButton" => {
            let c = gtk::CheckButton::with_mnemonic(&mnemonic(&caption));
            c.set_active(o.bool_or("Checked", false));
            c.upcast()
        }
        "TEdit" | "THotKey" => {
            let e = gtk::Entry::new();
            if let Some(t) = o.str("Text") {
                e.set_text(t);
            }
            if o.int("PasswordChar").is_some() || o.str("PasswordChar").is_some_and(|s| !s.is_empty()) {
                e.set_visibility(false);
            }
            if o.bool_or("ReadOnly", false) {
                e.set_editable(false);
            }
            if let Some(m) = o.int("MaxLength").filter(|&m| m > 0) {
                e.set_max_length(m as i32);
            }
            if o.ident("Alignment") == Some("taCenter") {
                EditableExt::set_alignment(&e, 0.5);
            } else if o.ident("Alignment") == Some("taRightJustify") {
                EditableExt::set_alignment(&e, 1.0);
            }
            e.upcast()
        }
        "TMemo" | "TRichEdit" => {
            let tv = gtk::TextView::new();
            tv.set_wrap_mode(if o.bool_or("WordWrap", true) { gtk::WrapMode::WordChar } else { gtk::WrapMode::None });
            if o.bool_or("ReadOnly", false) {
                tv.set_editable(false);
            }
            let lines = o.strings("Lines.Strings");
            if !lines.is_empty() {
                tv.buffer().set_text(&lines.join("\n"));
            }
            tv.set_left_margin(4);
            tv.set_right_margin(4);
            tv.set_top_margin(2);
            let sw = gtk::ScrolledWindow::new();
            sw.set_child(Some(&tv));
            sw.set_has_frame(true);
            tv.set_widget_name(&format!("{}.view", o.name));
            ctx.widgets.insert(format!("{}.view", o.name), tv.upcast());
            sw.upcast()
        }
        "TComboBox" | "TComboBoxEx" => {
            let editable = o.class == "TComboBox" && o.ident("Style") != Some("csDropDownList");
            #[allow(deprecated)]
            let cb = if editable { gtk::ComboBoxText::with_entry() } else { gtk::ComboBoxText::new() };
            #[allow(deprecated)]
            {
                for s in o.strings("Items.Strings") {
                    cb.append_text(&s);
                }
                if let Some(i) = o.int("ItemIndex").filter(|&i| i >= 0) {
                    cb.set_active(Some(i as u32));
                }
                if editable {
                    if let Some(t) = o.str("Text") {
                        if let Some(e) = cb.child().and_downcast::<gtk::Entry>() {
                            e.set_text(t);
                        }
                    }
                }
            }
            cb.upcast()
        }
        "TUpDown" => {
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let up = gtk::Button::from_icon_name("pan-up-symbolic");
            let down = gtk::Button::from_icon_name("pan-down-symbolic");
            for b in [&up, &down] {
                b.add_css_class("flat");
                b.set_vexpand(true);
                b.set_focusable(false);
                add_css(b, "min-height: 0; min-width: 0; padding: 0;");
                bx.append(b);
            }
            let ud = Rc::new(UpDown {
                widget: bx.clone(),
                min: RefCell::new(o.int_or("Min", 0)),
                max: RefCell::new(o.int_or("Max", 100)),
                increment: o.int_or("Increment", 1),
                entry: RefCell::new(None),
                position: RefCell::new(o.int_or("Position", 0)),
            });
            let u1 = ud.clone();
            up.connect_clicked(move |_| u1.step(1));
            let u2 = ud.clone();
            down.connect_clicked(move |_| u2.step(-1));
            ctx.updowns.insert(o.name.clone(), ud);
            bx.upcast()
        }
        "TGroupBox" | "TPanel" | "TTabSheet" | "TRadioGroup" | "TScrollBox" => {
            let (dw, dh) = if o.class == "TTabSheet" {
                design_size(o).unwrap_or(ctx.sheet_size)
            } else {
                (o.int_or("Width", 100), o.int_or("Height", 100))
            };
            let bx = VclBox::new(ctx.sc(dw), ctx.sc(dh));
            if o.class == "TGroupBox" || o.class == "TRadioGroup" {
                let fr = gtk::Frame::new(Some(&strip_mnemonic(&caption)));
                bx.put(&fr, 0, 0, ctx.sc(o.int_or("Width", 100)), ctx.sc(o.int_or("Height", 100)), 15, Align::None);
            } else if o.class == "TPanel" && !caption.is_empty() {
                let l = gtk::Label::new(Some(&caption));
                l.set_wrap(true);
                l.set_justify(gtk::Justification::Center);
                bx.put(&l, 0, 0, ctx.sc(o.int_or("Width", 100)), ctx.sc(o.int_or("Height", 100)), 15, Align::None);
                ctx.widgets.insert(format!("{}.caption", o.name), l.upcast());
            }
            if o.class == "TPanel" && o.ident("BevelOuter") != Some("bvNone") && o.align_is_none() {
                bx.add_css_class("frame");
            }
            if o.class == "TRadioGroup" {
                let items = o.strings("Items.Strings");
                let cols = o.int_or("Columns", 1).max(1) as usize;
                let rows = items.len().div_ceil(cols).max(1);
                let w = o.int_or("Width", 100);
                let h = o.int_or("Height", 100);
                let top = 22;
                let rh = (h - top - 6) / rows as i64;
                let cw = (w - 16) / cols as i64;
                let mut first: Option<gtk::CheckButton> = None;
                for (i, s) in items.iter().enumerate() {
                    let c = gtk::CheckButton::with_mnemonic(&mnemonic(&trl(s)));
                    if let Some(f) = &first {
                        c.set_group(Some(f));
                    } else {
                        first = Some(c.clone());
                    }
                    let (row, col) = (i % rows, i / rows);
                    bx.put(
                        &c,
                        ctx.sc(8 + col as i64 * cw),
                        ctx.sc(top + row as i64 * rh),
                        ctx.sc(cw),
                        ctx.sc(rh.max(20)),
                        AK_LEFT | AK_TOP,
                        Align::None,
                    );
                    ctx.widgets.insert(format!("{}.item{}", o.name, i), c.upcast());
                }
                if let Some(i) = o.int("ItemIndex").filter(|&i| i >= 0) {
                    if let Some(c) = ctx.widgets.get(&format!("{}.item{}", o.name, i)) {
                        c.downcast_ref::<gtk::CheckButton>().unwrap().set_active(true);
                    }
                }
            }
            ctx.boxes.insert(o.name.clone(), bx.clone());
            build_children(o, &bx, ctx, form_name);
            bx.upcast()
        }
        "TPageControl" => {
            let nb = gtk::Notebook::new();
            let active = o.ident("ActivePage").map(String::from);
            let mut active_idx = 0;
            // client area of the tab sheets (`TPageControl.DisplayRect`)
            let sheet_w = o.int_or("Width", 200) - 8;
            let sheet_h = o.int_or("Height", 200) - (ctx.form_text_height + 10) - 6;
            ctx.sheet_size = o
                .children
                .iter()
                .filter(|c| c.class == "TTabSheet")
                .find_map(design_size)
                .unwrap_or((sheet_w, sheet_h));
            for (i, sheet) in o.children.iter().filter(|c| c.class == "TTabSheet").enumerate() {
                let mut sheet = sheet.clone();
                let (sw, sh) = design_size(&sheet).unwrap_or(ctx.sheet_size);
                sheet.props.entry("Width").or_insert(serde_json::json!(sw));
                sheet.props.entry("Height").or_insert(serde_json::json!(sh));
                let sheet = &sheet;
                let page = create(sheet, ctx, form_name).unwrap();
                let label = gtk::Label::with_mnemonic(&mnemonic(&tr_caption(sheet.str("Caption").unwrap_or(""))));
                nb.append_page(&page, Some(&label));
                ctx.widgets.insert(sheet.name.clone(), page);
                if active.as_deref() == Some(sheet.name.as_str()) {
                    active_idx = i as u32;
                }
            }
            nb.set_current_page(Some(active_idx));
            // tabs scroll when they don't fit (like VCL's tab arrows)
            nb.set_scrollable(true);
            nb.upcast()
        }
        "TListView" | "TStringGrid" | "TDrawGrid" => {
            let sw = gtk::ScrolledWindow::new();
            sw.set_has_frame(true);
            sw.upcast()
        }
        "TListBox" | "TCheckListBox" => {
            let lb = gtk::ListBox::new();
            lb.set_selection_mode(gtk::SelectionMode::Single);
            for s in o.strings("Items.Strings") {
                let row: gtk::Widget = if o.class == "TCheckListBox" {
                    gtk::CheckButton::with_label(&trl(&s)).upcast()
                } else {
                    let l = gtk::Label::new(Some(&s));
                    l.set_xalign(0.0);
                    l.upcast()
                };
                lb.append(&row);
            }
            let sw = gtk::ScrolledWindow::new();
            sw.set_child(Some(&lb));
            sw.set_has_frame(true);
            ctx.widgets.insert(format!("{}.list", o.name), lb.upcast());
            sw.upcast()
        }
        "TStatusBar" => {
            let l = gtk::Label::new(o.str("SimpleText"));
            l.set_xalign(0.0);
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_margin_start(4);
            let fr = gtk::Frame::new(None);
            fr.set_child(Some(&l));
            ctx.widgets.insert(format!("{}.label", o.name), l.upcast());
            fr.upcast()
        }
        "TTrackBar" => {
            let s = gtk::Scale::with_range(
                gtk::Orientation::Horizontal,
                o.int_or("Min", 0) as f64,
                o.int_or("Max", 10) as f64,
                1.0,
            );
            s.set_value(o.int_or("Position", 0) as f64);
            s.set_draw_value(false);
            s.upcast()
        }
        "TCGauge" | "TProgressBar" => {
            let da = gtk::DrawingArea::new();
            da.upcast()
        }
        "TDateTimePicker" => {
            let mb = gtk::MenuButton::new();
            let cal = gtk::Calendar::new();
            let pop = gtk::Popover::new();
            pop.set_child(Some(&cal));
            mb.set_popover(Some(&pop));
            ctx.widgets.insert(format!("{}.calendar", o.name), cal.upcast());
            mb.upcast()
        }
        "TToolBar" => {
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            let list = o.ident("Images").map(String::from);
            for c in &o.children {
                if c.class == "TToolButton" {
                    if c.ident("Style") == Some("tbsSeparator") || c.ident("Style") == Some("tbsDivider") {
                        bx.append(&gtk::Separator::new(gtk::Orientation::Vertical));
                        continue;
                    }
                    let b = gtk::Button::new();
                    b.add_css_class("flat");
                    if let (Some(l), Some(i)) = (&list, c.int("ImageIndex")) {
                        if let Some(Some(tex)) = ctx.image_lists.get(l).and_then(|v| v.get(i as usize)) {
                            let size = *ctx.image_sizes.get(l).unwrap_or(&24);
                            b.set_child(Some(&image_widget(tex, size)));
                        }
                    }
                    if let Some(h) = c.str("Hint") {
                        b.set_tooltip_text(Some(&trl(h)));
                    }
                    ctx.widgets.insert(c.name.clone(), b.clone().upcast());
                    bx.append(&b);
                } else if let Some(w) = create(c, ctx, form_name) {
                    // other controls on a tool bar (search box etc.)
                    w.set_size_request(ctx.sc(c.int_or("Width", 100)), -1);
                    ctx.widgets.insert(c.name.clone(), w.clone());
                    bx.append(&w);
                }
            }
            bx.upcast()
        }
        "TSplitter" => {
            let orient = if matches!(align_of(o), Align::Top | Align::Bottom) {
                gtk::Orientation::Horizontal
            } else {
                gtk::Orientation::Vertical
            };
            gtk::Separator::new(orient).upcast()
        }
        "TBevel" => {
            let shape = o.ident("Shape").unwrap_or("bsBox");
            match shape {
                "bsTopLine" | "bsBottomLine" | "bsSpacer" => {
                    let s = gtk::Separator::new(gtk::Orientation::Horizontal);
                    s.set_valign(if shape == "bsBottomLine" { gtk::Align::End } else { gtk::Align::Start });
                    s.upcast()
                }
                "bsLeftLine" | "bsRightLine" => gtk::Separator::new(gtk::Orientation::Vertical).upcast(),
                _ => gtk::Frame::new(None).upcast(),
            }
        }
        "TImage" => gtk::Picture::new().upcast(),
        _ => return None,
    };

    if let Some(h) = o.str("Hint") {
        if !h.is_empty() {
            w.set_tooltip_text(Some(&trl(h)));
        }
    }
    if !o.bool_or("Visible", true) {
        w.set_visible(false);
    }
    if !o.bool_or("Enabled", true) {
        w.set_sensitive(false);
    }
    apply_font(&w, o, ctx.form_font_height);
    w.set_widget_name(&o.name);
    Some(w)
}

trait AlignNone {
    fn align_is_none(&self) -> bool;
}

impl AlignNone for FormObj {
    fn align_is_none(&self) -> bool {
        align_of(self) == Align::None
    }
}

fn build_children(parent: &FormObj, bx: &VclBox, ctx: &mut Ctx, form_name: &str) {
    // Windowed controls in tab order, others first (z-order below).
    let mut kids: Vec<&FormObj> = parent.children.iter().collect();
    kids.sort_by_key(|c| c.int("TabOrder").map(|t| t + 1).unwrap_or(0));
    for c in kids {
        if c.class == "TTabSheet" {
            continue;
        }
        let Some(w) = create(c, ctx, form_name) else { continue };
        let x = ctx.sc(c.int_or("Left", 0));
        let y = ctx.sc(c.int_or("Top", 0));
        let (nw, nh) = {
            let (_, nw, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
            let (_, nh, _, _) = w.measure(gtk::Orientation::Vertical, -1);
            (nw, nh)
        };
        let width = c.int("Width").map(|v| ctx.sc(v)).unwrap_or(nw);
        let height = c.int("Height").map(|v| ctx.sc(v)).unwrap_or(nh);
        let autosize = c.class == "TLabel" && c.bool_or("AutoSize", true) && !c.bool_or("WordWrap", false)
            && align_of(c) == Align::None;
        if autosize {
            let xa = match c.ident("Alignment") {
                Some("taCenter") => 0.5,
                Some("taRightJustify") => 1.0,
                _ => 0.0,
            };
            bx.put_autosize(&w, x, y, width, height, anchors_of(c, ctx.use_tag), xa);
        } else {
            bx.put(&w, x, y, width, height, anchors_of(c, ctx.use_tag), align_of(c));
        }
        ctx.widgets.insert(c.name.clone(), w);
    }
}

/// VCL `ShortCut` value → GTK accelerator string.
pub fn shortcut_to_accel(code: i64) -> Option<String> {
    let mut s = String::new();
    if code & 0x4000 != 0 {
        s.push_str("<Control>");
    }
    if code & 0x8000 != 0 {
        s.push_str("<Alt>");
    }
    if code & 0x2000 != 0 {
        s.push_str("<Shift>");
    }
    let key = (code & 0xff) as u32;
    let name = match key {
        0x70..=0x87 => format!("F{}", key - 0x6f),
        0x30..=0x39 => ((key as u8) as char).to_string(),
        0x41..=0x5a => ((key as u8 + 32) as char).to_string(),
        0x08 => "BackSpace".into(),
        0x09 => "Tab".into(),
        0x0d => "Return".into(),
        0x1b => "Escape".into(),
        0x20 => "space".into(),
        0x21 => "Page_Up".into(),
        0x22 => "Page_Down".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2d => "Insert".into(),
        0x2e => "Delete".into(),
        0x13 => "Pause".into(),
        0x60..=0x69 => format!("KP_{}", key - 0x60),
        _ => return None,
    };
    s.push_str(&name);
    Some(s)
}

/// VCL `TShortCut` of a key press (inverse of [`shortcut_to_accel`]);
/// `None` for keys without a virtual key code or bare modifiers.
pub fn accel_to_shortcut(key: gdk::Key, mods: gdk::ModifierType) -> Option<u16> {
    let kv = key.to_lower();
    let name = kv.name().map(|n| n.to_string()).unwrap_or_default();
    let vk: u16 = if let Some(c) = kv.to_unicode().filter(|c| c.is_ascii_alphanumeric()) {
        c.to_ascii_uppercase() as u16
    } else if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=24).contains(n)) {
        0x6f + n
    } else if let Some(n) = name.strip_prefix("KP_").and_then(|n| n.parse::<u16>().ok()).filter(|n| *n <= 9) {
        0x60 + n
    } else {
        match name.as_str() {
            "BackSpace" => 0x08,
            "Tab" | "ISO_Left_Tab" => 0x09,
            "Return" | "KP_Enter" => 0x0d,
            "Pause" => 0x13,
            "Escape" => 0x1b,
            "space" => 0x20,
            "Page_Up" => 0x21,
            "Page_Down" => 0x22,
            "End" => 0x23,
            "Home" => 0x24,
            "Left" => 0x25,
            "Up" => 0x26,
            "Right" => 0x27,
            "Down" => 0x28,
            "Insert" => 0x2d,
            "Delete" => 0x2e,
            _ => return None,
        }
    };
    let mut code = vk;
    if mods.contains(gdk::ModifierType::SHIFT_MASK) {
        code |= 0x2000;
    }
    if mods.contains(gdk::ModifierType::CONTROL_MASK) {
        code |= 0x4000;
    }
    if mods.contains(gdk::ModifierType::ALT_MASK) {
        code |= 0x8000;
    }
    Some(code)
}

/// `ShortCutToText()`: e.g. "Ctrl+Alt+P".
pub fn shortcut_to_text(code: u16) -> String {
    let mut s = String::new();
    if code & 0x2000 != 0 {
        s.push_str("Shift+");
    }
    if code & 0x4000 != 0 {
        s.push_str("Ctrl+");
    }
    if code & 0x8000 != 0 {
        s.push_str("Alt+");
    }
    let key = code & 0xff;
    let name = match key {
        0x30..=0x39 | 0x41..=0x5a => ((key as u8) as char).to_string(),
        0x60..=0x69 => format!("Num {}", key - 0x60),
        0x70..=0x87 => format!("F{}", key - 0x6f),
        0x08 => "BkSp".into(),
        0x09 => "Tab".into(),
        0x0d => "Enter".into(),
        0x13 => "Pause".into(),
        0x1b => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2d => "Ins".into(),
        0x2e => "Del".into(),
        _ => format!("#{key}"),
    };
    s.push_str(&name);
    s
}

#[derive(Default)]
struct MenuMaps {
    submenus: HashMap<String, gio::Menu>,
    items: HashMap<String, (gio::Menu, i32, gio::MenuItem)>,
}

fn build_menu(
    items: &[FormObj],
    actions: &gio::SimpleActionGroup,
    accels: &mut Vec<(String, String)>,
    maps: &mut MenuMaps,
    prefix: &str,
) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    for it in items.iter().filter(|c| c.class == "TMenuItem") {
        let caption = it.str("Caption").unwrap_or("");
        if caption == "-" {
            if section.n_items() > 0 {
                menu.append_section(None, &section);
            }
            section = gio::Menu::new();
            continue;
        }
        let label = mnemonic(&tr_caption(caption));
        let checkable = it.bool_or("AutoCheck", false) || it.int("Checked").is_some() || it.props.contains_key("Checked")
            || it.bool_or("RadioItem", false);
        let action = if checkable {
            gio::SimpleAction::new_stateful(&it.name, None, &it.bool_or("Checked", false).to_variant())
        } else {
            gio::SimpleAction::new(&it.name, None)
        };
        if !it.bool_or("Enabled", true) {
            action.set_enabled(false);
        }
        actions.add_action(&action);
        if let Some(sc) = it.int("ShortCut") {
            if let Some(a) = shortcut_to_accel(sc) {
                accels.push((format!("{prefix}.{}", it.name), a));
            }
        }
        let has_sub = it.children.iter().any(|c| c.class == "TMenuItem");
        if has_sub {
            let sub = build_menu(&it.children, actions, accels, maps, prefix);
            let mi = gio::MenuItem::new_submenu(Some(&label), &sub);
            mi.set_attribute_value("pwtech-name", Some(&it.name.to_variant()));
            maps.items.insert(it.name.clone(), (section.clone(), section.n_items(), mi.clone()));
            section.append_item(&mi);
            maps.submenus.insert(it.name.clone(), sub);
        } else {
            let mi = gio::MenuItem::new(Some(&label), Some(&format!("{prefix}.{}", it.name)));
            if !it.bool_or("Visible", true) {
                mi.set_attribute_value("hidden-when", Some(&"action-missing".to_variant()));
            }
            maps.items.insert(it.name.clone(), (section.clone(), section.n_items(), mi.clone()));
            section.append_item(&mi);
        }
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    menu
}

/// Build a form. `form_name` is the name of the JSON form (for icons).
pub fn build_form(form: FormObj, form_name: &str) -> Built {
    build_form_with_prefix(form, form_name, "m")
}

/// Like [`build_form`], with menu actions in the action group `prefix`
/// (insert [`Built::actions`] under that name) — windows whose menu item
/// names collide with those of the main window use their own prefix.
pub fn build_form_with_prefix(form: FormObj, form_name: &str, prefix: &str) -> Built {
    let scale = form_scale(&form);
    let mut ctx = Ctx {
        widgets: HashMap::new(),
        boxes: HashMap::new(),
        scale,
        use_tag: true,
        form_font_height: form.int_or("Font.Height", -14),
        form_text_height: form.int_or("TextHeight", 17),
        image_lists: HashMap::new(),
        image_sizes: HashMap::new(),
        updowns: HashMap::new(),
        sheet_size: (100, 100),
    };

    // image lists: TVirtualImageList → collection images
    let mut add_lists = |src: &FormObj, src_form: &str, prefix: &str, only: Option<&str>| {
        for c in &src.children {
            if c.class == "TVirtualImageList" && only.is_none_or(|n| n == c.name) {
                let coll = c.ident("ImageCollection").unwrap_or("").to_string();
                let list: Vec<Option<gdk::Texture>> = c
                    .collection("Images")
                    .iter()
                    .map(|i| {
                        let name = i.get("CollectionName").and_then(|v| v.as_str()).unwrap_or("");
                        icons::texture(src_form, &coll, name)
                    })
                    .collect();
                let key = format!("{prefix}{}", c.name);
                ctx.image_sizes.insert(key.clone(), (c.int_or("Width", 16) as f64 * scale).round() as i32);
                ctx.image_lists.insert(key, list);
            }
        }
    };
    add_lists(&form, form_name, "", None);
    // image lists of other forms ("MainForm.ImageList16")
    let mut external: Vec<String> = Vec::new();
    form.walk(&mut |o: &FormObj| {
        for prop in ["Images", "SmallImages", "LargeImages"] {
            if let Some(id) = o.ident(prop) {
                if id.contains('.') && !external.iter().any(|e| e == id) {
                    external.push(id.to_string());
                }
            }
        }
    });
    for id in external {
        let (fref, list) = id.split_once('.').unwrap();
        let fname = fref.strip_suffix("Form").or_else(|| fref.strip_suffix("Dlg")).unwrap_or(fref);
        if let Some(src) = super::form::try_load_form(fname) {
            add_lists(&src, fname, &format!("{fref}."), Some(list));
        }
    }

    let cw = ctx.sc(form.int("ClientWidth").or(form.int("Width")).unwrap_or(400));
    let ch = ctx.sc(form.int("ClientHeight").or(form.int("Height")).unwrap_or(300));
    let root = VclBox::new(cw, ch);
    let mut min = (
        form.int("Constraints.MinWidth").map(|v| ctx.sc(v) - (ctx.sc(form.int_or("Width", 0)) - cw).max(0)).unwrap_or(0),
        form.int("Constraints.MinHeight").map(|v| ctx.sc(v) - (ctx.sc(form.int_or("Height", 0)) - ch).max(0)).unwrap_or(0),
    );
    build_children(&form, &root, &mut ctx, form_name);

    // GTK notebook tabs are higher than VCL tabs: enlarge the window so that
    // the pages keep their design height
    let mut ch = ch;
    if collect_all(&form).iter().any(|c| c.class == "TPageControl") {
        let probe = gtk::Notebook::new();
        probe.append_page(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some(&gtk::Label::new(Some("Xg"))));
        let (header, _, _, _) = probe.measure(gtk::Orientation::Vertical, -1);
        let vcl_tabs = ctx.sc(form.int_or("TextHeight", 17) + 10);
        let extra = header - vcl_tabs;
        if extra > 0 {
            ch += extra;
            if min.1 > 0 {
                min.1 += extra;
            }
        }
    }

    // sub-widgets ("Name.view", "Name.item0", ...) get their key as name
    for (k, w) in &ctx.widgets {
        if k.contains('.') && w.widget_name().starts_with("Gtk") {
            w.set_widget_name(k);
        }
    }

    // associate up-down buttons with their edits
    for c in collect_all(&form) {
        if c.class == "TUpDown" {
            if let (Some(assoc), Some(ud)) = (c.ident("Associate"), ctx.updowns.get(&c.name)) {
                if let Some(e) = ctx.widgets.get(assoc).and_then(|w| w.clone().downcast::<gtk::Entry>().ok()) {
                    *ud.entry.borrow_mut() = Some(e.clone());
                    e.set_text(&ud.position.borrow().to_string());
                }
            }
        }
    }

    // menus
    let actions = gio::SimpleActionGroup::new();
    let mut accels = Vec::new();
    let mut menus = HashMap::new();
    let mut maps = MenuMaps::default();
    for c in collect_all(&form) {
        if c.class == "TMainMenu" || c.class == "TPopupMenu" {
            let m = build_menu(&c.children, &actions, &mut accels, &mut maps, prefix);
            menus.insert(c.name.clone(), m);
        }
    }

    let caption = form.str("Caption").map(tr_caption).unwrap_or_default();
    Built {
        root,
        widgets: ctx.widgets,
        boxes: ctx.boxes,
        scale,
        client: (cw, ch),
        min,
        caption,
        menus,
        actions,
        accels,
        image_lists: ctx.image_lists,
        updowns: ctx.updowns,
        submenus: maps.submenus,
        menu_items: RefCell::new(maps.items),
        form,
    }
}

fn collect_all(form: &FormObj) -> Vec<&FormObj> {
    let mut v = Vec::new();
    form.walk(&mut |o| v.push(o));
    v
}

/// Show a popup menu at the pointer position over `widget` on right click.
pub fn attach_context_menu(widget: &impl IsA<gtk::Widget>, menu: &gio::Menu) -> gtk::PopoverMenu {
    let pop = gtk::PopoverMenu::from_model(Some(menu));
    pop.set_parent(widget);
    pop.set_has_arrow(false);
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    let p = pop.clone();
    gesture.connect_pressed(move |g, _, x, y| {
        p.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        p.popup();
        g.set_state(gtk::EventSequenceState::Claimed);
    });
    widget.add_controller(gesture);
    pop
}

/// Run a closure on idle (deferred to the main loop).
pub fn idle(f: impl FnOnce() + 'static) {
    glib::idle_add_local_once(f);
}
