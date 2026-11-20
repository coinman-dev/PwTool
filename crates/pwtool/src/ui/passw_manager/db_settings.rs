//! Database settings dialog (`upstream/src/main/PasswMngDbSettings.*`).

#![allow(deprecated)]

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use pwtool_core::config::IniFile;
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::PasswordGenerator;
use pwtool_core::passw::database::{MAX_PASSW_HISTORY_SIZE, decode_expiry_date};
use pwtool_core::random::fast::Jsf32;
use pwtool_core::unicode::{str_to_w32, w32_to_string_secure};
use zeroize::Zeroizing;

use super::helpers::{DatePicker, close_on_escape, crlf_to_lf, lf_to_crlf, text_view_text, today};
use crate::ui::builder::{Built, build_form};
use crate::ui::dialogs::form_window;
use crate::ui::form::load_form;
use crate::ui::util::{MsgButtons, MsgIcon, msg_box};

const CONFIG_ID: &str = "PasswMngDbSettings";
const CIPHER_NAMES: [&str; 2] = ["Advanced Encryption Standard (AES-CBC)", "ChaCha20"];
const CIPHER_KEY_SIZES: [u32; 2] = [256, 256];

/// `PasswDbSettings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbSettings {
    pub default_user_name: Zeroizing<String>,
    pub passw_format_seq: Zeroizing<String>,
    /// With CRLF line breaks (as stored in the database).
    pub description: Zeroizing<String>,
    pub default_expiry_days: u32,
    pub master_passw_expiry_date: u32,
    pub default_max_passw_history_size: u32,
    pub cipher_type: u32,
    pub num_kdf_rounds: u32,
    pub compressed: bool,
    pub compression_level: i32,
}

/// Callback applying the settings (`TPasswMngForm::ApplyDbSettings()`);
/// returns `false` to keep the dialog open.
pub type ApplyFn = Box<dyn Fn(DbSettings) -> Pin<Box<dyn Future<Output = bool>>>>;

pub struct DbSettingsDlg {
    pub b: Built,
    pub win: gtk::Window,
    expiry: Rc<DatePicker>,
    has_recovery: Cell<bool>,
}

impl DbSettingsDlg {
    pub fn new(parent: &gtk::Window, ini: &IniFile) -> Rc<Self> {
        let b = build_form(load_form("PasswMngDbSettings"), "PasswMngDbSettings");
        let win = form_window(&b, Some(parent), true);
        win.set_hide_on_close(true);
        // "Label1" is a leftover of the form designer without function
        if let Some(l) = b.try_get::<gtk::Widget>("Label1") {
            l.set_visible(false);
        }
        let algo: gtk::ComboBoxText = b.get("EncryptionAlgoList");
        algo.remove_all();
        for (i, name) in CIPHER_NAMES.iter().enumerate() {
            algo.append_text(&trl_format("%1 with %2-bit key", &[name, &CIPHER_KEY_SIZES[i].to_string()]));
        }
        if let Some(u) = b.updowns.get("PasswHistorySpinBtn") {
            u.set_max(MAX_PASSW_HISTORY_SIZE as i64);
        }
        crate::ui::builder::add_css(&b.get::<gtk::Label>("ExpiryInfoLbl"), "color: red;");
        let expiry = DatePicker::new(b.get("ExpiryDatePicker"), b.get("ExpiryDatePicker.calendar"));
        let h = ini.read_int(CONFIG_ID, "WindowHeight", -1);
        let w = ini.read_int(CONFIG_ID, "WindowWidth", -1);
        if w > 0 && h > 0 {
            win.set_default_size(w, h);
        }
        close_on_escape(&win);
        let d = Rc::new(DbSettingsDlg { b, win, expiry, has_recovery: Cell::new(false) });
        let dd = d.clone();
        d.b.get::<gtk::CheckButton>("EnableCompressionCheck").connect_toggled(move |_| dd.enable_compression_check());
        let dd = d.clone();
        d.b.get::<gtk::CheckButton>("MasterPasswExpiryCheck").connect_toggled(move |_| dd.master_passw_expiry_check());
        let dd = d.clone();
        d.b.get::<gtk::Button>("CalcRoundsBtn").connect_clicked(move |_| dd.calc_rounds());
        let dd = d.clone();
        d.b.get::<gtk::Button>("PasswGenTestBtn").connect_clicked(move |_| dd.passw_gen_test());
        d
    }

