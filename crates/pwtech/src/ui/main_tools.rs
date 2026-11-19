//! Tools and menus of the main window (child module of `main_window`):
//! File | Profile menu, encryption of the clipboard, deterministic random
//! generator, password cache, list menu of the combo boxes, backup/restore
//! of settings, and the small tool dialogs (`ProvideEntropy`,
//! `CreateRandDataFile`, `CreateTrigramFile`, `About`).

#![allow(deprecated)]

use std::path::Path;

use pwtech_core::crypt_text::{CryptTextError, decrypt_text_with_password, encrypt_text_with_password};

use super::*;
use crate::ui::passw_enter::*;

/// Characters used for the profile shortcuts (Alt+Shift+...).
const PROFILES_MENU_SHORTCUTS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// State of the tools (dialogs are created on first use).
pub(super) struct ToolsState {
    pub passw_enter: RefCell<Option<Rc<PasswEnterDlg>>>,
    provide_entropy: RefCell<Option<(Built, gtk::Window)>>,
    rand_data_file: RefCell<Option<(Built, gtk::Window)>>,
    trigram_file: RefCell<Option<(Built, gtk::Window)>>,
    about: RefCell<Option<(Built, gtk::Window)>>,
    profile_section: gio::Menu,
    tray_profile_section: gio::Menu,
    profile_accels: RefCell<Vec<String>>,
    last_cache_countdown: Cell<i64>,
    last_total_bits: Cell<i64>,
}

impl ToolsState {
    pub fn new() -> Self {
        ToolsState {
            passw_enter: RefCell::new(None),
            provide_entropy: RefCell::new(None),
            rand_data_file: RefCell::new(None),
            trigram_file: RefCell::new(None),
            about: RefCell::new(None),
            profile_section: gio::Menu::new(),
            tray_profile_section: gio::Menu::new(),
            profile_accels: RefCell::new(Vec::new()),
            last_cache_countdown: Cell::new(-1),
            last_total_bits: Cell::new(-1),
        }
    }
}

impl MainWindow {
    pub(super) fn connect_tools(self: &Rc<Self>) {
        // File | Profile and Tray | Profile: profile entries before the editor
        for (menu, section) in [
            ("MainMenu_File_Profile", &self.tools.profile_section),
            ("TrayMenu_Profile", &self.tools.tray_profile_section),
        ] {
            if let Some(sub) = self.b.submenus.get(menu) {
                sub.prepend_section(None, section);
            }
        }
        let a = gio::SimpleAction::new("LoadProfileIdx", Some(glib::VariantTy::INT32));
        let w = Rc::downgrade(self);
        a.connect_activate(move |_, p| {
            if let (Some(w), Some(i)) = (w.upgrade(), p.and_then(|p| p.get::<i32>())) {
                w.profile_menu_click(i as usize);
            }
        });
        self.b.actions.add_action(&a);
        self.update_profile_menu();

        self.act("MainMenu_Tools_EncryptClip", |w| w.crypt_text_spawn(true, None));
        self.act("MainMenu_Tools_DecryptClip", |w| w.crypt_text_spawn(false, None));
        self.act("TrayMenu_EncryptClip", |w| w.crypt_text_spawn(true, None));
        self.act("TrayMenu_DecryptClip", |w| w.crypt_text_spawn(false, None));
        self.act("PasswBoxMenu_EncryptCopy", |w| {
            let e = w.entry("PasswBox");
            let text = match e.selection_bounds() {
                Some((s, end)) => Zeroizing::new(e.chars(s, end).to_string()),
                None => Zeroizing::new(String::new()),
            };
            w.crypt_text_spawn(true, Some(text));
        });
        self.act("MainMenu_Tools_DetermRandGen_Setup", |w| {
            let w = w.clone();
            glib::spawn_future_local(async move { w.det_rand_gen_setup().await });
        });
        self.act("MainMenu_Options_ClearPasswCache", |w| {
            w.passw_enter().clear_passw_cache();
            info_box(&w.win(), &trl("Password cache cleared."));
            w.update_passw_cache_menu();
        });
        self.act("MainMenu_Tools_ProvideAddEntropy_AsText", |w| w.provide_entropy());
        self.act("MainMenu_Tools_CreateRandDataFile", |w| w.create_rand_data_file());
        self.act("TrayMenu_CreateRandDataFile", |w| w.create_rand_data_file());
        self.act("MainMenu_Tools_CreateTrigramFile", |w| w.create_trigram_file());
        self.act("MainMenu_Help_About", |w| w.about());
        self.act("TrayMenu_About", |w| w.about());
        self.act("TrayMenu_OpenManual", |w| w.help_btn_click());
        self.act("MainMenu_File_BackupSettings", |w| w.backup_settings());
        self.act("MainMenu_File_RestoreSettings", |w| w.restore_settings());
        self.connect_list_menus();
        self.update_passw_cache_menu();
        self.update_total_ent_bits_menu();
        if let Some(b) = self.b.try_get::<gtk::Button>("CryptTextBtn") {
            // right click: decrypt (`CryptTextBtnMouseUp`)
            let g = gtk::GestureClick::new();
            g.set_button(3);
            let w = Rc::downgrade(self);
            g.connect_released(move |_, _, _, _| {
                if let Some(w) = w.upgrade() {
                    w.crypt_text_spawn(false, None);
                }
            });
            b.add_controller(g);
        }
    }

