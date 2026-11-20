//! Profile editor (`TProfileEditDlg`, `upstream/src/main/ProfileEditor.cpp`)
//! with `TMainForm::CreateProfile()`/`DeleteProfile()`. Child module of
//! `main_window`.

#![allow(deprecated)]

use pwtool_core::config::{PROFILES_MAX_NUM, find_profile};

use super::*;

const CONFIG_ID: &str = "ProfileEditor";

/// Result flags of [`MainWindow::profile_editor`].
pub const PROFILE_MODIFIED: u32 = 1;
pub const PROFILE_ADDED: u32 = 2;

pub(super) struct ProfileEditDlg {
    b: Built,
    win: gtk::Window,
    list: gtk::ListBox,
    modified: Cell<bool>,
    added: Cell<bool>,
    exit_after_add: Cell<bool>,
    done: RefCell<Option<async_channel::Sender<()>>>,
    main: std::rc::Weak<MainWindow>,
}

impl ProfileEditDlg {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let b = build_form(load_form("ProfileEditor"), "ProfileEditor");
        let win = super::super::dialogs::form_window(&b, Some(&main.win()), true);
        win.set_hide_on_close(true);
        {
            let core = main.core.borrow();
            win.set_default_size(
                core.ini.read_int(CONFIG_ID, "WindowWidth", b.client.0).max(b.client.0),
                core.ini.read_int(CONFIG_ID, "WindowHeight", b.client.1).max(b.client.1),
            );
            b.get::<gtk::CheckButton>("ConfirmCheck").set_active(core.ini.read_bool(CONFIG_ID, "Confirm", true));
        }
        let list: gtk::ListBox = b.get("ProfileList.list");
        list.set_selection_mode(gtk::SelectionMode::Multiple);
        let d = Rc::new(ProfileEditDlg {
            b,
            win,
            list,
            modified: Cell::new(false),
            added: Cell::new(false),
            exit_after_add: Cell::new(false),
            done: RefCell::new(None),
            main: Rc::downgrade(main),
        });
        d.fill_list();
        d.connect();
        main.attach_entropy_events(&d.win);
        d
    }

    fn btn(&self, n: &str) -> gtk::Button {
        self.b.get(n)
    }

    fn name_box(&self) -> gtk::Entry {
        self.b.get("ProfileNameBox")
    }

    fn fill_list(&self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let Some(main) = self.main.upgrade() else { return };
        for p in &main.core.borrow().profiles {
            let mut name = p.profile_name.clone();
            if p.advanced_passw_options.is_some() {
                name.push_str(" [+]");
            }
            let l = gtk::Label::new(Some(&name));
            l.set_xalign(0.0);
            l.set_margin_start(4);
            self.list.append(&l);
        }
    }

    fn selected(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.list.selected_rows().iter().map(|r| r.index() as usize).collect();
        v.sort_unstable();
        v
    }

    fn select_only(&self, idx: usize) {
        self.list.unselect_all();
        if let Some(r) = self.list.row_at_index(idx as i32) {
            self.list.select_row(Some(&r));
        }
    }

    fn connect(self: &Rc<Self>) {
        let d = Rc::downgrade(self);
        self.list.connect_selected_rows_changed(move |_| {
            if let Some(d) = d.upgrade() {
                d.list_click();
            }
        });
        let d = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, _| {
            if let Some(d) = d.upgrade() {
                d.load();
            }
        });
        let d = Rc::downgrade(self);
        self.name_box().connect_changed(move |e| {
            if let Some(d) = d.upgrade() {
                d.btn("AddBtn").set_sensitive(e.text_length() != 0);
            }
        });
        let d = Rc::downgrade(self);
        self.name_box().connect_activate(move |_| {
            if let Some(d) = d.upgrade() {
                glib::spawn_future_local(async move { d.add().await });
            }
        });
        let d = Rc::downgrade(self);
        self.btn("AddBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                glib::spawn_future_local(async move { d.add().await });
            }
        });
        let d = Rc::downgrade(self);
        self.btn("LoadBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.load();
            }
        });
        let d = Rc::downgrade(self);
        self.btn("DeleteBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                glib::spawn_future_local(async move { d.delete().await });
            }
        });
        let d = Rc::downgrade(self);
        self.btn("MoveUpBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.move_by(-1);
            }
        });
        let d = Rc::downgrade(self);
        self.btn("MoveDownBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.move_by(1);
            }
        });
        let w = self.win.clone();
        self.btn("CloseBtn").connect_clicked(move |_| w.close());
        let d = Rc::downgrade(self);
        self.win.connect_close_request(move |_| {
            if let Some(d) = d.upgrade() {
                if let Some(tx) = d.done.borrow_mut().take() {
                    let _ = tx.try_send(());
                }
            }
            glib::Propagation::Proceed
        });
    }

    /// `ProfileListClick()`.
    fn list_click(&self) {
        let sel = self.selected();
        let Some(main) = self.main.upgrade() else { return };
        match sel.len() {
            0 => {
                for n in ["LoadBtn", "DeleteBtn", "MoveUpBtn", "MoveDownBtn"] {
                    self.btn(n).set_sensitive(false);
                }
            }
            1 => {
                let i = sel[0];
                let count = main.core.borrow().profiles.len();
                if let Some(p) = main.core.borrow().profiles.get(i) {
                    self.name_box().set_text(&p.profile_name);
                    self.b
                        .get::<gtk::CheckButton>("SaveAdvancedOptionsCheck")
                        .set_active(p.advanced_passw_options.is_some());
                }
                self.btn("LoadBtn").set_sensitive(true);
                self.btn("DeleteBtn").set_sensitive(true);
                self.btn("MoveUpBtn").set_sensitive(count > 1 && i > 0);
                self.btn("MoveDownBtn").set_sensitive(count > 1 && i + 1 < count);
            }
            _ => {
                self.btn("LoadBtn").set_sensitive(false);
                self.btn("DeleteBtn").set_sensitive(true);
                self.btn("MoveUpBtn").set_sensitive(false);
                self.btn("MoveDownBtn").set_sensitive(false);
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
        if self.b.get::<gtk::CheckButton>("ConfirmCheck").is_active() {
            let m = if sel.len() == 1 {
                let name = main.core.borrow().profiles.get(sel[0]).map(|p| p.profile_name.clone()).unwrap_or_default();
                trl_format("Delete profile \"%1\"?", &[&name])
            } else {
                trl_format("Delete %1 profiles?", &[&sel.len().to_string()])
            };
            if msg_box(Some(&self.win), &m, MsgIcon::Warning, MsgButtons::YesNo, true).await != MsgResult::Yes {
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
    }

    async fn add(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let name = self.name_box().text().to_string();
        if name.is_empty() {
            return;
        }
        let save_adv = self.b.get::<gtk::CheckButton>("SaveAdvancedOptionsCheck").is_active();
        let found = find_profile(&main.core.borrow().profiles, &name);
        if found.is_none() && main.core.borrow().profiles.len() >= PROFILES_MAX_NUM {
            msg_box(
                Some(&self.win),
                &trl_format(
                    "Maximum number of profiles (%1) reached.\nPlease delete or overwrite profiles.",
                    &[&PROFILES_MAX_NUM.to_string()],
                ),
                MsgIcon::Warning,
                MsgButtons::Ok,
                false,
            )
            .await;
            return;
        }
        if let Some(i) = found {
            let existing = main.core.borrow().profiles[i].profile_name.clone();
            if self.b.get::<gtk::CheckButton>("ConfirmCheck").is_active()
                && msg_box(
                    Some(&self.win),
                    &trl_format("A profile with the name \"%1\" already exists.\nDo you want to overwrite it?", &[&existing]),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    != MsgResult::Yes
            {
                self.name_box().grab_focus();
                return;
            }
        }
        main.create_profile(&name, save_adv, found);
        self.fill_list();
        self.name_box().set_text("");
        self.name_box().grab_focus();
        self.modified.set(true);
        self.added.set(true);
        if self.exit_after_add.get() {
            self.win.close();
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
        self.select_only(j as usize);
        self.modified.set(true);
    }

    fn save_config(&self, ini: &mut pwtool_core::config::IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.default_height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.default_width() as i64);
        ini.write_bool(CONFIG_ID, "Confirm", self.b.get::<gtk::CheckButton>("ConfirmCheck").is_active());
    }
}

impl MainWindow {
    /// `TProfileEditDlg::Execute()`: returns `PROFILE_MODIFIED` /
    /// `PROFILE_ADDED` flags.
    pub(super) async fn profile_editor(self: &Rc<Self>, exit_after_add: bool) -> u32 {
        let existing = self.profile_dlg.borrow().clone();
        let dlg = match existing {
            Some(d) => d,
            None => {
                let d = ProfileEditDlg::new(self);
                *self.profile_dlg.borrow_mut() = Some(d.clone());
                d
            }
        };
        dlg.modified.set(false);
        dlg.added.set(false);
        dlg.exit_after_add.set(exit_after_add);
        dlg.fill_list();
        dlg.list.unselect_all();
        dlg.list_click();
        dlg.name_box().set_text("");
        dlg.btn("AddBtn").set_sensitive(false);
        let (tx, rx) = async_channel::bounded(1);
        *dlg.done.borrow_mut() = Some(tx);
        dlg.win.present();
        dlg.name_box().grab_focus();
        let _ = rx.recv().await;
        let mut flags = 0;
        if dlg.modified.get() {
            flags |= PROFILE_MODIFIED;
            self.update_profile_controls();
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

    pub(super) fn save_profile_editor_config(&self, ini: &mut pwtool_core::config::IniFile) {
        if let Some(d) = self.profile_dlg.borrow().as_ref() {
            d.save_config(ini);
        }
    }

    pub(super) fn connect_profile_editor(self: &Rc<Self>) {
        for name in ["MainMenu_File_Profile_ProfileEditor", "TrayMenu_Profile_ProfileEditor"] {
            self.act(name, |w| {
                let w = w.clone();
                glib::spawn_future_local(async move {
                    w.profile_editor(false).await;
                });
            });
        }
        self.click("AddProfileBtn", |w| {
            let w = w.clone();
            glib::spawn_future_local(async move {
                let flags = w.profile_editor(true).await;
                if flags & PROFILE_ADDED != 0 {
                    let n = w.core.borrow().profiles.len();
                    if n > 0 {
                        w.select_profile_in_list(n - 1);
                    }
                }
            });
        });
    }
}
