//! Password entry dialog (`upstream/src/main/PasswEnter.*`): used for
//! encrypting/decrypting text, the deterministic random generator, the
//! MP password generator and (later) password databases with key files.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use egui::ViewportId;
use pwtool_core::config::IniFile;
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::generator::PasswordGenerator;
use pwtool_core::random::{RandomGenerator, RandomPool};
use pwtool_core::util::floor_entropy_bits;
use zeroize::Zeroizing;

use super::dialogs::{ask, form_window, load, msg};
use super::quality_bar::QualityBar;
use crate::vcl::Form;
use crate::vcl::dialogs::{MsgButtons, MsgIcon};
use crate::windows;

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
const WINDOW_ID: &str = "PasswEnter";

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
    pub form: Rc<Form>,
    bar: QualityBar,
    flags: Cell<u32>,
    est_quality: Cell<bool>,
    /// `g_config.UseAdvancedPasswEst`, set by the main window.
    pub use_advanced_passw_est: Cell<bool>,
    cache: RefCell<Option<PasswCache>>,
    expiry_countdown: Cell<i64>,
    /// Incremented to stop a running expiry countdown.
    timer_gen: Cell<u64>,
    result: Cell<EnterResult>,
    done: RefCell<Option<async_channel::Sender<()>>>,
    window: Cell<ViewportId>,
    width: Cell<f32>,
}

impl PasswEnterDlg {
    pub fn new(ini: &IniFile) -> Rc<Self> {
        let form = load("PasswEnter");
        for n in ["PasswBox", "ConfirmPasswBox"] {
            form.set_secret(n);
            form.set_password_char(n, true);
        }
        form.set_items("KeyFileBox", vec![trl("(None)")]);
        form.set_item_index("KeyFileBox", Some(0));
        let bar = QualityBar::new(128);
        let b = bar.clone();
        form.set_painter("PasswGauge", move |ui, rect| b.paint(ui, rect));
        for n in ["PasswLbl", "KeyFileLbl"] {
            if form.has(n) {
                form.set_bold(n, true);
            }
        }
        let d = Rc::new(PasswEnterDlg {
            form,
            bar,
            flags: Cell::new(0),
            est_quality: Cell::new(false),
            use_advanced_passw_est: Cell::new(false),
            cache: RefCell::new(None),
            expiry_countdown: Cell::new(0),
            timer_gen: Cell::new(0),
            result: Cell::new(EnterResult::Cancel),
            done: RefCell::new(None),
            window: Cell::new(ViewportId::from_hash_of(WINDOW_ID)),
            width: Cell::new(0.0),
        });
        d.load_config(ini);
        d.connect();
        d
    }

    fn f(&self) -> &Form {
        &self.form
    }

    fn load_config(&self, ini: &IniFile) {
        self.f().set_position("RememberPasswTimeSpinBtn", ini.read_integer(CONFIG_ID, "RememberPasswTime", 60));
        self.f().set_checked("PasswQualityCheck", ini.read_bool(CONFIG_ID, "EstimatePasswQuality", true));
        self.width.set(ini.read_int(CONFIG_ID, "WindowWidth", 0) as f32);
    }

    /// `SaveConfig()`.
    pub fn save_config(&self, ini: &mut IniFile) {
        let client = self.f().client_size().x;
        ini.write_integer(CONFIG_ID, "WindowWidth", self.width.get().max(client) as i64);
        ini.write_integer(CONFIG_ID, "RememberPasswTime", self.remember_time());
        ini.write_bool(CONFIG_ID, "EstimatePasswQuality", self.f().checked("PasswQualityCheck"));
    }

    fn remember_time(&self) -> i64 {
        self.f().position("RememberPasswTimeSpinBtn")
    }

    fn connect(self: &Rc<Self>) {
        let on = |name: &str, f: fn(&Rc<Self>)| {
            let d = Rc::downgrade(self);
            self.form.on_click(name, move |_| {
                if let Some(d) = d.upgrade() {
                    f(&d);
                }
            });
        };
        on("OKBtn", |d| {
            let d = d.clone();
            crate::runtime::spawn(async move { d.ok_clicked().await });
        });
        on("CancelBtn", |d| d.finish(EnterResult::Cancel));
        on("SkipBtn", |d| d.finish(EnterResult::Skip));
        on("TogglePasswBtn", |d| d.toggle_passw());
        on("RememberPasswCheck", |d| d.remember_passw_check());
        on("PasswQualityCheck", |d| d.passw_quality_check());
        on("BrowseBtn", |d| {
            let d = d.clone();
            crate::runtime::spawn(async move { d.browse().await });
        });
        on("CreateKeyFileBtn", |d| {
            let d = d.clone();
            crate::runtime::spawn(async move { d.create_key_file().await });
        });
        let d = Rc::downgrade(self);
        self.form.on_change("PasswBox", move |_| {
            if let Some(d) = d.upgrade() {
                d.passw_box_change();
            }
        });
    }

