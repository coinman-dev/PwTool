//! Tools and menus of the main window (child module of `main_window`):
//! encryption of the clipboard, deterministic random generator, password
//! cache, list menu of the combo boxes, backup/restore of settings, the
//! small tool dialogs (`ProvideEntropy`, `CreateRandDataFile`,
//! `CreateTrigramFile`, `About`) and the update check.

use std::path::Path;

use pwtool_core::crypt_text::{CryptTextError, decrypt_text_with_password, encrypt_text_with_password};

use crate::ui::passw_enter::*;
use super::*;

/// State of the tools (dialogs are created on first use).
pub(super) struct ToolsState {
    pub passw_enter: RefCell<Option<Rc<PasswEnterDlg>>>,
    provide_entropy: RefCell<Option<Rc<Form>>>,
    rand_data_file: RefCell<Option<Rc<Form>>>,
    trigram_file: RefCell<Option<Rc<Form>>>,
    about: RefCell<Option<Rc<Form>>>,
    last_cache_countdown: Cell<i64>,
    last_total_bits: Cell<i64>,
    update_check_running: Cell<bool>,
    pub config_dlg: RefCell<Option<Rc<super::config_dlg::ConfigDlg>>>,
    pub profile_dlg: RefCell<Option<Rc<super::profile_editor::ProfileEditDlg>>>,
    pub char_set_builder: RefCell<Option<Rc<super::char_set_builder::CharSetBuilderWin>>>,
    pub mp_passw_gen: RefCell<Option<Rc<super::mp_passw_gen::MpPasswGenWin>>>,
    pub passw_mng: RefCell<Option<Rc<crate::ui::passw_manager::PasswManager>>>,
    /// Window sizes of the tool dialogs (`WindowWidth`/`WindowHeight`).
    sizes: RefCell<std::collections::HashMap<&'static str, egui::Vec2>>,
}

impl ToolsState {
    pub fn new() -> Self {
        ToolsState {
            passw_enter: RefCell::new(None),
            provide_entropy: RefCell::new(None),
            rand_data_file: RefCell::new(None),
            trigram_file: RefCell::new(None),
            about: RefCell::new(None),
            last_cache_countdown: Cell::new(-1),
            last_total_bits: Cell::new(-1),
            update_check_running: Cell::new(false),
            config_dlg: RefCell::new(None),
            profile_dlg: RefCell::new(None),
            char_set_builder: RefCell::new(None),
            mp_passw_gen: RefCell::new(None),
            passw_mng: RefCell::new(None),
            sizes: RefCell::new(std::collections::HashMap::new()),
        }
    }
}

/// A form window that is kept and shown again (`Show()`): a click on a
/// close button only closes the window.
pub(super) fn tool_window(id: &'static str, form: &Rc<Form>, parent: ViewportId, modal: bool, size: Option<egui::Vec2>) -> Rc<windows::Window> {
    let mut w = dialogs::form_window(id, form.clone(), parent, modal);
    let min = w.size.get();
    if let Some(s) = size {
        w.size.set(s.max(min));
    }
    if form.obj.ident("BorderStyle").is_none_or(|b| b == "bsSizeable" || b == "bsSizeToolWin") {
        w = w.resizable(true, min);
    }
    Rc::new(w)
}

