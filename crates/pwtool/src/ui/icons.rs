//! Icons of the original forms (PNG, embedded at build time).

use std::cell::RefCell;
use std::collections::HashMap;

use gtk4 as gtk;
use gtk::{gdk, glib};


thread_local! {
    static CACHE: RefCell<HashMap<String, Option<gdk::Texture>>> = RefCell::new(HashMap::new());
}

/// Texture of image `name` of image collection `collection` in form `form`.
pub fn texture(form: &str, collection: &str, name: &str) -> Option<gdk::Texture> {
    let key = format!("{form}/{collection}/{name}");
    CACHE.with(|c| {
        c.borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| {
                let bytes = pwtool_app::icons::form_icon(form, collection, name)?;
                gdk::Texture::from_bytes(&glib::Bytes::from_static(bytes)).ok()
            })
            .clone()
    })
}

/// Application icon (from `pwtool_default.ico` converted to PNG at runtime
/// is not possible without a decoder; the password icon of the main form is
/// used instead).
pub fn app_icon() -> Option<gdk::Texture> {
    texture("Main", "ImageCollection32", "password")
}
