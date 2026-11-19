//! Functions of the main window used by other windows that depend on parts
//! still being ported (autotype, password manager). Child module of
//! `main_window`.

use super::*;

impl MainWindow {
    /// Perform autotype with a sequence (`TSendKeysThread` /
    /// `SendKeys::SendComplexString()`).
    pub(super) fn perform_autotype(
        &self,
        parent: &gtk::Window,
        sequence: &str,
        title: Option<&str>,
        user: Option<&str>,
        param: Option<&str>,
        passw: Option<&str>,
    ) {
        let _ = (sequence, title, user, param, passw);
        msg(Some(parent), &trl("Autotype is not available yet in this version."), MsgIcon::Info);
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
