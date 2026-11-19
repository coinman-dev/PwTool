//! Helpers for windows: message boxes, snapshots, text of widgets.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gio, glib};
use pwtech_core::language::trl;

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
        MsgButtons::YesNo => (vec![trl("&Yes"), trl("&No")], vec![MsgResult::Yes, MsgResult::No]),
        MsgButtons::YesNoCancel => (
            vec![trl("&Yes"), trl("&No"), trl("Cancel")],
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
                &pwtech_core::language::trl_format("Could not open file/execute operation\n\"%1\".", &[&u]),
                MsgIcon::Error,
            );
        }
    });
}
