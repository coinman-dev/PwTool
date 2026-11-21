//! Clipboard for sensitive data (`SecureClipboard` of the original).
//! Passwords are marked so that clipboard histories leave them alone:
//! on Windows with the clipboard formats of `pwtool_app::platform::clipboard`,
//! on Linux with the KDE hint `x-kde-passwordManagerHint` (arboard's
//! `exclude_from_history`). A digest of the text is kept; the automatic
//! clearing clears the clipboard only if it still holds that text.

use std::cell::RefCell;

use zeroize::Zeroizing;

thread_local! {
    static BOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
    static DIGEST: RefCell<Option<[u8; 32]>> = const { RefCell::new(None) };
    static ON_SET: RefCell<Vec<Box<dyn Fn()>>> = RefCell::new(Vec::new());
}

fn digest(text: &str) -> [u8; 32] {
    let units: Zeroizing<Vec<u8>> = Zeroizing::new(text.encode_utf16().flat_map(u16::to_le_bytes).collect());
    pwtool_core::crypto::sha256(&units[..units.len().min(1_000_000)])
}

/// Run `f` with the clipboard (kept open: on Linux the content lives as
/// long as the clipboard object).
fn with_board<R>(f: impl FnOnce(&mut arboard::Clipboard) -> R) -> Option<R> {
    BOARD.with(|b| {
        let mut b = b.borrow_mut();
        if b.is_none() {
            *b = arboard::Clipboard::new().ok();
        }
        b.as_mut().map(f)
    })
}

/// Register a function called after sensitive data was put on the
/// clipboard (the main window starts the auto-clear timer).
pub fn on_set_data(f: impl Fn() + 'static) {
    ON_SET.with(|v| v.borrow_mut().push(Box::new(f)));
}

fn set_secret(text: &str) -> bool {
    #[cfg(windows)]
    if pwtool_app::platform::clipboard::set_secret_text(text) {
        return true;
    }
    with_board(|b| {
        #[cfg(target_os = "linux")]
        {
            use arboard::SetExtLinux;
            b.set().exclude_from_history().text(text).is_ok()
        }
        #[cfg(not(target_os = "linux"))]
        {
            b.set_text(text).is_ok()
        }
    })
    .unwrap_or(false)
}

/// Put sensitive text on the clipboard (`SecureClipboard::SetData`).
pub fn set_data(text: &str) {
    if text.is_empty() {
        return;
    }
    set_secret(text);
    DIGEST.with(|d| *d.borrow_mut() = Some(digest(text)));
    ON_SET.with(|v| {
        for f in v.borrow().iter() {
            f();
        }
    });
}

/// Sensitive text without automatic clearing (decrypted text).
pub fn set_hidden(text: &str) {
    set_secret(text);
}

/// Ordinary text.
pub fn set_plain(text: &str) {
    with_board(|b| b.set_text(text));
}

/// Text on the clipboard.
pub fn read_text() -> Option<Zeroizing<String>> {
    with_board(|b| b.get_text().ok().map(Zeroizing::new)).flatten()
}

/// Clear the clipboard (`ClearData`): with `force` always, otherwise only
/// if it still holds the text set by [`set_data`]. Returns whether the
/// clipboard could be accessed.
pub fn clear_data(force: bool) -> bool {
    let has_digest = DIGEST.with(|d| d.borrow().is_some());
    if !has_digest && !force {
        return true;
    }
    let ok = with_board(|b| {
        let current = b.get_text().ok().map(Zeroizing::new);
        let clear = match &current {
            Some(t) if !t.is_empty() => force || DIGEST.with(|d| *d.borrow() == Some(digest(t))),
            _ => false,
        };
        if clear {
            let _ = b.clear();
        }
        true
    })
    .unwrap_or(false);
    DIGEST.with(|d| *d.borrow_mut() = None);
    ok
}
