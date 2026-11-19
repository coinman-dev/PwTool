//! Functions of the main window used by other windows: autotype and the
//! password manager hooks. Child module of `main_window`.

use super::*;
use crate::ui::autotype;

const AUTOTYPE_TOKEN_KEY: &str = "AutotypeRestoreToken";

impl MainWindow {
    /// Perform autotype (`SendKeys` / `TSendKeysThread`): an empty
    /// `sequence` types `passw` only.
    pub(super) fn perform_autotype(
        self: &Rc<Self>,
        parent: &gtk::Window,
        sequence: &str,
        title: Option<&str>,
        user: Option<&str>,
        param: Option<&str>,
        passw: Option<&str>,
    ) {
        let _ = (title, user);
        let (delay, minimize) = {
            let c = &self.core.borrow().config;
            (c.autotype_delay.clamp(0, 10000) as u32, c.minimize_autotype)
        };
        let strokes = if sequence.is_empty() {
            let mut v = Vec::new();
            autotype::string_sequence(passw.unwrap_or(""), delay, &mut v);
            v
        } else {
            autotype::build_sequence(sequence, None, param, passw, delay)
        };
        self.send_keys(parent, strokes, minimize);
    }

    /// Type key strokes into the window that gets the focus next.
    pub(super) fn send_keys(self: &Rc<Self>, parent: &gtk::Window, strokes: Vec<autotype::KeyStroke>, minimize: bool) {
        if strokes.is_empty() {
            return;
        }
        let w = self.clone();
        let parent = parent.clone();
        glib::spawn_future_local(async move {
            let token = w.core.borrow().ini.read_string("Main", AUTOTYPE_TOKEN_KEY, "");
            // permission first: the desktop may show a dialog
            match autotype::prepare(Some(&parent), &token).await {
                Ok(Some(t)) => w.core.borrow_mut().ini.write_string("Main", AUTOTYPE_TOKEN_KEY, &t),
                Ok(None) => {}
                Err(e) => {
                    msg_box(
                        Some(&parent),
                        &format!("{}\n\n{e}", trl("Autotype is not available (RemoteDesktop portal).")),
                        MsgIcon::Error,
                        MsgButtons::Ok,
                        false,
                    )
                    .await;
                    return;
                }
            }
            if minimize {
                parent.minimize();
                w.window.minimize();
            } else {
                // `TSendKeysThread`: wait until another window is active
                let start = std::time::Instant::now();
                loop {
                    let ours_active = gtk::Window::list_toplevels()
                        .into_iter()
                        .filter_map(|x| x.downcast::<gtk::Window>().ok())
                        .any(|x| x.is_visible() && x.is_active());
                    if !ours_active {
                        break;
                    }
                    if start.elapsed().as_secs() >= 10 {
                        return;
                    }
                    glib::timeout_future(std::time::Duration::from_millis(100)).await;
                }
            }
            let abort = Rc::new(Cell::new(false));
            if let Err(e) = autotype::send(None, "", strokes, abort).await {
                show_error_somewhere(&w, &format!("{}\n\n{e}", trl("Autotype failed.")));
            }
        });
    }

    /// `TPasswMngForm::AddPassw()`: add a password to the open database.
    pub(super) fn add_passw_to_db(&self, passw: &str, show_dlg: bool, param: Option<&str>) {
        let _ = (passw, show_dlg, param);
        msg(Some(&self.win()), &trl("The password manager is not available yet in this version."), MsgIcon::Info);
    }

    /// `PasswMngBtnClick()`.
    pub(super) fn show_passw_manager(self: &Rc<Self>) {
        msg(Some(&self.win()), &trl("The password manager is not available yet in this version."), MsgIcon::Info);
    }
}

/// Error while the windows may be hidden: notification or message box.
fn show_error_somewhere(w: &Rc<MainWindow>, text: &str) {
    if w.window.is_visible() {
        msg(Some(&w.win()), text, MsgIcon::Error);
    } else {
        super::tray_main::show_notification(&w.window, text, true);
    }
}
