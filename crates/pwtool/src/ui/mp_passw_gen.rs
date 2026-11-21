//! MP Password Generator window (`TMPPasswGenForm`, `upstream/src/main/
//! MPPasswGen.cpp`): passwords derived from a master password and a
//! parameter. Child module of `main_window`.

use pwtool_core::mp_passw_gen::{MPPG_CHARSETS, PASSW_DEFAULT_LEN, PASSW_MAX_CHARS, PasswHashType, generate as mp_generate, generator_key, passw_checksum};
use pwtool_core::random::fast::fast_fill;

use crate::ui::passw_enter::*;
use super::*;

const CONFIG_ID: &str = "MPPasswGen";
const WINDOW_ID: &str = "MPPasswGen";

/// The key, XOR-masked with a random pad (`memcrypt` in the original).
struct MaskedKey {
    masked: Zeroizing<Vec<u8>>,
    pad: Zeroizing<Vec<u8>>,
}

impl MaskedKey {
    fn new(key: &[u8]) -> Self {
        let mut pad = Zeroizing::new(vec![0u8; key.len()]);
        fast_fill(&mut pad);
        let masked = Zeroizing::new(key.iter().zip(pad.iter()).map(|(a, b)| a ^ b).collect());
        MaskedKey { masked, pad }
    }

    fn plain(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.masked.iter().zip(self.pad.iter()).map(|(a, b)| a ^ b).collect())
    }
}

pub(super) struct MpPasswGenWin {
    form: Rc<Form>,
    key: RefCell<Option<MaskedKey>>,
    countdown: Cell<i64>,
    timer_gen: Cell<u64>,
    bar: QualityBar,
    main: Weak<MainWindow>,
}

fn win_id() -> ViewportId {
    ViewportId::from_hash_of(WINDOW_ID)
}

