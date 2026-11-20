//! Clipboard for sensitive data (`upstream/src/util/SecureClipboard.*`).
//!
//! Text is put on the clipboard together with the KDE hint
//! `x-kde-passwordManagerHint: secret`, so that Klipper does not keep it in
//! its history. A SHA-256 digest of the text is kept; automatic clearing only
//! clears the clipboard if it still holds that text.

use std::cell::RefCell;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, glib};
use zeroize::Zeroizing;

thread_local! {
    static DIGEST: RefCell<Option<[u8; 32]>> = const { RefCell::new(None) };
    static ON_SET: RefCell<Vec<Box<dyn Fn()>>> = RefCell::new(Vec::new());
}

fn digest(text: &str) -> [u8; 32] {
    let units: Zeroizing<Vec<u8>> = Zeroizing::new(text.encode_utf16().flat_map(u16::to_le_bytes).collect());
    pwtool_core::crypto::sha256(&units[..units.len().min(1_000_000)])
}

fn clipboard() -> Option<gdk::Clipboard> {
    gdk::Display::default().map(|d| d.clipboard())
}

/// Register a function called after sensitive data was put on the clipboard
/// (`RegisterOnSetDataFun`; the main window starts the auto-clear timer).
pub fn on_set_data(f: impl Fn() + 'static) {
    ON_SET.with(|v| v.borrow_mut().push(Box::new(f)));
}

/// Put sensitive text on the clipboard (`SecureClipboard::SetData`).
pub fn set_data(text: &str) {
    if text.is_empty() {
        return;
    }
    let Some(cb) = clipboard() else { return };
    let providers = [
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &glib::Bytes::from(text.as_bytes())),
        gdk::ContentProvider::for_value(&text.to_value()),
        gdk::ContentProvider::for_bytes("x-kde-passwordManagerHint", &glib::Bytes::from_static(b"secret")),
    ];
    let p = gdk::ContentProvider::new_union(&providers);
    let _ = cb.set_content(Some(&p));
    DIGEST.with(|d| *d.borrow_mut() = Some(digest(text)));
    ON_SET.with(|v| {
        for f in v.borrow().iter() {
            f();
        }
    });
}

/// Put sensitive text on the clipboard with the password-manager hint, but
/// without automatic clearing (decrypted text: the original uses the plain
/// clipboard here).
pub fn set_hidden(text: &str) {
    let Some(cb) = clipboard() else { return };
    let providers = [
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &glib::Bytes::from(text.as_bytes())),
        gdk::ContentProvider::for_value(&text.to_value()),
        gdk::ContentProvider::for_bytes("x-kde-passwordManagerHint", &glib::Bytes::from_static(b"secret")),
    ];
    let _ = cb.set_content(Some(&gdk::ContentProvider::new_union(&providers)));
}

/// Put ordinary (non-sensitive) text on the clipboard.
pub fn set_plain(text: &str) {
    if let Some(cb) = clipboard() {
        cb.set_text(text);
    }
}

/// Clear the clipboard (`ClearData`): with `force` always, otherwise only if
/// it still holds the text set by [`set_data`]. The callback receives
/// whether the operation succeeded.
pub fn clear_data(force: bool, done: impl FnOnce(bool) + 'static) {
    let has_digest = DIGEST.with(|d| d.borrow().is_some());
    if !has_digest && !force {
        done(true);
        return;
    }
    let Some(cb) = clipboard() else {
        done(false);
        return;
    };
    let cb2 = cb.clone();
    cb.read_text_async(gtk::gio::Cancellable::NONE, move |r| {
        let current = r.ok().flatten().map(|s| Zeroizing::new(s.to_string()));
        let clear = match &current {
            Some(t) if !t.is_empty() => force || DIGEST.with(|d| *d.borrow() == Some(digest(t))),
            _ => false,
        };
        if clear {
            let _ = cb2.set_content(None::<&gdk::ContentProvider>);
            cb2.set_text("");
        }
        DIGEST.with(|d| *d.borrow_mut() = None);
        done(true);
    });
}

/// Read the clipboard text.
pub fn read_text(done: impl FnOnce(Option<Zeroizing<String>>) + 'static) {
    let Some(cb) = clipboard() else {
        done(None);
        return;
    };
    cb.read_text_async(gtk::gio::Cancellable::NONE, move |r| {
        done(r.ok().flatten().map(|s| Zeroizing::new(s.to_string())));
    });
}