    pub fn save_config(&self, ini: &mut IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.width() as i64);
    }

    fn entry(&self, name: &str) -> gtk::Entry {
        self.b.get(name)
    }

    fn description_view(&self) -> gtk::TextView {
        self.b.get("DescriptionBox.view")
    }

    /// `GetSettings()`.
    pub fn get_settings(&self) -> DbSettings {
        let master_expiry = if self.b.get::<gtk::CheckButton>("MasterPasswExpiryCheck").is_active() {
            self.expiry.encoded()
        } else {
            0
        };
        let compressed = self.b.get::<gtk::CheckButton>("EnableCompressionCheck").is_active();
        DbSettings {
            default_user_name: Zeroizing::new(self.entry("DefUserNameBox").text().to_string()),
            passw_format_seq: Zeroizing::new(self.entry("PasswFormatSeqBox").text().to_string()),
            description: Zeroizing::new(lf_to_crlf(&text_view_text(&self.description_view()))),
            default_expiry_days: self.b.updowns.get("DefaultExpirySpinBtn").map(|u| u.position()).unwrap_or(0) as u32,
            master_passw_expiry_date: master_expiry,
            default_max_passw_history_size: self.b.updowns.get("PasswHistorySpinBtn").map(|u| u.position()).unwrap_or(0)
                as u32,
            cipher_type: self.b.get::<gtk::ComboBoxText>("EncryptionAlgoList").active().unwrap_or(0),
            num_kdf_rounds: self.entry("NumKdfRoundsBox").text().trim().parse().unwrap_or(0),
            compressed,
            compression_level: if compressed { self.b.get::<gtk::Scale>("CompressionLevelBar").value() as i32 } else { 0 },
        }
    }

    /// `SetSettings()`.
    pub fn set_settings(&self, s: &DbSettings, has_recovery_passw: bool) {
        self.has_recovery.set(has_recovery_passw);
        self.entry("DefUserNameBox").set_text(&s.default_user_name);
        self.entry("PasswFormatSeqBox").set_text(&s.passw_format_seq);
        self.description_view().buffer().set_text(&crlf_to_lf(&s.description));
        if let Some(u) = self.b.updowns.get("DefaultExpirySpinBtn") {
            u.set_position(s.default_expiry_days as i64);
        }
        if let Some(u) = self.b.updowns.get("PasswHistorySpinBtn") {
            u.set_position(s.default_max_passw_history_size as i64);
        }
        let info: gtk::Label = self.b.get("ExpiryInfoLbl");
        let check: gtk::CheckButton = self.b.get("MasterPasswExpiryCheck");
        match decode_expiry_date(s.master_passw_expiry_date) {
            Some((y, m, d)) => {
                check.set_active(true);
                self.expiry.set_date((y, m, d));
                let (ty, tm, td) = today();
                if (ty, tm, td) >= (y, m, d) {
                    info.set_text(&trl("WARNING: Master password expired!"));
                } else {
                    info.set_text("");
                }
            }
            None => {
                check.set_active(false);
                self.expiry.set_date(today());
                info.set_text("");
            }
        }
        self.master_passw_expiry_check();
        let algo: gtk::ComboBoxText = self.b.get("EncryptionAlgoList");
        algo.set_active(Some(s.cipher_type));
        self.set_recovery_dependencies(has_recovery_passw);
        self.entry("NumKdfRoundsBox").set_text(&s.num_kdf_rounds.to_string());
        self.b.get::<gtk::CheckButton>("EnableCompressionCheck").set_active(s.compressed);
        self.b
            .get::<gtk::Scale>("CompressionLevelBar")
            .set_value(if s.compressed { s.compression_level as f64 } else { 6.0 });
        self.enable_compression_check();
    }

    /// Controls that cannot be changed while a recovery password is set
    /// (`SetRecoveryKeyDependencies()` of the manager).
    pub fn set_recovery_dependencies(&self, has_recovery: bool) {
        self.has_recovery.set(has_recovery);
        self.b.w("EncryptionAlgoList").set_sensitive(!has_recovery);
        self.b.w("NumKdfRoundsBox").set_sensitive(!has_recovery);
        self.b.w("CalcRoundsBtn").set_sensitive(!has_recovery);
    }

    /// `Clear()`.
    pub fn clear(&self) {
        self.set_settings(&DbSettings::default(), false);
    }

    fn enable_compression_check(&self) {
        let on = self.b.get::<gtk::CheckButton>("EnableCompressionCheck").is_active();
        self.b.w("CompressionLevelBar").set_sensitive(on);
        self.b.w("CompressionLevelLbl").set_sensitive(on);
    }

    fn master_passw_expiry_check(&self) {
        let on = self.b.get::<gtk::CheckButton>("MasterPasswExpiryCheck").is_active();
        self.expiry.button.set_sensitive(on);
    }

    /// `CalcRoundsBtnClick()`: number of PBKDF2 rounds for a delay of 1 s.
    fn calc_rounds(&self) {
        let mut key = Zeroizing::new([0u8; 32]);
        let mut salt = [0u8; 32];
        pwtool_core::random::fast::fast_fill(&mut key[..]);
        pwtool_core::random::fast::fast_fill(&mut salt);
        const ROUGH_EST_ROUNDS: u32 = 10_000;
        const TEST_FACTOR: f64 = 2.0;
        let t = std::time::Instant::now();
        let _ = pwtool_core::crypto::pbkdf2_256bit(&key[..], &salt, ROUGH_EST_ROUNDS, None);
        let rough = (TEST_FACTOR * (ROUGH_EST_ROUNDS as f64 / t.elapsed().as_secs_f64().max(1e-6))).round() as u32;
        let t = std::time::Instant::now();
        let _ = pwtool_core::crypto::pbkdf2_256bit(&key[..], &salt, rough.max(1), None);
        let rounds = ((rough as f64 / t.elapsed().as_secs_f64().max(1e-6)).round() as u32).max(1);
        self.entry("NumKdfRoundsBox").set_text(&rounds.to_string());
    }

    /// `PasswGenTestBtnClick()`.
    fn passw_gen_test(&self) {
        let mut s = trl("Test:");
        let fmt = Zeroizing::new(self.entry("PasswFormatSeqBox").text().to_string());
        if !fmt.is_empty() {
            let pg = PasswordGenerator::new();
            let mut rng = Jsf32::new();
            let f32 = str_to_w32(&fmt);
            let r = pg.get_format_passw(&mut rng, &f32, 0, None, 16000, None);
            if !r.passw.is_empty() {
                s.push(' ');
                s.push_str(&w32_to_string_secure(&r.passw));
            }
        }
        self.entry("PasswGenTestBox").set_text(&s);
    }

    /// `ShowModal()`: OK validates the KDF rounds and calls `apply`.
    pub async fn show_modal(self: &Rc<Self>, apply: ApplyFn) -> bool {
        if let Some(nb) = self.b.try_get::<gtk::Notebook>("ConfigPages") {
            nb.set_current_page(Some(0));
        }
        self.entry("PasswGenTestBox").set_text(&trl("Test:"));
        let ok_pressed = Rc::new(Cell::new(false));
        let (tx, rx) = async_channel::bounded::<()>(1);
        let ok: gtk::Button = self.b.get("OKBtn");
        let cancel: gtk::Button = self.b.get("CancelBtn");
        let apply = Rc::new(apply);
        let busy = Rc::new(Cell::new(false));
        let r = ok_pressed.clone();
        let d = self.clone();
        let h1 = ok.connect_clicked(move |_| {
            if busy.get() {
                return;
            }
            let d = d.clone();
            let r = r.clone();
            let apply = apply.clone();
            let busy = busy.clone();
            glib::spawn_future_local(async move {
                let rounds: u32 = d.entry("NumKdfRoundsBox").text().trim().parse().unwrap_or(0);
                if rounds == 0 {
                    msg_box(Some(&d.win), &trl("Invalid number of key derivation rounds."), MsgIcon::Error, MsgButtons::Ok, false)
                        .await;
                    return;
                }
                busy.set(true);
                let ok = apply(d.get_settings()).await;
                busy.set(false);
                if ok {
                    r.set(true);
                    d.win.close();
                }
            });
        });
        let w = self.win.clone();
        let h2 = cancel.connect_clicked(move |_| w.close());
        let h3 = self.win.connect_close_request(move |_| {
            let _ = tx.try_send(());
            glib::Propagation::Proceed
        });
        self.win.set_default_widget(Some(&ok));
        self.win.present();
        self.entry("DefUserNameBox").grab_focus();
        let _ = rx.recv().await;
        ok.disconnect(h1);
        cancel.disconnect(h2);
        self.win.disconnect(h3);
        // `FormClose()`
        self.entry("DefUserNameBox").set_text("");
        self.entry("PasswFormatSeqBox").set_text("");
        self.entry("PasswGenTestBox").set_text("");
        ok_pressed.get()
    }
}
