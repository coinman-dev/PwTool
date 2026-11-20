//! Helpers for windows: message boxes, snapshots, text of widgets.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gio, glib};
use pwtool_core::language::trl;

use crate::program::PROGRAM_NAME;

/// Icon/kind of a message box (`MB_ICON…`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgIcon {
    Info,
    Warning,
    Error,
    Question,
}

/// Buttons of a message box (`MB_OK`, `MB_YESNO`, `MB_YESNOCANCEL`,
/// `MB_OKCANCEL`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgButtons {
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
}

/// Result of a message box (`IDOK`, `IDYES`…).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgResult {
    Ok,
    Cancel,
    Yes,
    No,
}

/// `MsgBox()`: modal message box; `default_second` selects the second
/// button as default (`MB_DEFBUTTON2`).
pub async fn msg_box(
    parent: Option<&gtk::Window>,
    text: &str,
    icon: MsgIcon,
    buttons: MsgButtons,
    default_second: bool,
) -> MsgResult {
    let (labels, results): (Vec<String>, Vec<MsgResult>) = match buttons {
        MsgButtons::Ok => (vec![trl("OK")], vec![MsgResult::Ok]),
        MsgButtons::OkCancel => (vec![trl("OK"), trl("Cancel")], vec![MsgResult::Ok, MsgResult::Cancel]),
        MsgButtons::YesNo => (vec![format!("&{}", trl("Yes")), format!("&{}", trl("No"))], vec![MsgResult::Yes, MsgResult::No]),
        MsgButtons::YesNoCancel => (
            vec![format!("&{}", trl("Yes")), format!("&{}", trl("No")), trl("Cancel")],
            vec![MsgResult::Yes, MsgResult::No, MsgResult::Cancel],
        ),
    };
    let labels: Vec<String> = labels.iter().map(|l| super::builder::mnemonic(l)).collect();
    let dlg = gtk::Window::builder().modal(true).resizable(false).title(PROGRAM_NAME).build();
    if let Some(p) = parent {
        dlg.set_transient_for(Some(p));
    }
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 12);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let icon_name = match icon {
        MsgIcon::Info => "dialog-information",
        MsgIcon::Warning => "dialog-warning",
        MsgIcon::Error => "dialog-error",
        MsgIcon::Question => "dialog-question",
    };
    let img = gtk::Image::from_icon_name(icon_name);
    img.set_pixel_size(40);
    img.set_valign(gtk::Align::Start);
    hbox.append(&img);
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_max_width_chars(70);
    label.set_selectable(true);
    hbox.append(&label);
    vbox.append(&hbox);
    let bbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bbox.set_halign(gtk::Align::End);
    let result: Rc<RefCell<Option<MsgResult>>> = Rc::new(RefCell::new(None));
    let (tx, rx) = async_channel::bounded::<()>(1);
    let mut default_btn = None;
    for (i, (l, r)) in labels.iter().zip(results.iter()).enumerate() {
        let b = gtk::Button::with_mnemonic(l);
        b.set_size_request(90, -1);
        let res = result.clone();
        let d = dlg.clone();
        let r = *r;
        b.connect_clicked(move |_| {
            *res.borrow_mut() = Some(r);
            d.close();
        });
        if (i == 0 && !default_second) || (i == 1 && default_second) {
            default_btn = Some(b.clone());
        }
        bbox.append(&b);
    }
    vbox.append(&bbox);
    dlg.set_child(Some(&vbox));
    if let Some(b) = default_btn {
        dlg.set_default_widget(Some(&b));
        b.grab_focus();
    }
    let esc = if buttons == MsgButtons::YesNo { MsgResult::No } else if buttons == MsgButtons::Ok { MsgResult::Ok } else { MsgResult::Cancel };
    dlg.connect_close_request(move |_| {
        let _ = tx.try_send(());
        glib::Propagation::Proceed
    });
    let key = gtk::EventControllerKey::new();
    let d2 = dlg.clone();
    key.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            d2.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dlg.add_controller(key);
    dlg.present();
    let _ = rx.recv().await;
    let r = result.borrow().unwrap_or(esc);
    r
}

/// Fire-and-forget message box.
pub fn msg(parent: Option<&gtk::Window>, text: &str, icon: MsgIcon) {
    let parent = parent.cloned();
    let text = text.to_string();
    glib::spawn_future_local(async move {
        msg_box(parent.as_ref(), &text, icon, MsgButtons::Ok, false).await;
    });
}

