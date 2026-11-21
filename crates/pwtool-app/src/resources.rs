//! Program data embedded in the executable: translations, the list of
//! common passwords, script examples, the license and the PDF manuals.
//!
//! The rest of the program works with files (translations are listed and
//! loaded from a directory, the manuals are opened with the system viewer),
//! so the data is written once to a per-user directory and used from there:
//! Windows `%TEMP%\PwTool\data-<version>-<hash>`, Linux
//! `$XDG_RUNTIME_DIR/pwtool/…` (or `~/.cache/pwtool/…`). A data directory on
//! disk (`share/pwtool` next to the program, `PWTOOL_DATA_DIR`) takes
//! precedence, see `AppPaths`.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Embedded files: path relative to the data directory, whether the
/// contents are deflate-compressed (see build.rs), contents.
pub static FILES: &[(&str, bool, &[u8])] = include!(concat!(env!("OUT_DIR"), "/resources.rs"));

/// Contents of an embedded table entry: unpacked if compressed.
pub fn unpack(compressed: bool, data: &'static [u8]) -> Cow<'static, [u8]> {
    if compressed {
        Cow::Owned(miniz_oxide::inflate::decompress_to_vec(data).expect("embedded data"))
    } else {
        Cow::Borrowed(data)
    }
}

/// Contents of an embedded file (`"languages/Russian.po"`, `"manual.pdf"`…).
pub fn file(rel: &str) -> Option<Cow<'static, [u8]>> {
    FILES.iter().find(|(n, _, _)| *n == rel).map(|(_, z, d)| unpack(*z, d))
}

/// FNV-1a over the names and contents: different data, different directory.
fn hash() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, _, data) in FILES {
        for b in name.bytes().chain([0]).chain(data.iter().copied()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// Base directory for the extracted data (per user).
fn base_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::temp_dir().join("PwTool")
    }
    #[cfg(not(windows))]
    {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_dir());
        let cache = || {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
                .unwrap_or_else(std::env::temp_dir)
        };
        runtime.unwrap_or_else(cache).join("pwtool")
    }
}

/// Directory with the embedded data, written there if needed. `None` if it
/// cannot be written (the program then runs without translations etc., as
/// with a missing data directory).
pub fn extract() -> Option<PathBuf> {
    let base = base_dir();
    let name = format!("data-{}-{:016x}", env!("CARGO_PKG_VERSION"), hash());
    let dir = base.join(&name);
    if dir.join(".complete").is_file() {
        return Some(dir);
    }
    create_private_dir(&base).ok()?;
    // write into a directory of this process, then rename: another instance
    // (console mode) may extract at the same time
    let tmp = base.join(format!("{name}.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let written = (|| -> std::io::Result<()> {
        for (rel, z, data) in FILES {
            let p = tmp.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&p, unpack(*z, data))?;
        }
        std::fs::write(tmp.join(".complete"), b"")
    })();
    if let Err(e) = written {
        eprintln!("pwtool: {}: {e}", tmp.display());
        let _ = std::fs::remove_dir_all(&tmp);
        return None;
    }
    if std::fs::rename(&tmp, &dir).is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
        if !dir.join(".complete").is_file() {
            return None;
        }
    }
    remove_old(&base, &name);
    Some(dir)
}

/// Remove the data of other versions (complete directories only: another
/// process may be writing its own right now).
fn remove_old(base: &Path, keep: &str) {
    for e in std::fs::read_dir(base).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with("data-") && n != keep && e.path().join(".complete").is_file() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if !dir.is_dir() {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_data() {
        assert!(file("common_passwords.txt").is_some_and(|d| d.len() > 10_000));
        assert!(file("languages/Russian.po").is_some());
        assert!(file("languages/Greek.lng").is_some());
        assert!(file("languages/port/ru.po").is_some());
        assert!(file("script_examples/leetspeak.lua").is_some());
        assert!(file("license.txt").is_some());
    }

    /// Text is stored compressed and unpacks to the original.
    #[test]
    fn embedded_data_compressed() {
        let (_, z, d) = FILES.iter().find(|(n, _, _)| *n == "languages/Russian.po").unwrap();
        assert!(*z);
        let text = file("languages/Russian.po").unwrap();
        assert!(d.len() * 2 < text.len());
        assert!(std::str::from_utf8(&text).unwrap().contains("msgid"));
    }
}
