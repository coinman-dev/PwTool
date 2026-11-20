//! Functions of the main window used by the password manager
//! (`MainForm->GeneratePassw(gpdGuiSingle, pEditBox)`,
//! `MainForm->LoadProfile(name)`).

use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use pwtool_core::generate::GenDest;
use zeroize::Zeroizing;

use crate::ui::main_window::MainWindow;

impl MainWindow {
    /// Generate a single password with the current settings of the main
    /// window and pass it to `f` (instead of showing it in the main
    /// window's password box).
    pub fn generate_passw_for(self: &Rc<Self>, f: impl Fn(&str) + 'static) {
        let target = gtk::Entry::new();
        target.set_buffer(&gtk::PasswordEntryBuffer::new().upcast::<gtk::EntryBuffer>());
        target.connect_changed(move |e| {
            let t = Zeroizing::new(e.text().to_string());
            if !t.is_empty() {
                f(&t);
            }
        });
        *self.gen_target.borrow_mut() = Some(target);
        self.generate_passw(GenDest::GuiSingle);
    }

    /// `LoadProfile(name)`: `false` if there is no such profile.
    pub fn load_profile_by_name(&self, name: &str) -> bool {
        let idx = pwtool_core::config::find_profile(&self.core.borrow().profiles, name);
        match idx {
            Some(i) => {
                self.load_profile(i);
                true
            }
            None => false,
        }
    }

    /// `g_config.UseAdvancedPasswEst`.
    pub fn use_advanced_passw_est(&self) -> bool {
        self.core.borrow().use_advanced_passw_est
    }
}