    fn finish(&self, r: EnterResult) {
        self.result.set(r);
        windows::close(self.window.get());
        if let Some(tx) = self.done.borrow_mut().take() {
            let _ = tx.try_send(());
        }
    }

    fn key_file_filters() -> [(String, &'static [&'static str]); 2] {
        [(format!("{} (*.key)", trl("Key files")), &["key"]), (format!("{} (*.*)", trl("All files")), &["*"])]
    }

    fn select_key_file(&self, path: &str) {
        let mut items = self.f().items("KeyFileBox");
        let i = match items.iter().skip(1).position(|t| t == path) {
            Some(i) => i + 1,
            None => {
                items.push(path.to_string());
                self.f().set_items("KeyFileBox", items.clone());
                items.len() - 1
            }
        };
        self.f().set_item_index("KeyFileBox", Some(i));
    }

    async fn browse(&self) {
        let fl = Self::key_file_filters();
        let filters: Vec<(&str, &[&str])> = fl.iter().map(|(n, e)| (n.as_str(), *e)).collect();
        if let Some(p) = crate::vcl::dialogs::open_file(&trl("Select key file"), None, &filters).await {
            self.select_key_file(&p.to_string_lossy());
        }
    }

    async fn create_key_file(&self) {
        let fl = Self::key_file_filters();
        let filters: Vec<(&str, &[&str])> = fl.iter().map(|(n, e)| (n.as_str(), *e)).collect();
        let Some(path) = crate::vcl::dialogs::save_file(&trl("Create key file"), None, None, &filters).await else { return };
        let parent = self.window.get();
        match pwtool_core::passw::database::create_key_file(&path) {
            Ok(()) => {
                msg(parent, &trl("Key file successfully created."), MsgIcon::Info);
                self.select_key_file(&path.to_string_lossy());
            }
            Err(e) => msg(parent, &trl_format("Error while creating key file:\n%1.", &[&e.to_string()]), MsgIcon::Error),
        }
    }

    fn toggle_passw(&self) {
        let hide = self.f().checked("TogglePasswBtn");
        self.f().set_password_char("PasswBox", hide);
        self.f().set_password_char("ConfirmPasswBox", hide);
    }

    fn remember_passw_check(&self) {
        let on = self.f().checked("RememberPasswCheck") && self.f().enabled("RememberPasswCheck");
        self.f().set_enabled("RememberPasswTimeBox", on);
        self.f().set_enabled("RememberPasswTimeSpinBtn", on);
    }

    fn passw_quality_check(&self) {
        let on = self.f().enabled("PasswQualityCheck") && self.f().checked("PasswQualityCheck");
        self.est_quality.set(on);
        self.f().set_enabled("PasswGauge", on);
        if !on {
            self.bar.clear();
        }
        self.passw_box_change();
    }

    /// `PasswBoxChange()`.
    fn passw_box_change(&self) {
        if self.flags.get() & (PASSWENTER_FLAG_PASSWQUALITY | PASSWENTER_FLAG_ENCRYPT) == 0 {
            return;
        }
        let passw = self.f().secret_text("PasswBox");
        if passw.is_empty() {
            self.bar.clear();
            self.f().set_text("PasswInfoLbl", "");
            return;
        }
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
        self.f().set_text("PasswInfoLbl", &s);
    }

    /// `ClearPasswCache()`.
    pub fn clear_passw_cache(&self) {
        if self.cache.borrow_mut().take().is_some() {
            self.expiry_countdown.set(0);
            self.timer_gen.set(self.timer_gen.get() + 1);
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
        let parent = self.window.get();
        let passw_empty = self.f().secret_text("PasswBox").is_empty();
        let mut err = String::new();
        if flags & PASSWENTER_FLAG_ENABLEKEYFILE != 0 {
            let kf = self.f().item_index("KeyFileBox").unwrap_or(0);
            if passw_empty && kf == 0 {
                err = trl("Enter a password and/or select a key file.");
            } else if kf > 0 {
                let name = self.f().text("KeyFileBox");
                if !std::path::Path::new(&name).is_file() {
                    err = trl_format("The selected key file does not exist:\n%1", &[&name]);
                }
            }
        } else if passw_empty {
            err = trl("Password is empty.");
        }
        if !err.is_empty() {
            ask(parent, &err, MsgIcon::Error, MsgButtons::Ok, false).await;
            self.f().focus("PasswBox");
            return;
        }
        if flags & PASSWENTER_FLAG_CONFIRMPASSW != 0 && *self.get_passw(0) != *self.get_passw(1) {
            ask(parent, &trl("Passwords are not identical."), MsgIcon::Error, MsgButtons::Ok, false).await;
            self.f().focus("PasswBox");
            return;
        }
        if flags & PASSWENTER_FLAG_ENABLEPASSWCACHE != 0 && self.f().checked("RememberPasswCheck") {
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
        let tgen = self.timer_gen.get() + 1;
        self.timer_gen.set(tgen);
        let weak = Rc::downgrade(self);
        crate::runtime::spawn(async move {
            loop {
                pwtool_app::rt::sleep(Duration::from_secs(1)).await;
                let Some(d) = weak.upgrade() else { return };
                if d.timer_gen.get() != tgen {
                    return;
                }
                let c = d.expiry_countdown.get() - 1;
                d.expiry_countdown.set(c);
                if c <= 0 {
                    d.cache.borrow_mut().take();
                    d.expiry_countdown.set(0);
                    return;
                }
            }
        });
    }

    /// `Execute()`: show the dialog modally. If a remembered password is
    /// available (and the cache is enabled), returns `Ok` immediately.
    pub async fn execute(self: &Rc<Self>, flags: u32, title: &str, parent: ViewportId) -> EnterResult {
        if !self.f().secret_text("PasswBox").is_empty() {
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
        let f = self.f();
        self.flags.set(flags);
        f.set_enabled("PasswQualityCheck", flags & (PASSWENTER_FLAG_PASSWQUALITY | PASSWENTER_FLAG_ENCRYPT) != 0);
        self.bar.clear();
        f.set_text("PasswInfoLbl", "");
        f.set_checked("TogglePasswBtn", true);
        self.toggle_passw();
        let confirm = flags & PASSWENTER_FLAG_CONFIRMPASSW != 0;
        f.set_enabled("ConfirmPasswLbl", confirm);
        f.set_enabled("ConfirmPasswBox", confirm);
        f.set_checked("OldVersionCheck", false);
        f.set_enabled("OldVersionCheck", flags & PASSWENTER_FLAG_ENABLEOLDVER != 0);
        f.set_checked("RememberPasswCheck", false);
        f.set_enabled("RememberPasswCheck", cache);
        self.remember_passw_check();
        let kf = flags & PASSWENTER_FLAG_ENABLEKEYFILE != 0;
        for n in ["KeyFileLbl", "KeyFileBox", "BrowseBtn"] {
            f.set_enabled(n, kf);
        }
        f.set_enabled("CreateKeyFileBtn", kf && flags & PASSWENTER_FLAG_ENABLEKEYFILECREATION != 0);
        f.set_visible("SkipBtn", flags & PASSWENTER_FLAG_ALLOWSKIP != 0);
        self.result.set(EnterResult::Cancel);
        let (tx, rx) = async_channel::bounded::<()>(1);
        *self.done.borrow_mut() = Some(tx);
        let mut win = form_window(WINDOW_ID, self.form.clone(), parent, true);
        *win.title.borrow_mut() = caption;
        let w = self.width.get();
        if w > win.size.get().x {
            win.size.set(egui::vec2(w, win.size.get().y));
        }
        let min = win.size.get();
        win = win.resizable(true, min);
        let d = Rc::downgrade(self);
        *win.on_close.borrow_mut() = Some(Box::new(move || {
            if let Some(d) = d.upgrade() {
                d.result.set(EnterResult::Cancel);
                if let Some(tx) = d.done.borrow_mut().take() {
                    let _ = tx.try_send(());
                }
            }
            true
        }));
        let win = Rc::new(win);
        self.window.set(win.id);
        windows::open(win.clone());
        self.passw_quality_check();
        f.focus("PasswBox");
        let _ = rx.recv().await;
        if let Some(s) = win.last_size.get() {
            self.width.set(s.x);
        }
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
        self.f().secret_text(if n == 0 { "PasswBox" } else { "ConfirmPasswBox" })
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
        if self.flags.get() & PASSWENTER_FLAG_ENABLEKEYFILE != 0 && self.f().item_index("KeyFileBox").unwrap_or(0) > 0 {
            Some(PathBuf::from(self.f().text("KeyFileBox")))
        } else {
            None
        }
    }

    /// "PWGen <2.3.0" checkbox.
    pub fn old_version(&self) -> bool {
        self.f().checked("OldVersionCheck")
    }

    /// `Clear()`: wipe the entered passwords.
    pub fn clear(&self) {
        self.f().set_text("PasswBox", "");
        if self.flags.get() & PASSWENTER_FLAG_CONFIRMPASSW != 0 {
            self.f().set_text("ConfirmPasswBox", "");
        }
        self.f().set_item_index("KeyFileBox", Some(0));
    }

    /// `AutoClearPasswDlg::Clear()`.
    pub fn clear_and_flush(&self) {
        self.clear();
        RandomPool::global().flush();
    }
}
