//! Translation lookup (`TRL()`, `TRLFormat()` of `upstream/src/main/Language.*`).
//!
//! The English strings of the original are the keys. Until a translation is
//! loaded, [`trl`] returns its argument unchanged. Loading of `.po` / `.lng`
//! files is implemented in [`crate::language_file`].

use std::collections::HashMap;
use std::sync::RwLock;

use crate::unicode::format_w;

static TRANSLATIONS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);
static IS_LNG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Install a loaded translation file.
pub fn set_language(lf: Option<crate::language_file::LanguageFile>) {
    match lf {
        Some(lf) => {
            IS_LNG.store(lf.format == crate::language_file::LanguageFormat::Lng, std::sync::atomic::Ordering::Relaxed);
            set_translations(Some(lf.into_map()));
        }
        None => set_translations(None),
    }
}

/// Install a translation table (upper-cased msgid → msgstr). `None` = English.
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
        if let Some(t) = map.get(&crate::language_file::key(s)) {
            if !t.is_empty() {
                return t.clone();
            }
        }
    }
    s.to_string()
}

/// `TRL(str, context)`: translation with context; untranslated string if
/// there is no translation for this context.
pub fn trl_ctx(s: &str, ctx: &str) -> String {
    if let Some(map) = TRANSLATIONS.read().unwrap().as_ref() {
        let k = if IS_LNG.load(std::sync::atomic::Ordering::Relaxed) {
            format!("{s} (({ctx}))")
        } else {
            format!("{s}||{ctx}")
        };
        if let Some(t) = map.get(&crate::language_file::key(&k)) {
            return t.clone();
        }
    }
    s.to_string()
}

/// `TRLFormat()`: translate a format string and substitute `%1`…`%N`.
pub fn trl_format(fmt: &str, args: &[&str]) -> String {
    format_w(&trl(fmt), args)
}
