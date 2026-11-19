//! Translation lookup (`TRL()`, `TRLFormat()` of `upstream/src/main/Language.*`).
//!
//! The English strings of the original are the keys. Until a translation is
//! loaded, [`trl`] returns its argument unchanged. Loading of `.po` / `.lng`
//! files is implemented in [`crate::language_file`].

use std::collections::HashMap;
use std::sync::RwLock;

use crate::unicode::format_w;

static TRANSLATIONS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Install a translation table (msgid → msgstr). `None` = English.
pub fn set_translations(map: Option<HashMap<String, String>>) {
    *TRANSLATIONS.write().unwrap() = map;
}

/// Whether a translation is active.
pub fn has_translation() -> bool {
    TRANSLATIONS.read().unwrap().is_some()
}

/// `TRL()`: translate a string.
pub fn trl(s: &str) -> String {
    if let Some(map) = TRANSLATIONS.read().unwrap().as_ref() {
        if let Some(t) = map.get(s) {
            if !t.is_empty() {
                return t.clone();
            }
        }
    }
    s.to_string()
}

/// `TRLFormat()`: translate a format string and substitute `%1`…`%N`.
pub fn trl_format(fmt: &str, args: &[&str]) -> String {
    format_w(&trl(fmt), args)
}