    /// Called every second by the main timer.
    pub(super) fn tools_timer(&self) {
        self.update_passw_cache_menu();
        self.update_total_ent_bits_menu();
    }

    pub(super) fn passw_enter(self: &Rc<Self>) -> Rc<PasswEnterDlg> {
        if let Some(d) = self.tools.passw_enter.borrow().as_ref() {
            d.use_advanced_passw_est.set(self.core.borrow().use_advanced_passw_est);
            return d.clone();
        }
        let d = PasswEnterDlg::new(&self.win(), &self.core.borrow().ini);
        self.attach_entropy_events(&d.win);
        d.use_advanced_passw_est.set(self.core.borrow().use_advanced_passw_est);
        *self.tools.passw_enter.borrow_mut() = Some(d.clone());
        d
    }

    /// Save the configuration of the tool dialogs (part of `SaveConfig()`).
    pub(super) fn save_tools_config(&self) {
        let mut core = self.core.borrow_mut();
        let ini = &mut core.ini;
        if let Some(d) = self.tools.passw_enter.borrow().as_ref() {
            d.save_config(ini);
        }
        if let Some((_, win)) = self.tools.provide_entropy.borrow().as_ref() {
            ini.write_integer("ProvideEntropy", "WindowHeight", win.default_height() as i64);
            ini.write_integer("ProvideEntropy", "WindowWidth", win.default_width() as i64);
        }
        if let Some((b, win)) = self.tools.rand_data_file.borrow().as_ref() {
            ini.write_integer("CreateRandDataFile", "WindowWidth", win.default_width() as i64);
            ini.write_string("CreateRandDataFile", "FileSize", &b.get::<gtk::Entry>("FileSizeBox").text());
            ini.write_integer(
                "CreateRandDataFile",
                "FileSizeUnitIdx",
                b.get::<gtk::ComboBoxText>("SizeUnitList").active().map(|i| i as i64).unwrap_or(0),
            );
        }
        if let Some((_, win)) = self.tools.trigram_file.borrow().as_ref() {
            ini.write_integer("CreateTrigramFile", "WindowWidth", win.default_width() as i64);
        }
    }

    // ------------------------------------------------------------------
    // profiles

    /// Profile entries of File | Profile and Tray | Profile
    /// (`UpdateProfileControls()`).
    pub(super) fn update_profile_menu(&self) {
        let names: Vec<String> = self.core.borrow().profiles.iter().map(|p| p.profile_name.clone()).collect();
        let app = self.window.application();
        for old in self.tools.profile_accels.borrow_mut().drain(..) {
            if let Some(app) = &app {
                app.set_accels_for_action(&old, &[]);
            }
        }
        for section in [&self.tools.profile_section, &self.tools.tray_profile_section] {
            section.remove_all();
        }
        let shortcuts: Vec<char> = PROFILES_MENU_SHORTCUTS.chars().collect();
        for (i, name) in names.iter().enumerate() {
            let mut caption = String::new();
            if let Some(&c) = shortcuts.get(i) {
                caption.push('_');
                caption.push(c);
                caption.push(' ');
            }
            caption.push_str(&name.replace('_', "__"));
            for section in [&self.tools.profile_section, &self.tools.tray_profile_section] {
                let item = gio::MenuItem::new(Some(&caption), None);
                item.set_action_and_target_value(Some("m.LoadProfileIdx"), Some(&(i as i32).to_variant()));
                section.append_item(&item);
            }
            if let (Some(&c), Some(app)) = (shortcuts.get(i), &app) {
                let detailed = format!("m.LoadProfileIdx({i})");
                let key = if c.is_ascii_digit() { c.to_string() } else { c.to_ascii_lowercase().to_string() };
                app.set_accels_for_action(&detailed, &[&format!("<Alt><Shift>{key}")]);
                self.tools.profile_accels.borrow_mut().push(detailed);
            }
        }
    }

    /// `MainMenu_File_ProfileClick()`.
    fn profile_menu_click(&self, i: usize) {
        let Some(name) = self.core.borrow().profiles.get(i).map(|p| p.profile_name.clone()) else { return };
        self.load_profile(i);
        info_box(&self.win(), &trl_format("Profile \"%1\" activated.", &[&name]));
    }

    // ------------------------------------------------------------------
    // encryption of text

    fn crypt_text_spawn(self: &Rc<Self>, encrypt: bool, text: Option<Zeroizing<String>>) {
        let w = self.clone();
        glib::spawn_future_local(async move { w.crypt_text(encrypt, text, None).await });
    }

