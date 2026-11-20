//! Password entry dialog (`upstream/src/main/PasswEnter.*`): used for
//! encrypting/decrypting text, the deterministic random generator, and
//! opening/creating password databases (with key files).

#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;
use pwtool_core::config::IniFile;
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::generator::PasswordGenerator;
use pwtool_core::random::{RandomGenerator, RandomPool};
use pwtool_core::util::floor_entropy_bits;
use zeroize::Zeroizing;

use super::builder::{Built, build_form};
use super::dialogs::form_window;
use super::form::load_form;
use super::quality_bar::QualityBar;
use super::util::{MsgButtons, MsgIcon, msg, msg_box};

pub const PASSWENTER_FLAG_ENCRYPT: u32 = 0x0001;
pub const PASSWENTER_FLAG_DECRYPT: u32 = 0x0002;
pub const PASSWENTER_FLAG_CONFIRMPASSW: u32 = 0x0004;
pub const PASSWENTER_FLAG_ENABLEOLDVER: u32 = 0x0008;
pub const PASSWENTER_FLAG_ENABLEPASSWCACHE: u32 = 0x0010;
pub const PASSWENTER_FLAG_ENABLEKEYFILE: u32 = 0x0020;
pub const PASSWENTER_FLAG_ENABLEKEYFILECREATION: u32 = 0x0040;
pub const PASSWENTER_FLAG_PASSWQUALITY: u32 = 0x0080;
pub const PASSWENTER_FLAG_ALLOWSKIP: u32 = 0x0100;

const CONFIG_ID: &str = "PasswEnter";
const PASSWORD_CHAR: char = '*';

/// Result of [`PasswEnterDlg::execute`] (`mrOk`, `mrCancel`, `mrIgnore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterResult {
    Ok,
    Cancel,
    Skip,
}

/// Remembered password, XOR-masked with a random pad (`memcrypt` with a
/// random key in the original).
struct PasswCache {
    masked: Zeroizing<Vec<u8>>,
    pad: Zeroizing<Vec<u8>>,
}

pub struct PasswEnterDlg {
    pub b: Built,
    pub win: gtk::Window,
    passw_box: gtk::Entry,
    confirm_box: gtk::Entry,
    key_file_box: gtk::ComboBoxText,
    bar: QualityBar,
    flags: Cell<u32>,
    est_quality: Cell<bool>,
    /// `g_config.UseAdvancedPasswEst`, set by the main window.
    pub use_advanced_passw_est: Cell<bool>,
    cache: RefCell<Option<PasswCache>>,
    expiry_countdown: Cell<i64>,
    timer: RefCell<Option<glib::SourceId>>,
    result: Cell<EnterResult>,
    done: RefCell<Option<async_channel::Sender<()>>>,
    suppress_change: Cell<bool>,
}

impl PasswEnterDlg {
    pub fn new(parent: &gtk::Window, ini: &IniFile) -> Rc<Self> {
        let b = build_form(load_form("PasswEnter"), "PasswEnter");
        let win = form_window(&b, Some(parent), true);
        win.set_hide_on_close(true);
        let passw_box: gtk::Entry = b.get("PasswBox");
        let confirm_box: gtk::Entry = b.get("ConfirmPasswBox");
        // password entry buffers keep their text in non-pageable memory
        // and wipe it when freed
        for e in [&passw_box, &confirm_box] {
            e.set_buffer(&gtk::PasswordEntryBuffer::new().upcast::<gtk::EntryBuffer>());
            e.set_invisible_char(Some(PASSWORD_CHAR));
        }
        let key_file_box: gtk::ComboBoxText = b.get("KeyFileBox");
        key_file_box.remove_all();
        key_file_box.append_text(&trl("(None)"));
        key_file_box.set_active(Some(0));
        let bar = QualityBar::new(b.get("PasswGauge"), 128);
        if let Some(l) = b.try_get::<gtk::Label>("PasswLbl") {
            l.add_css_class("heading");
        }
        if let Some(l) = b.try_get::<gtk::Label>("KeyFileLbl") {
            l.add_css_class("heading");
        }
        // the dialog is only resizable horizontally
        win.set_default_size(ini.read_int(CONFIG_ID, "WindowWidth", b.client.0).max(b.client.0), b.client.1);
        let d = Rc::new(PasswEnterDlg {
            b,
            win,
            passw_box,
            confirm_box,
            key_file_box,
            bar,
            flags: Cell::new(0),
            est_quality: Cell::new(false),
            use_advanced_passw_est: Cell::new(false),
            cache: RefCell::new(None),
            expiry_countdown: Cell::new(0),
            timer: RefCell::new(None),
            result: Cell::new(EnterResult::Cancel),
            done: RefCell::new(None),
            suppress_change: Cell::new(false),
        });
        d.load_config(ini);
        d.connect();
        d
    }

