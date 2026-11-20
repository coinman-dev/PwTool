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

    /// Autotype for other windows (password manager).
    pub fn send_keys_from(self: &Rc<Self>, parent: &gtk::Window, strokes: Vec<autotype::KeyStroke>, minimize: bool) {
        self.send_keys(parent, strokes, minimize);
    }

    /// The password manager window (created on first use).
    pub fn passw_manager(self: &Rc<Self>) -> Rc<crate::ui::passw_manager::PasswManager> {
        if let Some(m) = self.passw_mng.borrow().as_ref() {
            return m.clone();
        }
        let m = crate::ui::passw_manager::PasswManager::new(self);
        m.set_passw_enter_dlg(self.passw_enter());
        *self.passw_mng.borrow_mut() = Some(m.clone());
        m
    }

    /// `TPasswMngForm::AddPassw()`: add passwords to the open database
    /// (`show_num`: report the number of added passwords).
    pub(super) fn add_passw_to_db(self: &Rc<Self>, passw: &str, show_num: bool, param: Option<&str>) {
        let _ = param;
        let m = self.passw_manager();
        if !m.is_db_open() {
            return;
        }
        m.add_passw_to_db(passw, show_num);
    }

    /// `PasswMngBtnClick()`.
    pub(super) fn show_passw_manager(self: &Rc<Self>) {
        self.passw_manager().show();
    }

    /// `TPasswMngForm::SearchDbForKeyword()` (hot key action).
    pub(super) fn search_db_for_keyword(self: &Rc<Self>, autotype: bool) {
        self.passw_manager().search_db_for_keyword(autotype);
    }

    /// `OnEndSession()`: save the database and the configuration before the
    /// session ends.
    pub fn on_end_session(&self) {
        let mng = self.passw_mng.borrow().clone();
        if let Some(m) = mng {
            m.save_db_on_shutdown();
        }
        if self.action_checked("MainMenu_Options_SaveSettingsOnExit") && !self.core.borrow().cmd.config_readonly {
            self.save_config();
        }
        self.core.borrow().write_rand_seed_file();
        if let Some(d) = self.tools.passw_enter.borrow().as_ref() {
            d.clear_passw_cache();
        }
    }

    /// User input in any window (idle lock of the database).
    pub(super) fn notify_user_action(&self) {
        if let Some(m) = self.passw_mng.borrow().as_ref() {
            m.notify_user_action();
        }
    }
}

/// `ShowTrayInfo()` / message box: information while the windows may be
/// hidden.
pub fn show_tray_info(w: &Rc<MainWindow>, text: &str, warning: bool) {
    if w.window.is_visible() {
        msg(Some(&w.win()), text, if warning { MsgIcon::Warning } else { MsgIcon::Info });
    } else {
        super::tray_main::show_notification(&w.window, text, warning);
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