impl MainWindow {
    pub(super) fn connect_tools(self: &Rc<Self>) {
        self.menu("MainMenu_Tools_EncryptClip", |w| w.crypt_text_spawn(true, None, ROOT));
        self.menu("MainMenu_Tools_DecryptClip", |w| w.crypt_text_spawn(false, None, ROOT));
        self.menu("TrayMenu_EncryptClip", |w| w.crypt_text_spawn(true, None, ROOT));
        self.menu("TrayMenu_DecryptClip", |w| w.crypt_text_spawn(false, None, ROOT));
        self.menu("PasswBoxMenu_EncryptCopy", |w| {
            let text = w.f().selected_text("PasswBox");
            w.crypt_text_spawn(true, Some(text), ROOT);
        });
        self.menu("MainMenu_Tools_DetermRandGen_Setup", |w| {
            let w = w.clone();
            spawn(async move { w.det_rand_gen_setup().await });
        });
        self.menu("MainMenu_Options_ClearPasswCache", |w| {
            w.passw_enter().clear_passw_cache();
            info_box(ROOT, &trl("Password cache cleared."));
            w.update_passw_cache_menu();
        });
        self.menu("MainMenu_Tools_ProvideAddEntropy_AsText", |w| w.provide_entropy());
        self.menu("MainMenu_Tools_CreateRandDataFile", |w| w.create_rand_data_file());
        self.menu("TrayMenu_CreateRandDataFile", |w| {
            w.restore_from_tray();
            w.create_rand_data_file();
        });
        self.menu("MainMenu_Tools_CreateTrigramFile", |w| w.create_trigram_file());
        self.f().set_menu_caption("MainMenu_Help_About", &trl_format("About %1...", &[PROGRAM_NAME]));
        self.menu("MainMenu_Help_About", |w| w.about());
        self.menu("TrayMenu_About", |w| {
            w.restore_from_tray();
            w.about();
        });
        self.menu("TrayMenu_OpenManual", |w| w.help_btn_click());
        self.menu("MainMenu_File_BackupSettings", |w| {
            let w = w.clone();
            spawn(async move { w.backup_settings().await });
        });
        self.menu("MainMenu_File_RestoreSettings", |w| {
            let w = w.clone();
            spawn(async move { w.restore_settings().await });
        });
        for name in ["MainMenu_Tools_MPPasswGen", "TrayMenu_MPPasswGen"] {
            self.menu(name, |w| {
                w.restore_from_tray();
                w.mp_passw_gen();
            });
        }
        self.click("MPPasswGenBtn", |w| w.mp_passw_gen());
        for name in ["MainMenu_File_PasswManager", "TrayMenu_PasswManager"] {
            self.menu(name, |w| {
                w.restore_from_tray();
                w.show_passw_manager();
            });
        }
        self.click("PasswMngBtn", |w| w.show_passw_manager());
        self.menu("PasswBoxMenu_AddToDb", |w| {
            let passw = w.f().selected_text("PasswBox");
            if !passw.is_empty() {
                w.add_passw_to_db(&passw, false, None);
            }
        });
        self.menu("PasswBoxMenu_PerformAutotype", |w| {
            let passw = w.f().secret_text("PasswBox");
            w.perform_autotype("", None, Some(&passw));
        });
        self.click("CryptTextBtn", |w| w.crypt_text_spawn(true, None, ROOT));
        self.click("ConfigBtn", |w| {
            let w = w.clone();
            spawn(async move { w.configuration().await });
        });
        self.menu("MainMenu_Options_Config", |w| {
            let w = w.clone();
            spawn(async move { w.configuration().await });
        });
        self.menu("MainMenu_Help_CheckForUpdates", |w| {
            let w = w.clone();
            spawn(async move { w.check_for_updates(true).await });
        });
        self.click("BuildBtn", |w| w.char_set_builder());
        for name in ["MainMenu_File_Profile_ProfileEditor", "TrayMenu_Profile_ProfileEditor"] {
            self.menu(name, |w| {
                w.restore_from_tray();
                let w = w.clone();
                spawn(async move {
                    w.profile_editor(false).await;
                });
            });
        }
        self.click("ProfileEditorBtn", |w| {
            let w = w.clone();
            spawn(async move {
                w.profile_editor(false).await;
            });
        });
        self.click("AddProfileBtn", |w| {
            let w = w.clone();
            spawn(async move {
                let flags = w.profile_editor(true).await;
                if flags & super::profile_editor::PROFILE_ADDED != 0 {
                    let n = w.core.borrow().profiles.len();
                    if n > 0 {
                        w.profile_list_guard.set(true);
                        w.f().set_item_index("ProfileList", Some(n - 1));
                        w.profile_list_guard.set(false);
                    }
                }
            });
        });
        // list menu of the combo boxes (`ListMenu`)
        self.menu("ListMenu_RemoveEntry", |w| {
            let Some(name) = w.list_menu_owner() else { return };
            let text = w.f().text(&name);
            let mut items = w.f().items(&name);
            if let Some(i) = items.iter().position(|s| *s == text) {
                items.remove(i);
                w.f().set_items(&name, items);
                w.f().set_text(&name, &text);
            }
        });
        self.menu("ListMenu_ClearList", |w| {
            let Some(name) = w.list_menu_owner() else { return };
            if w.f().items(&name).is_empty() {
                return;
            }
            let w = w.clone();
            spawn(async move {
                if dialogs::ask(ROOT, &trl("All entries will be removed from the list.\nAre you sure?"), MsgIcon::Warning, MsgButtons::YesNo, true)
                    .await
                    == MsgResult::Yes
                {
                    w.f().set_items(&name, Vec::new());
                    w.f().set_text(&name, "");
                }
            });
        });
        // password box font (`PasswBoxMenu_ChangeFont`)
        let font = self.core.borrow().ini.read_string("Main", "PasswBoxFont", "");
        crate::vcl::fonts::set_passw_font(&windows::ctx(), crate::vcl::fonts::FontSpec::parse(&font));
        self.f().set_font_family("PasswBox", Some(crate::vcl::fonts::PASSW));
        self.menu("PasswBoxMenu_ChangeFont", |_| {
            spawn(async move {
                let cur = crate::vcl::fonts::passw_font();
                if let Some(f) = crate::ui::font_dialog::choose_font(ROOT, cur.as_ref(), true).await {
                    crate::vcl::fonts::set_passw_font(&windows::ctx(), Some(f));
                }
            });
        });
        self.menu("PasswBoxMenu_ResetFont", |_| crate::vcl::fonts::set_passw_font(&windows::ctx(), None));
        self.update_passw_cache_menu();
        self.update_total_ent_bits_menu();
    }

    /// Combo box whose list menu is open.
    fn list_menu_owner(&self) -> Option<String> {
        self.f().popup_owner().filter(|n| ["CharSetList", "WLFNList", "FormatList", "ScriptList"].contains(&n.as_str()))
    }

    /// Enabled state of the list menu items (every frame).
    pub(super) fn update_list_menu(&self) {
        let Some(name) = self.list_menu_owner() else { return };
        let items = self.f().items(&name);
        let text = self.f().text(&name);
        self.f().set_menu_enabled("ListMenu_RemoveEntry", items.iter().any(|s| *s == text));
        self.f().set_menu_enabled("ListMenu_ClearList", !items.is_empty());
    }

    /// Called every second by the main timer.
    pub(super) fn tools_timer(&self) {
        self.update_passw_cache_menu();
        self.update_total_ent_bits_menu();
    }

