//! Configuration dialog (`TConfigurationDlg`, `upstream/src/main/
//! Configuration.cpp`) and `TMainForm::ApplyConfig()`. Child module of
//! `main_window`.

use std::path::Path;

use pwtool_app::app::LanguageEntry;
use pwtool_app::app_config::*;
use pwtool_app::shortcut::shortcut_to_text;
use pwtool_core::language_file::LanguageFile;
use pwtool_core::random::pool::PoolCipherType;
use pwtool_core::text_file::CharEncoding;

use super::*;
use crate::vcl::fonts::FontSpec;

const WINDOW_ID: &str = "ConfigWin";

const RANDOM_POOL_CIPHER_NAMES: [&str; 3] = ["AES-CTR", "ChaCha20", "ChaCha8"];
const RANDOM_POOL_CIPHER_INFO: [(u32, u32); 3] = [(256, 128), (256, 512), (256, 512)];

/// Application icons (`AppIconNames`): display name, icon file.
pub const APP_ICON_NAMES: [(&str, &str); 6] = [
    ("Default (dark)", "pwtool_default.ico"),
    ("Blue", "pwtool_icon_blue.ico"),
    ("Red", "pwtool_icon_red.ico"),
    ("Orange", "pwtool_icon_orange.ico"),
    ("Green", "pwtool_icon_green.ico"),
    ("Colors", "pwtool_icon_colored.ico"),
];

/// User interface styles of the port ("Windows" of the original = default).
pub const UI_STYLES: [&str; 3] = ["Default", "Light", "Dark"];

fn win_id() -> ViewportId {
    ViewportId::from_hash_of(WINDOW_ID)
}

/// Rows of the hot key list (`HotKeyView`) and their selection.
#[derive(Default)]
struct HotKeyRows {
    rows: Vec<(u16, String, String)>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    changed: bool,
}

pub(super) struct ConfigDlg {
    form: Rc<Form>,
    hot_keys: RefCell<HotKeyList>,
    hot_key: Cell<u16>,
    rows: Rc<RefCell<HotKeyRows>>,
    font: RefCell<String>,
    result: Cell<bool>,
    done: RefCell<Option<async_channel::Sender<()>>>,
    main: Weak<MainWindow>,
}

/// The dialog's window content: the form plus the hot key capture.
struct ConfigContent(Rc<ConfigDlg>);

impl Content for ConfigContent {
    fn ui(&self, ui: &mut Ui) {
        let d = &self.0;
        Content::ui(&*d.form, ui);
        if d.form.has_focus("HotKeyBox") {
            let keys: Vec<(egui::Key, egui::Modifiers)> = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                        _ => None,
                    })
                    .collect()
            });
            for (key, mods) in keys {
                if key == egui::Key::Tab {
                    continue;
                }
                if key == egui::Key::Backspace && mods.is_none() {
                    d.set_hot_key(0);
                    continue;
                }
                if let Some(vk) = crate::vcl::render::key_to_vk(key) {
                    let mut code = vk;
                    if mods.shift {
                        code |= 0x2000;
                    }
                    if mods.ctrl {
                        code |= 0x4000;
                    }
                    if mods.alt {
                        code |= 0x8000;
                    }
                    d.set_hot_key(code);
                }
            }
        }
        let changed = std::mem::take(&mut d.rows.borrow_mut().changed);
        if changed {
            let any = d.rows.borrow().selected.iter().any(|x| *x);
            d.form.set_enabled("RemoveBtn", any);
        }
    }
}