impl MpPasswGenWin {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let form = dialogs::load("MPPasswGen");
        let bar = QualityBar::new(128);
        let b = bar.clone();
        form.set_painter("PasswGauge", move |ui, rect| b.paint(ui, rect));
        let d = Rc::new(MpPasswGenWin { form, key: RefCell::new(None), countdown: Cell::new(0), timer_gen: Cell::new(0), bar, main: Rc::downgrade(main) });
        let f = &d.form;
        let charsets: Vec<String> = vec![
            format!("1: {} (A-Z, a-z, 0-9)", trl("Alphanumeric")),
            format!("2: (1) {}", trl("without ambiguous characters")),
            format!("3: (1) + {} (!\"#$%...)", trl("special symbols")),
            format!("4: (3) {}", trl("without ambiguous characters")),
            format!("5: {} (0-9, a-f)", trl("Hexadecimal")),
            format!("6: {} (0-9, A-F)", trl("Hexadecimal")),
        ];
        f.set_items("CharSetList", charsets);
        let hashes: Vec<String> = f.items("PasswHashList").iter().map(|s| trl(s)).collect();
        f.set_items("PasswHashList", hashes);
        f.set_range("PasswLengthSpinBtn", 1, PASSW_MAX_CHARS as i64);
        f.set_secret("PasswBox");
        f.set_mono("PasswBox", true);
        f.set_font_family("PasswBox", Some(crate::vcl::fonts::PASSW));
        f.set_read_only("PasswStatusBox", true);
        f.set_popup_menu("PasswBox", "PasswBoxMenu");
        let hint = main.f().hint("PasswInfoLbl");
        if !hint.is_empty() {
            f.set_hint("PasswInfoLbl", &hint);
        }
        d.load_config(&main.core.borrow().ini);
        d.clear_key(false, false);
        f.set_text("PasswStatusBox", "");
        d.connect();
        d
    }

    fn f(&self) -> &Form {
        &self.form
    }

    fn load_config(&self, ini: &pwtool_core::config::IniFile) {
        let f = self.f();
        f.set_checked("ConfirmPasswCheck", ini.read_bool(CONFIG_ID, "ConfirmPassw", false));
        f.set_checked("ShowPasswHashCheck", ini.read_bool(CONFIG_ID, "ShowPasswHash", true));
        let h = ini.read_int(CONFIG_ID, "PasswHashType", PasswHashType::Hex16Bit as i32);
        f.set_item_index("PasswHashList", Some(if (0..=3).contains(&h) { h as usize } else { 2 }));
        self.show_passw_hash_check();
        f.set_checked("KeyExpiryCheck", ini.read_bool(CONFIG_ID, "KeyExpiry", true));
        self.key_expiry_check();
        f.set_position("KeyExpiryTimeSpinBtn", ini.read_integer(CONFIG_ID, "KeyExpiryTime", 300));
        f.set_checked("HashapassCompatCheck", ini.read_bool(CONFIG_ID, "HashapassCompatible", false));
        self.hashapass_compat_check();
        f.set_checked("AddPasswLenToParamCheck", ini.read_bool(CONFIG_ID, "AddPasswLenToParam", false));
        f.set_text("AutotypeBox", &ini.read_string(CONFIG_ID, "AutotypeSequence", "{parameter}{tab}{password}{enter}"));
        let mut cs = ini.read_int(CONFIG_ID, "CharSetListIdx", 0);
        if !(0..MPPG_CHARSETS.len() as i32).contains(&cs) {
            cs = 0;
        }
        f.set_item_index("CharSetList", Some(cs as usize));
        self.char_set_list_change();
        f.set_position("PasswLengthSpinBtn", ini.read_integer(CONFIG_ID, "PasswLength", PASSW_DEFAULT_LEN as i64));
        f.set_checked("TogglePasswBtn", ini.read_bool(CONFIG_ID, "HidePassw", false));
        self.toggle_passw();
    }

    pub(super) fn save_config(&self, ini: &mut pwtool_core::config::IniFile) {
        let f = self.f();
        ini.write_bool(CONFIG_ID, "ConfirmPassw", f.checked("ConfirmPasswCheck"));
        ini.write_bool(CONFIG_ID, "ShowPasswHash", f.checked("ShowPasswHashCheck"));
        ini.write_integer(CONFIG_ID, "PasswHashType", f.item_index("PasswHashList").unwrap_or(2) as i64);
        ini.write_bool(CONFIG_ID, "KeyExpiry", f.checked("KeyExpiryCheck"));
        ini.write_integer(CONFIG_ID, "KeyExpiryTime", f.position("KeyExpiryTimeSpinBtn"));
        ini.write_bool(CONFIG_ID, "HashapassCompatible", f.checked("HashapassCompatCheck"));
        ini.write_bool(CONFIG_ID, "AddPasswLenToParam", f.checked("AddPasswLenToParamCheck"));
        ini.write_string(CONFIG_ID, "AutotypeSequence", &f.text("AutotypeBox"));
        ini.write_integer(CONFIG_ID, "CharSetListIdx", f.item_index("CharSetList").unwrap_or(0) as i64);
        ini.write_integer(CONFIG_ID, "PasswLength", f.position("PasswLengthSpinBtn"));
        ini.write_bool(CONFIG_ID, "HidePassw", f.checked("TogglePasswBtn"));
    }

    fn connect(self: &Rc<Self>) {
        let dd = Rc::downgrade(self);
        self.form.on_any(move |e| {
            let Some(d) = dd.upgrade() else { return };
            use crate::vcl::Event::*;
            match e {
                Click(n) => match n.as_str() {
                    "EnterPasswBtn" => {
                        let d = d.clone();
                        spawn(async move { d.enter_passw().await });
                    }
                    "ClearKeyBtn" => d.clear_key(false, false),
                    "GenerateBtn" => d.generate(),
                    "ClearParameterBtn" => {
                        d.f().set_text("ParameterBox", "");
                        d.f().focus("ParameterBox");
                    }
                    "UseAsDefaultRNGBtn" => d.use_as_default_rng(),
                    "CloseBtn" => d.close(),
                    "TogglePasswBtn" => d.toggle_passw(),
                    "KeyExpiryCheck" => {
                        d.key_expiry_check();
                        if d.f().checked("KeyExpiryCheck") {
                            d.f().focus("KeyExpiryTimeBox");
                        }
                    }
                    "ShowPasswHashCheck" => d.show_passw_hash_check(),
                    "HashapassCompatCheck" => d.hashapass_compat_check(),
                    _ => {}
                },
                Change(n) if n == "CharSetList" => d.char_set_list_change(),
                Enter(n) if n == "ParameterBox" => d.generate(),
                Menu(n) => d.menu(n),
                _ => {}
            }
        });
    }

    fn close(&self) {
        if self.key.borrow().is_some() {
            self.clear_key(false, false);
        }
        if let Some(m) = self.main.upgrade() {
            m.close_tool(WINDOW_ID);
        }
    }

    /// Enabled state of the password box menu (`PasswBoxMenuPopup()`), every frame.
    pub(super) fn update_menu(&self) {
        let f = self.f();
        let selected = f.selection("PasswBox").is_some_and(|(a, b)| a != b);
        let has_text = !f.secret_text("PasswBox").is_empty();
        for (name, en) in [
            ("PasswBoxMenu_Undo", false),
            ("PasswBoxMenu_Cut", false),
            ("PasswBoxMenu_Copy", selected),
            ("PasswBoxMenu_EncryptCopy", selected),
            ("PasswBoxMenu_Paste", false),
            ("PasswBoxMenu_Delete", false),
            ("PasswBoxMenu_PerformAutotype", has_text),
            ("PasswBoxMenu_AddToDatabase", has_text && self.main.upgrade().is_some_and(|m| m.db_open())),
            ("PasswBoxMenu_SelectAll", has_text),
        ] {
            if f.has_menu_item(name) {
                f.set_menu_enabled(name, en);
            }
        }
    }

    fn menu(self: &Rc<Self>, name: &str) {
        let f = self.f();
        match name {
            "PasswBoxMenu_Copy" => {
                let t = f.selected_text("PasswBox");
                if !t.is_empty() {
                    clipboard::set_data(&t);
                }
            }
            "PasswBoxMenu_EncryptCopy" => {
                let t = f.selected_text("PasswBox");
                if let (false, Some(m)) = (t.is_empty(), self.main.upgrade()) {
                    m.crypt_text_spawn(true, Some(t), win_id());
                }
            }
            "PasswBoxMenu_PerformAutotype" => {
                let passw = f.secret_text("PasswBox");
                if passw.is_empty() {
                    return;
                }
                let param = Zeroizing::new(f.text("ParameterBox"));
                let seq = f.text("AutotypeBox");
                if let Some(m) = self.main.upgrade() {
                    m.perform_autotype(&seq, Some(&param), Some(&passw));
                }
            }
            "PasswBoxMenu_AddToDatabase" => {
                let passw = f.secret_text("PasswBox");
                let param = f.text("ParameterBox");
                if let Some(m) = self.main.upgrade() {
                    m.add_passw_to_db(&passw, false, Some(&param));
                }
            }
            "PasswBoxMenu_SelectAll" => f.select_all("PasswBox"),
            _ => {}
        }
    }

    fn toggle_passw(&self) {
        let hide = self.f().checked("TogglePasswBtn");
        self.f().set_password_char("PasswBox", hide);
    }

    fn key_expiry_check(&self) {
        let on = self.f().checked("KeyExpiryCheck");
        self.f().set_enabled("KeyExpiryTimeBox", on);
        self.f().set_enabled("KeyExpiryTimeSpinBtn", on);
    }

    fn show_passw_hash_check(&self) {
        self.f().set_enabled("PasswHashList", self.f().checked("ShowPasswHashCheck"));
    }

    fn hashapass_compat_check(&self) {
        let on = !self.f().checked("HashapassCompatCheck");
        for n in ["CharSetLbl", "CharSetList", "CharSetInfoLbl", "LengthLbl", "PasswLengthBox", "PasswLengthSpinBtn", "AddPasswLenToParamCheck"] {
            self.f().set_enabled(n, on);
        }
    }

    fn char_set_list_change(&self) {
        let i = self.f().item_index("CharSetList").unwrap_or(0);
        let n = MPPG_CHARSETS[i.min(MPPG_CHARSETS.len() - 1)].chars().count();
        let bits = pwtool_core::math::delphi_log2(n as f64);
        self.f().set_text("CharSetInfoLbl", &trl_format("%1 ch. / %2 bits per ch.", &[&n.to_string(), &format_float_1(bits)]));
    }

    fn now() -> String {
        jiff::Zoned::now().strftime("%H:%M:%S").to_string()
    }

    /// `ClearKey()`.
    fn clear_key(&self, expired: bool, key_only: bool) {
        let f = self.f();
        self.key.borrow_mut().take();
        if !key_only {
            f.set_text("PasswBox", "");
            f.set_text("ParameterBox", "");
            f.set_visible("PasswGauge", false);
            f.set_visible("PasswInfoLbl", false);
        }
        self.timer_gen.set(self.timer_gen.get() + 1);
        self.countdown.set(0);
        f.set_text("KeyExpiryCountdownLbl", "");
        f.set_enabled("GenerateBtn", false);
        f.set_enabled("UseAsDefaultRNGBtn", false);
        let info = format!("{} {}", Self::now(), trl(if expired { "Key has expired." } else { "Key has been cleared." }));
        f.set_text("PasswStatusBox", &info);
        f.set_enabled("HashapassCompatCheck", true);
        f.set_enabled("ClearKeyBtn", key_only);
    }

    /// `SetKeyExpiry()`.
    fn set_key_expiry(self: &Rc<Self>, setup: bool) {
        let f = self.f();
        let expire = f.checked("KeyExpiryCheck");
        let t = f.position("KeyExpiryTimeSpinBtn");
        if expire && t == 0 && !setup {
            self.clear_key(true, true);
        } else if expire && t != 0 {
            self.countdown.set(t);
            f.set_text("KeyExpiryCountdownLbl", &t.to_string());
            let tgen = self.timer_gen.get() + 1;
            self.timer_gen.set(tgen);
            let dd = Rc::downgrade(self);
            spawn(async move {
                loop {
                    pwtool_app::rt::sleep(Duration::from_secs(1)).await;
                    let Some(d) = dd.upgrade() else { return };
                    if d.timer_gen.get() != tgen {
                        return;
                    }
                    let c = d.countdown.get() - 1;
                    d.countdown.set(c);
                    d.f().set_text("KeyExpiryCountdownLbl", &c.to_string());
                    if c <= 0 {
                        d.clear_key(true, false);
                        return;
                    }
                }
            });
        } else {
            self.timer_gen.set(self.timer_gen.get() + 1);
            f.set_text("KeyExpiryCountdownLbl", "");
        }
    }

    async fn enter_passw(self: &Rc<Self>) {
        let Some(main) = self.main.upgrade() else { return };
        let dlg = main.passw_enter();
        let mut flags = PASSWENTER_FLAG_PASSWQUALITY;
        if self.f().checked("ConfirmPasswCheck") {
            flags |= PASSWENTER_FLAG_CONFIRMPASSW;
        }
        let ok = dlg.execute(flags, &trl("Master password"), win_id()).await == EnterResult::Ok;
        let passw = if ok { Some(dlg.get_passw(0)) } else { None };
        dlg.clear_and_flush();
        let Some(passw) = passw else { return };
        let f = self.f();
        let mut info = format!("{} {}", Self::now(), trl("New key generated."));
        let hashapass = f.checked("HashapassCompatCheck");
        *self.key.borrow_mut() = Some(MaskedKey::new(&generator_key(&passw, hashapass)));
        if f.checked("ShowPasswHashCheck") {
            let ht = PasswHashType::from_index(f.item_index("PasswHashList").unwrap_or(2) as i32);
            info.push_str(" Hash: ");
            info.push_str(&passw_checksum(&passw, ht));
        }
        f.set_text("PasswStatusBox", &info);
        f.set_enabled("ClearKeyBtn", true);
        f.set_enabled("GenerateBtn", true);
        f.set_enabled("HashapassCompatCheck", false);
        f.set_enabled("UseAsDefaultRNGBtn", !hashapass);
        self.set_key_expiry(true);
        f.focus("ParameterBox");
    }

    fn generate(self: &Rc<Self>) {
        let Some(key) = self.key.borrow().as_ref().map(|k| k.plain()) else { return };
        let f = self.f();
        let param = Zeroizing::new(f.text("ParameterBox"));
        let hashapass = f.checked("HashapassCompatCheck");
        let len = f.position("PasswLengthSpinBtn") as i32;
        let cs = f.item_index("CharSetList").unwrap_or(0);
        let (passw, bits) = mp_generate(&key, &param, cs.min(MPPG_CHARSETS.len() - 1), len, f.checked("AddPasswLenToParamCheck"), hashapass);
        let n = passw.chars().count();
        f.set_text("PasswBox", &passw);
        f.set_visible("PasswGauge", true);
        f.set_visible("PasswInfoLbl", true);
        self.bar.set_quality(bits);
        f.set_text("PasswInfoLbl", &format!("{} / {}", format_float_1(bits.min(256.0)), trl_format("%1 ch.", &[&n.to_string()])));
        self.set_key_expiry(false);
        f.focus("PasswBox");
    }

    fn use_as_default_rng(self: &Rc<Self>) {
        let Some(key) = self.key.borrow().as_ref().map(|k| k.plain()) else { return };
        let param = self.f().text("ParameterBox");
        let param_bytes: Zeroizing<Vec<u8>> = Zeroizing::new(param.encode_utf16().flat_map(u16::to_le_bytes).collect());
        if let Some(m) = self.main.upgrade() {
            m.use_key_seeded_prng_with(AesCtrPrng::from_key_and_param(&key, &param_bytes));
        }
        self.set_key_expiry(false);
    }
}