/// Save a picture of a window to a PNG file (for automatic visual checks).
pub fn snapshot_window(window: &gtk::Window, path: &str) -> bool {
    let w = window.width();
    let h = window.height();
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let Some(node) = snapshot.to_node() else { return false };
    let Some(native) = window.native() else { return false };
    let Some(renderer) = native.renderer() else { return false };
    let tex = renderer.render_texture(&node, None);
    tex.save_to_png(path).is_ok()
}

/// Text of an editable combo box (`TComboBox.Text`).
#[allow(deprecated)]
pub fn combo_text(cb: &gtk::ComboBoxText) -> String {
    match cb.child().and_downcast::<gtk::Entry>() {
        Some(e) => e.text().to_string(),
        None => cb.active_text().map(|s| s.to_string()).unwrap_or_default(),
    }
}

#[allow(deprecated)]
pub fn set_combo_text(cb: &gtk::ComboBoxText, text: &str) {
    if let Some(e) = cb.child().and_downcast::<gtk::Entry>() {
        e.set_text(text);
    }
}

#[allow(deprecated)]
pub fn combo_entry(cb: &gtk::ComboBoxText) -> Option<gtk::Entry> {
    cb.child().and_downcast::<gtk::Entry>()
}

/// Replace the items of a combo box.
#[allow(deprecated)]
pub fn set_combo_items(cb: &gtk::ComboBoxText, items: &[String]) {
    cb.remove_all();
    for s in items {
        cb.append_text(s);
    }
}

/// Open a URL or file with the desktop's default application
/// (`ExecuteShellOp`).
pub fn open_uri(parent: Option<&gtk::Window>, uri: &str) {
    let uri = if uri.contains("://") || uri.starts_with("mailto:") {
        uri.to_string()
    } else {
        gio::File::for_path(uri).uri().to_string()
    };
    let launcher = gtk::UriLauncher::new(&uri);
    let p = parent.cloned();
    let u = uri.clone();
    launcher.launch(parent, gio::Cancellable::NONE, move |r| {
        if r.is_err() {
            msg(
                p.as_ref(),
                &pwtool_core::language::trl_format("Could not open file/execute operation\n\"%1\".", &[&u]),
                MsgIcon::Error,
            );
        }
    });
}

/// Font of a `FontToString()` string ("Name;Size;Style;Color"; style bits:
/// 1 bold, 2 italic, 4 underline, 8 strikeout).
#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub name: String,
    pub size: i32,
    pub style: i32,
    pub color: String,
}

impl FontSpec {
    /// `StringToFont()`; `None` if name or size are missing.
    pub fn parse(s: &str) -> Option<FontSpec> {
        let mut it = s.split(';');
        let name = it.next()?.trim().to_string();
        let size: i32 = it.next()?.trim().parse().ok().filter(|&v| v != 0)?;
        let style = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let color = it.next().unwrap_or("clWindowText").trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some(FontSpec { name, size, style, color })
    }

    /// `FontToString()`.
    pub fn to_font_string(&self) -> String {
        format!("{};{};{};{}", self.name, self.size, self.style, self.color)
    }

    pub fn from_pango(d: &gtk::pango::FontDescription) -> FontSpec {
        let mut style = 0;
        if d.weight() >= gtk::pango::Weight::Bold {
            style |= 1;
        }
        if d.style() != gtk::pango::Style::Normal {
            style |= 2;
        }
        FontSpec {
            name: d.family().map(|f| f.to_string()).unwrap_or_default(),
            size: (d.size() as f64 / gtk::pango::SCALE as f64).round() as i32,
            style,
            color: "clWindowText".into(),
        }
    }

    pub fn to_pango(&self) -> gtk::pango::FontDescription {
        let mut d = gtk::pango::FontDescription::new();
        d.set_family(&self.name);
        d.set_size(self.size.abs() * gtk::pango::SCALE);
        if self.style & 1 != 0 {
            d.set_weight(gtk::pango::Weight::Bold);
        }
        if self.style & 2 != 0 {
            d.set_style(gtk::pango::Style::Italic);
        }
        d
    }

    /// CSS declarations for this font.
    pub fn css(&self) -> String {
        let mut c = format!("font-family: \"{}\"; font-size: {}pt;", self.name.replace('"', ""), self.size.abs());
        if self.style & 1 != 0 {
            c.push_str(" font-weight: bold;");
        }
        if self.style & 2 != 0 {
            c.push_str(" font-style: italic;");
        }
        let mut deco = Vec::new();
        if self.style & 4 != 0 {
            deco.push("underline");
        }
        if self.style & 8 != 0 {
            deco.push("line-through");
        }
        if !deco.is_empty() {
            c.push_str(&format!(" text-decoration: {};", deco.join(" ")));
        }
        c
    }
}

