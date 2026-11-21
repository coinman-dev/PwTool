//! Profile editor (`TProfileEditDlg`, `upstream/src/main/ProfileEditor.cpp`)
//! with `TMainForm::CreateProfile()`. Child module of `main_window`.

use pwtool_core::config::{PROFILES_MAX_NUM, find_profile};

use super::*;

const CONFIG_ID: &str = "ProfileEditor";
const WINDOW_ID: &str = "ProfileEditor";

/// Result flags of [`MainWindow::profile_editor`].
pub const PROFILE_MODIFIED: u32 = 1;
pub const PROFILE_ADDED: u32 = 2;

pub(super) struct ProfileEditDlg {
    form: Rc<Form>,
    modified: Cell<bool>,
    added: Cell<bool>,
    exit_after_add: Cell<bool>,
    done: RefCell<Option<async_channel::Sender<()>>>,
    main: Weak<MainWindow>,
}

fn parent() -> ViewportId {
    ViewportId::from_hash_of(WINDOW_ID)
}

impl ProfileEditDlg {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let form = dialogs::load("ProfileEditor");
        form.set_checked("ConfirmCheck", main.core.borrow().ini.read_bool(CONFIG_ID, "Confirm", true));
        let d = Rc::new(ProfileEditDlg {
            form,
            modified: Cell::new(false),
            added: Cell::new(false),
            exit_after_add: Cell::new(false),
            done: RefCell::new(None),
            main: Rc::downgrade(main),
        });
        d.fill_list();
        let dd = Rc::downgrade(&d);
        d.form.on_any(move |e| {
            let Some(d) = dd.upgrade() else { return };
            use crate::vcl::Event::*;
            match e {
                Change(n) if n == "ProfileList" => d.list_click(),
                DblClick(n) if n == "ProfileList" => d.load(),
                Change(n) if n == "ProfileNameBox" => d.form.set_enabled("AddBtn", !d.form.text("ProfileNameBox").is_empty()),
                Enter(n) if n == "ProfileNameBox" => {
                    let d = d.clone();
                    spawn(async move { d.add().await });
                }
                Click(n) => match n.as_str() {
                    "AddBtn" => {
                        let d = d.clone();
                        spawn(async move { d.add().await });
                    }
                    "LoadBtn" => d.load(),
                    "DeleteBtn" => {
                        let d = d.clone();
                        spawn(async move { d.delete().await });
                    }
                    "MoveUpBtn" => d.move_by(-1),
                    "MoveDownBtn" => d.move_by(1),
                    "CloseBtn" => d.close(),
                    _ => {}
                },
                _ => {}
            }
        });
        d
    }

    fn close(&self) {
        if let Some(m) = self.main.upgrade() {
            m.close_tool(WINDOW_ID);
        }
        if let Some(tx) = self.done.borrow_mut().take() {
            let _ = tx.try_send(());
        }
    }

    fn fill_list(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let names: Vec<String> = main
            .core
            .borrow()
            .profiles
            .iter()
            .map(|p| if p.advanced_passw_options.is_some() { format!("{} [+]", p.profile_name) } else { p.profile_name.clone() })
            .collect();
        self.form.set_items("ProfileList", names);
    }

    fn selected(&self) -> Vec<usize> {
        self.form.list_selected("ProfileList")
    }

    /// `ProfileListClick()`.
    fn list_click(&self) {
        let sel = self.selected();
        let Some(main) = self.main.upgrade() else { return };
        let f = &self.form;
        match sel.len() {
            0 => {
                for n in ["LoadBtn", "DeleteBtn", "MoveUpBtn", "MoveDownBtn"] {
                    f.set_enabled(n, false);
                }
            }
            1 => {
                let i = sel[0];
                let count = main.core.borrow().profiles.len();
                if let Some(p) = main.core.borrow().profiles.get(i) {
                    f.set_text("ProfileNameBox", &p.profile_name);
                    f.set_checked("SaveAdvancedOptionsCheck", p.advanced_passw_options.is_some());
                }
                f.set_enabled("AddBtn", true);
                f.set_enabled("LoadBtn", true);
                f.set_enabled("DeleteBtn", true);
                f.set_enabled("MoveUpBtn", count > 1 && i > 0);
                f.set_enabled("MoveDownBtn", count > 1 && i + 1 < count);
            }
            _ => {
                f.set_enabled("LoadBtn", false);
                f.set_enabled("DeleteBtn", true);
                f.set_enabled("MoveUpBtn", false);
                f.set_enabled("MoveDownBtn", false);
            }
        }
    }

    fn load(&self) {
        let sel = self.selected();
        if let (Some(main), Some(&i)) = (self.main.upgrade(), sel.first()) {
            main.load_profile(i);
        }
    }

    async fn delete(&self) {
        let sel = self.selected();
        let Some(main) = self.main.upgrade() else { return };
        if sel.is_empty() {
            return;
        }
        if self.form.checked("ConfirmCheck") {
            let m = if sel.len() == 1 {
                let name = main.core.borrow().profiles.get(sel[0]).map(|p| p.profile_name.clone()).unwrap_or_default();
                trl_format("Delete profile \"%1\"?", &[&name])
            } else {
                trl_format("Delete %1 profiles?", &[&sel.len().to_string()])
            };
            if dialogs::ask(parent(), &m, MsgIcon::Warning, MsgButtons::YesNo, true).await != MsgResult::Yes {
                return;
            }
        }
        {
            let mut core = main.core.borrow_mut();
            for &i in sel.iter().rev() {
                if i < core.profiles.len() {
                    core.profiles.remove(i);
                }
            }
        }
        self.fill_list();
        self.list_click();
        self.modified.set(true);
        main.update_profile_controls();
        main.update_tray_icon();
    }

    async fn add(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let name = self.form.text("ProfileNameBox");
        if name.is_empty() {
            return;
        }
        let save_adv = self.form.checked("SaveAdvancedOptionsCheck");
        let found = find_profile(&main.core.borrow().profiles, &name);
        if found.is_none() && main.core.borrow().profiles.len() >= PROFILES_MAX_NUM {
            let m = trl_format("Maximum number of profiles (%1) reached.\nPlease delete or overwrite profiles.", &[&PROFILES_MAX_NUM.to_string()]);
            dialogs::ask(parent(), &m, MsgIcon::Warning, MsgButtons::Ok, false).await;
            return;
        }
        if let Some(i) = found {
            let existing = main.core.borrow().profiles[i].profile_name.clone();
            if self.form.checked("ConfirmCheck")
                && dialogs::ask(
                    parent(),
                    &trl_format("A profile with the name \"%1\" already exists.\nDo you want to overwrite it?", &[&existing]),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    != MsgResult::Yes
            {
                self.form.focus("ProfileNameBox");
                return;
            }
        }
        main.create_profile(&name, save_adv, found);
        self.fill_list();
        self.list_click();
        self.form.set_text("ProfileNameBox", "");
        self.form.set_enabled("AddBtn", false);
        self.form.focus("ProfileNameBox");
        self.modified.set(true);
        self.added.set(true);
        main.update_profile_controls();
        main.update_tray_icon();
        if self.exit_after_add.get() {
            self.close();
        }
    }

    fn move_by(&self, delta: i32) {
        let sel = self.selected();
        let Some(main) = self.main.upgrade() else { return };
        let Some(&i) = sel.first() else { return };
        let count = main.core.borrow().profiles.len();
        let j = i as i32 + delta;
        if j < 0 || j as usize >= count {
            return;
        }
        main.core.borrow_mut().profiles.swap(i, j as usize);
        self.fill_list();
        self.form.set_item_index("ProfileList", Some(j as usize));
        self.list_click();
        self.modified.set(true);
        main.update_profile_controls();
        main.update_tray_icon();
    }

    pub(super) fn save_config(&self, ini: &mut pwtool_core::config::IniFile) {
        ini.write_bool(CONFIG_ID, "Confirm", self.form.checked("ConfirmCheck"));
    }
}