    /// `CryptText()`: encrypt/decrypt `text` (or the clipboard text) and put
    /// the result on the clipboard.
    pub async fn crypt_text(self: &Rc<Self>, encrypt: bool, text: Option<Zeroizing<String>>, parent: Option<gtk::Window>) {
        let dlg = self.passw_enter();
        let mut flags = PASSWENTER_FLAG_ENABLEPASSWCACHE;
        flags |= if encrypt {
            PASSWENTER_FLAG_ENCRYPT | PASSWENTER_FLAG_CONFIRMPASSW
        } else {
            PASSWENTER_FLAG_DECRYPT | PASSWENTER_FLAG_ENABLEOLDVER
        };
        let parent = parent.unwrap_or_else(|| self.win());
        if dlg.execute(flags, "", Some(&parent)).await != EnterResult::Ok {
            dlg.clear_and_flush();
            return;
        }
        let passw = dlg.get_passw(0);
        let old_version = dlg.old_version();
        dlg.clear_and_flush();
        self.update_passw_cache_menu();

        let text = match text {
            Some(t) => Some(t),
            None => {
                let (tx, rx) = async_channel::bounded(1);
                super::super::clipboard::read_text(move |t| {
                    let _ = tx.try_send(t);
                });
                rx.recv().await.ok().flatten()
            }
        };
        let result: Result<(), CryptTextError> = match text {
            None => Err(CryptTextError::NoText),
            Some(t) if t.is_empty() => Err(CryptTextError::NoText),
            Some(t) => {
                if encrypt {
                    let r = {
                        let mut pool = RandomPool::global();
                        let r = encrypt_text_with_password(&t, &passw, &mut *pool);
                        // flush the pool to make sure we're back in the "add
                        // entropy" state
                        pool.flush();
                        r
                    };
                    r.map(|out| super::super::clipboard::set_plain(&out))
                } else {
                    decrypt_text_with_password(&t, &passw, old_version)
                        .map(|out| super::super::clipboard::set_hidden(&out.text))
                }
            }
        };
        match result {
            Ok(()) => {
                let m = trl(if encrypt { "Text successfully encrypted." } else { "Text successfully decrypted." });
                info_box(&parent, &m);
            }
            Err(e) => {
                let m = match e {
                    CryptTextError::Clipboard => trl("Could not open clipboard."),
                    CryptTextError::NoText => trl("No text available to process."),
                    CryptTextError::TextTooLong => trl("The text to be encrypted is too long."),
                    CryptTextError::OutOfMemory => {
                        trl("Not enough memory available to perform\nthe operation. Try to use a shorter text.")
                    }
                    CryptTextError::TextCorrupted | CryptTextError::BadKey => {
                        let mut m = trl("Decryption failed. This may be attributed\nto the following reasons:");
                        m.push('\n');
                        if e == CryptTextError::BadKey {
                            m.push_str(&trl("- You entered a wrong password."));
                            m.push('\n');
                        }
                        m.push_str(&trl("- The text is corrupted.\n- The text is not encrypted."));
                        m
                    }
                    CryptTextError::DecomprFailed => {
                        trl("This should not have happened:\nDecryption successful, but decompression\nfailed.")
                    }
                };
                msg_box(Some(&parent), &m, MsgIcon::Error, MsgButtons::Ok, false).await;
            }
        }
    }

    // ------------------------------------------------------------------
    // deterministic random generator, password cache

    /// `MainMenu_Tools_DetermRandGen_SetupClick()`.
    async fn det_rand_gen_setup(self: &Rc<Self>) {
        let dlg = self.passw_enter();
        let ok = dlg
            .execute(
                PASSWENTER_FLAG_CONFIRMPASSW | PASSWENTER_FLAG_ENABLEPASSWCACHE | PASSWENTER_FLAG_PASSWQUALITY,
                &trl("Master password"),
                Some(&self.win()),
            )
            .await
            == EnterResult::Ok;
        let passw = if ok { Some(dlg.get_passw(0)) } else { None };
        dlg.clear_and_flush();
        self.update_passw_cache_menu();
        if let Some(p) = passw {
            self.use_key_seeded_prng(&p);
        }
    }

    /// Caption/state of Options | Clear Password Cache
    /// (`MainMenu_OptionsClick()`).
    fn update_passw_cache_menu(&self) {
        let n = self.tools.passw_enter.borrow().as_ref().map(|d| d.passw_cache_expiry_countdown()).unwrap_or(0);
        if n == self.tools.last_cache_countdown.get() {
            return;
        }
        self.tools.last_cache_countdown.set(n);
        let mut caption = trl("Clear Password Cache");
        if n > 0 {
            caption.push_str(&if n > 99 { format!(" ({:.1}min)", n as f64 / 60.0) } else { format!(" ({n}sec)") });
        }
        self.b.set_menu_label("MainMenu_Options_ClearPasswCache", &super::super::builder::mnemonic(&caption));
        self.b.action("MainMenu_Options_ClearPasswCache").set_enabled(n > 0);
    }

    /// Caption of Help | Total Entropy Bits (`MainMenu_HelpClick()`).
    pub(super) fn update_total_ent_bits_menu(&self) {
        let total = EntropyManager::global().total_entropy_bits();
        if self.tools.last_total_bits.replace(total as i64) == total as i64 {
            return;
        }
        self.b.set_menu_label("MainMenu_Help_TotalEntBits", &trl_format("Total Entropy Bits: %1", &[&total.to_string()]));
        self.b.action("MainMenu_Help_TotalEntBits").set_enabled(false);
    }

    // ------------------------------------------------------------------
    // list menu of the combo boxes

