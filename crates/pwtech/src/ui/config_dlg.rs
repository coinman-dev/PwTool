//! Configuration dialog (`TConfigurationDlg`, `upstream/src/main/
//! Configuration.cpp`) and `TMainForm::ApplyConfig()`. Child module of
//! `main_window`.

#![allow(deprecated)]

use std::path::Path;

use pwtech_core::language_file::LanguageFile;
use pwtech_core::random::pool::PoolCipherType;
use pwtech_core::text_file::CharEncoding;

use super::*;
use crate::app::LanguageEntry;
use crate::app_config::*;
use crate::ui::builder::{accel_to_shortcut, shortcut_to_text};
use crate::ui::util::FontSpec;

const CONFIG_ID: &str = "ConfigWin";

const RANDOM_POOL_CIPHER_NAMES: [&str; 3] = ["AES-CTR", "ChaCha20", "ChaCha8"];
const RANDOM_POOL_CIPHER_INFO: [(u32, u32); 3] = [(256, 128), (256, 512), (256, 512)];

/// Application icons (`AppIconNames`): display name, icon file.
pub const APP_ICON_NAMES: [(&str, &str); 6] = [
    ("Default (dark)", "pwtech_default.ico"),
    ("Blue", "pwtech_icon_blue.ico"),
    ("Red", "pwtech_icon_red.ico"),
    ("Orange", "pwtech_icon_orange.ico"),
    ("Green", "pwtech_icon_green.ico"),
    ("Colors", "pwtech_icon_colored.ico"),
];

/// User interface styles of the port ("Windows" of the original = default).
const UI_STYLES: [&str; 3] = ["Default", "Light", "Dark"];

pub(super) struct ConfigDlg {
    b: Built,
    win: gtk::Window,
    hot_keys: RefCell<HotKeyList>,
    hot_key: Cell<u16>,
    hot_key_list: gtk::ListBox,
    font: RefCell<String>,
    result: Cell<bool>,
    done: RefCell<Option<async_channel::Sender<()>>>,
    main: std::rc::Weak<MainWindow>,
}