thread_local! {
    static GUI_FONT_CSS: std::cell::RefCell<Option<gtk::CssProvider>> = const { std::cell::RefCell::new(None) };
}

/// `ChangeGUIFont()`: font for all windows (empty string = default font).
pub fn set_gui_font(font_string: &str) {
    let Some(display) = gtk::gdk::Display::default() else { return };
    GUI_FONT_CSS.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(old) = p.take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        if let Some(f) = FontSpec::parse(font_string) {
            let prov = gtk::CssProvider::new();
            prov.load_from_data(&format!("window, window * {{ {} }}", f.css()));
            gtk::style_context_add_provider_for_display(&display, &prov, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            *p = Some(prov);
        }
    });
}

/// User interface style: "Light"/"Dark" force the colour scheme, "Default"
/// follows the desktop: the values GTK got from the desktop (KDE's GTK
/// settings, the colour scheme of the settings portal) are left alone, or
/// restored when switching back from a forced style.
pub fn set_ui_style(name: &str) {
    thread_local! {
        static SYSTEM: std::cell::RefCell<Option<(bool, Option<glib::Value>)>> = const { std::cell::RefCell::new(None) };
    }
    let Some(s) = gtk::Settings::default() else { return };
    let scheme_prop = s.find_property("gtk-interface-color-scheme");
    SYSTEM.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            let scheme = scheme_prop.as_ref().map(|_| s.property_value("gtk-interface-color-scheme"));
            *c = Some((s.is_gtk_application_prefer_dark_theme(), scheme));
        }
        let (sys_dark, sys_scheme) = c.clone().unwrap();
        let forced = match name {
            "Dark" => Some((true, "dark")),
            "Light" => Some((false, "light")),
            _ => None,
        };
        match forced {
            Some((dark, nick)) => {
                s.set_gtk_application_prefer_dark_theme(dark);
                if let Some(pspec) = &scheme_prop {
                    if let Some(v) = glib::EnumClass::with_type(pspec.value_type()).and_then(|cl| cl.to_value_by_nick(nick)) {
                        s.set_property_from_value("gtk-interface-color-scheme", &v);
                    }
                }
            }
            None => {
                if s.is_gtk_application_prefer_dark_theme() != sys_dark {
                    s.set_gtk_application_prefer_dark_theme(sys_dark);
                }
                if let Some(v) = sys_scheme {
                    if s.property_value("gtk-interface-color-scheme").transform::<i32>().ok().and_then(|x| x.get::<i32>().ok())
                        != v.transform::<i32>().ok().and_then(|x| x.get::<i32>().ok())
                    {
                        s.set_property_from_value("gtk-interface-color-scheme", &v);
                    }
                }
            }
        }
    });
}

thread_local! {
    static CLASS_FONTS: std::cell::RefCell<std::collections::HashMap<String, gtk::CssProvider>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Font for all widgets with the CSS class `class` (`None` = default font).
pub fn set_class_font(class: &str, font: Option<&FontSpec>) {
    let Some(display) = gtk::gdk::Display::default() else { return };
    CLASS_FONTS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(old) = m.remove(class) {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        if let Some(f) = font {
            let prov = gtk::CssProvider::new();
            prov.load_from_data(&format!(".{class}, .{class} text {{ {} }}", f.css()));
            gtk::style_context_add_provider_for_display(&display, &prov, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
            m.insert(class.to_string(), prov);
        }
    });
}

/// Font dialog (`TFontDialog::Execute()`); calls `done` with the chosen font.
pub fn choose_font(parent: &gtk::Window, current: Option<&FontSpec>, done: impl FnOnce(FontSpec) + 'static) {
    let dlg = gtk::FontDialog::new();
    let cur = current.map(|f| f.to_pango());
    dlg.choose_font(Some(parent), cur.as_ref(), gtk::gio::Cancellable::NONE, move |r| {
        if let Ok(d) = r {
            done(FontSpec::from_pango(&d));
        }
    });
}

/// Drag the text returned by `text` from `widget` to other programs
/// (`StartEditBoxDragDrop()`: the password is dragged from the quality bar or
/// the info label while the left mouse button is held).
pub fn add_text_drag_source(widget: &impl IsA<gtk::Widget>, text: impl Fn() -> zeroize::Zeroizing<String> + 'static) {
    let src = gtk::DragSource::new();
    src.set_actions(gtk::gdk::DragAction::COPY);
    src.connect_prepare(move |_, _, _| {
        let t = text();
        if t.is_empty() {
            return None;
        }
        Some(gtk::gdk::ContentProvider::for_value(&t.as_str().to_value()))
    });
    widget.add_controller(src);
}