    pub(crate) fn passw_enter(&self) -> Rc<PasswEnterDlg> {
        if let Some(d) = self.tools.passw_enter.borrow().as_ref() {
            d.use_advanced_passw_est.set(self.core.borrow().use_advanced_passw_est);
            return d.clone();
        }
        let d = PasswEnterDlg::new(&self.core.borrow().ini);
        d.use_advanced_passw_est.set(self.core.borrow().use_advanced_passw_est);
        *self.tools.passw_enter.borrow_mut() = Some(d.clone());
        d
    }

    pub(super) fn tool_size(&self, id: &'static str, ini_id: &str, height: bool) -> Option<egui::Vec2> {
        if let Some(s) = self.tools.sizes.borrow().get(id) {
            return Some(*s);
        }
        let core = self.core.borrow();
        let w = core.ini.read_int(ini_id, "WindowWidth", 0);
        let h = if height { core.ini.read_int(ini_id, "WindowHeight", 0) } else { 0 };
        (w > 0).then(|| egui::vec2(w as f32, h as f32))
    }

    pub(super) fn remember_size(&self, id: &'static str, win: &windows::Window) {
        if let Some(s) = win.last_size.get() {
            self.tools.sizes.borrow_mut().insert(id, s);
        }
    }

    /// Show a kept tool window; its size is remembered when it closes.
    pub(super) fn show_tool(self: &Rc<Self>, id: &'static str, ini_id: &str, height: bool, form: &Rc<Form>, parent: ViewportId, modal: bool) -> Rc<windows::Window> {
        let win = tool_window(id, form, parent, modal, self.tool_size(id, ini_id, height));
        let w = self.weak();
        let wid = Rc::downgrade(&win);
        *win.on_close.borrow_mut() = Some(Box::new(move || {
            if let (Some(w), Some(win)) = (w.upgrade(), wid.upgrade()) {
                w.remember_size(id, &win);
            }
            true
        }));
        windows::present(win.clone());
        win
    }

    /// Close a kept tool window, remembering its size.
    pub(super) fn close_tool(&self, id: &'static str) {
        let vid = ViewportId::from_hash_of(id);
        if let Some(s) = windows::last_size(vid) {
            self.tools.sizes.borrow_mut().insert(id, s);
        }
        windows::close(vid);
    }

    /// Save the configuration of the tool dialogs (part of `SaveConfig()`).
    pub(super) fn save_tools_config(&self) {
        for (id, _) in [("ProvideEntropy", 0), ("CreateRandDataFile", 0), ("CreateTrigramFile", 0), ("ConfigWin", 0), ("ProfileEditor", 0), ("CharSetBuilder", 0), ("MPPasswGen", 0)] {
            if let Some(s) = windows::last_size(ViewportId::from_hash_of(id)) {
                self.tools.sizes.borrow_mut().insert(id, s);
            }
        }
        let sizes = self.tools.sizes.borrow().clone();
        let mut core = self.core.borrow_mut();
        let ini = &mut core.ini;
        for (id, s) in sizes {
            ini.write_integer(id, "WindowWidth", s.x as i64);
            // windows of a fixed height
            if !["CreateRandDataFile", "CreateTrigramFile", "MPPasswGen"].contains(&id) {
                ini.write_integer(id, "WindowHeight", s.y as i64);
            }
        }
        ini.write_string("Main", "PasswBoxFont", &crate::vcl::fonts::passw_font().map(|f| f.to_font_string()).unwrap_or_default());
        if let Some(d) = self.tools.passw_enter.borrow().as_ref() {
            d.save_config(ini);
        }
        if let Some(f) = self.tools.rand_data_file.borrow().as_ref() {
            ini.write_string("CreateRandDataFile", "FileSize", &f.text("FileSizeBox"));
            ini.write_integer("CreateRandDataFile", "FileSizeUnitIdx", f.item_index("SizeUnitList").unwrap_or(0) as i64);
        }
        if let Some(d) = self.tools.profile_dlg.borrow().as_ref() {
            d.save_config(ini);
        }
        if let Some(d) = self.tools.mp_passw_gen.borrow().as_ref() {
            d.save_config(ini);
        }
        if let Some(pm) = self.tools.passw_mng.borrow().as_ref() {
            pm.save_config(ini);
        }
    }

    // ------------------------------------------------------------------
    // password manager

    /// The password manager (created on first use).
    pub fn passw_manager(self: &Rc<Self>) -> Rc<crate::ui::passw_manager::PasswManager> {
        if let Some(pm) = self.tools.passw_mng.borrow().as_ref() {
            return pm.clone();
        }
        let pm = crate::ui::passw_manager::PasswManager::new(self);
        *self.tools.passw_mng.borrow_mut() = Some(pm.clone());
        pm
    }

    /// `MainMenu_File_PasswManagerClick()`.
    pub fn show_passw_manager(self: &Rc<Self>) {
        self.passw_manager().show();
    }

    /// Whether a database is open in the password manager.
    pub(super) fn db_open(&self) -> bool {
        self.tools.passw_mng.borrow().as_ref().is_some_and(|pm| pm.is_db_open())
    }

    /// "Add to Database" (one entry per line).
    pub fn add_passw_to_db(self: &Rc<Self>, passw: &str, show_num_passw: bool, param: Option<&str>) -> usize {
        let pm = self.tools.passw_mng.borrow().clone();
        match pm {
            Some(pm) if pm.is_db_open() => pm.add_passw_to_db(passw, show_num_passw, param),
            _ => 0,
        }
    }