impl ConfigDlg {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let b = build_form(load_form("Configuration"), "Configuration");
        let win = super::super::dialogs::form_window(&b, Some(&main.win()), true);
        win.set_hide_on_close(true);
        {
            let core = main.core.borrow();
            win.set_default_size(
                core.ini.read_int(CONFIG_ID, "WindowWidth", b.client.0).max(b.client.0),
                core.ini.read_int(CONFIG_ID, "WindowHeight", b.client.1).max(b.client.1),
            );
        }
        // fixed lists
        let combo = |n: &str| b.get::<gtk::ComboBoxText>(n);
        set_combo_items(&combo("UiStylesList"), &UI_STYLES.iter().map(|s| trl(s)).collect::<Vec<_>>());
        set_combo_items(&combo("AppIconList"), &APP_ICON_NAMES.iter().map(|(n, _)| trl(n)).collect::<Vec<_>>());
        let ciphers: Vec<String> = (0..3)
            .map(|i| {
                trl_format(
                    "%1 (%2-bit key, operates on %3-bit blocks)",
                    &[
                        RANDOM_POOL_CIPHER_NAMES[i],
                        &RANDOM_POOL_CIPHER_INFO[i].0.to_string(),
                        &RANDOM_POOL_CIPHER_INFO[i].1.to_string(),
                    ],
                )
            })
            .collect();
        set_combo_items(&combo("RandomPoolCipherList"), &ciphers);
        set_combo_items(&combo("BenchmarkMemList"), &(0..=10).map(|i| format!("{} MB", 1 << i)).collect::<Vec<_>>());
        combo("BenchmarkMemList").set_active(Some(6));
        for name in ["HotKeyActionsList", "AutoSaveList"] {
            let items: Vec<String> =
                b.obj(name).map(|o| o.strings("Items.Strings")).unwrap_or_default().iter().map(|s| trl(s)).collect();
            set_combo_items(&combo(name), &items);
        }
        for u in [("AutoClearClipTimeSpinBtn", AUTOCLEARCLIPTIME_MIN, AUTOCLEARCLIPTIME_MAX), ("AutoClearPasswTimeSpinBtn", AUTOCLEARPASSWTIME_MIN, AUTOCLEARPASSWTIME_MAX)] {
            if let Some(ud) = b.updowns.get(u.0) {
                ud.set_min(u.1 as i64);
                ud.set_max(u.2 as i64);
            }
        }
        b.get::<gtk::CheckButton>("UseAdvancedPasswEst")
            .set_label(Some(&trl_format("Use advanced password strength estimation (%1)", &["zxcvbn"])));
        // hot key list (TListView with two columns)
        let sw: gtk::ScrolledWindow = b.get("HotKeyView");
        let hot_key_list = gtk::ListBox::new();
        hot_key_list.set_selection_mode(gtk::SelectionMode::Multiple);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let cols: Vec<String> = b
            .obj("HotKeyView")
            .map(|o| {
                o.collection("Columns")
                    .iter()
                    .map(|c| c.get("Caption").and_then(|v| v.as_str()).unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        for i in 0..2 {
            let col = cols.get(i).cloned().unwrap_or_default();
            let l = gtk::Label::new(Some(&trl(&col)));
            l.set_xalign(0.0);
            l.add_css_class("heading");
            if i == 0 {
                l.set_width_chars(18);
            } else {
                l.set_hexpand(true);
            }
            l.set_margin_start(6);
            header.append(&l);
        }
        let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vb.append(&header);
        vb.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        vb.append(&hot_key_list);
        sw.set_child(Some(&vb));

        let d = Rc::new(ConfigDlg {
            b,
            win,
            hot_keys: RefCell::new(HotKeyList::new()),
            hot_key: Cell::new(0),
            hot_key_list,
            font: RefCell::new(String::new()),
            result: Cell::new(false),
            done: RefCell::new(None),
            main: Rc::downgrade(main),
        });
        d.connect();
        main.attach_entropy_events(&d.win);
        d
    }

    fn check(&self, n: &str) -> gtk::CheckButton {
        self.b.get(n)
    }

    fn combo(&self, n: &str) -> gtk::ComboBoxText {
        self.b.get(n)
    }

    fn ud(&self, n: &str) -> &Rc<crate::ui::builder::UpDown> {
        &self.b.updowns[n]
    }

    fn radio_index(&self, group: &str) -> i32 {
        for i in 0..16 {
            match self.b.try_get::<gtk::CheckButton>(&format!("{group}.item{i}")) {
                Some(c) if c.is_active() => return i,
                Some(_) => {}
                None => break,
            }
        }
        -1
    }

    fn set_radio_index(&self, group: &str, idx: i32) {
        if let Some(c) = self.b.try_get::<gtk::CheckButton>(&format!("{group}.item{idx}")) {
            c.set_active(true);
        }
    }

    fn enable(&self, names: &[&str], on: bool) {
        for n in names {
            if let Some(u) = self.b.updowns.get(*n) {
                u.set_sensitive(on);
            } else if let Some(w) = self.b.widgets.get(*n) {
                w.set_sensitive(on);
            }
        }
    }

    fn connect(self: &Rc<Self>) {
        let pairs: [(&str, &[&str]); 6] = [
            ("AutoClearClipCheck", &["AutoClearClipTimeBox", "AutoClearClipTimeSpinBtn"]),
            ("AutoClearPasswCheck", &["AutoClearPasswTimeBox", "AutoClearPasswTimeSpinBtn"]),
            ("LoadProfileStartupCheck", &["LoadProfileBox"]),
            ("LockIdleCheck", &["LockIdleTimeBox", "LockIdleTimeSpinBtn"]),
            ("AutoSaveCheck", &["AutoSaveList"]),
            ("WarnEntriesExpireSoonCheck", &[]),
        ];
        for (check, deps) in pairs {
            let d = Rc::downgrade(self);
            let deps: Vec<String> = deps.iter().map(|s| s.to_string()).collect();
            self.check(check).connect_toggled(move |c| {
                if let Some(d) = d.upgrade() {
                    let v: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
                    d.enable(&v, c.is_active());
                }
            });
        }
        let d = Rc::downgrade(self);
        let f = move || {
            if let Some(d) = d.upgrade() {
                let cb = d.check("CreateBackupCheck").is_active();
                d.check("NumberBackupsCheck").set_sensitive(cb);
                let on = cb && d.check("NumberBackupsCheck").is_active();
                d.enable(&["MaxNumBackupsBox", "MaxNumBackupsSpinBtn"], on);
            }
        };
        let f2 = f.clone();
        self.check("CreateBackupCheck").connect_toggled(move |_| f2());
        self.check("NumberBackupsCheck").connect_toggled(move |_| f());

        // OK / Cancel
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("OKBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                glib::spawn_future_local(async move { d.ok_clicked().await });
            }
        });
        let w = self.win.clone();
        self.b.get::<gtk::Button>("CancelBtn").connect_clicked(move |_| w.close());
        let d = Rc::downgrade(self);
        self.win.connect_close_request(move |_| {
            if let Some(d) = d.upgrade() {
                if let Some(tx) = d.done.borrow_mut().take() {
                    let _ = tx.try_send(());
                }
            }
            glib::Propagation::Proceed
        });

        // font selection (split button with "Restore default")
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("SelectFontBtn").connect_clicked(move |_| {
            let Some(d) = d.upgrade() else { return };
            let dlg = gtk::FontDialog::new();
            let cur = FontSpec::parse(&d.font.borrow()).map(|f| f.to_pango());
            let d2 = d.clone();
            dlg.choose_font(Some(&d.win), cur.as_ref(), gio::Cancellable::NONE, move |r| {
                if let Ok(desc) = r {
                    let f = FontSpec::from_pango(&desc);
                    *d2.font.borrow_mut() = f.to_font_string();
                    d2.show_font_sample();
                }
            });
        });
        if let Some(m) = self.b.menus.get("SelectFontMenu") {
            super::super::builder::attach_context_menu(&self.b.get::<gtk::Button>("SelectFontBtn"), m);
        }
        let d = Rc::downgrade(self);
        self.b.action("SelectFontMenu_RestoreDefault").connect_activate(move |_, _| {
            if let Some(d) = d.upgrade() {
                d.font.borrow_mut().clear();
                d.show_font_sample();
            }
        });

        // hot keys
        let hk: gtk::Entry = self.b.get("HotKeyBox");
        hk.set_editable(false);
        hk.set_text(&trl("None"));
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let d = Rc::downgrade(self);
        key.connect_key_pressed(move |_, k, _, mods| {
            let Some(d) = d.upgrade() else { return glib::Propagation::Proceed };
            if k == gdk::Key::Tab || k == gdk::Key::ISO_Left_Tab {
                return glib::Propagation::Proceed;
            }
            if k == gdk::Key::BackSpace && mods.is_empty() {
                d.set_hot_key(0);
                return glib::Propagation::Stop;
            }
            if let Some(code) = accel_to_shortcut(k, mods) {
                d.set_hot_key(code);
            }
            glib::Propagation::Stop
        });
        hk.add_controller(key);
        self.b.get::<gtk::Button>("AddBtn").set_sensitive(false);
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("AddBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                glib::spawn_future_local(async move { d.add_hot_key().await });
            }
        });
        self.b.get::<gtk::Button>("RemoveBtn").set_sensitive(false);
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("RemoveBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                let rows = d.hot_key_list.selected_rows();
                let mut keys = d.hot_keys.borrow_mut();
                for r in rows {
                    let k = r.widget_name().parse::<u16>().unwrap_or(0);
                    keys.remove(&k);
                }
                drop(keys);
                d.update_hot_key_list();
            }
        });
        let d = Rc::downgrade(self);
        self.hot_key_list.connect_selected_rows_changed(move |l| {
            if let Some(d) = d.upgrade() {
                d.b.w("RemoveBtn").set_sensitive(!l.selected_rows().is_empty());
            }
        });

        // security
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("BenchmarkBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.benchmark();
            }
        });

        // language
        let d = Rc::downgrade(self);
        self.combo("LanguageList").connect_changed(move |_| {
            if let Some(d) = d.upgrade() {
                d.language_list_select();
            }
        });
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("InstallLanguageBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.install_language();
            }
        });
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("RemoveLanguageBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                glib::spawn_future_local(async move { d.remove_language().await });
            }
        });
        let d = Rc::downgrade(self);
        self.b.get::<gtk::Button>("ConvertLangFileBtn").connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.convert_lang_file();
            }
        });
    }

    fn show_font_sample(&self) {
        let l: gtk::Label = self.b.get("FontSampleLbl");
        let f = FontSpec::parse(&self.font.borrow());
        match f {
            Some(f) => {
                l.set_text(&format!("{}, {}pt", f.name, f.size));
                let attrs = gtk::pango::AttrList::new();
                attrs.insert(gtk::pango::AttrFontDesc::new(&f.to_pango()));
                l.set_attributes(Some(&attrs));
            }
            None => {
                let desc = l.pango_context().font_description().unwrap_or_default();
                let f = FontSpec::from_pango(&desc);
                l.set_text(&format!("{}, {}pt", f.name, f.size));
                l.set_attributes(None);
            }
        }
    }

    fn set_hot_key(&self, code: u16) {
        self.hot_key.set(code);
        self.b
            .get::<gtk::Entry>("HotKeyBox")
            .set_text(&if code == 0 { trl("None") } else { shortcut_to_text(code) });
        self.b.w("AddBtn").set_sensitive(code != 0);
    }

    fn update_hot_key_list(&self) {
        while let Some(c) = self.hot_key_list.first_child() {
            self.hot_key_list.remove(&c);
        }
        let show_caption = super::super::builder::strip_mnemonic(&self.check("HotKeyShowMainWinCheck").label().unwrap_or_default());
        let actions = self.combo("HotKeyActionsList");
        let n_actions = actions.model().map(|m| m.iter_n_children(None)).unwrap_or(0);
        let action_names: Vec<String> = (0..n_actions)
            .map(|i| {
                let m = actions.model().unwrap();
                let it = m.iter_nth_child(None, i).unwrap();
                m.get::<String>(&it, 0)
            })
            .collect();
        for (k, e) in self.hot_keys.borrow().iter() {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&k.to_string());
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let l1 = gtk::Label::new(Some(&shortcut_to_text(*k)));
            l1.set_xalign(0.0);
            l1.set_width_chars(18);
            l1.set_margin_start(6);
            let mut action = String::new();
            if e.show_main_win {
                action = show_caption.clone();
            }
            if e.action != hot_key_action::NONE {
                if !action.is_empty() {
                    action.push_str(", ");
                }
                action.push_str(action_names.get(e.action as usize).map(|s| s.as_str()).unwrap_or(""));
            }
            let l2 = gtk::Label::new(Some(&action));
            l2.set_xalign(0.0);
            l2.set_hexpand(true);
            l2.set_margin_start(6);
            l2.set_ellipsize(gtk::pango::EllipsizeMode::End);
            bx.append(&l1);
            bx.append(&l2);
            row.set_child(Some(&bx));
            self.hot_key_list.append(&row);
        }
        self.b.w("RemoveBtn").set_sensitive(false);
    }

    async fn add_hot_key(&self) {
        if self.hot_keys.borrow().len() >= HOTKEYS_MAX_NUM {
            msg_box(
                Some(&self.win),
                &trl_format("Maximum number of hot keys (%1) reached.", &[&HOTKEYS_MAX_NUM.to_string()]),
                MsgIcon::Warning,
                MsgButtons::Ok,
                false,
            )
            .await;
            return;
        }
        let action = self.combo("HotKeyActionsList").active().map(|i| i as i32).unwrap_or(-1);
        let show = self.check("HotKeyShowMainWinCheck").is_active();
        if !show && action <= 0 {
            msg_box(Some(&self.win), &trl("Please select a hot key action."), MsgIcon::Warning, MsgButtons::Ok, false)
                .await;
            return;
        }
        let key = self.hot_key.get();
        let inserted = {
            let mut keys = self.hot_keys.borrow_mut();
            if keys.contains_key(&key) {
                false
            } else {
                keys.insert(key, HotKeyEntry { action: action.max(0), show_main_win: show });
                true
            }
        };
        if inserted {
            self.update_hot_key_list();
        } else {
            msg_box(Some(&self.win), &trl("This hot key has already been assigned."), MsgIcon::Warning, MsgButtons::Ok, false)
                .await;
        }
    }

    fn benchmark(&self) {
        let mb: usize = 1 << self.combo("BenchmarkMemList").active().unwrap_or(0).min(10);
        let mut buf = vec![0u8; mb << 20];
        let mut rp = RandomPool::new(PoolCipherType::AesCtr, None, false);
        rp.randomize_system();
        let mut result = String::new();
        for (i, name) in RANDOM_POOL_CIPHER_NAMES.iter().enumerate() {
            if let Some(c) = PoolCipherType::from_index(i as i32) {
                rp.set_cipher(c);
            }
            let t = std::time::Instant::now();
            rp.get_data(&mut buf);
            let rate = mb as f64 / t.elapsed().as_secs_f64();
            result.push_str(&format!("\n{name}: {rate:.2} MB/s"));
        }
        msg(
            Some(&self.win),
            &(trl_format("Benchmark results (data size: %1 MB):", &[&mb.to_string()]) + &result),
            MsgIcon::Info,
        );
    }

    fn languages(&self) -> Vec<LanguageEntry> {
        self.main.upgrade().map(|m| m.core.borrow().languages.clone()).unwrap_or_default()
    }

    fn load_languages(&self) {
        let items: Vec<String> = self.languages().iter().map(|e| format!("{} (v{})", e.name, e.version)).collect();
        set_combo_items(&self.combo("LanguageList"), &items);
    }

    fn language_list_select(&self) {
        let idx = self.combo("LanguageList").active().map(|i| i as usize).unwrap_or(0);
        let langs = self.languages();
        let lng = idx > 0
            && langs.get(idx).and_then(|e| e.file_name.as_ref()).is_some_and(|f| {
                f.extension().is_some_and(|e| e.eq_ignore_ascii_case("lng"))
            });
        self.b.w("ConvertLangFileBtn").set_sensitive(lng);
        self.b.w("RemoveLanguageBtn").set_sensitive(idx > 0);
    }

    fn lang_filters() -> gio::ListStore {
        let store = gio::ListStore::new::<gtk::FileFilter>();
        let f = gtk::FileFilter::new();
        f.set_name(Some(&format!("{} (*.po)", trl("Language files"))));
        f.add_pattern("*.po");
        store.append(&f);
        let f = gtk::FileFilter::new();
        f.set_name(Some(&format!("{} (*.*)", trl("All files"))));
        f.add_pattern("*");
        store.append(&f);
        store
    }

    fn install_language(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::new();
        dlg.set_filters(Some(&Self::lang_filters()));
        let d = self.clone();
        dlg.open(Some(&self.win), gio::Cancellable::NONE, move |r| {
            let Some(src) = r.ok().and_then(|f| f.path()) else { return };
            let m = match d.install_language_file(&src) {
                Ok(name) => {
                    msg(Some(&d.win), &trl_format("Language \"%1\" installed successfully.", &[&name]), MsgIcon::Info);
                    return;
                }
                Err(e) => e,
            };
            msg(Some(&d.win), &trl_format("Could not install language:\n%1.", &[&m]), MsgIcon::Error);
        });
    }

    fn install_language_file(&self, src: &Path) -> Result<String, String> {
        let main = self.main.upgrade().ok_or_else(String::new)?;
        let lf = LanguageFile::load(src, true).map_err(|e| e.to_string())?;
        if main.core.borrow().languages.iter().any(|l| l.code == lf.code) {
            return Err(trl("Language already installed"));
        }
        let data_dir = main.core.borrow().paths.data_dir.clone();
        let dest = data_dir.join(src.file_name().unwrap_or_default());
        if dest.exists() {
            return Err(trl_format("Language file already exists:\n\"%1\"", &[&dest.to_string_lossy()]));
        }
        let _ = std::fs::create_dir_all(&data_dir);
        std::fs::copy(src, &dest).map_err(|_| {
            trl_format("Could not copy file \"%1\" to\n\"%2\"", &[&src.to_string_lossy(), &dest.to_string_lossy()])
        })?;
        // deliberate fix: the original records the source file here (and
        // "Remove" would then delete the source)
        main.core.borrow_mut().languages.push(LanguageEntry {
            code: lf.code,
            name: lf.name.clone(),
            version: lf.version,
            file_name: Some(dest),
        });
        let idx = self.combo("LanguageList").active();
        self.load_languages();
        self.combo("LanguageList").set_active(idx);
        Ok(lf.name)
    }

    async fn remove_language(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let idx = self.combo("LanguageList").active().map(|i| i as usize).unwrap_or(0);
        let Some(e) = main.core.borrow().languages.get(idx).cloned() else { return };
        if idx == 0 {
            return;
        }
        if msg_box(
            Some(&self.win),
            &trl_format("Are you sure you want to remove\n\"%1\"?", &[&e.name]),
            MsgIcon::Question,
            MsgButtons::YesNo,
            true,
        )
        .await
            != MsgResult::Yes
        {
            return;
        }
        let file = e.file_name.clone().unwrap_or_default();
        if std::fs::remove_file(&file).is_ok() {
            {
                let mut core = main.core.borrow_mut();
                core.languages.remove(idx);
                if core.language == idx {
                    core.language = 0;
                } else if core.language > idx {
                    core.language -= 1;
                }
            }
            self.load_languages();
            self.combo("LanguageList").set_active(Some(0));
            msg_box(
                Some(&self.win),
                &trl_format("Language \"%1\" successfully removed.", &[&e.name]),
                MsgIcon::Info,
                MsgButtons::Ok,
                false,
            )
            .await;
        } else {
            msg_box(
                Some(&self.win),
                &trl_format("Could not delete file\n\"%1\".", &[&file.to_string_lossy()]),
                MsgIcon::Error,
                MsgButtons::Ok,
                false,
            )
            .await;
        }
    }

    fn convert_lang_file(self: &Rc<Self>) {
        let idx = self.combo("LanguageList").active().map(|i| i as usize).unwrap_or(0);
        let langs = self.languages();
        let Some(file) = langs.get(idx).and_then(|e| e.file_name.clone()) else { return };
        if idx == 0 {
            return;
        }
        let enc = self.main.upgrade().map(|m| m.core.borrow().config.file_encoding).unwrap_or(CharEncoding::Utf8);
        let dlg = gtk::FileDialog::new();
        let d = self.clone();
        dlg.save(Some(&self.win), gio::Cancellable::NONE, move |r| {
            let Some(dest) = r.ok().and_then(|f| f.path()) else { return };
            let res = LanguageFile::load(&file, false).and_then(|lf| {
                lf.save_to_po(
                    &dest,
                    if enc == CharEncoding::Ansi { CharEncoding::Utf8 } else { enc },
                    PROGRAM_NAME,
                    PROGRAM_VERSION,
                )
            });
            match res {
                Ok(()) => msg(Some(&d.win), "File successfully converted.", MsgIcon::Info),
                Err(e) => msg(Some(&d.win), &format!("{e}."), MsgIcon::Error),
            }
        });
    }

    /// `SetOptions()`.
    fn set_options(&self, c: &Configuration) {
        let main = self.main.upgrade();
        // profile list (`UpdateProfileList()`)
        if let Some(m) = &main {
            let names: Vec<String> = m.core.borrow().profiles.iter().map(|p| p.profile_name.clone()).collect();
            let cb = self.combo("LoadProfileBox");
            set_combo_items(&cb, &names);
            cb.set_active(names.iter().position(|n| *n == c.load_profile_name).map(|i| i as u32));
        }
        let style = UI_STYLES.iter().position(|s| *s == c.ui_style_name).unwrap_or(0);
        self.combo("UiStylesList").set_active(Some(style as u32));
        let icon = APP_ICON_NAMES.iter().position(|(n, _)| *n == c.app_icon_name).unwrap_or(0);
        self.combo("AppIconList").set_active(Some(icon as u32));
        *self.font.borrow_mut() = c.gui_font_string.clone();
        self.show_font_sample();
        self.combo("RandomPoolCipherList").set_active(Some(c.random_pool_cipher.clamp(0, 2) as u32));
        self.check("TestCommonPasswCheck").set_active(c.test_common_passw);
        self.check("UseAdvancedPasswEst").set_active(c.use_advanced_passw_est);
        self.check("AutoClearClipCheck").set_active(c.auto_clear_clip);
        self.ud("AutoClearClipTimeSpinBtn").set_position(c.auto_clear_clip_time as i64);
        self.enable(&["AutoClearClipTimeBox", "AutoClearClipTimeSpinBtn"], c.auto_clear_clip);
        self.check("AutoClearPasswCheck").set_active(c.auto_clear_passw);
        self.ud("AutoClearPasswTimeSpinBtn").set_position(c.auto_clear_passw_time as i64);
        self.enable(&["AutoClearPasswTimeBox", "AutoClearPasswTimeSpinBtn"], c.auto_clear_passw);
        self.check("MinimizeAutotypeCheck").set_active(c.minimize_autotype);
        self.ud("AutotypeDelaySpinBtn").set_position(c.autotype_delay as i64);
        self.check("ShowSysTrayIconConstCheck").set_active(c.show_sys_tray_icon_const);
        self.check("MinimizeToSysTrayCheck").set_active(c.minimize_to_sys_tray);
        self.check("AskBeforeExitCheck").set_active(c.confirm_exit);
        self.check("LoadProfileStartupCheck").set_active(c.load_profile_startup);
        self.enable(&["LoadProfileBox"], c.load_profile_startup);
        self.check("LaunchSystemStartupCheck").set_active(c.launch_system_startup);
        self.set_radio_index("UpdateCheckGroup", c.auto_check_updates as i32);
        self.set_radio_index("CharEncodingGroup", c.file_encoding as i32);
        self.set_radio_index("NewlineCharGroup", c.file_newline_char);
        self.load_languages();
        let lang = main
            .as_ref()
            .and_then(|m| m.core.borrow().languages.iter().position(|l| l.code == c.language_code))
            .unwrap_or(0);
        self.combo("LanguageList").set_active(Some(lang as u32));
        self.language_list_select();
        *self.hot_keys.borrow_mut() = c.hot_keys.clone();
        self.update_hot_key_list();
        self.set_hot_key(0);
        let db = &c.database;
        self.check("ClearClipCloseLockCheck").set_active(db.clear_clip_close_lock);
        self.check("LockMinimizeCheck").set_active(db.lock_minimize);
        self.check("LockIdleCheck").set_active(db.lock_idle);
        self.enable(&["LockIdleTimeBox", "LockIdleTimeSpinBtn"], db.lock_idle);
        self.ud("LockIdleTimeSpinBtn").set_position(db.lock_idle_time as i64);
        self.check("LockAutoSaveCheck").set_active(db.lock_auto_save);
        self.check("CreateBackupCheck").set_active(db.create_backup);
        self.check("NumberBackupsCheck").set_active(db.number_backups);
        self.check("NumberBackupsCheck").set_sensitive(db.create_backup);
        self.enable(&["MaxNumBackupsBox", "MaxNumBackupsSpinBtn"], db.create_backup && db.number_backups);
        self.ud("MaxNumBackupsSpinBtn").set_position(db.max_num_backups as i64);
        self.check("OpenWindowOnStartupCheck").set_active(db.open_window_on_startup);
        self.check("OpenDbOnStartupCheck").set_active(db.open_last_db_on_startup);
        self.check("KeepRecentFilesCheck").set_active(db.keep_recent_files);
        self.check("WarnExpiredEntriesCheck").set_active(db.warn_expired_entries);
        self.check("WarnEntriesExpireSoonCheck").set_active(db.warn_entries_expire_soon);
        self.ud("WarnExpireNumDaysSpinBtn").set_position(db.warn_expire_num_days as i64);
        self.check("AutoSaveCheck").set_active(db.auto_save);
        self.combo("AutoSaveList").set_active(Some(db.auto_save_option.clamp(0, 1) as u32));
        self.enable(&["AutoSaveList"], db.auto_save);
        self.b.get::<gtk::Entry>("DefaultAutotypeSeqBox").set_text(&db.default_autotype_sequence);
    }

    /// `GetOptions()`.
    fn get_options(&self, old: &Configuration) -> Configuration {
        let mut c = old.clone();
        c.ui_style_name = UI_STYLES[self.combo("UiStylesList").active().unwrap_or(0).min(2) as usize].to_string();
        c.app_icon_name = APP_ICON_NAMES[self.combo("AppIconList").active().unwrap_or(0).min(5) as usize].0.to_string();
        c.gui_font_string = self.font.borrow().clone();
        c.auto_clear_clip = self.check("AutoClearClipCheck").is_active();
        c.auto_clear_clip_time = self.ud("AutoClearClipTimeSpinBtn").position() as i32;
        c.auto_clear_passw = self.check("AutoClearPasswCheck").is_active();
        c.auto_clear_passw_time = self.ud("AutoClearPasswTimeSpinBtn").position() as i32;
        c.minimize_autotype = self.check("MinimizeAutotypeCheck").is_active();
        c.autotype_delay = self.ud("AutotypeDelaySpinBtn").position() as i32;
        c.confirm_exit = self.check("AskBeforeExitCheck").is_active();
        c.load_profile_startup = self.check("LoadProfileStartupCheck").is_active();
        c.load_profile_name = combo_text(&self.combo("LoadProfileBox"));
        c.launch_system_startup = self.check("LaunchSystemStartupCheck").is_active();
        c.random_pool_cipher = self.combo("RandomPoolCipherList").active().map(|i| i as i32).unwrap_or(1);
        c.test_common_passw = self.check("TestCommonPasswCheck").is_active();
        c.use_advanced_passw_est = self.check("UseAdvancedPasswEst").is_active();
        c.show_sys_tray_icon_const = self.check("ShowSysTrayIconConstCheck").is_active();
        c.minimize_to_sys_tray = self.check("MinimizeToSysTrayCheck").is_active();
        c.auto_check_updates =
            AutoCheckUpdates::from_index(self.radio_index("UpdateCheckGroup")).unwrap_or(AutoCheckUpdates::Weekly);
        c.file_encoding = CharEncoding::from_index(self.radio_index("CharEncodingGroup")).unwrap_or(CharEncoding::Utf8);
        c.file_newline_char = self.radio_index("NewlineCharGroup").clamp(0, 1);
        c.hot_keys = self.hot_keys.borrow().clone();
        let idx = self.combo("LanguageList").active().map(|i| i as usize).unwrap_or(0);
        if let Some(l) = self.languages().get(idx) {
            c.language_code = l.code.clone();
        }
        let db = &mut c.database;
        db.clear_clip_close_lock = self.check("ClearClipCloseLockCheck").is_active();
        db.lock_minimize = self.check("LockMinimizeCheck").is_active();
        db.lock_idle = self.check("LockIdleCheck").is_active();
        db.lock_idle_time = self.ud("LockIdleTimeSpinBtn").position() as i32;
        db.lock_auto_save = self.check("LockAutoSaveCheck").is_active();
        db.create_backup = self.check("CreateBackupCheck").is_active();
        db.number_backups = self.check("NumberBackupsCheck").is_active();
        db.max_num_backups = self.ud("MaxNumBackupsSpinBtn").position() as i32;
        db.open_window_on_startup = self.check("OpenWindowOnStartupCheck").is_active();
        db.open_last_db_on_startup = self.check("OpenDbOnStartupCheck").is_active();
        db.warn_expired_entries = self.check("WarnExpiredEntriesCheck").is_active();
        db.warn_entries_expire_soon = self.check("WarnEntriesExpireSoonCheck").is_active();
        db.warn_expire_num_days = self.ud("WarnExpireNumDaysSpinBtn").position() as i32;
        db.auto_save = self.check("AutoSaveCheck").is_active();
        db.auto_save_option = self.combo("AutoSaveList").active().map(|i| i as i32).unwrap_or(0);
        db.default_autotype_sequence = self.b.get::<gtk::Entry>("DefaultAutotypeSeqBox").text().to_string();
        db.keep_recent_files = self.check("KeepRecentFilesCheck").is_active();
        c
    }

    async fn ok_clicked(self: &Rc<Self>) {
        let Some(main) = self.main.upgrade() else { return };
        let old = main.core.borrow().config.clone();
        let c = self.get_options(&old);
        if main.apply_config(c, &self.win).await {
            self.result.set(true);
            self.win.close();
        }
    }

    fn save_config(&self, ini: &mut pwtech_core::config::IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.default_height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.default_width() as i64);
    }
}