    fn load_config(&self, ini: &IniFile) {
        if let Some(u) = self.b.updowns.get("RememberPasswTimeSpinBtn") {
            u.set_position(ini.read_integer(CONFIG_ID, "RememberPasswTime", 60));
        }
        self.check("PasswQualityCheck")
            .set_active(ini.read_bool(CONFIG_ID, "EstimatePasswQuality", true));
    }

    /// `SaveConfig()`.
    pub fn save_config(&self, ini: &mut IniFile) {
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.width().max(self.b.client.0) as i64);
        ini.write_integer(CONFIG_ID, "RememberPasswTime", self.remember_time());
        ini.write_bool(CONFIG_ID, "EstimatePasswQuality", self.check("PasswQualityCheck").is_active());
    }

    fn check(&self, name: &str) -> gtk::CheckButton {
        self.b.get(name)
    }

    fn remember_time(&self) -> i64 {
        self.b.updowns.get("RememberPasswTimeSpinBtn").map(|u| u.position()).unwrap_or(60)
    }

    fn connect(self: &Rc<Self>) {
        let d = self.clone();
        self.b.get::<gtk::Button>("OKBtn").connect_clicked(move |_| {
            let d2 = d.clone();
            glib::spawn_future_local(async move { d2.ok_clicked().await });
        });
        let d = self.clone();
        self.b.get::<gtk::Button>("CancelBtn").connect_clicked(move |_| d.finish(EnterResult::Cancel));
        let d = self.clone();
        self.b.get::<gtk::Button>("SkipBtn").connect_clicked(move |_| d.finish(EnterResult::Skip));
        let d = self.clone();
        self.win.connect_close_request(move |_| {
            if let Some(tx) = d.done.borrow_mut().take() {
                let _ = tx.try_send(());
            }
            glib::Propagation::Proceed
        });
        let d = self.clone();
        self.b.get::<gtk::ToggleButton>("TogglePasswBtn").connect_toggled(move |_| d.toggle_passw());
        let d = self.clone();
        self.check("RememberPasswCheck").connect_toggled(move |_| d.remember_passw_check());
        let d = self.clone();
        self.check("PasswQualityCheck").connect_toggled(move |_| d.passw_quality_check());
        let d = self.clone();
        self.passw_box.connect_changed(move |_| d.passw_box_change());
        let d = self.clone();
        self.passw_box.connect_activate(move |_| d.b.get::<gtk::Button>("OKBtn").emit_clicked());
        let d = self.clone();
        self.confirm_box.connect_activate(move |_| d.b.get::<gtk::Button>("OKBtn").emit_clicked());
        let d = self.clone();
        self.b.get::<gtk::Button>("BrowseBtn").connect_clicked(move |_| d.browse());
        let d = self.clone();
        self.b.get::<gtk::Button>("CreateKeyFileBtn").connect_clicked(move |_| d.create_key_file());
        let d = self.clone();
        let esc = gtk::EventControllerKey::new();
        esc.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                d.finish(EnterResult::Cancel);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.win.add_controller(esc);
    }

    fn finish(&self, r: EnterResult) {
        self.result.set(r);
        self.win.close();
    }

    fn key_file_filters() -> gtk::gio::ListStore {
        let store = gtk::gio::ListStore::new::<gtk::FileFilter>();
        let f = gtk::FileFilter::new();
        f.set_name(Some(&format!("{} (*.key)", trl("Key files"))));
        f.add_pattern("*.key");
        store.append(&f);
        let f = gtk::FileFilter::new();
        f.set_name(Some(&format!("{} (*.*)", trl("All files"))));
        f.add_pattern("*");
        store.append(&f);
        store
    }

    fn select_key_file(&self, path: &str) {
        let n = self.key_file_box.model().map(|m| m.iter_n_children(None)).unwrap_or(0);
        let mut found = None;
        for i in 1..n {
            self.key_file_box.set_active(Some(i as u32));
            if self.key_file_box.active_text().is_some_and(|t| t == path) {
                found = Some(i as u32);
                break;
            }
        }
        match found {
            Some(i) => self.key_file_box.set_active(Some(i)),
            None => {
                self.key_file_box.append_text(path);
                self.key_file_box.set_active(Some(n as u32));
            }
        }
    }

    fn browse(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::new();
        dlg.set_filters(Some(&Self::key_file_filters()));
        let d = self.clone();
        dlg.open(Some(&self.win), gtk::gio::Cancellable::NONE, move |r| {
            if let Ok(p) = r.map(|f| f.path()) {
                if let Some(p) = p {
                    d.select_key_file(&p.to_string_lossy());
                }
            }
        });
    }

    fn create_key_file(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::new();
        dlg.set_filters(Some(&Self::key_file_filters()));
        let d = self.clone();
        dlg.save(Some(&self.win), gtk::gio::Cancellable::NONE, move |r| {
            let Some(path) = r.ok().and_then(|f| f.path()) else { return };
            match pwtool_core::passw::database::create_key_file(&path) {
                Ok(()) => {
                    msg(Some(&d.win), &trl("Key file successfully created."), MsgIcon::Info);
                    d.select_key_file(&path.to_string_lossy());
                }
                Err(e) => msg(
                    Some(&d.win),
                    &trl_format("Error while creating key file:\n%1.", &[&e.to_string()]),
                    MsgIcon::Error,
                ),
            }
        });
    }

    fn toggle_passw(&self) {
        let hide = self.b.get::<gtk::ToggleButton>("TogglePasswBtn").is_active();
        self.suppress_change.set(true);
        self.passw_box.set_visibility(!hide);
        self.suppress_change.set(false);
        self.confirm_box.set_visibility(!hide);
    }

    fn remember_passw_check(&self) {
        let on = self.check("RememberPasswCheck").is_active() && self.check("RememberPasswCheck").is_sensitive();
        self.b.w("RememberPasswTimeBox").set_sensitive(on);
        if let Some(u) = self.b.updowns.get("RememberPasswTimeSpinBtn") {
            u.set_sensitive(on);
        }
    }

    fn passw_quality_check(&self) {
        let c = self.check("PasswQualityCheck");
        let on = c.is_sensitive() && c.is_active();
        self.est_quality.set(on);
        self.bar.area.set_sensitive(on);
        if !on {
            self.bar.clear();
        }
        self.passw_box_change();
    }

    /// `PasswBoxChange()`.
    fn passw_box_change(&self) {
        if self.flags.get() & (PASSWENTER_FLAG_PASSWQUALITY | PASSWENTER_FLAG_ENCRYPT) == 0
            || self.suppress_change.get()
            || !self.win.is_visible()
        {
            return;
        }
        let info: gtk::Label = self.b.get("PasswInfoLbl");
        if self.passw_box.text_length() == 0 {
            self.bar.clear();
            info.set_text("");
            return;
        }
        let passw = Zeroizing::new(self.passw_box.text().to_string());
        let mut s = String::new();
        if self.est_quality.get() {
            let bits = if self.use_advanced_passw_est.get() {
                pwtool_core::zxcvbn::zxcvbn_match(&passw)
            } else {
                PasswordGenerator::estimate_passw_security(&passw)
            };
            self.bar.set_quality(bits);
            s = format!("{} / ", floor_entropy_bits(bits));
        }
        s.push_str(&trl_format("%1 ch.", &[&passw.chars().count().to_string()]));
        info.set_text(&s);
    }

    /// `ClearPasswCache()`.
    pub fn clear_passw_cache(&self) {
        if self.cache.borrow_mut().take().is_some() {
            self.expiry_countdown.set(0);
            if let Some(t) = self.timer.borrow_mut().take() {
                t.remove();
            }
        }
    }

    /// Seconds until the remembered password is cleared (0 = no expiry or
    /// nothing remembered); `GetPasswCacheExpiryCountdown()`.
    pub fn passw_cache_expiry_countdown(&self) -> i64 {
        self.expiry_countdown.get()
    }

    pub fn has_cached_passw(&self) -> bool {
        self.cache.borrow().is_some()
    }

    async fn ok_clicked(self: &Rc<Self>) {
        let flags = self.flags.get();
        let mut err = String::new();
        if flags & PASSWENTER_FLAG_ENABLEKEYFILE != 0 {
            let kf = self.key_file_box.active().unwrap_or(0);
            if self.passw_box.text_length() == 0 && kf == 0 {
                err = trl("Enter a password and/or select a key file.");
            } else if kf > 0 {
                let name = self.key_file_box.active_text().map(|t| t.to_string()).unwrap_or_default();
                if !std::path::Path::new(&name).is_file() {
                    err = trl_format("The selected key file does not exist:\n%1", &[&name]);
                }
            }
        } else if self.passw_box.text_length() == 0 {
            err = trl("Password is empty.");
        }
        if !err.is_empty() {
            msg_box(Some(&self.win), &err, MsgIcon::Error, MsgButtons::Ok, false).await;
            self.passw_box.grab_focus();
            return;
        }
        if flags & PASSWENTER_FLAG_CONFIRMPASSW != 0 && *self.get_passw(0) != *self.get_passw(1) {
            msg_box(Some(&self.win), &trl("Passwords are not identical."), MsgIcon::Error, MsgButtons::Ok, false)
                .await;
            self.passw_box.grab_focus();
            return;
        }
        if flags & PASSWENTER_FLAG_ENABLEPASSWCACHE != 0 && self.check("RememberPasswCheck").is_active() {
            let passw = self.get_passw(0);
            let mut pad = Zeroizing::new(vec![0u8; passw.len()]);
            RandomPool::global().get_data(&mut pad);
            let masked = Zeroizing::new(passw.bytes().zip(pad.iter()).map(|(a, b)| a ^ b).collect::<Vec<u8>>());
            *self.cache.borrow_mut() = Some(PasswCache { masked, pad });
            let t = self.remember_time();
            if t > 0 {
                self.expiry_countdown.set(t);
                self.start_expiry_timer();
            }
        }
        self.finish(EnterResult::Ok);
    }

    fn start_expiry_timer(self: &Rc<Self>) {
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local(1, move || {
            let Some(d) = weak.upgrade() else { return glib::ControlFlow::Break };
            let c = d.expiry_countdown.get() - 1;
            d.expiry_countdown.set(c);
            if c <= 0 {
                d.timer.borrow_mut().take();
                d.cache.borrow_mut().take();
                d.expiry_countdown.set(0);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.timer.borrow_mut() = Some(id);
    }

    /// `Execute()`: show the dialog modally. If a remembered password is
    /// available (and the cache is enabled), returns `Ok` immediately.
    pub async fn execute(self: &Rc<Self>, flags: u32, title: &str, parent: Option<&gtk::Window>) -> EnterResult {
        if self.passw_box.text_length() != 0 {
            self.clear();
        }
        let cache = flags & PASSWENTER_FLAG_ENABLEPASSWCACHE != 0;
        if cache && self.cache.borrow().is_some() {
            // reset countdown upon every request
            if self.expiry_countdown.get() > 0 {
                self.expiry_countdown.set(self.remember_time());
            }
            self.flags.set(flags);
            return EnterResult::Ok;
        }
        let caption = if !title.is_empty() {
            title.to_string()
        } else if flags & PASSWENTER_FLAG_ENCRYPT != 0 {
            trl("Encrypt")
        } else if flags & PASSWENTER_FLAG_DECRYPT != 0 {
            trl("Decrypt")
        } else {
            trl("Enter password")
        };
        self.win.set_title(Some(&caption));
        if let Some(p) = parent {
            self.win.set_transient_for(Some(p));
        }
        self.flags.set(flags);
        self.check("PasswQualityCheck")
            .set_sensitive(flags & (PASSWENTER_FLAG_PASSWQUALITY | PASSWENTER_FLAG_ENCRYPT) != 0);
        self.bar.clear();
        self.b.get::<gtk::Label>("PasswInfoLbl").set_text("");
        let tb: gtk::ToggleButton = self.b.get("TogglePasswBtn");
        tb.set_active(true);
        self.toggle_passw();
        let confirm = flags & PASSWENTER_FLAG_CONFIRMPASSW != 0;
        self.b.w("ConfirmPasswLbl").set_sensitive(confirm);
        self.confirm_box.set_sensitive(confirm);
        let old = self.check("OldVersionCheck");
        old.set_active(false);
        old.set_sensitive(flags & PASSWENTER_FLAG_ENABLEOLDVER != 0);
        let rem = self.check("RememberPasswCheck");
        rem.set_active(false);
        rem.set_sensitive(cache);
        self.remember_passw_check();
        let kf = flags & PASSWENTER_FLAG_ENABLEKEYFILE != 0;
        self.b.w("KeyFileLbl").set_sensitive(kf);
        self.key_file_box.set_sensitive(kf);
        self.b.w("BrowseBtn").set_sensitive(kf);
        self.b
            .w("CreateKeyFileBtn")
            .set_sensitive(kf && flags & PASSWENTER_FLAG_ENABLEKEYFILECREATION != 0);
        self.b.w("SkipBtn").set_visible(flags & PASSWENTER_FLAG_ALLOWSKIP != 0);
        self.result.set(EnterResult::Cancel);
        let (tx, rx) = async_channel::bounded::<()>(1);
        *self.done.borrow_mut() = Some(tx);
        self.win.set_default_widget(Some(&self.b.get::<gtk::Button>("OKBtn")));
        self.win.present();
        self.passw_quality_check();
        self.passw_box.grab_focus();
        let _ = rx.recv().await;
        self.result.get()
    }

    /// `GetPassw()`: the remembered password if the cache is enabled,
    /// otherwise the text of the password (0) or confirmation (1) box.
    pub fn get_passw(&self, n: i32) -> Zeroizing<String> {
        if self.flags.get() & PASSWENTER_FLAG_ENABLEPASSWCACHE != 0 {
            if let Some(c) = self.cache.borrow().as_ref() {
                let bytes = Zeroizing::new(c.masked.iter().zip(c.pad.iter()).map(|(a, b)| a ^ b).collect::<Vec<u8>>());
                return Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        let e = if n == 0 { &self.passw_box } else { &self.confirm_box };
        Zeroizing::new(e.text().to_string())
    }

    /// `GetPasswBinary()`: UTF-8 or UTF-16LE bytes of the password.
    pub fn get_passw_binary(&self, utf8: bool) -> Zeroizing<Vec<u8>> {
        let p = self.get_passw(0);
        if utf8 {
            Zeroizing::new(p.as_bytes().to_vec())
        } else {
            Zeroizing::new(p.encode_utf16().flat_map(u16::to_le_bytes).collect())
        }
    }

    /// `GetKeyFileName()`.
    pub fn key_file_name(&self) -> Option<PathBuf> {
        if self.flags.get() & PASSWENTER_FLAG_ENABLEKEYFILE != 0 && self.key_file_box.active().unwrap_or(0) > 0 {
            self.key_file_box.active_text().map(|t| PathBuf::from(t.as_str()))
        } else {
            None
        }
    }

    /// "PWGen <2.3.0" checkbox.
    pub fn old_version(&self) -> bool {
        self.check("OldVersionCheck").is_active()
    }

    /// `Clear()`: wipe the entered passwords.
    pub fn clear(&self) {
        self.suppress_change.set(true);
        self.passw_box.set_text("");
        self.suppress_change.set(false);
        if self.flags.get() & PASSWENTER_FLAG_CONFIRMPASSW != 0 {
            self.confirm_box.set_text("");
        }
        self.key_file_box.set_active(Some(0));
    }

    /// `AutoClearPasswDlg::Clear()`.
    pub fn clear_and_flush(&self) {
        self.clear();
        RandomPool::global().flush();
    }
}