impl MainWindow {
    /// `MPPasswGenBtnClick()`.
    pub(super) fn mp_passw_gen(self: &Rc<Self>) {
        let existing = self.tools.mp_passw_gen.borrow().clone();
        let d = match existing {
            Some(d) => d,
            None => {
                let d = MpPasswGenWin::new(self);
                *self.tools.mp_passw_gen.borrow_mut() = Some(d.clone());
                d
            }
        };
        let win = self.show_tool(WINDOW_ID, CONFIG_ID, true, &d.form, ROOT, false);
        let w = self.weak();
        let wid = Rc::downgrade(&win);
        let dd = Rc::downgrade(&d);
        *win.on_close.borrow_mut() = Some(Box::new(move || {
            if let (Some(w), Some(win)) = (w.upgrade(), wid.upgrade()) {
                w.remember_size(WINDOW_ID, &win);
            }
            if let Some(d) = dd.upgrade() {
                if d.key.borrow().is_some() {
                    d.clear_key(false, false);
                }
            }
            true
        }));
        if d.key.borrow().is_none() {
            d.form.focus("EnterPasswBtn");
        } else {
            d.form.focus("ParameterBox");
        }
    }

    /// Every frame: menus of the tool windows.
    pub(super) fn update_tool_menus(&self) {
        if let Some(d) = self.tools.mp_passw_gen.borrow().as_ref() {
            d.update_menu();
        }
        self.update_list_menu();
    }
}