    /// Generate a single password with the current settings and pass it to
    /// `f` instead of showing it (`GeneratePassw(gpdGuiSingle, pEditBox)`).
    pub fn generate_passw_for(self: &Rc<Self>, f: Rc<dyn Fn(&str)>) {
        *self.gen_target.borrow_mut() = Some(f);
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

    /// Formatted password from the deterministic generator if active, else
    /// the random pool (PassCube's "FormatPW" key and new entries).
    pub fn format_passw_with_active_rng(&self, format: &str) -> Option<Zeroizing<String>> {
        let pg = PasswordGenerator::new();
        let f = pwtool_core::unicode::str_to_w32(format);
        let r = if let Some(det) = self.key_seeded_prng.borrow_mut().as_mut() {
            pg.get_format_passw(det, &f, 0, None, 16000, None)
        } else {
            pg.get_format_passw(&mut *RandomPool::global(), &f, 0, None, 16000, None)
        };
        if r.passw.is_empty() { None } else { Some(pwtool_core::unicode::w32_to_string_secure(&r.passw)) }
    }

    // ------------------------------------------------------------------
    // encryption of text

    pub(super) fn crypt_text_spawn(self: &Rc<Self>, encrypt: bool, text: Option<Zeroizing<String>>, parent: ViewportId) {
        let w = self.clone();
        spawn(async move { w.crypt_text(encrypt, text, parent).await });
    }

    /// `CryptText()`: encrypt/decrypt `text` (or the clipboard text) and put
    /// the result on the clipboard.
    pub async fn crypt_text(self: &Rc<Self>, encrypt: bool, text: Option<Zeroizing<String>>, parent: ViewportId) {
        let dlg = self.passw_enter();
        let mut flags = PASSWENTER_FLAG_ENABLEPASSWCACHE;
        flags |= if encrypt {
            PASSWENTER_FLAG_ENCRYPT | PASSWENTER_FLAG_CONFIRMPASSW
        } else {
            PASSWENTER_FLAG_DECRYPT | PASSWENTER_FLAG_ENABLEOLDVER
        };
        if dlg.execute(flags, "", parent).await != EnterResult::Ok {
            dlg.clear_and_flush();
            return;
        }
        let passw = dlg.get_passw(0);
        let old_version = dlg.old_version();
        dlg.clear_and_flush();
        self.update_passw_cache_menu();

        let text = text.or_else(clipboard::read_text);
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
                    r.map(|out| clipboard::set_plain(&out))
                } else {
                    decrypt_text_with_password(&t, &passw, old_version).map(|out| clipboard::set_hidden(&out.text))
                }
            }
        };
        match result {
            Ok(()) => {
                let m = trl(if encrypt { "Text successfully encrypted." } else { "Text successfully decrypted." });
                if self.is_hidden() {
                    self.show_tray_info(&m, false);
                } else {
                    info_box(parent, &m);
                }
            }
            Err(e) => {
                let m = match e {
                    CryptTextError::Clipboard => trl("Could not open clipboard."),
                    CryptTextError::NoText => trl("No text available to process."),
                    CryptTextError::TextTooLong => trl("The text to be encrypted is too long."),
                    CryptTextError::OutOfMemory => trl("Not enough memory available to perform\nthe operation. Try to use a shorter text."),
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
                    CryptTextError::DecomprFailed => trl("This should not have happened:\nDecryption successful, but decompression\nfailed."),
                };
                if self.is_hidden() {
                    self.show_tray_info(&m, true);
                } else {
                    dialogs::ask(parent, &m, MsgIcon::Error, MsgButtons::Ok, false).await;
                }
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
                ROOT,
            )
            .await
            == EnterResult::Ok;
        let passw = if ok { Some(dlg.get_passw(0)) } else { None };
        dlg.clear_and_flush();
        self.update_passw_cache_menu();
        if let Some(p) = passw {
            self.use_key_seeded_prng_with(pwtool_core::mp_passw_gen::deterministic_generator(&p));
        }
    }

    /// Caption/state of Options | Clear Password Cache
    /// (`MainMenu_OptionsClick()`).
    fn update_passw_cache_menu(&self) {
        let d = self.tools.passw_enter.borrow().clone();
        let n = d.as_ref().map(|d| d.passw_cache_expiry_countdown()).unwrap_or(0);
        let cached = d.as_ref().is_some_and(|d| d.has_cached_passw());
        let key = if cached { n.max(0) + 1 } else { 0 };
        if key == self.tools.last_cache_countdown.get() {
            return;
        }
        self.tools.last_cache_countdown.set(key);
        let mut caption = trl("Clear Password Cache");
        if n > 0 {
            caption.push_str(&if n > 99 { format!(" ({:.1}min)", n as f64 / 60.0) } else { format!(" ({n}sec)") });
        }
        self.f().set_menu_caption("MainMenu_Options_ClearPasswCache", &caption);
        self.f().set_menu_enabled("MainMenu_Options_ClearPasswCache", cached);
    }

    /// Caption of Help | Total Entropy Bits (`MainMenu_HelpClick()`).
    pub(super) fn update_total_ent_bits_menu(&self) {
        let total = EntropyManager::global().total_entropy_bits();
        if self.tools.last_total_bits.replace(total as i64) == total as i64 {
            return;
        }
        self.f().set_menu_caption("MainMenu_Help_TotalEntBits", &trl_format("Total Entropy Bits: %1", &[&total.to_string()]));
        self.f().set_menu_enabled("MainMenu_Help_TotalEntBits", false);
    }

    // ------------------------------------------------------------------
    // autotype

    /// Perform autotype (`SendKeys` / `TSendKeysThread`): an empty
    /// `sequence` types `passw` only.
    pub(super) fn perform_autotype(self: &Rc<Self>, sequence: &str, param: Option<&str>, passw: Option<&str>) {
        let (delay, minimize) = {
            let c = &self.core.borrow().config;
            (c.autotype_delay.clamp(0, 10000) as u32, c.minimize_autotype)
        };
        let strokes = if sequence.is_empty() {
            let mut v = Vec::new();
            pwtool_app::autotype::string_sequence(passw.unwrap_or(""), delay, &mut v);
            v
        } else {
            pwtool_app::autotype::build_sequence(sequence, None, param, passw, delay)
        };
        self.send_keys(strokes, minimize);
    }

    // ------------------------------------------------------------------
    // backup / restore settings

    async fn backup_settings(self: &Rc<Self>) {
        let Some(dir) = crate::vcl::dialogs::pick_folder(&trl("Select folder to store backup")).await else { return };
        let ini_path = dir.join(PROGRAM_INIFILE);
        if !self.save_config_to(Some(&ini_path)) {
            return;
        }
        let mut failed = Vec::new();
        for le in &self.core.borrow().languages {
            let Some(src) = &le.file_name else { continue };
            let Some(name) = src.file_name() else { continue };
            if std::fs::copy(src, dir.join(name)).is_err() {
                failed.push(name.to_string_lossy().into_owned());
            }
        }
        if !failed.is_empty() {
            let m = trl_format("Could not copy the following language file(s):\n%1", &[&failed.join("\n")]);
            dialogs::ask(ROOT, &m, MsgIcon::Warning, MsgButtons::Ok, false).await;
        }
        dialogs::ask(
            ROOT,
            &trl_format("Backup of settings and language files\nstored in \"%1\".", &[&dir.to_string_lossy()]),
            MsgIcon::Info,
            MsgButtons::Ok,
            false,
        )
        .await;
    }

    async fn restore_settings(self: &Rc<Self>) {
        let Some(dir) = crate::vcl::dialogs::pick_folder(&trl("Select folder containing backup to restore")).await else { return };
        self.restore_settings_from(&dir).await;
    }

    async fn restore_settings_from(self: &Rc<Self>, dir: &Path) {
        let mut src = dir.join(PROGRAM_INIFILE);
        if !src.is_file() {
            // a backup of the original program (Password Tech)
            src = dir.join("PwTech.ini");
        }
        if !src.is_file() {
            dialogs::ask(ROOT, &trl_format("Configuration file \"%1\" not found in folder.", &[PROGRAM_INIFILE]), MsgIcon::Error, MsgButtons::Ok, false).await;
            return;
        }
        if let Ok(ini) = pwtool_core::config::IniFile::load(&src) {
            let mut v = ini.read_string("Main", "LastVersion", "");
            if v.is_empty() {
                v = "0".into();
            }
            if pwtool_app::app::compare_versions(&v, PROGRAM_VERSION) < 0
                && dialogs::ask(
                    ROOT,
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
            dialogs::ask(ROOT, &trl_format("Could not copy configuration file to\n\"%1\".", &[&dest.to_string_lossy()]), MsgIcon::Error, MsgButtons::Ok, false)
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
            let m = trl_format("Could not copy the following language file(s):\n%1", &[&failed.join("\n")]);
            dialogs::ask(ROOT, &m, MsgIcon::Warning, MsgButtons::Ok, false).await;
        }
        // ensure that the restored file will not be overwritten during runtime
        self.core.borrow_mut().fake_ini = true;
        if dialogs::ask(
            ROOT,
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
            dialogs::ask(
                ROOT,
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
        pwtool_app::app::request_restart();
        windows::ctx().send_viewport_cmd(ViewportCommand::Close);
    }

    // ------------------------------------------------------------------
    // tool dialogs

    /// `MainMenu_Tools_ProvideAddEntropy_AsTextClick()`.
    fn provide_entropy(self: &Rc<Self>) {
        let form = self.tools.provide_entropy.borrow().clone();
        let form = match form {
            Some(f) => f,
            None => {
                let f = dialogs::load("ProvideEntropy");
                f.set_secret("TextBox");
                f.set_enabled("OKBtn", false);
                let ff = Rc::downgrade(&f);
                f.on_change("TextBox", move |_| {
                    if let Some(f) = ff.upgrade() {
                        f.set_enabled("OKBtn", !f.secret_text("TextBox").is_empty());
                    }
                });
                let ww = self.weak();
                f.on_click("CancelBtn", move |_| {
                    if let Some(w) = ww.upgrade() {
                        w.f_provide_entropy_clear();
                        w.close_tool("ProvideEntropy");
                    }
                });
                let ww = self.weak();
                f.on_click("OKBtn", move |_| {
                    let Some(w) = ww.upgrade() else { return };
                    let Some(f) = w.tools.provide_entropy.borrow().clone() else { return };
                    let text = f.secret_text("TextBox");
                    w.f_provide_entropy_clear();
                    w.close_tool("ProvideEntropy");
                    if !text.is_empty() {
                        let bits = EntropyManager::global().add_data(&mut RandomPool::global(), text.as_bytes(), 0.437, 3.5);
                        w.update_entropy_progress(false);
                        msg(ROOT, &trl_format("%1 bits of entropy have been added to the random pool.", &[&bits.to_string()]), MsgIcon::Info);
                    }
                });
                *self.tools.provide_entropy.borrow_mut() = Some(f.clone());
                f
            }
        };
        let win = self.show_tool("ProvideEntropy", "ProvideEntropy", true, &form, ROOT, true);
        let w = self.weak();
        let wid = Rc::downgrade(&win);
        *win.on_close.borrow_mut() = Some(Box::new(move || {
            if let (Some(w), Some(win)) = (w.upgrade(), wid.upgrade()) {
                w.remember_size("ProvideEntropy", &win);
                w.f_provide_entropy_clear();
            }
            true
        }));
        form.focus("TextBox");
    }

    fn f_provide_entropy_clear(&self) {
        if let Some(f) = self.tools.provide_entropy.borrow().as_ref() {
            f.set_text("TextBox", "");
            f.set_enabled("OKBtn", false);
        }
    }

    /// `MainMenu_Tools_CreateRandDataFileClick()`.
    fn create_rand_data_file(self: &Rc<Self>) {
        let form = self.tools.rand_data_file.borrow().clone();
        let form = match form {
            Some(f) => f,
            None => {
                let f = dialogs::load("CreateRandDataFile");
                {
                    let core = self.core.borrow();
                    f.set_text("FileSizeBox", &core.ini.read_string("CreateRandDataFile", "FileSize", ""));
                    let units: Vec<String> = f.items("SizeUnitList").iter().map(|s| trl(s)).collect();
                    let n = units.len() as i32;
                    f.set_items("SizeUnitList", units);
                    let idx = core.ini.read_int("CreateRandDataFile", "FileSizeUnitIdx", 0);
                    f.set_item_index("SizeUnitList", Some(if idx >= 0 && idx < n { idx as usize } else { 0 }));
                }
                let update = |f: &Form| f.set_enabled("CreateFileBtn", !f.text("FileNameBox").is_empty() && !f.text("FileSizeBox").is_empty());
                update(&f);
                for n in ["FileNameBox", "FileSizeBox"] {
                    let ff = Rc::downgrade(&f);
                    f.on_change(n, move |_| {
                        if let Some(f) = ff.upgrade() {
                            update(&f);
                        }
                    });
                }
                let ww = self.weak();
                f.on_click("CloseBtn", move |_| {
                    if let Some(w) = ww.upgrade() {
                        w.close_tool("CreateRandDataFile");
                    }
                });
                let ff = Rc::downgrade(&f);
                f.on_click("BrowseBtn", move |_| {
                    let ff = ff.clone();
                    spawn(async move {
                        if let Some(p) = crate::vcl::dialogs::save_file(&trl("Create random data file"), None, None, &[]).await {
                            if let Some(f) = ff.upgrade() {
                                f.set_text("FileNameBox", &p.to_string_lossy());
                                f.push_event(crate::vcl::Event::Change("FileNameBox".into()));
                                f.focus("FileNameBox");
                            }
                        }
                    });
                });
                let ww = self.weak();
                f.on_click("CreateFileBtn", move |_| {
                    if let Some(w) = ww.upgrade() {
                        spawn(async move { w.create_rand_data_file_run().await });
                    }
                });
                *self.tools.rand_data_file.borrow_mut() = Some(f.clone());
                f
            }
        };
        self.show_tool("CreateRandDataFile", "CreateRandDataFile", false, &form, ROOT, false);
        form.focus("FileNameBox");
    }

    /// `CreateFileBtnClick()` of `TCreateRandDataFileDlg`.
    async fn create_rand_data_file_run(self: &Rc<Self>) {
        let Some(f) = self.tools.rand_data_file.borrow().clone() else { return };
        let parent = ViewportId::from_hash_of("CreateRandDataFile");
        let (name, size_text, unit) = (f.text("FileNameBox"), f.text("FileSizeBox"), f.item_index("SizeUnitList").unwrap_or(0));
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
            dialogs::ask(parent, &trl("Invalid file size."), MsgIcon::Error, MsgButtons::Ok, false).await;
            return;
        }
        if let Some(free) = pwtool_app::platform::sys::free_disk_space(&path) {
            if free < size {
                dialogs::ask(parent, &trl("Not enough free disk space available\nto create the file."), MsgIcon::Error, MsgButtons::Ok, false).await;
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
                let mut file = std::fs::File::create(&path2)?;
                let mut buf = Zeroizing::new(vec![0u8; if size >= 10_485_760 { 1_048_576 } else { 65_536 }]);
                let mut rest = size;
                while rest > 0 && !c2.load(Ordering::Relaxed) {
                    let n = rest.min(buf.len() as u64) as usize;
                    rng.get_data(&mut buf[..n]);
                    file.write_all(&buf[..n])?;
                    written += n as u64;
                    rest -= n as u64;
                    p2.store(written / step, Ordering::Relaxed);
                }
                file.flush()
            })();
            let _ = tx.send_blocking((rng, written, r));
        });
        f.set_enabled("CreateFileBtn", false);
        let mut progress_win: Option<Rc<ProgressWindow>> = None;
        let start = std::time::Instant::now();
        let fmt = trl("%1 of %2 %3 written.").replace("%3", step_name);
        let (rng, written, r) = loop {
            if let Ok(r) = rx.try_recv() {
                break r;
            }
            pwtool_app::rt::sleep(Duration::from_millis(20)).await;
            if progress_win.is_none() && start.elapsed().as_millis() >= 1000 {
                progress_win = Some(ProgressWindow::new(parent, &trl("Creating random data file ..."), ""));
            }
            if let Some(pw) = &progress_win {
                if pw.cancel.get() {
                    cancel.store(true, Ordering::Relaxed);
                }
                pw.set(progress.load(Ordering::Relaxed), size / step, &fmt);
            }
        };
        if let Some(pw) = progress_win {
            pw.close();
        }
        f.set_enabled("CreateFileBtn", true);
        if let GenRng::Det(g) = rng {
            *self.key_seeded_prng.borrow_mut() = Some(g);
        }
        let err = if cancel.load(Ordering::Relaxed) { Some(trl("Process was cancelled by the user")) } else { r.err().map(|e| e.to_string()) };
        match err {
            Some(e) => {
                let m = trl_format("Error while creating file\n\"%1\":\n%2.\n\n%3 bytes written.", &[&path.to_string_lossy(), &e, &written.to_string()]);
                dialogs::ask(parent, &m, MsgIcon::Error, MsgButtons::Ok, false).await;
            }
            None => {
                let fname = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let m = trl_format("File \"%1\" successfully created.\n\n%2 bytes written.", &[&fname, &written.to_string()]);
                dialogs::ask(parent, &m, MsgIcon::Info, MsgButtons::Ok, false).await;
            }
        }
        self.update_entropy_progress(false);
    }

    /// `MainMenu_Tools_CreateTrigramFileClick()`.
    fn create_trigram_file(self: &Rc<Self>) {
        let form = self.tools.trigram_file.borrow().clone();
        let form = match form {
            Some(f) => f,
            None => {
                let f = dialogs::load("CreateTrigramFile");
                let update = |f: &Form| f.set_enabled("CreateFileBtn", !f.text("SourceFileBox").is_empty() && !f.text("DestFileBox").is_empty());
                update(&f);
                for n in ["SourceFileBox", "DestFileBox"] {
                    let ff = Rc::downgrade(&f);
                    f.on_change(n, move |_| {
                        if let Some(f) = ff.upgrade() {
                            update(&f);
                        }
                    });
                }
                let ww = self.weak();
                f.on_click("CloseBtn", move |_| {
                    if let Some(w) = ww.upgrade() {
                        w.close_tool("CreateTrigramFile");
                    }
                });
                for (btn, edit, save) in [("BrowseBtn", "SourceFileBox", false), ("BrowseBtn2", "DestFileBox", true)] {
                    let ff = Rc::downgrade(&f);
                    f.on_click(btn, move |_| {
                        let ff = ff.clone();
                        spawn(async move {
                            let p = if save {
                                let t = format!("{} (*.tgm)", trl("Trigram files"));
                                let all = format!("{} (*.*)", trl("All files"));
                                crate::vcl::dialogs::save_file(&trl("Save trigram file"), None, None, &[(&t, &["tgm"]), (&all, &["*"])]).await.map(|mut p| {
                                    if p.extension().is_none() {
                                        p.set_extension("tgm");
                                    }
                                    p
                                })
                            } else {
                                crate::vcl::dialogs::open_file(&trl("Select source file"), None, &[]).await
                            };
                            if let (Some(p), Some(f)) = (p, ff.upgrade()) {
                                f.set_text(edit, &p.to_string_lossy());
                                f.push_event(crate::vcl::Event::Change(edit.into()));
                                f.focus(edit);
                            }
                        });
                    });
                }
                let ww = self.weak();
                f.on_click("CreateFileBtn", move |_| {
                    let Some(w) = ww.upgrade() else { return };
                    let Some(f) = w.tools.trigram_file.borrow().clone() else { return };
                    let data_dir = w.core.borrow().paths.data_dir.clone();
                    let abs = |s: &str| {
                        let p = PathBuf::from(s);
                        if p.parent().is_none_or(|x| x.as_os_str().is_empty()) { data_dir.join(p) } else { p }
                    };
                    let (sp, dp) = (abs(&f.text("SourceFileBox")), abs(&f.text("DestFileBox")));
                    let enc = w.core.borrow().file_encoding;
                    let (m, icon) = match PasswordGenerator::create_trigram_file(&sp, &dp, enc) {
                        Ok((n, bits)) => (
                            trl_format(
                                "Trigram file \"%1\" successfully created.\n\n%2 trigrams evaluated.\n%3 bits of entropy per letter.",
                                &[&dp.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), &n.to_string(), &format!("{bits:.2}")],
                            ),
                            MsgIcon::Info,
                        ),
                        Err(e) => (trl_format("Error while creating file\n\"%1\":\n%2.", &[&dp.to_string_lossy(), &trl(&e.to_string())]), MsgIcon::Error),
                    };
                    msg(ViewportId::from_hash_of("CreateTrigramFile"), &m, icon);
                });
                *self.tools.trigram_file.borrow_mut() = Some(f.clone());
                f
            }
        };
        self.show_tool("CreateTrigramFile", "CreateTrigramFile", false, &form, ROOT, false);
        form.focus("SourceFileBox");
    }

    /// `MainMenu_Help_AboutClick()`.
    fn about(self: &Rc<Self>) {
        let form = self.tools.about.borrow().clone();
        let form = match form {
            Some(f) => f,
            None => {
                let f = dialogs::load("About");
                f.set_caption(&trl_format("About %1", &[PROGRAM_NAME]));
                f.set_text("ProgramLbl", PROGRAM_NAME);
                f.set_text("VersionLbl", &format!("Version {PORT_VERSION} ({}-bit), based on {ORIGINAL_NAME} {PROGRAM_VERSION}", usize::BITS));
                let link = |url: &str, text: &str| format!("<a href=\"{url}\">{text}</a>");
                // the original author (without his e-mail address) and the port
                f.set_control_caption("AuthorLink", &format!("{PROGRAM_COPYRIGHT} {PROGRAM_AUTHOR}
{PORT_COPYRIGHT}"));
                let (x, y, _, _) = f.bounds("AuthorLink");
                let (_, www_y, _, _) = f.bounds("WWWLink");
                f.set_bounds("AuthorLink", x, y, 393, www_y - y - 2);
                let www = if PORT_REPOSITORY.is_empty() { PROGRAM_URL_ORIGINAL } else { PORT_REPOSITORY };
                f.set_control_caption("WWWLink", &link(www, www));
                f.set_control_caption("LicenseLink", &link("license", &trl("View license")));
                f.set_hint("LicenseLink", PROGRAM_LICENSEFILE);
                let (lang_name, translator) = {
                    let core = self.core.borrow();
                    let le = &core.languages[core.language];
                    if le.file_name.is_some() {
                        (le.name.clone(), pwtool_core::language::translator_name().unwrap_or_default())
                    } else {
                        (LANGUAGE_DEFAULT_NAME.to_string(), "(original language)".to_string())
                    }
                };
                f.set_text("LanguageInfoLbl", &trl_format("Current language: %1\nTranslator: %2", &[&lang_name, &translator]));
                if f.has("DonorLbl") {
                    f.set_text("DonorLbl", &trl("Community"));
                }
                if f.has("Logo") {
                    f.set_image("Logo", Some(include_bytes!("../../../../assets/about_logo.png").to_vec()));
                }
                let ww = self.weak();
                f.on_any(move |e| {
                    let Some(w) = ww.upgrade() else { return };
                    match e {
                        crate::vcl::Event::Link(_, url) if url == "license" => {
                            let file = w.core.borrow().paths.resources.join(PROGRAM_LICENSEFILE);
                            if file.exists() {
                                open_uri(&file.to_string_lossy());
                            } else {
                                open_uri(&format!("{PORT_REPOSITORY}/blob/main/LICENSE"));
                            }
                        }
                        crate::vcl::Event::Link(_, url) => open_uri(url),
                        crate::vcl::Event::Click(n) if n == "OKBtn" => windows::close(ViewportId::from_hash_of("About")),
                        _ => {}
                    }
                });
                *self.tools.about.borrow_mut() = Some(f.clone());
                f
            }
        };
        let win = Rc::new(dialogs::form_window("About", form, ROOT, true));
        windows::present(win);
    }

    // ------------------------------------------------------------------
    // update check

    /// `MainMenu_Help_CheckForUpdatesClick()` (manual) and the automatic
    /// check at start-up (`FormActivate()`).
    pub(super) async fn check_for_updates(self: &Rc<Self>, manual: bool) {
        use crate::ui::update_check::{CheckResult, check_for_updates};
        if self.tools.update_check_running.replace(true) {
            return;
        }
        self.f().set_menu_enabled("MainMenu_Help_CheckForUpdates", false);
        let r = check_for_updates(ROOT, manual).await;
        self.f().set_menu_enabled("MainMenu_Help_CheckForUpdates", true);
        self.tools.update_check_running.set(false);
        match r {
            CheckResult::NotConfigured => {
                if manual {
                    dialogs::ask(ROOT, &trl("The update check is not configured in this build."), MsgIcon::Info, MsgButtons::Ok, false).await;
                }
                return;
            }
            CheckResult::Negative if manual => {
                dialogs::ask(ROOT, &trl_format("Your version of %1 is up-to-date.", &[PROGRAM_NAME]), MsgIcon::Info, MsgButtons::Ok, false).await;
            }
            _ => {}
        }
        if matches!(r, CheckResult::Negative | CheckResult::Positive) {
            let d = jiff::Zoned::now().date();
            let s = pwtool_core::util::format_short_date(d.year() as i32, d.month() as u32, d.day() as u32);
            self.core.borrow_mut().ini.write_string("Main", "LastUpdateCheck", &s);
        }
    }

    /// Automatic check at start-up if due (`AutoCheckUpdates`).
    pub(super) fn auto_check_updates(self: &Rc<Self>) {
        use pwtool_app::app_config::AutoCheckUpdates;
        let (mode, last) = {
            let core = self.core.borrow();
            (core.config.auto_check_updates, core.ini.read_string("Main", "LastUpdateCheck", ""))
        };
        if mode == AutoCheckUpdates::Disabled {
            return;
        }
        let now = jiff::Zoned::now().date();
        let due = match pwtool_core::util::parse_short_date(&last).and_then(|(y, m, d)| jiff::civil::Date::new(y as i16, m as i8, d as i8).ok()) {
            None => true,
            Some(l) => {
                let days = now.since(l).map(|s| s.get_days() as i64).unwrap_or(0);
                match mode {
                    AutoCheckUpdates::Daily => days >= 1,
                    AutoCheckUpdates::Weekly => days >= 7,
                    _ => {
                        let months = (now.year() as i32 - l.year() as i32) * 12 + now.month() as i32 - l.month() as i32 - i32::from(now.day() < l.day());
                        months >= 1
                    }
                }
            }
        };
        if due {
            let w = self.clone();
            spawn(async move { w.check_for_updates(false).await });
        }
    }
}