impl MainWindow {
    /// `TProfileEditDlg::Execute()`: returns `PROFILE_MODIFIED` /
    /// `PROFILE_ADDED` flags.
    pub(super) async fn profile_editor(self: &Rc<Self>, exit_after_add: bool) -> u32 {
        let existing = self.tools.profile_dlg.borrow().clone();
        let dlg = match existing {
            Some(d) => d,
            None => {
                let d = ProfileEditDlg::new(self);
                *self.tools.profile_dlg.borrow_mut() = Some(d.clone());
                d
            }
        };
        if windows::is_open(parent()) {
            return 0;
        }
        dlg.modified.set(false);
        dlg.added.set(false);
        dlg.exit_after_add.set(exit_after_add);
        dlg.fill_list();
        dlg.form.clear_selection("ProfileList");
        dlg.list_click();
        dlg.form.set_text("ProfileNameBox", "");
        dlg.form.set_enabled("AddBtn", false);
        let (tx, rx) = async_channel::bounded(1);
        *dlg.done.borrow_mut() = Some(tx.clone());
        let win = self.show_tool(WINDOW_ID, CONFIG_ID, true, &dlg.form, ROOT, true);
        let w = self.weak();
        let wid = Rc::downgrade(&win);
        *win.on_close.borrow_mut() = Some(Box::new(move || {
            if let (Some(w), Some(win)) = (w.upgrade(), wid.upgrade()) {
                w.remember_size(WINDOW_ID, &win);
            }
            let _ = tx.try_send(());
            true
        }));
        dlg.form.focus("ProfileNameBox");
        let _ = rx.recv().await;
        let mut flags = 0;
        if dlg.modified.get() {
            flags |= PROFILE_MODIFIED;
        }
        if dlg.added.get() {
            flags |= PROFILE_ADDED;
        }
        flags
    }

    /// `CreateProfile()`: profile from the current settings.
    pub(super) fn create_profile(&self, name: &str, save_advanced_options: bool, idx: Option<usize>) {
        self.sync_settings();
        let mut core = self.core.borrow_mut();
        let p = core.settings.to_profile(name, save_advanced_options);
        match idx {
            Some(i) if i < core.profiles.len() => core.profiles[i] = p,
            _ => core.profiles.push(p),
        }
    }
}