    fn connect_list_menus(self: &Rc<Self>) {
        let remove = gio::SimpleAction::new("ListRemoveEntry", Some(glib::VariantTy::STRING));
        let w = Rc::downgrade(self);
        remove.connect_activate(move |_, p| {
            let (Some(w), Some(name)) = (w.upgrade(), p.and_then(|p| p.get::<String>())) else { return };
            let cb = w.combo(&name);
            if let Some(i) = cb.active() {
                let text = combo_text(&cb);
                cb.remove(i as i32);
                set_combo_text(&cb, &text);
            }
        });
        self.b.actions.add_action(&remove);
        let clear = gio::SimpleAction::new("ListClearList", Some(glib::VariantTy::STRING));
        let w = Rc::downgrade(self);
        clear.connect_activate(move |_, p| {
            let (Some(w), Some(name)) = (w.upgrade(), p.and_then(|p| p.get::<String>())) else { return };
            glib::spawn_future_local(async move {
                let cb = w.combo(&name);
                if cb.model().map(|m| m.iter_n_children(None)).unwrap_or(0) == 0 {
                    return;
                }
                if msg_box(
                    Some(&w.win()),
                    &trl("All entries will be removed from the list.\nAre you sure?"),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    == MsgResult::Yes
                {
                    let text = combo_text(&cb);
                    cb.remove_all();
                    set_combo_text(&cb, &text);
                }
            });
        });
        self.b.actions.add_action(&clear);
        let label = |name: &str, def: &str| {
            self.b
                .obj(name)
                .and_then(|o| o.str("Caption"))
                .map(|c| super::super::builder::mnemonic(&super::super::builder::tr_caption(c)))
                .unwrap_or_else(|| def.to_string())
        };
        let remove_label = label("ListMenu_RemoveEntry", "Remove Entry");
        let clear_label = label("ListMenu_ClearList", "Clear List");
        for name in ["CharSetList", "WLFNList", "FormatList", "ScriptList"] {
            let Some(e) = combo_entry(&self.combo(name)) else { continue };
            let menu = gio::Menu::new();
            let item = gio::MenuItem::new(Some(&remove_label), None);
            item.set_action_and_target_value(Some("m.ListRemoveEntry"), Some(&name.to_variant()));
            menu.append_item(&item);
            let item = gio::MenuItem::new(Some(&clear_label), None);
            item.set_action_and_target_value(Some("m.ListClearList"), Some(&name.to_variant()));
            menu.append_item(&item);
            e.set_extra_menu(Some(&menu));
            e.set_enable_undo(true);
        }
    }

    // ------------------------------------------------------------------
    // backup / restore settings

    fn backup_settings(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::new();
        dlg.set_title(&trl("Select folder to store backup"));
        let w = self.clone();
        dlg.select_folder(Some(&self.window), gio::Cancellable::NONE, move |r| {
            let Some(dir) = r.ok().and_then(|f| f.path()) else { return };
            let ini_path = dir.join(PROGRAM_INIFILE);
            if !w.save_config_to(Some(&ini_path)) {
                return;
            }
            let mut failed = Vec::new();
            for le in &w.core.borrow().languages {
                let Some(src) = &le.file_name else { continue };
                let Some(name) = src.file_name() else { continue };
                if std::fs::copy(src, dir.join(name)).is_err() {
                    failed.push(name.to_string_lossy().into_owned());
                }
            }
            let w2 = w.clone();
            glib::spawn_future_local(async move {
                if !failed.is_empty() {
                    let m = format!("{}\n{}", trl("The following files could not be copied:"), failed.join("\n"));
                    msg_box(Some(&w2.win()), &m, MsgIcon::Error, MsgButtons::Ok, false).await;
                }
                msg_box(
                    Some(&w2.win()),
                    &trl_format("Backup of settings and language files\nstored in \"%1\".", &[&dir.to_string_lossy()]),
                    MsgIcon::Info,
                    MsgButtons::Ok,
                    false,
                )
                .await;
            });
        });
    }

    fn restore_settings(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::new();
        dlg.set_title(&trl("Select folder containing backup to restore"));
        let w = self.clone();
        dlg.select_folder(Some(&self.window), gio::Cancellable::NONE, move |r| {
            let Some(dir) = r.ok().and_then(|f| f.path()) else { return };
            let w = w.clone();
            glib::spawn_future_local(async move { w.restore_settings_from(&dir).await });
        });
    }

    async fn restore_settings_from(self: &Rc<Self>, dir: &Path) {
        let parent = self.win();
        let src = dir.join(PROGRAM_INIFILE);
        if !src.is_file() {
            msg_box(
                Some(&parent),
                &trl_format("Configuration file \"%1\" not found in folder.", &[PROGRAM_INIFILE]),
                MsgIcon::Error,
                MsgButtons::Ok,
                false,
            )
            .await;
            return;
        }
        if let Ok(ini) = pwtech_core::config::IniFile::load(&src) {
            let mut v = ini.read_string("Main", "LastVersion", "");
            if v.is_empty() {
                v = "0".into();
            }
            if crate::app::compare_versions(&v, PROGRAM_VERSION) < 0
                && msg_box(
                    Some(&parent),
                    &trl_format("Backup was created with an older version (%1)\nof the program. Continue anyway?", &[&v]),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    false,
                )
                .await
                    == MsgResult::No
            {
                return;
            }
        }
        let (data_dir, dest) = {
            let core = self.core.borrow();
            (core.paths.data_dir.clone(), core.paths.ini_file.clone())
        };
        let _ = std::fs::create_dir_all(&data_dir);
        if std::fs::copy(&src, &dest).is_err() {
            msg_box(
                Some(&parent),
                &trl_format("Could not copy configuration file to\n\"%1\".", &[&dest.to_string_lossy()]),
                MsgIcon::Error,
                MsgButtons::Ok,
                false,
            )
            .await;
            return;
        }
        let mut failed = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                if ext != "lng" && ext != "po" {
                    continue;
                }
                if let Some(name) = p.file_name() {
                    if std::fs::copy(&p, data_dir.join(name)).is_err() {
                        failed.push(p.to_string_lossy().into_owned());
                    }
                }
            }
        }
        if !failed.is_empty() {
            let m = format!("{}\n{}", trl("The following files could not be copied:"), failed.join("\n"));
            msg_box(Some(&parent), &m, MsgIcon::Error, MsgButtons::Ok, false).await;
        }
        // ensure that the restored file will not be overwritten during runtime
        self.core.borrow_mut().fake_ini = true;
        if msg_box(
            Some(&parent),
            &trl("Restored settings will only take effect after\na program restart. Do you want to restart now?"),
            MsgIcon::Question,
            MsgButtons::YesNo,
            false,
        )
        .await
            == MsgResult::Yes
        {
            self.restart_program();
        } else {
            msg_box(
                Some(&parent),
                &trl("Configuration settings will not be saved until\nthe next time the program is started."),
                MsgIcon::Info,
                MsgButtons::Ok,
                false,
            )
            .await;
        }
    }

    /// Close the program and start it again (`TerminateAction::RestartProgram`).
    pub(super) fn restart_program(&self) {
        crate::app::request_restart();
        self.window.close();
    }

    // ------------------------------------------------------------------
    // tool dialogs

    /// `MainMenu_Tools_ProvideAddEntropy_AsTextClick()`.
    fn provide_entropy(self: &Rc<Self>) {
        if self.tools.provide_entropy.borrow().is_none() {
            let b = build_form(load_form("ProvideEntropy"), "ProvideEntropy");
            let win = super::super::dialogs::form_window(&b, Some(&self.win()), true);
            win.set_hide_on_close(true);
            {
                let core = self.core.borrow();
                win.set_default_size(
                    core.ini.read_int("ProvideEntropy", "WindowWidth", b.client.0).max(b.client.0),
                    core.ini.read_int("ProvideEntropy", "WindowHeight", b.client.1).max(b.client.1),
                );
            }
            if let Some(l) = b.try_get::<gtk::Label>("InfoLbl") {
                l.set_wrap(true);
            }
            let view: gtk::TextView = b.get("TextBox.view");
            view.set_wrap_mode(gtk::WrapMode::WordChar);
            let ok: gtk::Button = b.get("OKBtn");
            ok.set_sensitive(false);
            let ok2 = ok.clone();
            view.buffer().connect_changed(move |buf| ok2.set_sensitive(buf.char_count() != 0));
            let win2 = win.clone();
            b.get::<gtk::Button>("CancelBtn").connect_clicked(move |_| win2.close());
            let w = Rc::downgrade(self);
            let win2 = win.clone();
            let view2 = view.clone();
            ok.connect_clicked(move |_| {
                let Some(w) = w.upgrade() else { return };
                let buf = view2.buffer();
                let text = Zeroizing::new(buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string());
                buf.set_text("");
                if !text.is_empty() {
                    let bits = EntropyManager::global().add_data(&mut RandomPool::global(), text.as_bytes(), 0.437, 3.5);
                    w.update_entropy_progress(false);
                    msg(
                        Some(&win2),
                        &trl_format("%1 bits of entropy have been added to the random pool.", &[&bits.to_string()]),
                        MsgIcon::Info,
                    );
                }
                win2.close();
            });
            let view2 = view.clone();
            win.connect_hide(move |_| view2.buffer().set_text(""));
            win.set_default_widget(Some(&ok));
            self.attach_entropy_events(&win);
            *self.tools.provide_entropy.borrow_mut() = Some((b, win));
        }
        if let Some((b, win)) = self.tools.provide_entropy.borrow().as_ref() {
            win.present();
            b.w("TextBox.view").grab_focus();
        }
    }

    /// `MainMenu_Tools_CreateRandDataFileClick()`.
    fn create_rand_data_file(self: &Rc<Self>) {
        if self.tools.rand_data_file.borrow().is_none() {
            let b = build_form(load_form("CreateRandDataFile"), "CreateRandDataFile");
            let win = super::super::dialogs::form_window(&b, Some(&self.win()), true);
            win.set_hide_on_close(true);
            {
                let core = self.core.borrow();
                win.set_default_size(
                    core.ini.read_int("CreateRandDataFile", "WindowWidth", b.client.0).max(b.client.0),
                    b.client.1,
                );
                b.get::<gtk::Entry>("FileSizeBox").set_text(&core.ini.read_string("CreateRandDataFile", "FileSize", ""));
                let units: gtk::ComboBoxText = b.get("SizeUnitList");
                let items: Vec<String> = b.obj("SizeUnitList").map(|o| o.strings("Items.Strings")).unwrap_or_default().iter().map(|s| trl(s)).collect();
                set_combo_items(&units, &items);
                let n = units.model().map(|m| m.iter_n_children(None)).unwrap_or(0);
                let idx = core.ini.read_int("CreateRandDataFile", "FileSizeUnitIdx", 0);
                units.set_active(Some(if idx >= 0 && idx < n { idx as u32 } else { 0 }));
            }
            let create: gtk::Button = b.get("CreateFileBtn");
            let name_box: gtk::Entry = b.get("FileNameBox");
            let size_box: gtk::Entry = b.get("FileSizeBox");
            let update = {
                let (create, name_box, size_box) = (create.clone(), name_box.clone(), size_box.clone());
                move || create.set_sensitive(name_box.text_length() != 0 && size_box.text_length() != 0)
            };
            update();
            let u = update.clone();
            name_box.connect_changed(move |_| u());
            let u = update.clone();
            size_box.connect_changed(move |_| u());
            let win2 = win.clone();
            b.get::<gtk::Button>("CloseBtn").connect_clicked(move |_| win2.close());
            let win2 = win.clone();
            let nb = name_box.clone();
            b.get::<gtk::Button>("BrowseBtn").connect_clicked(move |_| {
                let dlg = gtk::FileDialog::new();
                let nb = nb.clone();
                dlg.save(Some(&win2), gio::Cancellable::NONE, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        nb.set_text(&p.to_string_lossy());
                        nb.grab_focus();
                    }
                });
            });
            let w = Rc::downgrade(self);
            create.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    glib::spawn_future_local(async move { w.create_rand_data_file_run().await });
                }
            });
            win.set_default_widget(Some(&create));
            self.attach_entropy_events(&win);
            *self.tools.rand_data_file.borrow_mut() = Some((b, win));
        }
        if let Some((b, win)) = self.tools.rand_data_file.borrow().as_ref() {
            win.present();
            b.w("FileNameBox").grab_focus();
        }
    }

    /// `CreateFileBtnClick()` of `TCreateRandDataFileDlg`.
    async fn create_rand_data_file_run(self: &Rc<Self>) {
        let (name, size_text, unit, win) = {
            let r = self.tools.rand_data_file.borrow();
            let (b, win) = r.as_ref().unwrap();
            (
                b.get::<gtk::Entry>("FileNameBox").text().to_string(),
                b.get::<gtk::Entry>("FileSizeBox").text().to_string(),
                b.get::<gtk::ComboBoxText>("SizeUnitList").active().unwrap_or(0),
                win.clone(),
            )
        };
        let mut path = PathBuf::from(&name);
        if path.parent().is_none_or(|p| p.as_os_str().is_empty()) {
            path = self.core.borrow().paths.data_dir.join(&name);
        }
        let size: u64 = if size_text.contains('.') {
            let mut d: f64 = size_text.trim().parse().unwrap_or(0.0);
            for _ in 0..unit {
                d *= 1024.0;
            }
            if d.is_finite() && d > 0.0 { d as u64 } else { 0 }
        } else {
            let mut q: u64 = size_text.trim().parse::<i64>().unwrap_or(0).max(0) as u64;
            for _ in 0..unit {
                q = q.saturating_mul(1024);
            }
            q
        };
        // limit file size to 10 TB
        if size == 0 || size > 0xa00_0000_0000 {
            msg_box(Some(&win), &trl("Invalid file size."), MsgIcon::Error, MsgButtons::Ok, false).await;
            return;
        }
        if let Some(free) = free_disk_space(&path) {
            if free < size {
                msg_box(
                    Some(&win),
                    &trl("Not enough free disk space available\nto create the file."),
                    MsgIcon::Error,
                    MsgButtons::Ok,
                    false,
                )
                .await;
                return;
            }
        }
        // random source: a copy of the pool, or the deterministic generator
        let det = self.key_seeded_prng.borrow_mut().take();
        let mut rng: GenRng = match det {
            Some(g) => GenRng::Det(g),
            None => {
                let mut pool = RandomPool::global();
                let mut p = RandomPool::derive_from(&mut pool);
                drop(pool);
                p.randomize_system();
                EntropyManager::global().consume_entropy_bits(MAX_ENTROPY);
                GenRng::Pool(p)
            }
        };
        let (step, step_name) = if size >= 104_857_600 { (1_048_576u64, "MB") } else { (1024u64, "KB") };
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let (tx, rx) = async_channel::bounded(1);
        let (c2, p2, path2) = (cancel.clone(), progress.clone(), path.clone());
        std::thread::spawn(move || {
            let mut written: u64 = 0;
            let r = (|| -> std::io::Result<()> {
                use std::io::Write;
                let mut f = std::fs::File::create(&path2)?;
                let mut buf = Zeroizing::new(vec![0u8; if size >= 10_485_760 { 1_048_576 } else { 65_536 }]);
                let mut rest = size;
                while rest > 0 && !c2.load(Ordering::Relaxed) {
                    let n = rest.min(buf.len() as u64) as usize;
                    rng.get_data(&mut buf[..n]);
                    f.write_all(&buf[..n])?;
                    written += n as u64;
                    rest -= n as u64;
                    p2.store(written / step, Ordering::Relaxed);
                }
                f.flush()
            })();
            let _ = tx.send_blocking((rng, written, r));
        });
        win.set_sensitive(false);
        let mut progress_win: Option<Rc<ProgressWindow>> = None;
        let start = std::time::Instant::now();
        let fmt = trl("%1 of %2 %3 written.").replace("%3", step_name);
        let (rng, written, r) = loop {
            if let Ok(r) = rx.try_recv() {
                break r;
            }
            glib::timeout_future(std::time::Duration::from_millis(20)).await;
            if progress_win.is_none() && start.elapsed().as_millis() >= 1000 {
                let pw = ProgressWindow::new(&win, &trl("Creating random data file ..."), "");
                pw.win.present();
                progress_win = Some(pw);
            }
            if let Some(pw) = &progress_win {
                if pw.cancel.get() {
                    cancel.store(true, Ordering::Relaxed);
                }
                pw.set(progress.load(Ordering::Relaxed), size / step, &fmt);
            }
        };
        if let Some(pw) = progress_win {
            pw.win.destroy();
        }
        win.set_sensitive(true);
        if let GenRng::Det(g) = rng {
            *self.key_seeded_prng.borrow_mut() = Some(g);
        }
        let err = match r {
            Err(e) => Some(e.to_string()),
            Ok(()) if cancel.load(Ordering::Relaxed) => Some(trl("Process cancelled by user")),
            Ok(()) => None,
        };
        match err {
            Some(e) => {
                msg_box(
                    Some(&win),
                    &trl_format(
                        "Error while creating file\n\"%1\":\n%2.\n\n%3 bytes written.",
                        &[&path.to_string_lossy(), &e, &written.to_string()],
                    ),
                    MsgIcon::Error,
                    MsgButtons::Ok,
                    false,
                )
                .await;
            }
            None => {
                let fname = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                msg_box(
                    Some(&win),
                    &trl_format("File \"%1\" successfully created.\n\n%2 bytes written.", &[&fname, &written.to_string()]),
                    MsgIcon::Info,
                    MsgButtons::Ok,
                    false,
                )
                .await;
            }
        }
        self.update_entropy_progress(false);
    }

    /// `MainMenu_Tools_CreateTrigramFileClick()`.
    fn create_trigram_file(self: &Rc<Self>) {
        if self.tools.trigram_file.borrow().is_none() {
            let b = build_form(load_form("CreateTrigramFile"), "CreateTrigramFile");
            let win = super::super::dialogs::form_window(&b, Some(&self.win()), true);
            win.set_hide_on_close(true);
            win.set_default_size(
                self.core.borrow().ini.read_int("CreateTrigramFile", "WindowWidth", b.client.0).max(b.client.0),
                b.client.1,
            );
            let create: gtk::Button = b.get("CreateFileBtn");
            let src: gtk::Entry = b.get("SourceFileBox");
            let dest: gtk::Entry = b.get("DestFileBox");
            let update = {
                let (create, src, dest) = (create.clone(), src.clone(), dest.clone());
                move || create.set_sensitive(src.text_length() != 0 && dest.text_length() != 0)
            };
            update();
            let u = update.clone();
            src.connect_changed(move |_| u());
            let u = update.clone();
            dest.connect_changed(move |_| u());
            let win2 = win.clone();
            b.get::<gtk::Button>("CloseBtn").connect_clicked(move |_| win2.close());
            let (win2, src2) = (win.clone(), src.clone());
            b.get::<gtk::Button>("BrowseBtn").connect_clicked(move |_| {
                let dlg = gtk::FileDialog::new();
                let e = src2.clone();
                dlg.open(Some(&win2), gio::Cancellable::NONE, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        e.set_text(&p.to_string_lossy());
                        e.grab_focus();
                    }
                });
            });
            let (win2, dest2) = (win.clone(), dest.clone());
            b.get::<gtk::Button>("BrowseBtn2").connect_clicked(move |_| {
                let dlg = gtk::FileDialog::new();
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                let f = gtk::FileFilter::new();
                f.set_name(Some(&format!("{} (*.tgm)", trl("Trigram files"))));
                f.add_pattern("*.tgm");
                filters.append(&f);
                let f = gtk::FileFilter::new();
                f.set_name(Some(&format!("{} (*.*)", trl("All files"))));
                f.add_pattern("*");
                filters.append(&f);
                dlg.set_filters(Some(&filters));
                let e = dest2.clone();
                dlg.save(Some(&win2), gio::Cancellable::NONE, move |r| {
                    if let Some(mut p) = r.ok().and_then(|f| f.path()) {
                        if p.extension().is_none() {
                            p.set_extension("tgm");
                        }
                        e.set_text(&p.to_string_lossy());
                        e.grab_focus();
                    }
                });
            });
            let w = Rc::downgrade(self);
            let win2 = win.clone();
            create.connect_clicked(move |_| {
                let Some(w) = w.upgrade() else { return };
                let (src_t, dest_t) = {
                    let r = w.tools.trigram_file.borrow();
                    let b = &r.as_ref().unwrap().0;
                    (b.get::<gtk::Entry>("SourceFileBox").text(), b.get::<gtk::Entry>("DestFileBox").text())
                };
                let data_dir = w.core.borrow().paths.data_dir.clone();
                let abs = |s: &str| {
                    let p = PathBuf::from(s);
                    if p.parent().is_none_or(|x| x.as_os_str().is_empty()) { data_dir.join(p) } else { p }
                };
                let (sp, dp) = (abs(&src_t), abs(&dest_t));
                let enc = w.core.borrow().file_encoding;
                let (m, icon) = match PasswordGenerator::create_trigram_file(&sp, &dp, enc) {
                    Ok((n, bits)) => (
                        trl_format(
                            "Trigram file \"%1\" successfully created.\n\n%2 trigrams evaluated.\n%3 bits of entropy per letter.",
                            &[
                                &dp.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                                &n.to_string(),
                                &format!("{bits:.2}"),
                            ],
                        ),
                        MsgIcon::Info,
                    ),
                    Err(e) => (
                        trl_format("Error while creating file\n\"%1\":\n%2.", &[&dp.to_string_lossy(), &trl(&e.to_string())]),
                        MsgIcon::Error,
                    ),
                };
                msg(Some(&win2), &m, icon);
            });
            win.set_default_widget(Some(&create));
            self.attach_entropy_events(&win);
            *self.tools.trigram_file.borrow_mut() = Some((b, win));
        }
        if let Some((b, win)) = self.tools.trigram_file.borrow().as_ref() {
            win.present();
            b.w("SourceFileBox").grab_focus();
        }
    }

    /// `MainMenu_Help_AboutClick()`.
    fn about(self: &Rc<Self>) {
        if self.tools.about.borrow().is_none() {
            let b = build_form(load_form("About"), "About");
            let win = super::super::dialogs::form_window(&b, Some(&self.win()), true);
            win.set_hide_on_close(true);
            win.set_resizable(false);
            win.set_title(Some(&trl_format("About %1", &[PROGRAM_NAME])));
            b.get::<gtk::Label>("ProgramLbl").set_text(PROGRAM_NAME);
            b.get::<gtk::Label>("VersionLbl").set_text(&format!(
                "Version {} ({}-bit, Linux port {PORT_VERSION})",
                PROGRAM_VERSION,
                usize::BITS
            ));
            let link = |url: &str, text: &str| {
                format!(
                    "<a href=\"{}\">{}</a>",
                    glib::markup_escape_text(url),
                    glib::markup_escape_text(text)
                )
            };
            b.get::<gtk::Label>("AuthorLink").set_markup(&format!(
                "{} {}",
                glib::markup_escape_text(PROGRAM_COPYRIGHT),
                link(&format!("mailto:{PROGRAM_AUTHOR_EMAIL}"), PROGRAM_AUTHOR)
            ));
            let www = if PORT_REPOSITORY.is_empty() { PROGRAM_URL_ORIGINAL } else { PORT_REPOSITORY };
            b.get::<gtk::Label>("WWWLink").set_markup(&link(www, www));
            let license = self.core.borrow().paths.resources.join(PROGRAM_LICENSEFILE);
            let license_uri = gio::File::for_path(&license).uri();
            b.get::<gtk::Label>("LicenseLink").set_markup(&link(&license_uri, &trl("View license")));
            b.get::<gtk::Label>("LicenseLink").set_tooltip_text(Some(PROGRAM_LICENSEFILE));
            for name in ["AuthorLink", "WWWLink", "LicenseLink"] {
                let wv = win.clone();
                b.get::<gtk::Label>(name).connect_activate_link(move |_, uri| {
                    open_uri(Some(&wv), uri);
                    glib::Propagation::Stop
                });
            }
            let (lang_name, translator) = {
                let core = self.core.borrow();
                let le = &core.languages[core.language];
                if le.file_name.is_some() {
                    (
                        format!("{} (v{})", le.name, le.version),
                        pwtech_core::language::translator_name().unwrap_or_default(),
                    )
                } else {
                    (LANGUAGE_DEFAULT_NAME.to_string(), "(original language)".to_string())
                }
            };
            let li: gtk::Label = b.get("LanguageInfoLbl");
            li.set_wrap(true);
            li.set_text(&trl_format("Current language: %1\nTranslator: %2", &[&lang_name, &translator]));
            if let Some(l) = b.try_get::<gtk::Label>("LicenseLbl") {
                l.set_wrap(true);
            }
            if let Some(l) = b.try_get::<gtk::Label>("DonorLbl") {
                l.set_text("Community");
            }
            if let Some(p) = b.try_get::<gtk::Picture>("Logo") {
                let bytes = glib::Bytes::from_static(include_bytes!("../../../../assets/about_logo.png"));
                if let Ok(t) = gdk::Texture::from_bytes(&bytes) {
                    p.set_paintable(Some(&t));
                }
            }
            let win2 = win.clone();
            let ok: gtk::Button = b.get("OKBtn");
            ok.connect_clicked(move |_| win2.close());
            win.set_default_widget(Some(&ok));
            *self.tools.about.borrow_mut() = Some((b, win));
        }
        if let Some((_, win)) = self.tools.about.borrow().as_ref() {
            win.present();
        }
    }
}

/// Free space on the file system of `path` (or its directory).
fn free_disk_space(path: &Path) -> Option<u64> {
    let dir = if path.is_dir() { path } else { path.parent().filter(|p| !p.as_os_str().is_empty())? };
    let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid C string and output struct
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}
