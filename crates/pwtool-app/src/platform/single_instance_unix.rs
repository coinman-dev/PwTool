//! One instance per user on Linux without `GApplication`: the running
//! instance listens on a Unix socket in `$XDG_RUNTIME_DIR` (private to the
//! user); a second start connects to it, asks it to show its window and
//! exits.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

const ACTIVATE: &[u8] = b"activate\n";

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        // SAFETY: getuid has no preconditions.
        .unwrap_or_else(|| std::env::temp_dir().join(format!("pwtool-{}", unsafe { libc::getuid() })));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{}.instance", crate::program::APP_ID))
}

/// Held by the running instance.
pub struct Instance {
    listener: Option<UnixListener>,
    path: PathBuf,
}

impl Instance {
    /// Become the running instance. If another instance runs, it is asked
    /// to show its window and `None` is returned.
    pub fn acquire() -> Option<Instance> {
        let path = socket_path();
        if let Ok(mut s) = UnixStream::connect(&path) {
            let _ = s.write_all(ACTIVATE);
            let mut ack = [0u8; 1];
            let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let _ = s.read(&mut ack);
            return None;
        }
        // no instance answers: a socket file left by a crash is replaced
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => Some(l),
            Err(e) => {
                eprintln!("pwtool: single instance socket {}: {e}", path.display());
                None
            }
        };
        Some(Instance { listener, path })
    }

    /// Call `on_activate` (show the main window) on the GUI thread when
    /// another start is attempted.
    pub fn watch(&self, on_activate: impl Fn() + 'static) {
        let Some(l) = self.listener.as_ref().and_then(|l| l.try_clone().ok()) else { return };
        let (tx, rx) = async_channel::unbounded::<()>();
        let _ = std::thread::Builder::new().name("pwtool-instance".into()).spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 16];
                let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                if matches!(s.read(&mut buf), Ok(n) if buf[..n].starts_with(b"activate")) {
                    let _ = s.write_all(b"k");
                    if tx.send_blocking(()).is_err() {
                        break;
                    }
                }
            }
        });
        crate::rt::spawn_local(async move {
            while rx.recv().await.is_ok() {
                on_activate();
            }
        });
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.listener.take().is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
