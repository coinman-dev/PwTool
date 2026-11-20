//! MP Password Generator window (`TMPPasswGenForm`, `upstream/src/main/
//! MPPasswGen.cpp`): passwords derived from a master password and a
//! parameter. Child module of `main_window`.

#![allow(deprecated)]

use pwtool_core::mp_passw_gen::{
    MPPG_CHARSETS, PASSW_DEFAULT_LEN, PASSW_MAX_CHARS, PasswHashType, generate as mp_generate, generator_key,
    passw_checksum,
};
use pwtool_core::random::fast::fast_fill;

use super::*;
use crate::ui::passw_enter::*;

const CONFIG_ID: &str = "MPPasswGen";
const PASSWORD_CHAR: char = '*';

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
    b: Built,
    win: gtk::Window,
    key: RefCell<Option<MaskedKey>>,
    countdown: Cell<i64>,
    timer: RefCell<Option<glib::SourceId>>,
    bar: QualityBar,
    main: std::rc::Weak<MainWindow>,
}

impl MpPasswGenWin {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let b = build_form(load_form("MPPasswGen"), "MPPasswGen");
        let win = super::super::dialogs::form_window(&b, None, false);
        win.set_hide_on_close(true);
        if let Some(app) = main.window.application() {
            win.set_application(Some(&app));
        }
        let bar = QualityBar::new(b.get("PasswGauge"), 128);
        let d = Rc::new(MpPasswGenWin {
            b,
            win,
            key: RefCell::new(None),
            countdown: Cell::new(0),
            timer: RefCell::new(None),
            bar,
            main: Rc::downgrade(main),
        });
        let charsets: Vec<String> = vec![
            format!("1: {} (A-Z, a-z, 0-9)", trl("Alphanumeric")),
            format!("2: (1) {}", trl("without ambiguous characters")),
            format!("3: (1) + {} (!\"#$%...)", trl("special symbols")),
            format!("4: (3) {}", trl("without ambiguous characters")),
            format!("5: {} (0-9, a-f)", trl("Hexadecimal")),
            format!("6: {} (0-9, A-F)", trl("Hexadecimal")),
        ];
        set_combo_items(&d.combo("CharSetList"), &charsets);
        set_combo_items(
            &d.combo("PasswHashList"),
            &d.b.obj("PasswHashList").map(|o| o.strings("Items.Strings")).unwrap_or_default().iter().map(|x| trl(x)).collect::<Vec<_>>(),
        );
        d.b.updowns["PasswLengthSpinBtn"].set_max(PASSW_MAX_CHARS as i64);
        d.b.updowns["PasswLengthSpinBtn"].set_min(1);
        let pb = d.entry("PasswBox");
        pb.set_buffer(&gtk::PasswordEntryBuffer::new().upcast::<gtk::EntryBuffer>());
        pb.set_invisible_char(Some(PASSWORD_CHAR));
        super::super::builder::add_css(&pb, "font-family: monospace;");
        pb.add_css_class(super::tools::PASSW_FONT_CLASS);
        d.entry("PasswStatusBox").set_editable(false);
        if let Some(t) = main.b.try_get::<gtk::Label>("PasswInfoLbl").and_then(|l| l.tooltip_text()) {
            d.b.w("PasswInfoLbl").set_tooltip_text(Some(&t));
        }
        d.load_config(&main.core.borrow().ini);
        d.connect();
        for name in ["PasswInfoLbl", "PasswGauge"] {
            let dd = Rc::downgrade(&d);
            crate::ui::util::add_text_drag_source(&d.b.w(name), move || {
                dd.upgrade().map(|d| Zeroizing::new(d.entry("PasswBox").text().to_string())).unwrap_or_default()
            });
        }
        let dd = Rc::downgrade(&d);
        crate::ui::util::add_text_drag_source(&d.b.w("ParameterLbl"), move || {
            dd.upgrade().map(|d| Zeroizing::new(d.entry("ParameterBox").text().to_string())).unwrap_or_default()
        });
        main.attach_entropy_events(&d.win);
        d
    }

    fn entry(&self, n: &str) -> gtk::Entry {
        self.b.get(n)
    }

    fn check(&self, n: &str) -> gtk::CheckButton {
        self.b.get(n)
    }

    fn combo(&self, n: &str) -> gtk::ComboBoxText {
        self.b.get(n)
    }

    fn load_config(&self, ini: &pwtool_core::config::IniFile) {
        self.check("ConfirmPasswCheck").set_active(ini.read_bool(CONFIG_ID, "ConfirmPassw", false));
        self.check("ShowPasswHashCheck").set_active(ini.read_bool(CONFIG_ID, "ShowPasswHash", true));
        let h = ini.read_int(CONFIG_ID, "PasswHashType", PasswHashType::Hex16Bit as i32);
        self.combo("PasswHashList").set_active(Some(if (0..=3).contains(&h) { h as u32 } else { 2 }));
        self.show_passw_hash_check();
        self.check("KeyExpiryCheck").set_active(ini.read_bool(CONFIG_ID, "KeyExpiry", true));
        self.key_expiry_check();
        self.b.updowns["KeyExpiryTimeSpinBtn"].set_position(ini.read_integer(CONFIG_ID, "KeyExpiryTime", 300));
        self.check("HashapassCompatCheck").set_active(ini.read_bool(CONFIG_ID, "HashapassCompatible", false));
        self.hashapass_compat_check();
        self.check("AddPasswLenToParamCheck").set_active(ini.read_bool(CONFIG_ID, "AddPasswLenToParam", false));
        self.entry("AutotypeBox")
            .set_text(&ini.read_string(CONFIG_ID, "AutotypeSequence", "{parameter}{tab}{password}{enter}"));
        let mut cs = ini.read_int(CONFIG_ID, "CharSetListIdx", 0);
        if !(0..MPPG_CHARSETS.len() as i32).contains(&cs) {
            cs = 0;
        }
        self.combo("CharSetList").set_active(Some(cs as u32));
        self.char_set_list_change();
        self.b.updowns["PasswLengthSpinBtn"].set_position(ini.read_integer(CONFIG_ID, "PasswLength", PASSW_DEFAULT_LEN as i64));
        self.b.get::<gtk::ToggleButton>("TogglePasswBtn").set_active(ini.read_bool(CONFIG_ID, "HidePassw", false));
        self.toggle_passw();
    }

    fn save_config(&self, ini: &mut pwtool_core::config::IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.default_height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.default_width() as i64);
        ini.write_bool(CONFIG_ID, "ConfirmPassw", self.check("ConfirmPasswCheck").is_active());
        ini.write_bool(CONFIG_ID, "ShowPasswHash", self.check("ShowPasswHashCheck").is_active());
        ini.write_integer(CONFIG_ID, "PasswHashType", self.combo("PasswHashList").active().unwrap_or(2) as i64);
        ini.write_bool(CONFIG_ID, "KeyExpiry", self.check("KeyExpiryCheck").is_active());
        ini.write_integer(CONFIG_ID, "KeyExpiryTime", self.b.updowns["KeyExpiryTimeSpinBtn"].position());
        ini.write_bool(CONFIG_ID, "HashapassCompatible", self.check("HashapassCompatCheck").is_active());
        ini.write_bool(CONFIG_ID, "AddPasswLenToParam", self.check("AddPasswLenToParamCheck").is_active());
        ini.write_string(CONFIG_ID, "AutotypeSequence", &self.entry("AutotypeBox").text());
        ini.write_integer(CONFIG_ID, "CharSetListIdx", self.combo("CharSetList").active().unwrap_or(0) as i64);
        ini.write_integer(CONFIG_ID, "PasswLength", self.b.updowns["PasswLengthSpinBtn"].position());
        ini.write_bool(CONFIG_ID, "HidePassw", self.b.get::<gtk::ToggleButton>("TogglePasswBtn").is_active());
    }

    fn connect(self: &Rc<Self>) {
        macro_rules! on_click {
            ($name:expr, $f:expr) => {{
                let d = Rc::downgrade(self);
                self.b.get::<gtk::Button>($name).connect_clicked(move |_| {
                    if let Some(d) = d.upgrade() {
                        #[allow(clippy::redundant_closure_call)]
                        ($f)(&d);
                    }
                });
            }};
        }
        on_click!("EnterPasswBtn", |d: &Rc<Self>| {
            let d = d.clone();
            glib::spawn_future_local(async move { d.enter_passw().await });
        });
        on_click!("ClearKeyBtn", |d: &Rc<Self>| d.clear_key(false, false));
        on_click!("GenerateBtn", |d: &Rc<Self>| d.generate());
        on_click!("ClearParameterBtn", |d: &Rc<Self>| {
            d.entry("ParameterBox").set_text("");
            d.entry("ParameterBox").grab_focus();
        });
        on_click!("UseAsDefaultRNGBtn", |d: &Rc<Self>| d.use_as_default_rng());
        on_click!("CloseBtn", |d: &Rc<Self>| d.win.close());
        let d = Rc::downgrade(self);
        self.b.get::<gtk::ToggleButton>("TogglePasswBtn").connect_toggled(move |_| {
            if let Some(d) = d.upgrade() {
                d.toggle_passw();
            }
        });
        let d = Rc::downgrade(self);
        self.check("KeyExpiryCheck").connect_toggled(move |_| {
            if let Some(d) = d.upgrade() {
                d.key_expiry_check();
                if d.check("KeyExpiryCheck").is_active() {
                    d.entry("KeyExpiryTimeBox").grab_focus();
                }
            }
        });
        let d = Rc::downgrade(self);
        self.check("ShowPasswHashCheck").connect_toggled(move |_| {
            if let Some(d) = d.upgrade() {
                d.show_passw_hash_check();
            }
        });
        let d = Rc::downgrade(self);
        self.check("HashapassCompatCheck").connect_toggled(move |_| {
            if let Some(d) = d.upgrade() {
                d.hashapass_compat_check();
            }
        });
        let d = Rc::downgrade(self);
        self.combo("CharSetList").connect_changed(move |_| {
            if let Some(d) = d.upgrade() {
                d.char_set_list_change();
            }
        });
        let d = Rc::downgrade(self);
        self.entry("ParameterBox").connect_activate(move |_| {
            if let Some(d) = d.upgrade() {
                d.generate();
            }
        });
        let d = Rc::downgrade(self);
        self.win.connect_close_request(move |_| {
            if let Some(d) = d.upgrade() {
                if d.key.borrow().is_some() {
                    d.clear_key(false, false);
                }
            }
            glib::Propagation::Proceed
        });
        // password box menu
        if let Some(m) = self.b.menus.get("PasswBoxMenu") {
            let pop = super::super::builder::attach_context_menu_capture(&self.entry("PasswBox"), m);
            let d = Rc::downgrade(self);
            pop.connect_show(move |_| {
                if let Some(d) = d.upgrade() {
                    let e = d.entry("PasswBox");
                    let editable = e.is_editable();
                    let selected = e.selection_bounds().is_some();
                    let has_text = e.text_length() > 0;
                    let db_open = d
                        .main
                        .upgrade()
                        .and_then(|m| m.passw_mng.borrow().as_ref().map(|pm| pm.is_db_open()))
                        .unwrap_or(false);
                    for (name, en) in [
                        ("PasswBoxMenu_Undo", editable),
                        ("PasswBoxMenu_Cut", editable && selected),
                        ("PasswBoxMenu_Copy", selected),
                        ("PasswBoxMenu_EncryptCopy", selected),
                        ("PasswBoxMenu_Paste", editable),
                        ("PasswBoxMenu_Delete", editable && selected),
                        ("PasswBoxMenu_PerformAutotype", has_text),
                        ("PasswBoxMenu_AddToDatabase", has_text && db_open),
                        ("PasswBoxMenu_SelectAll", has_text),
                    ] {
                        if d.b.has_action(name) {
                            d.b.action(name).set_enabled(en);
                        }
                    }
                }
            });
        }
        let acts: [(&str, fn(&Rc<Self>)); 5] = [
            ("PasswBoxMenu_Copy", |d| {
                let e = d.entry("PasswBox");
                if let Some((s, end)) = e.selection_bounds() {
                    super::super::clipboard::set_data(&Zeroizing::new(e.chars(s, end).to_string()));
                }
            }),
            ("PasswBoxMenu_EncryptCopy", |d| {
                let e = d.entry("PasswBox");
                let text = match e.selection_bounds() {
                    Some((s, end)) => Zeroizing::new(e.chars(s, end).to_string()),
                    None => return,
                };
                if let Some(m) = d.main.upgrade() {
                    let w = d.win.clone();
                    glib::spawn_future_local(async move { m.crypt_text(true, Some(text), Some(w)).await });
                }
            }),
            ("PasswBoxMenu_PerformAutotype", |d| {
                let passw = Zeroizing::new(d.entry("PasswBox").text().to_string());
                if passw.is_empty() {
                    return;
                }
                let param = Zeroizing::new(d.entry("ParameterBox").text().to_string());
                let seq = d.entry("AutotypeBox").text().to_string();
                if let Some(m) = d.main.upgrade() {
                    m.perform_autotype(&d.win, &seq, None, None, Some(&param), Some(&passw));
                }
            }),
            ("PasswBoxMenu_AddToDatabase", |d| {
                let passw = Zeroizing::new(d.entry("PasswBox").text().to_string());
                let param = d.entry("ParameterBox").text().to_string();
                if let Some(m) = d.main.upgrade() {
                    m.add_passw_to_db(&passw, false, Some(&param));
                }
            }),
            ("PasswBoxMenu_SelectAll", |d| d.entry("PasswBox").select_region(0, -1)),
        ];
        for (name, f) in acts {
            if self.b.has_action(name) {
                let d = Rc::downgrade(self);
                self.b.action(name).connect_activate(move |_, _| {
                    if let Some(d) = d.upgrade() {
                        f(&d);
                    }
                });
            }
        }
        // standard edit actions are provided by the entry's own menu
        for name in ["PasswBoxMenu_Undo", "PasswBoxMenu_Cut", "PasswBoxMenu_Paste", "PasswBoxMenu_Delete"] {
            if self.b.has_action(name) {
                let d = Rc::downgrade(self);
                let n = name.to_string();
                self.b.action(name).connect_activate(move |_, _| {
                    if let Some(d) = d.upgrade() {
                        let e = d.entry("PasswBox");
                        match n.as_str() {
                            "PasswBoxMenu_Undo" => {
                                let _ = e.activate_action("text.undo", None);
                            }
                            "PasswBoxMenu_Cut" => {
                                if let Some((s, end)) = e.selection_bounds() {
                                    super::super::clipboard::set_data(&Zeroizing::new(e.chars(s, end).to_string()));
                                    e.delete_selection();
                                }
                            }
                            "PasswBoxMenu_Paste" => {
                                let _ = e.activate_action("clipboard.paste", None);
                            }
                            _ => e.delete_selection(),
                        }
                    }
                });
            }
        }
    }

    fn toggle_passw(&self) {
        let hide = self.b.get::<gtk::ToggleButton>("TogglePasswBtn").is_active();
        self.entry("PasswBox").set_visibility(!hide);
    }

    fn key_expiry_check(&self) {
        let on = self.check("KeyExpiryCheck").is_active();
        self.b.w("KeyExpiryTimeBox").set_sensitive(on);
        self.b.updowns["KeyExpiryTimeSpinBtn"].set_sensitive(on);
    }

    fn show_passw_hash_check(&self) {
        self.b.w("PasswHashList").set_sensitive(self.check("ShowPasswHashCheck").is_active());
    }

    fn hashapass_compat_check(&self) {
        let on = !self.check("HashapassCompatCheck").is_active();
        for n in ["CharSetLbl", "CharSetList", "CharSetInfoLbl", "LengthLbl", "PasswLengthBox", "AddPasswLenToParamCheck"] {
            self.b.w(n).set_sensitive(on);
        }
        self.b.updowns["PasswLengthSpinBtn"].set_sensitive(on);
    }

    fn char_set_list_change(&self) {
        let i = self.combo("CharSetList").active().unwrap_or(0) as usize;
        let n = MPPG_CHARSETS[i.min(MPPG_CHARSETS.len() - 1)].chars().count();
        let bits = pwtool_core::math::delphi_log2(n as f64);
        self.b
            .get::<gtk::Label>("CharSetInfoLbl")
            .set_text(&trl_format("%1 ch. / %2 bits per ch.", &[&n.to_string(), &format_float_1(bits)]));
    }

    fn now() -> String {
        glib::DateTime::now_local().ok().and_then(|t| t.format("%X").ok()).map(|s| s.to_string()).unwrap_or_default()
    }

    /// `ClearKey()`.
    fn clear_key(&self, expired: bool, key_only: bool) {
        self.key.borrow_mut().take();
        if !key_only {
            self.entry("PasswBox").set_text("");
            self.entry("ParameterBox").set_text("");
            self.bar.area.set_visible(false);
            self.b.w("PasswInfoLbl").set_visible(false);
        }
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
        self.countdown.set(0);
        self.b.get::<gtk::Label>("KeyExpiryCountdownLbl").set_text("");
        self.b.w("GenerateBtn").set_sensitive(false);
        self.b.w("UseAsDefaultRNGBtn").set_sensitive(false);
        let info = format!(
            "{} {}",
            Self::now(),
            trl(if expired { "Key has expired." } else { "Key has been cleared." })
        );
        self.entry("PasswStatusBox").set_text(&info);
        self.check("HashapassCompatCheck").set_sensitive(true);
        self.b.w("ClearKeyBtn").set_sensitive(key_only);
    }

    /// `SetKeyExpiry()`.
    fn set_key_expiry(self: &Rc<Self>, setup: bool) {
        let expire = self.check("KeyExpiryCheck").is_active();
        let t = self.b.updowns["KeyExpiryTimeSpinBtn"].position();
        if expire && t == 0 && !setup {
            self.clear_key(true, true);
        } else if expire && t != 0 {
            self.countdown.set(t);
            self.b.get::<gtk::Label>("KeyExpiryCountdownLbl").set_text(&t.to_string());
            if self.timer.borrow().is_none() {
                let d = Rc::downgrade(self);
                let id = glib::timeout_add_seconds_local(1, move || {
                    let Some(d) = d.upgrade() else { return glib::ControlFlow::Break };
                    let c = d.countdown.get() - 1;
                    d.countdown.set(c);
                    d.b.get::<gtk::Label>("KeyExpiryCountdownLbl").set_text(&c.to_string());
                    if c <= 0 {
                        d.timer.borrow_mut().take();
                        d.clear_key(true, false);
                        return glib::ControlFlow::Break;
                    }
                    glib::ControlFlow::Continue
                });
                *self.timer.borrow_mut() = Some(id);
            }
        } else {
            if let Some(t) = self.timer.borrow_mut().take() {
                t.remove();
            }
            self.b.get::<gtk::Label>("KeyExpiryCountdownLbl").set_text("");
        }
    }

    async fn enter_passw(self: &Rc<Self>) {
        let Some(main) = self.main.upgrade() else { return };
        let dlg = main.passw_enter();
        let mut flags = PASSWENTER_FLAG_PASSWQUALITY;
        if self.check("ConfirmPasswCheck").is_active() {
            flags |= PASSWENTER_FLAG_CONFIRMPASSW;
        }
        let ok = dlg.execute(flags, &trl("Master password"), Some(&self.win)).await == EnterResult::Ok;
        let passw = if ok { Some(dlg.get_passw(0)) } else { None };
        dlg.clear_and_flush();
        let Some(passw) = passw else { return };
        let mut info = format!("{} {}", Self::now(), trl("New key generated."));
        let hashapass = self.check("HashapassCompatCheck").is_active();
        *self.key.borrow_mut() = Some(MaskedKey::new(&generator_key(&passw, hashapass)));
        if self.check("ShowPasswHashCheck").is_active() {
            let ht = PasswHashType::from_index(self.combo("PasswHashList").active().unwrap_or(2) as i32);
            info.push_str(" Hash: ");
            info.push_str(&passw_checksum(&passw, ht));
        }
        self.entry("PasswStatusBox").set_text(&info);
        self.b.w("ClearKeyBtn").set_sensitive(true);
        self.b.w("GenerateBtn").set_sensitive(true);
        self.check("HashapassCompatCheck").set_sensitive(false);
        self.b.w("UseAsDefaultRNGBtn").set_sensitive(!hashapass);
        self.set_key_expiry(true);
        self.entry("ParameterBox").grab_focus();
    }

    fn generate(self: &Rc<Self>) {
        let Some(key) = self.key.borrow().as_ref().map(|k| k.plain()) else { return };
        let param = Zeroizing::new(self.entry("ParameterBox").text().to_string());
        let hashapass = self.check("HashapassCompatCheck").is_active();
        let len = self.b.updowns["PasswLengthSpinBtn"].position() as i32;
        let cs = self.combo("CharSetList").active().unwrap_or(0) as usize;
        let (passw, bits) = mp_generate(
            &key,
            &param,
            cs.min(MPPG_CHARSETS.len() - 1),
            len,
            self.check("AddPasswLenToParamCheck").is_active(),
            hashapass,
        );
        let n = passw.chars().count();
        self.entry("PasswBox").set_text(&passw);
        self.bar.area.set_visible(true);
        self.b.w("PasswInfoLbl").set_visible(true);
        self.bar.set_quality(bits);
        self.b.get::<gtk::Label>("PasswInfoLbl").set_text(&format!(
            "{} / {}",
            format_float_1(bits.min(256.0)),
            trl_format("%1 ch.", &[&n.to_string()])
        ));
        self.set_key_expiry(false);
        self.entry("PasswBox").grab_focus();
    }

    fn use_as_default_rng(self: &Rc<Self>) {
        let Some(key) = self.key.borrow().as_ref().map(|k| k.plain()) else { return };
        let param = self.entry("ParameterBox").text().to_string();
        let param_bytes: Zeroizing<Vec<u8>> =
            Zeroizing::new(param.encode_utf16().flat_map(u16::to_le_bytes).collect());
        if let Some(m) = self.main.upgrade() {
            m.use_key_seeded_prng_with(AesCtrPrng::from_key_and_param(&key, &param_bytes));
        }
        self.set_key_expiry(false);
    }
}

impl MainWindow {
    /// `MPPasswGenBtnClick()`.
    pub(super) fn mp_passw_gen(self: &Rc<Self>) {
        let existing = self.mp_passw_gen.borrow().clone();
        let d = match existing {
            Some(d) => d,
            None => {
                let d = MpPasswGenWin::new(self);
                *self.mp_passw_gen.borrow_mut() = Some(d.clone());
                d
            }
        };
        d.win.present();
        if d.key.borrow().is_none() {
            d.b.w("EnterPasswBtn").grab_focus();
        } else {
            d.b.w("ParameterBox").grab_focus();
        }
    }

    pub(super) fn save_mp_passw_gen_config(&self, ini: &mut pwtool_core::config::IniFile) {
        if let Some(d) = self.mp_passw_gen.borrow().as_ref() {
            d.save_config(ini);
        }
    }
}