impl ConfigDlg {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let form = dialogs::load("Configuration");
        let f = &form;
        f.set_items("UiStylesList", UI_STYLES.iter().map(|s| trl(s)).collect());
        f.set_items("AppIconList", APP_ICON_NAMES.iter().map(|(n, _)| trl(n)).collect());
        let ciphers: Vec<String> = (0..3)
            .map(|i| {
                trl_format(
                    "%1 (%2-bit key, operates on %3-bit blocks)",
                    &[RANDOM_POOL_CIPHER_NAMES[i], &RANDOM_POOL_CIPHER_INFO[i].0.to_string(), &RANDOM_POOL_CIPHER_INFO[i].1.to_string()],
                )
            })
            .collect();
        f.set_items("RandomPoolCipherList", ciphers);
        f.set_items("BenchmarkMemList", (0..=10).map(|i| format!("{} MB", 1 << i)).collect());
        f.set_item_index("BenchmarkMemList", Some(6));
        for name in ["HotKeyActionsList", "AutoSaveList"] {
            let items: Vec<String> = f.items(name).iter().map(|s| trl(s)).collect();
            f.set_items(name, items);
        }
        f.set_range("AutoClearClipTimeSpinBtn", AUTOCLEARCLIPTIME_MIN as i64, AUTOCLEARCLIPTIME_MAX as i64);
        f.set_range("AutoClearPasswTimeSpinBtn", AUTOCLEARPASSWTIME_MIN as i64, AUTOCLEARPASSWTIME_MAX as i64);
        f.set_control_caption("UseAdvancedPasswEst", &trl_format("Use advanced password strength estimation (%1)", &["zxcvbn"]));
        f.set_read_only("HotKeyBox", true);
        f.set_popup_menu("SelectFontBtn", "SelectFontMenu");
        let d = Rc::new(ConfigDlg {
            form,
            hot_keys: RefCell::new(HotKeyList::new()),
            hot_key: Cell::new(0),
            rows: Rc::new(RefCell::new(HotKeyRows::default())),
            font: RefCell::new(String::new()),
            result: Cell::new(false),
            done: RefCell::new(None),
            main: Rc::downgrade(main),
        });
        d.setup_hot_key_view();
        d.connect();
        d
    }

    fn f(&self) -> &Form {
        &self.form
    }

    fn enable(&self, names: &[&str], on: bool) {
        for n in names {
            self.f().set_enabled(n, on);
        }
    }

    /// `HotKeyView`: a list with the columns "Shortcut" and "Action".
    fn setup_hot_key_view(&self) {
        let cols: Vec<(String, i64)> = self
            .f()
            .obj
            .find("HotKeyView")
            .map(|o| {
                o.collection("Columns")
                    .iter()
                    .map(|c| {
                        (
                            trl(c.get("Caption").and_then(|v| v.as_str()).unwrap_or("")),
                            c.get("Width").and_then(|v| v.as_i64()).unwrap_or(120),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let scale = self.f().scale();
        let rows = self.rows.clone();
        self.f().set_painter("HotKeyView", move |ui, rect| {
            let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
            ui.painter().rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
            ui.painter().rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
            let w0 = cols.first().map_or(125.0, |c| c.1 as f32 * scale);
            let inner = rect.shrink(2.0);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
            let ui = &mut child;
            ui.set_clip_rect(inner);
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
            // header
            let head = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), row_h));
            ui.painter().rect_filled(head, 0.0, ui.visuals().faint_bg_color);
            let font = egui::TextStyle::Body.resolve(ui.style());
            let color = ui.visuals().text_color();
            for (i, (c, _)) in cols.iter().take(2).enumerate() {
                let x = if i == 0 { head.min.x + 4.0 } else { head.min.x + w0 + 4.0 };
                ui.painter().text(egui::pos2(x, head.center().y), egui::Align2::LEFT_CENTER, c, font.clone(), color);
            }
            ui.painter().vline(head.min.x + w0, head.y_range(), stroke);
            ui.painter().hline(head.x_range(), head.max.y, stroke);
            let body = egui::Rect::from_min_max(egui::pos2(inner.min.x, head.max.y + 1.0), inner.max);
            let mut r = rows.borrow_mut();
            let HotKeyRows { rows, selected, anchor, changed } = &mut *r;
            let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            egui::ScrollArea::vertical().id_salt("hotkeys").auto_shrink(false).show(&mut body_ui, |ui| {
                for (i, (_, key, action)) in rows.iter().enumerate() {
                    let (rr, resp) = ui.allocate_exact_size(egui::vec2(body.width(), row_h), egui::Sense::click());
                    if selected[i] {
                        ui.painter().rect_filled(rr, 0.0, ui.visuals().selection.bg_fill);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rr, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
                    }
                    let tc = if selected[i] { ui.visuals().selection.stroke.color } else { color };
                    ui.painter().text(egui::pos2(rr.min.x + 4.0, rr.center().y), egui::Align2::LEFT_CENTER, key, font.clone(), tc);
                    let clip = egui::Rect::from_min_max(egui::pos2(rr.min.x + w0, rr.min.y), rr.max);
                    ui.painter().with_clip_rect(clip).text(egui::pos2(rr.min.x + w0 + 4.0, rr.center().y), egui::Align2::LEFT_CENTER, action, font.clone(), tc);
                    if resp.clicked() {
                        let mods = ui.input(|i| i.modifiers);
                        if mods.command {
                            selected[i] = !selected[i];
                        } else if let (true, Some(a)) = (mods.shift, *anchor) {
                            let (lo, hi) = (a.min(i), a.max(i));
                            for (j, s) in selected.iter_mut().enumerate() {
                                *s = (lo..=hi).contains(&j);
                            }
                        } else {
                            selected.iter_mut().for_each(|s| *s = false);
                            selected[i] = true;
                        }
                        if !mods.shift {
                            *anchor = Some(i);
                        }
                        *changed = true;
                    }
                }
            });
        });
    }

    fn connect(self: &Rc<Self>) {
        let dd = Rc::downgrade(self);
        self.form.on_any(move |e| {
            let Some(d) = dd.upgrade() else { return };
            use crate::vcl::Event::*;
            match e {
                Click(n) => d.click(n),
                Change(n) if n == "LanguageList" => d.language_list_select(),
                Menu(n) if n == "SelectFontMenu_RestoreDefault" => {
                    d.font.borrow_mut().clear();
                    d.show_font_sample();
                }
                _ => {}
            }
        });
    }

    fn click(self: &Rc<Self>, n: &str) {
        let pairs: [(&str, &[&str]); 5] = [
            ("AutoClearClipCheck", &["AutoClearClipTimeBox", "AutoClearClipTimeSpinBtn"]),
            ("AutoClearPasswCheck", &["AutoClearPasswTimeBox", "AutoClearPasswTimeSpinBtn"]),
            ("LoadProfileStartupCheck", &["LoadProfileBox"]),
            ("LockIdleCheck", &["LockIdleTimeBox", "LockIdleTimeSpinBtn"]),
            ("AutoSaveCheck", &["AutoSaveList"]),
        ];
        if let Some((c, deps)) = pairs.iter().find(|(c, _)| *c == n) {
            self.enable(deps, self.f().checked(c));
            return;
        }
        match n {
            "CreateBackupCheck" | "NumberBackupsCheck" => self.backup_checks(),
            "OKBtn" => {
                let d = self.clone();
                spawn(async move { d.ok_clicked().await });
            }
            "CancelBtn" => self.close(),
            "SelectFontBtn" => {
                let d = self.clone();
                spawn(async move {
                    let cur = FontSpec::parse(&d.font.borrow());
                    if let Some(f) = crate::ui::font_dialog::choose_font(win_id(), cur.as_ref(), false).await {
                        *d.font.borrow_mut() = f.to_font_string();
                        d.show_font_sample();
                    }
                });
            }
            "DefaultFontBtn" => {
                self.font.borrow_mut().clear();
                self.show_font_sample();
            }
            "AddBtn" => {
                let d = self.clone();
                spawn(async move { d.add_hot_key().await });
            }
            "RemoveBtn" => {
                {
                    let r = self.rows.borrow();
                    let mut keys = self.hot_keys.borrow_mut();
                    for (i, (k, _, _)) in r.rows.iter().enumerate() {
                        if r.selected[i] {
                            keys.remove(k);
                        }
                    }
                }
                self.update_hot_key_list();
            }
            "BenchmarkBtn" => self.benchmark(),
            "InstallLanguageBtn" => {
                let d = self.clone();
                spawn(async move { d.install_language().await });
            }
            "RemoveLanguageBtn" => {
                let d = self.clone();
                spawn(async move { d.remove_language().await });
            }
            "ConvertLangFileBtn" => {
                let d = self.clone();
                spawn(async move { d.convert_lang_file().await });
            }
            _ => {}
        }
    }

    fn backup_checks(&self) {
        let cb = self.f().checked("CreateBackupCheck");
        self.f().set_enabled("NumberBackupsCheck", cb);
        let on = cb && self.f().checked("NumberBackupsCheck");
        self.enable(&["MaxNumBackupsBox", "MaxNumBackupsSpinBtn"], on);
    }

    fn close(&self) {
        if let Some(m) = self.main.upgrade() {
            m.close_tool(WINDOW_ID);
        }
        if let Some(tx) = self.done.borrow_mut().take() {
            let _ = tx.try_send(());
        }
    }

    fn show_font_sample(&self) {
        let text = match FontSpec::parse(&self.font.borrow()) {
            Some(f) => format!("{}, {}pt", f.name, f.size.abs()),
            None => {
                let size = (crate::vcl::fonts::DEFAULT_SIZE * 0.75).round();
                format!("{}, {size}pt", trl("Default"))
            }
        };
        self.f().set_text("FontSampleLbl", &text);
    }

    fn set_hot_key(&self, code: u16) {
        self.hot_key.set(code);
        self.f().set_text("HotKeyBox", &if code == 0 { trl("None") } else { shortcut_to_text(code) });
        self.f().set_enabled("AddBtn", code != 0);
    }

    fn update_hot_key_list(&self) {
        let show_caption = crate::vcl::render::strip_amp(&self.f().control_caption("HotKeyShowMainWinCheck"));
        let action_names = self.f().items("HotKeyActionsList");
        let rows: Vec<(u16, String, String)> = self
            .hot_keys
            .borrow()
            .iter()
            .map(|(k, e)| {
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
                (*k, shortcut_to_text(*k), action)
            })
            .collect();
        let mut r = self.rows.borrow_mut();
        r.selected = vec![false; rows.len()];
        r.rows = rows;
        r.anchor = None;
        r.changed = false;
        drop(r);
        self.f().set_enabled("RemoveBtn", false);
    }

    async fn add_hot_key(&self) {
        if self.hot_keys.borrow().len() >= HOTKEYS_MAX_NUM {
            dialogs::ask(win_id(), &trl_format("Maximum number of hot keys (%1) reached.", &[&HOTKEYS_MAX_NUM.to_string()]), MsgIcon::Warning, MsgButtons::Ok, false)
                .await;
            return;
        }
        let action = self.f().item_index("HotKeyActionsList").map(|i| i as i32).unwrap_or(-1);
        let show = self.f().checked("HotKeyShowMainWinCheck");
        if !show && action <= 0 {
            dialogs::ask(win_id(), &trl("Please select a hot key action."), MsgIcon::Warning, MsgButtons::Ok, false).await;
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
            dialogs::ask(win_id(), &trl("This hot key has already been assigned."), MsgIcon::Warning, MsgButtons::Ok, false).await;
        }
    }

    fn benchmark(&self) {
        let mb: usize = 1 << self.f().item_index("BenchmarkMemList").unwrap_or(0).min(10);
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
        msg(win_id(), &(trl_format("Benchmark results (data size: %1 MB):", &[&mb.to_string()]) + &result), MsgIcon::Info);
    }

    fn languages(&self) -> Vec<LanguageEntry> {
        self.main.upgrade().map(|m| m.core.borrow().languages.clone()).unwrap_or_default()
    }

    fn load_languages(&self) {
        // names only, without the version of the language file
        let items: Vec<String> = self.languages().iter().map(|e| e.name.clone()).collect();
        self.f().set_items("LanguageList", items);
    }

    fn language_list_select(&self) {
        let idx = self.f().item_index("LanguageList").unwrap_or(0);
        let langs = self.languages();
        let lng = idx > 0
            && langs.get(idx).and_then(|e| e.file_name.as_ref()).is_some_and(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("lng")));
        self.f().set_enabled("ConvertLangFileBtn", lng);
        self.f().set_enabled("RemoveLanguageBtn", idx > 0);
    }

    fn lang_filters() -> [(String, &'static [&'static str]); 2] {
        [(format!("{} (*.po)", trl("Language files")), &["po"]), (format!("{} (*.*)", trl("All files")), &["*"])]
    }

    async fn install_language(&self) {
        let fl = Self::lang_filters();
        let filters: Vec<(&str, &[&str])> = fl.iter().map(|(n, e)| (n.as_str(), *e)).collect();
        let Some(src) = crate::vcl::dialogs::open_file(&trl("Select language file to install"), None, &filters).await else { return };
        match self.install_language_file(&src) {
            Ok(name) => msg(win_id(), &trl_format("Language \"%1\" installed successfully.", &[&name]), MsgIcon::Info),
            Err(e) => msg(win_id(), &trl_format("Could not install language:\n%1.", &[&e]), MsgIcon::Error),
        }
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
        std::fs::copy(src, &dest).map_err(|_| trl_format("Could not copy file \"%1\" to\n\"%2\"", &[&src.to_string_lossy(), &dest.to_string_lossy()]))?;
        // deliberate fix: the original records the source file here (and
        // "Remove" would then delete the source)
        main.core.borrow_mut().languages.push(LanguageEntry { code: lf.code, name: lf.name.clone(), version: lf.version, file_name: Some(dest) });
        let idx = self.f().item_index("LanguageList");
        self.load_languages();
        self.f().set_item_index("LanguageList", idx);
        Ok(lf.name)
    }

    async fn remove_language(&self) {
        let Some(main) = self.main.upgrade() else { return };
        let idx = self.f().item_index("LanguageList").unwrap_or(0);
        let Some(e) = main.core.borrow().languages.get(idx).cloned() else { return };
        if idx == 0 {
            return;
        }
        if dialogs::ask(win_id(), &trl_format("Are you sure you want to remove\n\"%1\"?", &[&e.name]), MsgIcon::Question, MsgButtons::YesNo, true).await
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
            self.f().set_item_index("LanguageList", Some(0));
            self.language_list_select();
            dialogs::ask(win_id(), &trl_format("Language \"%1\" successfully removed.", &[&e.name]), MsgIcon::Info, MsgButtons::Ok, false).await;
        } else {
            dialogs::ask(win_id(), &trl_format("Could not delete file\n\"%1\".", &[&file.to_string_lossy()]), MsgIcon::Error, MsgButtons::Ok, false).await;
        }
    }

    async fn convert_lang_file(&self) {
        let idx = self.f().item_index("LanguageList").unwrap_or(0);
        let langs = self.languages();
        let Some(file) = langs.get(idx).and_then(|e| e.file_name.clone()) else { return };
        if idx == 0 {
            return;
        }
        let enc = self.main.upgrade().map(|m| m.core.borrow().config.file_encoding).unwrap_or(CharEncoding::Utf8);
        let fl = Self::lang_filters();
        let filters: Vec<(&str, &[&str])> = fl.iter().map(|(n, e)| (n.as_str(), *e)).collect();
        let Some(dest) = crate::vcl::dialogs::save_file(&trl("Save converted language file"), None, None, &filters).await else { return };
        let res = LanguageFile::load(&file, false)
            .and_then(|lf| lf.save_to_po(&dest, if enc == CharEncoding::Ansi { CharEncoding::Utf8 } else { enc }, PROGRAM_NAME, PROGRAM_VERSION));
        match res {
            Ok(()) => msg(win_id(), "File successfully converted.", MsgIcon::Info),
            Err(e) => msg(win_id(), &format!("{e}."), MsgIcon::Error),
        }
    }

    /// `SetOptions()`.
    fn set_options(&self, c: &Configuration) {
        let main = self.main.upgrade();
        let f = self.f();
        // profile list (`UpdateProfileList()`)
        if let Some(m) = &main {
            let names: Vec<String> = m.core.borrow().profiles.iter().map(|p| p.profile_name.clone()).collect();
            f.set_items("LoadProfileBox", names);
            f.set_text("LoadProfileBox", &c.load_profile_name);
        }
        f.set_item_index("UiStylesList", Some(UI_STYLES.iter().position(|s| *s == c.ui_style_name).unwrap_or(0)));
        f.set_item_index("AppIconList", Some(APP_ICON_NAMES.iter().position(|(n, _)| *n == c.app_icon_name).unwrap_or(0)));
        *self.font.borrow_mut() = c.gui_font_string.clone();
        self.show_font_sample();
        f.set_item_index("RandomPoolCipherList", Some(c.random_pool_cipher.clamp(0, 2) as usize));
        f.set_checked("TestCommonPasswCheck", c.test_common_passw);
        f.set_checked("UseAdvancedPasswEst", c.use_advanced_passw_est);
        f.set_checked("AutoClearClipCheck", c.auto_clear_clip);
        f.set_position("AutoClearClipTimeSpinBtn", c.auto_clear_clip_time as i64);
        self.enable(&["AutoClearClipTimeBox", "AutoClearClipTimeSpinBtn"], c.auto_clear_clip);
        f.set_checked("AutoClearPasswCheck", c.auto_clear_passw);
        f.set_position("AutoClearPasswTimeSpinBtn", c.auto_clear_passw_time as i64);
        self.enable(&["AutoClearPasswTimeBox", "AutoClearPasswTimeSpinBtn"], c.auto_clear_passw);
        f.set_checked("MinimizeAutotypeCheck", c.minimize_autotype);
        f.set_position("AutotypeDelaySpinBtn", c.autotype_delay as i64);
        f.set_checked("ShowSysTrayIconConstCheck", c.show_sys_tray_icon_const);
        f.set_checked("MinimizeToSysTrayCheck", c.minimize_to_sys_tray);
        f.set_checked("AskBeforeExitCheck", c.confirm_exit);
        f.set_checked("LoadProfileStartupCheck", c.load_profile_startup);
        self.enable(&["LoadProfileBox"], c.load_profile_startup);
        f.set_checked("LaunchSystemStartupCheck", c.launch_system_startup);
        f.set_item_index("UpdateCheckGroup", Some(c.auto_check_updates as usize));
        f.set_item_index("CharEncodingGroup", Some(c.file_encoding as usize));
        f.set_item_index("NewlineCharGroup", Some(c.file_newline_char.clamp(0, 1) as usize));
        self.load_languages();
        let lang = main.as_ref().and_then(|m| m.core.borrow().languages.iter().position(|l| l.code == c.language_code)).unwrap_or(0);
        f.set_item_index("LanguageList", Some(lang));
        self.language_list_select();
        *self.hot_keys.borrow_mut() = c.hot_keys.clone();
        self.update_hot_key_list();
        self.set_hot_key(0);
        let db = &c.database;
        f.set_checked("ClearClipCloseLockCheck", db.clear_clip_close_lock);
        f.set_checked("LockMinimizeCheck", db.lock_minimize);
        f.set_checked("LockIdleCheck", db.lock_idle);
        self.enable(&["LockIdleTimeBox", "LockIdleTimeSpinBtn"], db.lock_idle);
        f.set_position("LockIdleTimeSpinBtn", db.lock_idle_time as i64);
        f.set_checked("LockAutoSaveCheck", db.lock_auto_save);
        f.set_checked("CreateBackupCheck", db.create_backup);
        f.set_checked("NumberBackupsCheck", db.number_backups);
        self.backup_checks();
        f.set_position("MaxNumBackupsSpinBtn", db.max_num_backups as i64);
        f.set_checked("OpenWindowOnStartupCheck", db.open_window_on_startup);
        f.set_checked("OpenDbOnStartupCheck", db.open_last_db_on_startup);
        f.set_checked("KeepRecentFilesCheck", db.keep_recent_files);
        f.set_checked("WarnExpiredEntriesCheck", db.warn_expired_entries);
        f.set_checked("WarnEntriesExpireSoonCheck", db.warn_entries_expire_soon);
        f.set_position("WarnExpireNumDaysSpinBtn", db.warn_expire_num_days as i64);
        f.set_checked("AutoSaveCheck", db.auto_save);
        f.set_item_index("AutoSaveList", Some(db.auto_save_option.clamp(0, 1) as usize));
        self.enable(&["AutoSaveList"], db.auto_save);
        f.set_text("DefaultAutotypeSeqBox", &db.default_autotype_sequence);
    }

    /// `GetOptions()`.
    fn get_options(&self, old: &Configuration) -> Configuration {
        let f = self.f();
        let mut c = old.clone();
        c.ui_style_name = UI_STYLES[f.item_index("UiStylesList").unwrap_or(0).min(2)].to_string();
        c.app_icon_name = APP_ICON_NAMES[f.item_index("AppIconList").unwrap_or(0).min(5)].0.to_string();
        c.gui_font_string = self.font.borrow().clone();
        c.auto_clear_clip = f.checked("AutoClearClipCheck");
        c.auto_clear_clip_time = f.position("AutoClearClipTimeSpinBtn") as i32;
        c.auto_clear_passw = f.checked("AutoClearPasswCheck");
        c.auto_clear_passw_time = f.position("AutoClearPasswTimeSpinBtn") as i32;
        c.minimize_autotype = f.checked("MinimizeAutotypeCheck");
        c.autotype_delay = f.position("AutotypeDelaySpinBtn") as i32;
        c.confirm_exit = f.checked("AskBeforeExitCheck");
        c.load_profile_startup = f.checked("LoadProfileStartupCheck");
        c.load_profile_name = f.text("LoadProfileBox");
        c.launch_system_startup = f.checked("LaunchSystemStartupCheck");
        c.random_pool_cipher = f.item_index("RandomPoolCipherList").map(|i| i as i32).unwrap_or(1);
        c.test_common_passw = f.checked("TestCommonPasswCheck");
        c.use_advanced_passw_est = f.checked("UseAdvancedPasswEst");
        c.show_sys_tray_icon_const = f.checked("ShowSysTrayIconConstCheck");
        c.minimize_to_sys_tray = f.checked("MinimizeToSysTrayCheck");
        let idx = |n: &str| f.item_index(n).map(|i| i as i32).unwrap_or(-1);
        c.auto_check_updates = AutoCheckUpdates::from_index(idx("UpdateCheckGroup")).unwrap_or(AutoCheckUpdates::Weekly);
        c.file_encoding = CharEncoding::from_index(idx("CharEncodingGroup")).unwrap_or(CharEncoding::Utf8);
        c.file_newline_char = idx("NewlineCharGroup").clamp(0, 1);
        c.hot_keys = self.hot_keys.borrow().clone();
        let li = f.item_index("LanguageList").unwrap_or(0);
        if let Some(l) = self.languages().get(li) {
            c.language_code = l.code.clone();
        }
        let db = &mut c.database;
        db.clear_clip_close_lock = f.checked("ClearClipCloseLockCheck");
        db.lock_minimize = f.checked("LockMinimizeCheck");
        db.lock_idle = f.checked("LockIdleCheck");
        db.lock_idle_time = f.position("LockIdleTimeSpinBtn") as i32;
        db.lock_auto_save = f.checked("LockAutoSaveCheck");
        db.create_backup = f.checked("CreateBackupCheck");
        db.number_backups = f.checked("NumberBackupsCheck");
        db.max_num_backups = f.position("MaxNumBackupsSpinBtn") as i32;
        db.open_window_on_startup = f.checked("OpenWindowOnStartupCheck");
        db.open_last_db_on_startup = f.checked("OpenDbOnStartupCheck");
        db.warn_expired_entries = f.checked("WarnExpiredEntriesCheck");
        db.warn_entries_expire_soon = f.checked("WarnEntriesExpireSoonCheck");
        db.warn_expire_num_days = f.position("WarnExpireNumDaysSpinBtn") as i32;
        db.auto_save = f.checked("AutoSaveCheck");
        db.auto_save_option = f.item_index("AutoSaveList").map(|i| i as i32).unwrap_or(0);
        db.default_autotype_sequence = f.text("DefaultAutotypeSeqBox");
        db.keep_recent_files = f.checked("KeepRecentFilesCheck");
        c
    }

    async fn ok_clicked(self: &Rc<Self>) {
        let Some(main) = self.main.upgrade() else { return };
        let old = main.core.borrow().config.clone();
        let c = self.get_options(&old);
        if main.apply_config(c, win_id()).await {
            self.result.set(true);
            self.close();
        }
    }
}