impl MainWindow {
    /// `MainMenu_Options_ConfigClick()`.
    pub(super) async fn configuration(self: &Rc<Self>) {
        let existing = self.config_dlg.borrow().clone();
        let dlg = match existing {
            Some(d) => d,
            None => {
                let d = ConfigDlg::new(self);
                *self.config_dlg.borrow_mut() = Some(d.clone());
                d
            }
        };
        let config = self.core.borrow().config.clone();
        dlg.set_options(&config);
        if let Some(nb) = dlg.b.try_get::<gtk::Notebook>("ConfigPages") {
            nb.set_current_page(Some(0));
        }
        dlg.result.set(false);
        let (tx, rx) = async_channel::bounded(1);
        *dlg.done.borrow_mut() = Some(tx);
        dlg.win.set_default_widget(Some(&dlg.b.get::<gtk::Button>("OKBtn")));
        dlg.win.present();
        let _ = rx.recv().await;
        if dlg.result.get() && crate::app::restart_requested() {
            self.window.close();
        }
    }

    pub(super) fn save_config_dlg_config(&self, ini: &mut pwtech_core::config::IniFile) {
        if let Some(d) = self.config_dlg.borrow().as_ref() {
            d.save_config(ini);
        }
    }

    /// `ApplyConfig()`: returns false if the configuration was rejected.
    pub(super) async fn apply_config(self: &Rc<Self>, c: Configuration, parent: &gtk::Window) -> bool {
        let old = self.core.borrow().config.clone();
        let mut lang_changed = false;
        if c.language_code != old.language_code {
            let version = self
                .core
                .borrow()
                .languages
                .iter()
                .find(|l| l.code == c.language_code)
                .map(|l| l.version.clone())
                .unwrap_or_default();
            if crate::app::compare_versions(&version, PROGRAM_LANGVER_MIN) < 0 {
                if msg_box(
                    Some(parent),
                    &trl_format(
                        "The version of this language (%1) is not\ncompatible with this program version.\nThis version of %2 requires a language\nversion of at least %3.\n\nDo you want to use it anyway?",
                        &[&version, PROGRAM_NAME, PROGRAM_LANGVER_MIN],
                    ),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    == MsgResult::No
                {
                    return false;
                }
            } else if crate::app::compare_versions(&version, PROGRAM_VERSION) > 0
                && msg_box(
                    Some(parent),
                    &trl_format(
                        "The version of this language (%1) is higher\nthan the version of this program (%2).\nTherefore, version incompatibilities may occur.\n\nDo you want to use it anyway?",
                        &[&version, PROGRAM_VERSION],
                    ),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    == MsgResult::No
            {
                return false;
            }
            lang_changed = true;
        }

        if c.hot_keys != old.hot_keys {
            self.activate_hot_keys(&c.hot_keys);
        }
        if c.gui_font_string != old.gui_font_string {
            super::super::util::set_gui_font(&c.gui_font_string);
        }
        if c.ui_style_name != old.ui_style_name {
            super::super::util::set_ui_style(&c.ui_style_name);
        }
        if c.launch_system_startup != old.launch_system_startup {
            if let Err(e) = set_autostart(c.launch_system_startup) {
                msg_box(
                    Some(parent),
                    &trl_format("Could not access registry key\n%1.", &[&e]),
                    MsgIcon::Error,
                    MsgButtons::Ok,
                    false,
                )
                .await;
            }
        }
        if let Some(cipher) = PoolCipherType::from_index(c.random_pool_cipher) {
            RandomPool::global().set_cipher(cipher);
        }
        if !c.auto_clear_clip {
            self.auto_clear_clip_cnt.set(0);
        }
        let style_changed = false;
        {
            let mut core = self.core.borrow_mut();
            core.file_encoding = c.file_encoding;
            core.newline = c.newline().to_string();
            core.use_advanced_passw_est = c.use_advanced_passw_est;
            if c.test_common_passw != core.test_common_passw {
                core.test_common_passw = c.test_common_passw;
                if c.test_common_passw && core.common_passw.is_none() {
                    core.load_common_passwords();
                }
            }
            core.config = c;
        }
        self.update_tray_icon();
        if (lang_changed || style_changed)
            && msg_box(
                Some(parent),
                &trl("The program has to be restarted in order\nfor the changes to take effect.\n\nDo you want to restart now?"),
                MsgIcon::Question,
                MsgButtons::YesNo,
                false,
            )
            .await
                == MsgResult::Yes
        {
            crate::app::request_restart();
        }
        true
    }

    /// `ActivateHotKeys()`: registers the global hot keys (see `hotkeys`).
    pub(super) fn activate_hot_keys(&self, keys: &HotKeyList) {
        *self.hot_keys_active.borrow_mut() = keys.clone();
    }

    /// Show or hide the tray icon according to the configuration.
    pub(super) fn update_tray_icon(&self) {}
}

/// Autostart entry (`LaunchSystemStartup`): `~/.config/autostart/
/// pwtech.desktop` starting the program with `--silent` (the original
/// writes the Run key of the registry).
pub fn set_autostart(enable: bool) -> Result<(), String> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .ok_or_else(|| "~/.config/autostart".to_string())?
        .join("autostart");
    let file = dir.join("pwtech.desktop");
    if enable {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|_| dir.to_string_lossy().into_owned())?;
        let content = format!(
            "[Desktop Entry]\nType=Application\nName={PROGRAM_NAME}\nExec=\"{}\" --silent\nIcon={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=false\n",
            exe.to_string_lossy(),
            super::super::APP_ID
        );
        std::fs::write(&file, content).map_err(|_| file.to_string_lossy().into_owned())
    } else if file.exists() {
        std::fs::remove_file(&file).map_err(|_| file.to_string_lossy().into_owned())
    } else {
        Ok(())
    }
}