impl MainWindow {
    /// `MainMenu_Options_ConfigClick()`.
    pub(super) async fn configuration(self: &Rc<Self>) {
        if windows::is_open(win_id()) {
            windows::focus(win_id());
            return;
        }
        let existing = self.tools.config_dlg.borrow().clone();
        let dlg = match existing {
            Some(d) => d,
            None => {
                let d = ConfigDlg::new(self);
                *self.tools.config_dlg.borrow_mut() = Some(d.clone());
                d
            }
        };
        let config = self.core.borrow().config.clone();
        dlg.set_options(&config);
        dlg.f().set_item_index("ConfigPages", Some(0));
        dlg.result.set(false);
        let (tx, rx) = async_channel::bounded(1);
        *dlg.done.borrow_mut() = Some(tx.clone());
        let w = dialogs::form_window(WINDOW_ID, dlg.form.clone(), ROOT, true);
        let min = w.size.get();
        if let Some(s) = self.tool_size(WINDOW_ID, WINDOW_ID, true) {
            w.size.set(s.max(min));
        }
        let w = Rc::new(w.resizable(true, min).with_content(Rc::new(ConfigContent(dlg.clone()))));
        let ww = self.weak();
        let wid = Rc::downgrade(&w);
        *w.on_close.borrow_mut() = Some(Box::new(move || {
            if let (Some(m), Some(win)) = (ww.upgrade(), wid.upgrade()) {
                m.remember_size(WINDOW_ID, &win);
            }
            let _ = tx.try_send(());
            true
        }));
        windows::open(w);
        let _ = rx.recv().await;
        if dlg.result.get() && pwtool_app::app::restart_requested() {
            windows::ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// `ApplyConfig()`: returns false if the configuration was rejected.
    pub(super) async fn apply_config(self: &Rc<Self>, c: Configuration, parent: ViewportId) -> bool {
        let old = self.core.borrow().config.clone();
        let mut restart_needed = false;
        if c.language_code != old.language_code {
            let version =
                self.core.borrow().languages.iter().find(|l| l.code == c.language_code).map(|l| l.version.clone()).unwrap_or_default();
            if pwtool_app::app::compare_versions(&version, PROGRAM_LANGVER_MIN) < 0 {
                if dialogs::ask(
                    parent,
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
            } else if pwtool_app::app::compare_versions(&version, PROGRAM_VERSION) > 0
                && dialogs::ask(
                    parent,
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
            restart_needed = true;
        }
        if c.hot_keys != old.hot_keys {
            self.activate_hot_keys(&c.hot_keys);
        }
        // the forms are laid out for the font at start-up
        if c.gui_font_string != old.gui_font_string {
            restart_needed = true;
        }
        if c.ui_style_name != old.ui_style_name {
            apply_ui_style(&windows::ctx(), &c.ui_style_name);
        }
        if c.app_icon_name != old.app_icon_name {
            if let Some(icon) = app_icon(&c.app_icon_name) {
                windows::ctx().send_viewport_cmd(ViewportCommand::Icon(Some(Arc::new(icon))));
            }
        }
        if c.launch_system_startup != old.launch_system_startup {
            if let Err(e) = pwtool_app::platform::autostart::set_autostart(c.launch_system_startup) {
                dialogs::ask(parent, &trl_format("Could not access registry key\n%1.", &[&e]), MsgIcon::Error, MsgButtons::Ok, false).await;
            }
        }
        if let Some(cipher) = PoolCipherType::from_index(c.random_pool_cipher) {
            RandomPool::global().set_cipher(cipher);
        }
        if !c.auto_clear_clip {
            self.auto_clear_clip_cnt.set(0);
        }
        let mut common_errs = Vec::new();
        {
            let mut core = self.core.borrow_mut();
            core.file_encoding = c.file_encoding;
            core.newline = c.newline().to_string();
            core.use_advanced_passw_est = c.use_advanced_passw_est;
            if c.test_common_passw != core.test_common_passw {
                core.test_common_passw = c.test_common_passw;
                if c.test_common_passw && core.common_passw.is_none() {
                    let err_start = core.startup_errors.len();
                    core.load_common_passwords();
                    common_errs = core.startup_errors.drain(err_start..).collect();
                }
            }
            core.config = c;
        }
        for e in common_errs {
            dialogs::ask(parent, &e, MsgIcon::Error, MsgButtons::Ok, false).await;
        }
        self.update_tray_icon();
        if restart_needed
            && dialogs::ask(
                parent,
                &trl("The program has to be restarted in order\nfor the changes to take effect.\n\nDo you want to restart now?"),
                MsgIcon::Question,
                MsgButtons::YesNo,
                false,
            )
            .await
                == MsgResult::Yes
        {
            pwtool_app::app::request_restart();
        }
        true
    }
}

/// User interface style: "Light"/"Dark" force the colour scheme, "Default"
/// follows the system.
pub fn apply_ui_style(ctx: &egui::Context, name: &str) {
    let pref = match name {
        "Light" => egui::ThemePreference::Light,
        "Dark" => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    };
    ctx.options_mut(|o| o.theme_preference = pref);
}

/// Window icon for `Configuration::AppIconName`.
pub fn app_icon(name: &str) -> Option<egui::IconData> {
    let img = pwtool_app::icons::decode_png(pwtool_app::icons::app_icon_pngs(name)[2])?;
    Some(egui::IconData { rgba: img.pixels, width: img.width, height: img.height })
}
