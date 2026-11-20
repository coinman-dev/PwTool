//! Main window (`TMainForm`, `upstream/src/main/Main.cpp`).

#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, gio, glib};
use pwtool_core::config::{LISTS_MAX_ENTRIES, WLFNLIST_DEFAULT, remove_comment_and_trim};
use pwtool_core::generate::{
    GenDest, GenRequest, GenSummary, PASSWOPTION_EACHCHARONLYONCE, PASSWOPTION_EACHWORDONLYONCE,
    PASSWOPTIONS_NUM, PASSWOPTIONS_STARRED, PASSW_MAX_CHARS, PASSW_MAX_NUM, PASSW_MAX_WORDS, SinkAction,
    generate, parse_num_passwords, parse_passphrase_length, passw_list_header, passwoptions_starred_bitfield,
};
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::PasswordGenerator;
use pwtool_core::passw::generator::CharSetType;
use pwtool_core::random::pool::MAX_ENTROPY;
use pwtool_core::random::{AesCtrPrng, EntropyEventType, EntropyManager, RandomGenerator, RandomPool};
use pwtool_core::scripting::{FLAG_STANDALONE_PASSW_GEN, LuaScript};
use pwtool_core::unicode::w32_to_string;
use pwtool_core::util::format_float_1;
use zeroize::Zeroizing;

use super::builder::{Built, attach_context_menu, attach_context_menu_capture, build_form};
use super::dialogs::{PasswListWindow, PasswOptionsDlg, ProgressWindow, format_quick_help, info_box, quick_help};
use super::form::load_form;
use super::quality_bar::QualityBar;
use super::util::{MsgButtons, MsgIcon, MsgResult, combo_entry, combo_text, msg, msg_box, open_uri, set_combo_items, set_combo_text};
use crate::app::AppCore;
use crate::program::*;

#[path = "main_tools.rs"]
mod tools;
#[path = "config_dlg.rs"]
mod config_dlg;
#[path = "profile_editor.rs"]
mod profile_editor;
#[path = "char_set_builder.rs"]
mod char_set_builder;
#[path = "mp_passw_gen.rs"]
mod mp_passw_gen;
#[path = "main_integration.rs"]
mod integration;
pub use integration::show_tray_info;
#[path = "main_tray.rs"]
mod tray_main;

const ENTROPY_SYSTEM: u32 = 24;
const ENTROPY_MOUSECLICK: u32 = 2;
const ENTROPY_MOUSEMOVE: u32 = 2;
const ENTROPY_KEYBOARD: u32 = 1;
const TIMER_TOUCHPOOL: u32 = 2;
const TIMER_RANDOMIZE: u32 = 10;
const TIMER_MOVEPOOL: u32 = 300;
const TIMER_WRITESEEDFILE: u32 = 457;
const PASSWBOX_TAG_PASSW_TEST: u32 = 1;
const PASSWBOX_TAG_PASSW_GEN: u32 = 2;

/// Random generator used for one generation run: a pool derived from the
/// main pool, or the deterministic generator.
enum GenRng {
    Pool(RandomPool),
    Det(AesCtrPrng),
}

impl RandomGenerator for GenRng {
    fn seed(&mut self, seed: &[u8]) {
        match self {
            GenRng::Pool(p) => p.seed(seed),
            GenRng::Det(d) => d.seed(seed),
        }
    }
    fn get_data(&mut self, buf: &mut [u8]) {
        match self {
            GenRng::Pool(p) => p.get_data(buf),
            GenRng::Det(d) => d.get_data(buf),
        }
    }
    fn get_byte(&mut self) -> u8 {
        match self {
            GenRng::Pool(p) => p.get_byte(),
            GenRng::Det(d) => d.get_byte(),
        }
    }
}

/// Output of a generation run in the worker thread.
struct GenOutput {
    pg: PasswordGenerator,
    rng: GenRng,
    script: Option<LuaScript>,
    result: Result<GenSummary, String>,
    single: Zeroizing<String>,
    list: Zeroizing<String>,
    mem_limit_reached: bool,
}

pub struct MainWindow {
    pub window: gtk::ApplicationWindow,
    pub b: Built,
    pub core: RefCell<AppCore>,
    passw_bar: QualityBar,
    char_set_input: RefCell<String>,
    wl_file_name: RefCell<String>,
    wl_file_name_err: RefCell<String>,
    auto_clear_clip_cnt: Cell<i32>,
    auto_clear_passw_cnt: Cell<i32>,
    passw_box_tag: Cell<u32>,
    show_ent_progress: Cell<bool>,
    timer_cnt: Cell<[u32; 4]>,
    key_seeded_prng: RefCell<Option<AesCtrPrng>>,
    script: RefCell<Option<LuaScript>>,
    generating: Cell<bool>,
    passw_options_dlg: RefCell<Option<Rc<PasswOptionsDlg>>>,
    passw_list: RefCell<Option<Rc<PasswListWindow>>>,
    char_set_help: String,
    format_passw_help: String,
    default_title: String,
    mouse_move_cnt: Cell<i32>,
    mouse_wheel_cnt: Cell<i32>,
    tools: tools::ToolsState,
    config_dlg: RefCell<Option<Rc<config_dlg::ConfigDlg>>>,
    hot_keys_active: RefCell<crate::app_config::HotKeyList>,
    profile_dlg: RefCell<Option<Rc<profile_editor::ProfileEditDlg>>>,
    profile_list_guard: Cell<bool>,
    char_set_builder: RefCell<Option<Rc<char_set_builder::CharSetBuilderWin>>>,
    mp_passw_gen: RefCell<Option<Rc<mp_passw_gen::MpPasswGenWin>>>,
    close_confirmed: Cell<bool>,
    tray: tray_main::TrayState,
    passw_mng: RefCell<Option<Rc<crate::ui::passw_manager::PasswManager>>>,
    close_queried: Cell<bool>,
    autotype_active: Cell<bool>,
    autotype_abort: RefCell<Option<Rc<Cell<bool>>>>,
    startup_shown: Cell<bool>,
    /// Edit box receiving the next single password instead of `PasswBox`
    /// (`GeneratePassw(gpdGuiSingle, pEditBox)`).
    pub(super) gen_target: RefCell<Option<gtk::Entry>>,
}

impl MainWindow {
    pub fn new(app: &gtk::Application, core: AppCore) -> Rc<Self> {
        let b = build_form(load_form("Main"), "Main");
        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some(PROGRAM_NAME));
        if let Some(t) = super::icons::app_icon() {
            let _ = t;
        }
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let menubar = gtk::PopoverMenuBar::from_model(b.menus.get("MainMenu"));
        vbox.append(&menubar);
        b.root.set_vexpand(true);
        b.root.set_hexpand(true);
        vbox.append(&b.root);
        window.set_child(Some(&vbox));
        // the window has a fixed size and cannot be maximized, see
        // `dialogs::form_window()`
        let (_, mh, _, _) = menubar.measure(gtk::Orientation::Vertical, -1);
        let (cw, ch) = super::dialogs::fixed_client_size(&b);
        b.root.set_fixed_size(cw, ch);
        window.set_resizable(false);
        window.set_default_size(cw, ch + mh);
        window.insert_action_group("m", Some(&b.actions));
        // Only register MainMenu accelerators application-wide; popup menu
        // shortcuts (PasswBoxMenu, ListMenu) must not hijack Ctrl+C/X/V/A/Z
        // in other widgets and dialogs.
        for (action, accel) in &b.accels {
            if action.starts_with("m.MainMenu_") {
                app.set_accels_for_action(action, &[accel.as_str()]);
            }
        }
        let pb_sc = gtk::ShortcutController::new();
        pb_sc.set_scope(gtk::ShortcutScope::Local);
        for (action, accel) in &b.accels {
            if action.starts_with("m.PasswBoxMenu_") {
                if let Some(t) = gtk::ShortcutTrigger::parse_string(accel) {
                    pb_sc.add_shortcut(gtk::Shortcut::new(Some(t), Some(gtk::NamedAction::new(action))));
                }
            }
        }
        b.get::<gtk::Entry>("PasswBox").add_controller(pb_sc);

        let passw_bar = QualityBar::new(b.get("PasswGauge"), 128);
        let char_set_help = build_char_set_help();
        let format_passw_help = build_format_help();
        let w = Rc::new(MainWindow {
            window,
            b,
            core: RefCell::new(core),
            passw_bar,
            char_set_input: RefCell::new(String::new()),
            wl_file_name: RefCell::new(String::new()),
            wl_file_name_err: RefCell::new(String::new()),
            auto_clear_clip_cnt: Cell::new(0),
            auto_clear_passw_cnt: Cell::new(0),
            passw_box_tag: Cell::new(0),
            show_ent_progress: Cell::new(true),
            timer_cnt: Cell::new([0; 4]),
            key_seeded_prng: RefCell::new(None),
            script: RefCell::new(None),
            generating: Cell::new(false),
            passw_options_dlg: RefCell::new(None),
            passw_list: RefCell::new(None),
            char_set_help,
            format_passw_help,
            default_title: PROGRAM_NAME.to_string(),
            mouse_move_cnt: Cell::new(60),
            mouse_wheel_cnt: Cell::new(5),
            tools: tools::ToolsState::new(),
            config_dlg: RefCell::new(None),
            hot_keys_active: RefCell::new(Default::default()),
            profile_dlg: RefCell::new(None),
            profile_list_guard: Cell::new(false),
            char_set_builder: RefCell::new(None),
            mp_passw_gen: RefCell::new(None),
            close_confirmed: Cell::new(false),
            tray: tray_main::TrayState::new(),
            passw_mng: RefCell::new(None),
            close_queried: Cell::new(false),
            autotype_active: Cell::new(false),
            autotype_abort: RefCell::new(None),
            startup_shown: Cell::new(false),
            gen_target: RefCell::new(None),
        });
        w.setup();
        w
    }

    fn win(&self) -> gtk::Window {
        self.window.clone().upcast()
    }

    fn entry(&self, name: &str) -> gtk::Entry {
        self.b.get(name)
    }

    fn check(&self, name: &str) -> gtk::CheckButton {
        self.b.get(name)
    }

    fn combo(&self, name: &str) -> gtk::ComboBoxText {
        self.b.get(name)
    }

    fn label(&self, name: &str) -> gtk::Label {
        self.b.get(name)
    }

    fn button(&self, name: &str) -> gtk::Button {
        self.b.get(name)
    }

    fn act(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        if !self.b.has_action(name) {
            eprintln!("pwtool: menu item {name} not found");
            return;
        }
        let w = Rc::downgrade(self);
        self.b.action(name).connect_activate(move |a, _| {
            if let Some(s) = a.state() {
                if let Some(v) = s.get::<bool>() {
                    a.set_state(&(!v).to_variant());
                }
            }
            if let Some(w) = w.upgrade() {
                f(&w);
            }
        });
    }

    fn click(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        let w = Rc::downgrade(self);
        self.button(name).connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                f(&w);
            }
        });
    }

    fn action_checked(&self, name: &str) -> bool {
        self.b.action(name).state().and_then(|s| s.get::<bool>()).unwrap_or(false)
    }

    fn set_action_checked(&self, name: &str, v: bool) {
        let a = self.b.action(name);
        if a.state().is_some() {
            a.set_state(&v.to_variant());
        }
    }

    // ----------------------------------------------------------------------
    // set-up

    fn setup(self: &Rc<Self>) {
        let b = &self.b;
        for n in [
            "MainMenu_Options_HideEntProgress",
            "MainMenu_Options_AlwaysOnTop",
            "MainMenu_Options_SaveSettingsOnExit",
            "PasswBoxMenu_Editable",
            "PasswBoxMenu_EnablePasswTest",
            "GenerateMenu_IncludeHeader",
        ] {
            b.make_checkable(n, false);
        }
        {
            let core = self.core.borrow();
            EntropyManager::global().set_system_entropy_bits(ENTROPY_SYSTEM);
            b.updowns["CharsLengthSpinBtn"].set_max(PASSW_MAX_CHARS as i64);
            b.updowns["WordsNumSpinBtn"].set_max(PASSW_MAX_WORDS as i64);
            let _ = &core;
        }
        let logo_lbl = self.label("LogoLbl");
        logo_lbl.set_text("PW GEN");
        logo_lbl.set_justify(gtk::Justification::Center);
        logo_lbl.set_xalign(0.5);
        logo_lbl.set_yalign(0.5);
        logo_lbl.set_ellipsize(gtk::pango::EllipsizeMode::None);
        if let Some(p) = self.b.try_get::<gtk::Picture>("LogoImg") {
            p.set_can_shrink(true);
            p.set_content_fit(gtk::ContentFit::Contain);
            let bytes = glib::Bytes::from_static(include_bytes!("../../../../assets/about_logo.png"));
            if let Ok(t) = gdk::Texture::from_bytes(&bytes) {
                p.set_paintable(Some(&t));
            }
        }
        super::builder::add_css(&logo_lbl, "font-family: serif; font-weight: bold;");
        self.label("PasswInfoLbl").set_text(&trl("Click on \"Generate\""));
        self.entry("PasswBox").add_css_class("monospace");
        super::builder::add_css(&self.entry("PasswBox"), "font-family: monospace;");
        let status: gtk::Label = b.get("StatusBar.label");
        status.set_text(&format!("{PROGRAM_NAME} {}", version_string()));
        b.get::<gtk::Button>("TogglePasswBtn").add_css_class("flat");

        {
            let core = self.core.borrow();
            super::util::set_gui_font(&core.config.gui_font_string);
            super::util::set_ui_style(&core.config.ui_style_name);
            let keys = core.config.hot_keys.clone();
            drop(core);
            self.activate_hot_keys(&keys);
        }
        self.load_config_to_controls();
        self.startup_profile();
        self.connect_events();
        self.connect_tools();
        self.connect_profile_editor();
        self.setup_tray();
        self.setup_timer();
        self.setup_entropy_events();
        self.update_entropy_progress(true);
        self.set_advanced_btn_caption();

        let w = Rc::downgrade(self);
        self.window.connect_close_request(move |win| {
            if let Some(w) = w.upgrade() {
                // `FormCloseQuery()`
                if !w.close_confirmed.get() && w.core.borrow().config.confirm_exit && !crate::app::restart_requested() {
                    let (w2, win2) = (w.clone(), win.clone());
                    glib::spawn_future_local(async move {
                        if msg_box(
                            Some(win2.upcast_ref()),
                            &trl_format("Are you sure you want to exit\n%1?", &[PROGRAM_NAME]),
                            MsgIcon::Question,
                            MsgButtons::YesNo,
                            true,
                        )
                        .await
                            == MsgResult::Yes
                        {
                            w2.close_confirmed.set(true);
                            win2.close();
                        }
                    });
                    return glib::Propagation::Stop;
                }
                // `PasswMngForm->CloseQuery()`: unsaved database
                let mng = w.passw_mng.borrow().clone();
                if let Some(m) = mng.filter(|_| !w.close_queried.get()) {
                    let (w2, win2) = (w.clone(), win.clone());
                    glib::spawn_future_local(async move {
                        if m.close_query().await {
                            w2.close_queried.set(true);
                            w2.close_confirmed.set(true);
                            win2.close();
                        } else {
                            w2.close_confirmed.set(false);
                        }
                    });
                    return glib::Propagation::Stop;
                }
                w.on_close();
                // closing the main window terminates the program, also
                // when other windows are open
                if let Some(app) = w.window.application() {
                    glib::idle_add_local_once(move || app.quit());
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        super::clipboard::on_set_data(move || {
            if let Some(w) = w.upgrade() {
                let core = w.core.borrow();
                if core.config.auto_clear_clip {
                    w.auto_clear_clip_cnt.set(core.config.auto_clear_clip_time);
                }
            }
        });
    }

    /// Generator controls from the settings (`LoadConfig`).
    fn load_config_to_controls(self: &Rc<Self>) {
        super::dialogs::load_quick_help_config(&self.core.borrow().ini);
        let core = self.core.borrow();
        let s = &core.settings;
        self.check("IncludeCharsCheck").set_active(s.include_chars);
        self.b.updowns["CharsLengthSpinBtn"].set_position(s.chars_length as i64);
        set_combo_items(&self.combo("CharSetList"), &s.char_set_list);
        set_combo_text(&self.combo("CharSetList"), &s.char_set_text);
        *self.char_set_input.borrow_mut() = s.char_set_text.clone();
        self.check("IncludeWordsCheck").set_active(s.include_words);
        self.b.updowns["WordsNumSpinBtn"].set_position(s.words_num as i64);
        set_combo_items(&self.combo("WLFNList"), &s.wlfn_list);
        set_combo_text(&self.combo("WLFNList"), &s.wlfn_text);
        if core.word_list_error.is_none() {
            *self.wl_file_name.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
            self.wl_file_name_err.borrow_mut().clear();
        } else {
            *self.wl_file_name_err.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
        }
        self.check("CombineWordsCharsCheck").set_active(s.combine_words_chars);
        self.check("SpecifyLengthCheck").set_active(s.passphr_spec_len);
        self.entry("SpecifyLengthBox").set_text(&s.passphr_spec_len_string);
        self.check("FormatPasswCheck").set_active(s.format_passw);
        set_combo_items(&self.combo("FormatList"), &s.format_list);
        set_combo_text(&self.combo("FormatList"), &s.format_text);
        self.check("RunScriptCheck").set_active(s.run_script);
        set_combo_items(&self.combo("ScriptList"), &s.script_list);
        set_combo_text(&self.combo("ScriptList"), &s.script_text);
        self.entry("NumPasswBox").set_text(&s.num_passw);
        self.set_action_checked("GenerateMenu_IncludeHeader", s.include_header);
        let ini = &core.ini;
        let hide = ini.read_bool("Main", "HidePassw", false);
        self.b.get::<gtk::ToggleButton>("TogglePasswBtn").set_active(hide);
        self.entry("PasswBox").set_visibility(!hide);
        self.show_ent_progress.set(!ini.read_bool("Main", "HideEntropyProgress", false));
        self.set_action_checked("MainMenu_Options_HideEntProgress", !self.show_ent_progress.get());
        self.set_action_checked("MainMenu_Options_SaveSettingsOnExit", ini.read_bool("Main", "SaveSettingsOnExit", true));
        self.set_action_checked("MainMenu_Options_AlwaysOnTop", ini.read_bool("Main", "AlwaysOnTop", false));
        let editable = ini.read_bool("Main", "PasswEditable", true);
        self.set_action_checked("PasswBoxMenu_Editable", editable);
        self.set_action_checked("PasswBoxMenu_EnablePasswTest", ini.read_bool("Main", "EnablePasswTest", true));
        self.b.action("PasswBoxMenu_EnablePasswTest").set_enabled(editable);
        let pb = self.entry("PasswBox");
        pb.set_editable(editable);
        pb.set_enable_undo(true);
        let tag = if self.action_checked("PasswBoxMenu_EnablePasswTest") { PASSWBOX_TAG_PASSW_TEST } else { 0 };
        self.passw_box_tag.set(tag);
        drop(core);
        self.update_char_set_info();
        self.update_word_list_info();
        self.update_profile_controls();
        self.include_chars_check_click();
    }

    fn update_char_set_info(&self) {
        let core = self.core.borrow();
        let l = self.label("CharSetInfoLbl");
        l.set_text(&core.char_set_info);
        if core.char_set_error {
            l.add_css_class("error");
        } else {
            l.remove_css_class("error");
        }
    }

    fn update_word_list_info(&self) {
        let core = self.core.borrow();
        let l = self.label("WordListInfoLbl");
        match &core.word_list_error {
            Some(e) => {
                l.set_text(e);
                l.add_css_class("error");
            }
            None => {
                l.set_text(&core.word_list_info);
                l.remove_css_class("error");
            }
        }
    }

    /// `UpdateProfileControls()`.
    fn update_profile_controls(&self) {
        let core = self.core.borrow();
        let names: Vec<String> = core.profiles.iter().map(|p| p.profile_name.clone()).collect();
        set_combo_items(&self.combo("ProfileList"), &names);
        drop(core);
        // File | Profile menu: profiles with Alt+Shift+0..Z, then editor
        self.update_profile_menu();
    }

    /// `IncludeCharsCheckClick()`: enable dependent controls.
    fn include_chars_check_click(&self) {
        let c1 = self.check("IncludeCharsCheck").is_active();
        let c2 = self.check("IncludeWordsCheck").is_active();
        let c3 = self.check("FormatPasswCheck").is_active();
        let c4 = self.check("RunScriptCheck").is_active();
        for n in ["CharsLengthLbl", "CharsLengthBox", "CharsLengthSpinBtn"] {
            self.b.w(n).set_sensitive(c1);
        }
        self.b.w("CombineWordsCharsCheck").set_sensitive(c1 && c2);
        for n in ["WordsNumLbl", "WordsNumBox", "WordsNumSpinBtn", "SpecifyLengthCheck"] {
            self.b.w(n).set_sensitive(c2);
        }
        self.b.w("SpecifyLengthBox").set_sensitive(c2 && self.check("SpecifyLengthCheck").is_active());
        self.b.w("FormatList").set_sensitive(c3);
        self.b.w("FormatPasswInfoLbl").set_sensitive(c3);
        for n in ["ScriptList", "ReloadScriptBtn", "BrowseBtn2"] {
            self.b.w(n).set_sensitive(c4);
        }
        let en = c1 || c2 || c3 || c4;
        for n in ["NumPasswBox", "GenerateBtn", "GenerateBtn2"] {
            self.b.w(n).set_sensitive(en);
        }
    }

    // ----------------------------------------------------------------------
    // events

    fn connect_events(self: &Rc<Self>) {
        for n in ["IncludeCharsCheck", "IncludeWordsCheck", "FormatPasswCheck", "RunScriptCheck", "SpecifyLengthCheck"] {
            let w = Rc::downgrade(self);
            let is_spec = n == "SpecifyLengthCheck";
            self.check(n).connect_toggled(move |c| {
                if let Some(w) = w.upgrade() {
                    w.include_chars_check_click();
                    if is_spec && c.is_active() {
                        w.entry("SpecifyLengthBox").grab_focus();
                    }
                }
            });
        }
        // combo boxes with history
        self.on_combo_leave("CharSetList", |w| w.char_set_list_exit());
        self.on_combo_activate("CharSetList", |w| w.char_set_info_btn_click());
        self.on_combo_leave("WLFNList", |w| w.wlfn_list_exit());
        self.on_combo_activate("WLFNList", |w| w.word_list_info_btn_click());
        self.on_combo_leave("FormatList", |w| w.format_list_exit());
        for n in ["CharSetList", "WLFNList", "FormatList", "ScriptList"] {
            let w = Rc::downgrade(self);
            let name = n.to_string();
            self.combo(n).connect_changed(move |cb| {
                if cb.active().is_some() {
                    if let Some(w) = w.upgrade() {
                        match name.as_str() {
                            "CharSetList" => w.char_set_list_exit(),
                            "WLFNList" => w.wlfn_list_exit(),
                            _ => {}
                        }
                    }
                }
            });
        }
        if let Some(m) = self.b.menus.get("ListMenu") {
            for n in ["CharSetList", "WLFNList", "FormatList", "ScriptList"] {
                if let Some(e) = combo_entry(&self.combo(n)) {
                    let _ = m;
                    let _ = e;
                }
            }
        }
        // number boxes
        for (box_name, ud) in [("CharsLengthBox", "CharsLengthSpinBtn"), ("WordsNumBox", "WordsNumSpinBtn")] {
            let w = Rc::downgrade(self);
            let ud = ud.to_string();
            let fc = gtk::EventControllerFocus::new();
            fc.connect_leave(move |_| {
                if let Some(w) = w.upgrade() {
                    let u = &w.b.updowns[&ud];
                    u.set_position(u.position());
                }
            });
            self.entry(box_name).add_controller(fc);
        }
        let w = Rc::downgrade(self);
        self.entry("NumPasswBox").connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.generate_passw(GenDest::GuiList);
            }
        });
        let w = Rc::downgrade(self);
        self.entry("PasswBox").connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.generate_passw(GenDest::GuiSingle);
            }
        });
        let w = Rc::downgrade(self);
        self.entry("PasswBox").connect_changed(move |_| {
            if let Some(w) = w.upgrade() {
                w.passw_box_change();
            }
        });
        if let Some(m) = self.b.menus.get("PasswBoxMenu") {
            let e = self.entry("PasswBox");
            let pop = attach_context_menu_capture(&e, m);
            let w = Rc::downgrade(self);
            pop.connect_show(move |_| {
                if let Some(w) = w.upgrade() {
                    w.update_passw_box_menu();
                }
            });
        }
        let w = Rc::downgrade(self);
        self.b.get::<gtk::ToggleButton>("TogglePasswBtn").connect_toggled(move |t| {
            if let Some(w) = w.upgrade() {
                w.entry("PasswBox").set_visibility(!t.is_active());
            }
        });

        self.click("GenerateBtn", |w| w.generate_passw(GenDest::GuiSingle));
        self.click("GenerateBtn2", |w| w.generate_passw(GenDest::GuiList));
        if let Some(m) = self.b.menus.get("GenerateMenu") {
            attach_context_menu(&self.button("GenerateBtn2"), m);
        }
        self.act("GenerateMenu_Clipboard", |w| w.generate_passw(GenDest::ClipboardList));
        self.act("GenerateMenu_File", |w| w.generate_passw(GenDest::FileList));
        self.act("GenerateMenu_IncludeHeader", |_| {});
        self.click("AdvancedBtn", |w| w.advanced_btn_click());
        if let Some(m) = self.b.menus.get("AdvancedOptionsMenu") {
            attach_context_menu(&self.button("AdvancedBtn"), m);
        }
        self.act("AdvancedOptionsMenu_DeactivateAll", |w| w.deactivate_options(u32::MAX));
        self.act("AdvancedOptionsMenu_DeactivateAllStarred", |w| {
            w.check("SpecifyLengthCheck").set_active(false);
            w.deactivate_options(passwoptions_starred_bitfield());
            w.set_advanced_btn_caption();
        });
        self.click("CharSetInfoBtn", |w| w.char_set_info_btn_click());
        self.click("WordListInfoBtn", |w| w.word_list_info_btn_click());
        self.click("CharSetHelpBtn", |w| quick_help(&w.win(), &w.char_set_help));
        self.click("FormatPasswHelpBtn", |w| quick_help(&w.win(), &w.format_passw_help));
        self.click("BrowseBtn", |w| w.browse_file("WLFNList", &trl("Select word list file"), true));
        self.click("BrowseBtn2", |w| w.browse_file("ScriptList", &trl("Select Lua script file"), false));
        self.click("ReloadScriptBtn", |w| {
            *w.script.borrow_mut() = None;
        });
        self.click("HelpBtn", |w| w.help_btn_click());
        self.click("BuildBtn", |w| w.char_set_builder());
        self.click("ClearClipBtn", |w| w.clear_clip_btn_click());
        self.click("ReloadProfileBtn", |w| {
            if let Some(i) = w.combo("ProfileList").active() {
                w.load_profile(i as usize);
            }
        });
        let w = Rc::downgrade(self);
        self.combo("ProfileList").connect_changed(move |cb| {
            if let (Some(w), Some(i)) = (w.upgrade(), cb.active()) {
                if !w.profile_list_guard.get() {
                    w.load_profile(i as usize);
                }
            }
        });

        // menu actions
        self.act("MainMenu_File_Exit", |w| w.window.close());
        self.act("TrayMenu_Exit", |w| w.window.close());
        self.act("MainMenu_Tools_ClearClipboard", |w| w.clear_clip_btn_click());
        self.act("TrayMenu_ClearClip", |w| w.clear_clip_btn_click());
        self.act("MainMenu_Help_OpenManual", |w| w.help_btn_click());
        self.act("MainMenu_Help_CmdLineArgs", |w| msg(Some(&w.win()), &AppCore::cmd_line_info(), MsgIcon::Info));
        self.act("MainMenu_Help_Donate", |w| open_uri(Some(&w.win()), PROGRAM_URL_DONATE));
        self.act("MainMenu_Help_VisitWebsite", |w| {
            let url = if PORT_REPOSITORY.is_empty() { PROGRAM_URL_ORIGINAL } else { PORT_REPOSITORY };
            open_uri(Some(&w.win()), url);
        });
        self.act("MainMenu_Help_GetTranslations", |w| {
            let url = if PORT_REPOSITORY.is_empty() { PROGRAM_URL_ORIGINAL } else { PORT_REPOSITORY };
            open_uri(Some(&w.win()), url);
        });
        self.act("MainMenu_Help_TimerInfo", |w| w.timer_info());
        self.act("MainMenu_Options_SaveSettingsNow", |w| {
            if w.save_config() {
                info_box(&w.win(), &trl("Settings saved."));
            }
        });
        self.act("MainMenu_Options_HideEntProgress", |w| {
            w.show_ent_progress.set(!w.action_checked("MainMenu_Options_HideEntProgress"));
            w.update_entropy_progress(true);
        });
        self.act("MainMenu_Options_SaveSettingsOnExit", |_| {});
        self.act("PasswBoxMenu_Editable", |w| {
            let editable = w.action_checked("PasswBoxMenu_Editable");
            w.entry("PasswBox").set_editable(editable);
            w.b.action("PasswBoxMenu_EnablePasswTest").set_enabled(editable);
        });
        self.act("PasswBoxMenu_EnablePasswTest", |w| {
            let on = w.action_checked("PasswBoxMenu_EnablePasswTest");
            let t = w.passw_box_tag.get();
            w.passw_box_tag.set(if on { t | PASSWBOX_TAG_PASSW_TEST } else { t & !PASSWBOX_TAG_PASSW_TEST });
        });
        self.act("PasswBoxMenu_Copy", |w| w.passw_box_copy());
        self.act("PasswBoxMenu_SelectAll", |w| w.entry("PasswBox").select_region(0, -1));
        self.act("PasswBoxMenu_Cut", |w| {
            let e = w.entry("PasswBox");
            if !e.is_editable() {
                return;
            }
            if e.selection_bounds().is_some() {
                w.passw_box_copy();
                e.delete_selection();
            }
        });
        self.act("PasswBoxMenu_Delete", |w| {
            let e = w.entry("PasswBox");
            if e.is_editable() {
                e.delete_selection();
            }
        });
        self.act("PasswBoxMenu_Paste", |w| {
            let e = w.entry("PasswBox");
            if !e.is_editable() {
                return;
            }
            super::clipboard::read_text(move |t| {
                if let Some(t) = t {
                    if !e.is_editable() {
                        return;
                    }
                    e.delete_selection();
                    let mut pos = e.position();
                    e.insert_text(&t, &mut pos);
                }
            });
        });
        self.act("PasswBoxMenu_Undo", |w| {
            let e = w.entry("PasswBox");
            if e.is_editable() {
                let _ = e.activate_action("text.undo", None);
            }
        });
        self.act("PasswBoxMenu_SaveAsFile", |w| w.passw_box_save_as_file());
        self.act("MainMenu_Tools_ProvideAddEntropy_FromFile", |w| w.add_entropy_from_file());
        self.act("MainMenu_Tools_DetermRandGen_Reset", |w| {
            if let Some(g) = w.key_seeded_prng.borrow_mut().as_mut() {
                g.reset();
            }
            info_box(&w.win(), &trl("Deterministic random generator has been RESET."));
        });
        self.act("MainMenu_Tools_DetermRandGen_Deactivate", |w| w.deactivate_det_rand_gen());
        self.act("TrayMenu_GenPassw", |w| w.generate_passw(GenDest::Clipboard));
        self.act("TrayMenu_GenAndShowPassw", |w| w.generate_passw(GenDest::MsgBox));
        self.act("TrayMenu_GenAndAutoTypePassw", |w| w.generate_passw(GenDest::Autotype));
    }

    fn on_combo_leave(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        if let Some(e) = combo_entry(&self.combo(name)) {
            let w = Rc::downgrade(self);
            let fc = gtk::EventControllerFocus::new();
            fc.connect_leave(move |_| {
                if let Some(w) = w.upgrade() {
                    f(&w);
                }
            });
            e.add_controller(fc);
        }
    }

    fn on_combo_activate(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        if let Some(e) = combo_entry(&self.combo(name)) {
            let w = Rc::downgrade(self);
            e.connect_activate(move |_| {
                if let Some(w) = w.upgrade() {
                    f(&w);
                }
            });
        }
    }

    /// `AddEntryToList()`.
    fn add_entry_to_list(&self, combo: &str, entry: &str, case_sensitive: bool) {
        let cb = self.combo(combo);
        let mut items: Vec<String> = Vec::new();
        let model = cb.model();
        if let Some(m) = model {
            if let Some(mut iter) = m.iter_first() {
                loop {
                    let v: String = m.get(&iter, 0);
                    items.push(v);
                    if !m.iter_next(&mut iter) {
                        break;
                    }
                }
            }
        }
        let pos = items.iter().position(|s| if case_sensitive { s == entry } else { s.to_lowercase() == entry.to_lowercase() });
        match pos {
            Some(p) => {
                let s = items.remove(p);
                items.insert(0, s);
            }
            None => {
                if items.len() == LISTS_MAX_ENTRIES {
                    items.pop();
                }
                items.insert(0, entry.to_string());
            }
        }
        let text = combo_text(&cb);
        set_combo_items(&cb, &items);
        set_combo_text(&cb, &text);
    }

    fn combo_items(&self, combo: &str) -> Vec<String> {
        let mut items = Vec::new();
        if let Some(m) = self.combo(combo).model() {
            if let Some(mut iter) = m.iter_first() {
                loop {
                    items.push(m.get::<String>(&iter, 0));
                    if !m.iter_next(&mut iter) {
                        break;
                    }
                }
            }
        }
        items
    }

    /// `CharSetListExit()` → `LoadCharSet()`.
    fn char_set_list_exit(&self) {
        let text = combo_text(&self.combo("CharSetList"));
        if text != *self.char_set_input.borrow() {
            self.load_char_set(&text);
        }
    }

    fn load_char_set(&self, text: &str) -> usize {
        *self.char_set_input.borrow_mut() = text.to_string();
        let n = self.core.borrow_mut().load_char_set(text);
        if n > 0 {
            self.add_entry_to_list("CharSetList", text, true);
        }
        self.update_char_set_info();
        n
    }

    /// `WLFNListExit()`.
    fn wlfn_list_exit(&self) {
        let text = combo_text(&self.combo("WLFNList"));
        let input = if text.is_empty() { WLFNLIST_DEFAULT.to_string() } else { text.clone() };
        if remove_comment_and_trim(&input) == *self.wl_file_name.borrow() && self.wl_file_name_err.borrow().is_empty() {
            return;
        }
        self.load_word_list_file(&input);
    }

    fn load_word_list_file(&self, input: &str) -> i32 {
        let n = self.core.borrow_mut().load_word_list_file(input);
        let trimmed = remove_comment_and_trim(input);
        if n > 0 {
            *self.wl_file_name.borrow_mut() = trimmed;
            self.wl_file_name_err.borrow_mut().clear();
            self.add_entry_to_list("WLFNList", input, false);
        } else {
            *self.wl_file_name_err.borrow_mut() = trimmed;
        }
        self.update_word_list_info();
        n
    }

    /// `FormatListExit()`.
    fn format_list_exit(&self) {
        let text = combo_text(&self.combo("FormatList"));
        if !text.is_empty() {
            self.add_entry_to_list("FormatList", &text, true);
        }
    }

    /// `CharSetInfoBtnClick()`.
    fn char_set_info_btn_click(&self) {
        self.char_set_list_exit();
        let core = self.core.borrow();
        let mut chars = core.passw_gen.custom_char_set_w32().to_vec();
        let n = (chars.len().saturating_sub(1)) / 40;
        for i in 0..n {
            chars.insert((i + 1) * 40 + i, '\n' as u32);
        }
        let mut s = w32_to_string(&chars);
        match core.passw_gen.custom_char_set_type() {
            CharSetType::StandardWithFreq => {
                s.push_str("\n\n");
                s.push_str(&trl("(with different frequencies)"));
            }
            CharSetType::Phonetic | CharSetType::PhoneticUpperCase | CharSetType::PhoneticMixedCase => {
                let src = if core.settings.passw_options.trigram_file_name.is_empty() {
                    trl("default (English) trigrams")
                } else {
                    trl_format("File \"%1\"", &[&core.settings.passw_options.trigram_file_name])
                };
                s.push_str("\n\n");
                s.push_str(&trl_format(
                    "(phonetic, based on trigram frequencies\nfrom the following source:\n<%1>)",
                    &[&src],
                ));
            }
            _ => {}
        }
        let mut m = String::new();
        if core.char_set_error {
            m = trl("The character set you entered is invalid.\nIt must contain at least 2 different characters.") + "\n\n";
        }
        m.push_str(&pwtool_core::unicode::format_w(
            "%1\n\n%2\n\n%3.",
            &[&trl("Currently loaded character set:"), &s, &core.char_set_info],
        ));
        msg(Some(&self.win()), &m, MsgIcon::Info);
    }

    /// `WordListInfoBtnClick()`.
    fn word_list_info_btn_click(&self) {
        self.wlfn_list_exit();
        let core = self.core.borrow();
        let mut m = String::new();
        if !self.wl_file_name_err.borrow().is_empty() {
            m = trl_format(
                "Error while loading the file\n\"%1\":\n%2",
                &[&self.wl_file_name_err.borrow(), &self.label("WordListInfoLbl").text()],
            ) + "\n\n";
        }
        let name = self.wl_file_name.borrow().clone();
        let wl = if name.is_empty() || name == WLFNLIST_DEFAULT {
            format!("<{}>", trl("default (English) word list"))
        } else {
            format!("\"{name}\"")
        };
        let g = &core.passw_gen;
        let n = g.word_list_size();
        m.push_str(&pwtool_core::unicode::format_w(
            "%1\n%2.\n\n%3.\n\n%4 %5\n%6 %7\n%8 %9",
            &[
                &trl("Currently loaded word list:"),
                &wl,
                &core.word_list_info,
                &trl("First word:"),
                &g.get_word(0),
                &trl("Random word:"),
                &g.get_word(pwtool_core::random::fprng_rand_range(n as u32) as i32),
                &trl("Last word:"),
                &g.get_word(n - 1),
            ],
        ));
        msg(Some(&self.win()), &m, MsgIcon::Info);
    }

    fn browse_file(self: &Rc<Self>, combo: &str, title: &str, word_list: bool) {
        let dlg = gtk::FileDialog::new();
        dlg.set_title(title);
        let w = self.clone();
        let combo = combo.to_string();
        dlg.open(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r {
                if let Some(p) = f.path() {
                    let cb = w.combo(&combo);
                    set_combo_text(&cb, &p.to_string_lossy());
                    if word_list {
                        w.wlfn_list_exit();
                    }
                    if let Some(e) = combo_entry(&cb) {
                        e.grab_focus();
                    }
                }
            }
        });
    }

    fn help_btn_click(&self) {
        let p = self.core.borrow().help_file_name.clone();
        if p.exists() {
            open_uri(Some(&self.win()), &p.to_string_lossy());
        } else {
            let alt = self.core.borrow().paths.resources.join("manual").join("manual.pdf");
            open_uri(Some(&self.win()), &if alt.exists() { alt } else { p }.to_string_lossy());
        }
    }

    /// `ClearClipBtnClick()`.
    fn clear_clip_btn_click(self: &Rc<Self>) {
        let w = self.clone();
        super::clipboard::clear_data(true, move |ok| {
            let m = trl(if ok { "Clipboard cleared." } else { "Could not open clipboard." });
            if w.tray.hidden.get() {
                show_tray_info(&w, &m, !ok);
            } else if ok {
                info_box(&w.win(), &m);
            } else {
                msg(Some(&w.win()), &m, MsgIcon::Error);
            }
        });
    }

    /// `PasswBoxMenuPopup()`: update enabled state of PasswBoxMenu items.
    fn update_passw_box_menu(&self) {
        let e = self.entry("PasswBox");
        let editable = e.is_editable();
        let selected = e.selection_bounds().is_some();
        let has_text = e.text_length() > 0;
        let db_open = self.passw_mng.borrow().as_ref().is_some_and(|m| m.is_db_open());
        let has_clip_text = gdk::Display::default()
            .map(|d| d.clipboard().formats().contains_type(glib::types::Type::STRING))
            .unwrap_or(true);
        self.b.action("PasswBoxMenu_Undo").set_enabled(editable);
        self.b.action("PasswBoxMenu_Cut").set_enabled(editable && selected);
        self.b.action("PasswBoxMenu_Copy").set_enabled(selected);
        self.b.action("PasswBoxMenu_EncryptCopy").set_enabled(selected);
        self.b.action("PasswBoxMenu_Paste").set_enabled(editable && has_clip_text);
        self.b.action("PasswBoxMenu_PerformAutotype").set_enabled(has_text);
        self.b.action("PasswBoxMenu_Delete").set_enabled(editable && selected);
        self.b.action("PasswBoxMenu_AddToDb").set_enabled(selected && db_open);
        self.b.action("PasswBoxMenu_SelectAll").set_enabled(has_text);
        self.b.action("PasswBoxMenu_SaveAsFile").set_enabled(has_text);
        self.b.action("PasswBoxMenu_EnablePasswTest").set_enabled(editable);
    }

    fn passw_box_copy(&self) {
        let e = self.entry("PasswBox");
        if let Some((s, end)) = e.selection_bounds() {
            let text = Zeroizing::new(e.chars(s, end).to_string());
            super::clipboard::set_data(&text);
        }
    }

    fn passw_box_save_as_file(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::new();
        dlg.set_title(&trl("Save password"));
        let w = self.clone();
        dlg.save(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r {
                if let Some(p) = f.path() {
                    let core = w.core.borrow();
                    let text = Zeroizing::new(w.entry("PasswBox").text().to_string());
                    let res = pwtool_core::text_file::TextFileWriter::create(&p, core.file_encoding, true)
                        .and_then(|mut f| f.write_str(&text));
                    drop(core);
                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                    match res {
                        Ok(()) => msg(
                            Some(&w.win()),
                            &trl_format("File \"%1\" successfully created.", &[&name]),
                            MsgIcon::Info,
                        ),
                        Err(e) => msg(
                            Some(&w.win()),
                            &trl_format("Error while creating file\n\"%1\":\n%2.", &[&p.to_string_lossy(), &e.to_string()]),
                            MsgIcon::Error,
                        ),
                    }
                }
            }
        });
    }

    /// `PasswBoxChange()`: password quality test while typing.
    fn passw_box_change(&self) {
        let tag = self.passw_box_tag.get();
        if tag & PASSWBOX_TAG_PASSW_GEN != 0 {
            self.passw_box_tag.set(tag & !PASSWBOX_TAG_PASSW_GEN);
            self.auto_clear_passw_cnt.set(0);
        }
        if tag & PASSWBOX_TAG_PASSW_TEST == 0 {
            return;
        }
        let text = Zeroizing::new(self.entry("PasswBox").text().to_string());
        let len = text.chars().count();
        let core = self.core.borrow();
        let mut bits = 0.0;
        let mut common = false;
        if len != 0 {
            if core.test_common_passw {
                if let Some(c) = core.common_passw.as_ref().filter(|c| !c.is_empty()) {
                    if c.contains(&text) {
                        common = true;
                        bits = c.entropy();
                    }
                }
            }
            if !common {
                bits = if core.use_advanced_passw_est {
                    pwtool_core::zxcvbn::zxcvbn_match(&text)
                } else {
                    PasswordGenerator::estimate_passw_security(&text)
                };
            }
        }
        drop(core);
        self.show_passw_info(len, bits, common, true);
    }

    /// `ShowPasswInfo()`.
    fn show_passw_info(&self, len: usize, bits: f64, common: bool, estimated: bool) {
        self.passw_bar.set_quality(bits);
        let l = self.label("PasswInfoLbl");
        let mut caption = String::new();
        if common {
            caption.push_str("**");
            l.set_tooltip_text(Some(&trl("Matches common password!")));
        } else {
            if estimated {
                caption.push('*');
            }
            l.set_tooltip_text(Some(&trl("Hold left mouse button to drag & drop password")));
        }
        caption.push_str(&trl_format("%1 bits / %2 characters", &[&format_float_1(bits), &len.to_string()]));
        l.set_text(&caption);
    }

    /// `SetAdvancedBtnCaption()`.
    fn set_advanced_btn_caption(&self) {
        let flags = self.core.borrow().settings.passw_options.flags;
        let num = flags.count_ones();
        let mut hint = String::new();
        let dlg = self.passw_options_dlg.borrow().clone();
        if num > 0 {
            for i in 0..PASSWOPTIONS_NUM {
                if flags & (1 << i) != 0 {
                    let mut opt = String::new();
                    if PASSWOPTIONS_STARRED[i] {
                        opt.push('*');
                    }
                    opt.push_str(&dlg.as_ref().map(|d| d.bit_to_string(i)).unwrap_or_else(|| PasswOptionsDlg::bit_name(i)));
                    if num > 5 && opt.chars().count() > 40 {
                        opt = opt.chars().take(37).collect::<String>() + "...";
                    }
                    hint.push('\n');
                    hint.push_str(&opt);
                }
            }
        }
        let mut caption = trl("Advanced");
        let hint = format!(
            "{} - {}{hint}",
            trl("Advanced password options"),
            trl_format("%1 option(s) active", &[&num.to_string()])
        );
        if num > 0 {
            caption.push_str(&format!("|{num}"));
            if flags & passwoptions_starred_bitfield() != 0 {
                caption.push('*');
            }
        }
        let b = self.button("AdvancedBtn");
        b.set_label(&caption);
        b.set_tooltip_text(Some(&hint));
    }

    fn options_dlg(&self) -> Rc<PasswOptionsDlg> {
        if self.passw_options_dlg.borrow().is_none() {
            *self.passw_options_dlg.borrow_mut() = Some(PasswOptionsDlg::new(&self.win()));
        }
        self.passw_options_dlg.borrow().clone().unwrap()
    }

    /// `AdvancedBtnClick()`.
    fn advanced_btn_click(self: &Rc<Self>) {
        let dlg = self.options_dlg();
        let w = self.clone();
        glib::spawn_future_local(async move {
            let old = w.core.borrow().settings.passw_options.clone();
            if let Some(new) = dlg.run(&old).await {
                w.apply_passw_options(old, new);
            }
        });
    }

    fn apply_passw_options(&self, old: pwtool_core::generate::PasswOptions, new: pwtool_core::generate::PasswOptions) {
        self.core.borrow_mut().settings.passw_options = new.clone();
        if new.min_word_len != old.min_word_len
            || new.max_word_len != old.max_word_len
            || (new.flags ^ old.flags) & pwtool_core::generate::PASSWOPTION_LOWERCASEWORDS != 0
        {
            let wl = combo_text(&self.combo("WLFNList"));
            self.load_word_list_file(&wl);
        }
        if new.trigram_file_name != old.trigram_file_name {
            let ok = self.core.borrow_mut().load_trigram_file(&new.trigram_file_name) > 0;
            let errs: Vec<String> = self.core.borrow_mut().startup_errors.drain(..).collect();
            for e in errs {
                msg(Some(&self.win()), &e, MsgIcon::Error);
            }
            if !ok {
                self.core.borrow_mut().settings.passw_options.trigram_file_name = old.trigram_file_name.clone();
            }
        }
        let cs = combo_text(&self.combo("CharSetList"));
        self.load_char_set(&cs);
        {
            let mut core = self.core.borrow_mut();
            core.passw_gen.set_word_separator(&new.word_separator);
            core.passw_gen.set_word_char_separator(&new.word_char_separator);
        }
        self.set_advanced_btn_caption();
    }

    fn deactivate_options(&self, mask: u32) {
        let old = self.core.borrow().settings.passw_options.clone();
        let mut new = old.clone();
        new.flags &= !mask;
        if new.flags != old.flags {
            self.apply_passw_options(old, new);
        }
    }

    /// `LoadProfile(index)`.
    pub(super) fn load_profile(&self, i: usize) {
        self.load_profile_inner(i, false);
    }

    fn load_profile_inner(&self, i: usize, is_startup: bool) {
        if i >= self.core.borrow().profiles.len() {
            return;
        }
        let err_start = self.core.borrow().startup_errors.len();
        self.core.borrow_mut().load_profile(i);
        if !is_startup {
            let errs: Vec<String> = self.core.borrow_mut().startup_errors.drain(err_start..).collect();
            for e in errs {
                msg(Some(&self.win()), &e, MsgIcon::Error);
            }
        }
        // reflect settings in the controls without mutating combo history lists
        let (s, wl_err) = {
            let core = self.core.borrow();
            (core.settings.clone(), core.word_list_error.is_some())
        };
        self.check("IncludeCharsCheck").set_active(s.include_chars);
        self.b.updowns["CharsLengthSpinBtn"].set_position(s.chars_length as i64);
        set_combo_text(&self.combo("CharSetList"), &s.char_set_text);
        *self.char_set_input.borrow_mut() = s.char_set_text.clone();
        self.check("IncludeWordsCheck").set_active(s.include_words);
        self.b.updowns["WordsNumSpinBtn"].set_position(s.words_num as i64);
        set_combo_text(&self.combo("WLFNList"), &s.wlfn_text);
        if !wl_err {
            *self.wl_file_name.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
            self.wl_file_name_err.borrow_mut().clear();
        } else {
            *self.wl_file_name_err.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
        }
        self.check("CombineWordsCharsCheck").set_active(s.combine_words_chars);
        self.check("SpecifyLengthCheck").set_active(s.passphr_spec_len);
        self.entry("SpecifyLengthBox").set_text(&s.passphr_spec_len_string);
        self.check("FormatPasswCheck").set_active(s.format_passw);
        set_combo_text(&self.combo("FormatList"), &s.format_text);
        self.label("FormatPasswInfoLbl").set_text("");
        self.check("RunScriptCheck").set_active(s.run_script);
        set_combo_text(&self.combo("ScriptList"), &s.script_text);
        self.entry("NumPasswBox").set_text(&s.num_passw);
        self.set_action_checked("GenerateMenu_IncludeHeader", s.include_header);
        self.update_char_set_info();
        self.update_word_list_info();
        self.set_advanced_btn_caption();
        self.include_chars_check_click();
        self.select_profile_in_list(i);
    }

    /// `ProfileList->ItemIndex = i` without loading the profile again.
    fn select_profile_in_list(&self, i: usize) {
        self.profile_list_guard.set(true);
        self.combo("ProfileList").set_active(Some(i as u32));
        self.profile_list_guard.set(false);
    }

    /// Copy the controls into the generator settings.
    fn sync_settings(&self) {
        let mut core = self.core.borrow_mut();
        let s = &mut core.settings;
        s.include_chars = self.check("IncludeCharsCheck").is_active();
        s.chars_length = self.b.updowns["CharsLengthSpinBtn"].position() as i32;
        s.char_set_text = combo_text(&self.combo("CharSetList"));
        s.char_set_list = self.combo_items("CharSetList");
        if s.char_set_list.is_empty() {
            s.char_set_list.push(pwtool_core::config::CHARSETLIST_DEFAULTENTRIES[0].into());
        }
        s.include_words = self.check("IncludeWordsCheck").is_active();
        s.words_num = self.b.updowns["WordsNumSpinBtn"].position() as i32;
        s.wlfn_text = combo_text(&self.combo("WLFNList"));
        s.wlfn_list = self.combo_items("WLFNList");
        if s.wlfn_list.is_empty() {
            s.wlfn_list.push(WLFNLIST_DEFAULT.into());
        }
        s.combine_words_chars = self.check("CombineWordsCharsCheck").is_active();
        s.passphr_spec_len = self.check("SpecifyLengthCheck").is_active();
        s.passphr_spec_len_string = self.entry("SpecifyLengthBox").text().to_string();
        s.format_passw = self.check("FormatPasswCheck").is_active();
        s.format_text = combo_text(&self.combo("FormatList"));
        s.format_list = self.combo_items("FormatList");
        s.run_script = self.check("RunScriptCheck").is_active();
        s.script_text = combo_text(&self.combo("ScriptList"));
        s.script_list = self.combo_items("ScriptList");
        s.num_passw = self.entry("NumPasswBox").text().to_string();
        s.include_header = self.action_checked("GenerateMenu_IncludeHeader");
    }

    // ----------------------------------------------------------------------
    // generation

    /// `GeneratePassw()`.
    pub(super) fn generate_passw(self: &Rc<Self>, dest: GenDest) {
        if self.generating.get() {
            self.gen_target.borrow_mut().take();
            return;
        }
        self.generating.set(true);
        self.char_set_list_exit();
        self.wlfn_list_exit();
        self.sync_settings();
        let w = self.clone();
        glib::spawn_future_local(async move { w.generate_passw_async(dest).await });
    }

    /// Generate a password from a hot key when the main window was hidden, and
    /// re-minimize/hide once done (matches `Main.cpp:OnHotKey`).
    pub(super) fn generate_passw_and_rehide(self: &Rc<Self>, dest: GenDest) {
        if self.generating.get() {
            self.gen_target.borrow_mut().take();
            return;
        }
        self.generating.set(true);
        self.char_set_list_exit();
        self.wlfn_list_exit();
        self.sync_settings();
        let w = self.clone();
        glib::spawn_future_local(async move {
            let w2 = w.clone();
            w.generate_passw_async(dest).await;
            w2.window.minimize();
        });
    }

    async fn generate_passw_async(self: Rc<Self>, dest: GenDest) {
        struct GenGuard(Rc<MainWindow>);
        impl Drop for GenGuard {
            fn drop(&mut self) {
                self.0.generating.set(false);
            }
        }
        let gen_guard = GenGuard(self.clone());
        let gen_target = self.gen_target.borrow_mut().take();
        let win = self.win();
        let mut num: u64 = 1;
        let mut file: Option<(PathBuf, bool)> = None;
        if dest.is_list() {
            match parse_num_passwords(&self.entry("NumPasswBox").text()) {
                Some(n) => num = n,
                None => {
                    msg_box(
                        Some(&win),
                        &trl_format("Invalid number of passwords.\nAllowed range is %1-%2.", &["2", &PASSW_MAX_NUM.to_string()]),
                        MsgIcon::Error,
                        MsgButtons::Ok,
                        false,
                    )
                    .await;
                    self.entry("NumPasswBox").grab_focus();
                    return;
                }
            }
            if dest == GenDest::FileList {
                let dlg = gtk::FileDialog::new();
                let Ok(f) = dlg.save_future(Some(&self.window)).await else { return };
                let Some(path) = f.path() else { return };
                let mut append = false;
                if path.exists() {
                    match msg_box(
                        Some(&win),
                        &trl("The selected file already exists. Do you\nwant to append passwords to this file\n(existing passwords will be preserved),\nor do you want to completely overwrite it?\n\nYes -> append passwords,\nNo -> overwrite file,\nCancel -> cancel process."),
                        MsgIcon::Question,
                        MsgButtons::YesNoCancel,
                        false,
                    )
                    .await
                    {
                        MsgResult::Yes => append = true,
                        MsgResult::Cancel => return,
                        _ => {}
                    }
                }
                file = Some((path, append));
            }
        }

        if dest == GenDest::GuiList {
            if let Some(pl) = self.passw_list.borrow().as_ref() {
                pl.win.close();
            }
        }

        // limits, passphrase length, script
        let mut req: GenRequest;
        {
            let core = self.core.borrow();
            let s = &core.settings;
            if s.include_chars
                && s.passw_options.flags & PASSWOPTION_EACHCHARONLYONCE != 0
                && s.chars_length > core.passw_gen.custom_char_set_unique_size()
            {
                let n = core.passw_gen.custom_char_set_unique_size();
                drop(core);
                msg_box(
                    Some(&win),
                    &trl_format(
                        "If the option \"Each character must occur\nonly once\" is activated, the password length\nis limited to %1 unique characters.",
                        &[&n.to_string()],
                    ),
                    MsgIcon::Warning,
                    MsgButtons::Ok,
                    false,
                )
                .await;
                self.b.updowns["CharsLengthSpinBtn"].set_position(n as i64);
                self.core.borrow_mut().settings.chars_length = n;
            }
        }
        if self.core.borrow().settings.include_words && self.core.borrow().settings.passphr_spec_len {
            let t = self.core.borrow().settings.passphr_spec_len_string.clone();
            if let Err(e) = parse_passphrase_length(&t) {
                msg_box(Some(&win), &e, MsgIcon::Error, MsgButtons::Ok, false).await;
                return;
            }
        }
        {
            let core = self.core.borrow();
            let s = &core.settings;
            if s.include_words
                && s.passw_options.flags & PASSWOPTION_EACHWORDONLYONCE != 0
                && s.words_num > core.passw_gen.word_list_size()
            {
                let n = core.passw_gen.word_list_size();
                drop(core);
                msg_box(
                    Some(&win),
                    &trl_format(
                        "If the option \"Each word must occur only\nonce\" is activated, the number of words\nis limited to the size of the word list (%1).",
                        &[&n.to_string()],
                    ),
                    MsgIcon::Warning,
                    MsgButtons::Ok,
                    false,
                )
                .await;
                self.b.updowns["WordsNumSpinBtn"].set_position(n as i64);
                self.core.borrow_mut().settings.words_num = n;
            }
        }
        match self.core.borrow().gen_request(dest, num) {
            Ok(r) => req = r,
            Err(e) => {
                msg(Some(&win), &e, MsgIcon::Error);
                return;
            }
        }

        // script
        let script_file = self.core.borrow().script_file();
        let mut script: Option<LuaScript> = None;
        if let Some(path) = script_file {
            let loaded = self.script.borrow_mut().take().filter(|s| s.file_name() == path);
            let s = match loaded {
                Some(mut s) => {
                    s.reset_init();
                    s
                }
                None => {
                    let mut s = LuaScript::new();
                    let core = self.core.borrow();
                    let r = s.load_file(&path, Some(&core.passw_gen), &mut *RandomPool::global());
                    drop(core);
                    if let Err(e) = r {
                        msg(Some(&win), &trl_format("Error while loading script:\n%1.", &[&e.to_string()]), MsgIcon::Error);
                        return;
                    }
                    s
                }
            };
            let text = combo_text(&self.combo("ScriptList"));
            self.add_entry_to_list("ScriptList", &text, true);
            script = Some(s);
        }
        if req.chars_len == 0 && req.num_words == 0 && req.format.is_empty() && script.is_none() {
            return;
        }
        if dest == GenDest::FileList {
            req.file_newline = self.core.borrow().newline.clone();
        }

        // random generator: deterministic, or a pool derived from the main pool
        let det = self.key_seeded_prng.borrow_mut().take();
        let is_pool = det.is_none();
        let rng = match det {
            Some(g) => GenRng::Det(g),
            None => {
                let mut pool = RandomPool::global();
                EntropyManager::global().add_system_entropy(&mut pool);
                GenRng::Pool(RandomPool::derive_from(&mut pool))
            }
        };

        let pg = std::mem::take(&mut self.core.borrow_mut().passw_gen);
        let common = self.core.borrow_mut().common_passw.take();
        let file_encoding = self.core.borrow().file_encoding;
        let script_cancel = script.as_ref().map(|s| s.cancel_flag());
        if let Some(sc) = &script_cancel {
            sc.store(false, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let (tx, rx) = async_channel::bounded::<(GenOutput, Option<pwtool_core::generate::CommonPasswords>)>(1);
        let c2 = cancel.clone();
        let p2 = progress.clone();
        let format_text = req.format.clone();
        let file_t = file.clone();
        std::thread::spawn(move || {
            let file = file_t;
            let mut rng = rng;
            let mut script = script;
            let mut single = Zeroizing::new(String::new());
            let mut list = Zeroizing::new(String::new());
            let mut list_utf16_units: u64 = 0;
            let mut first_list_item = true;
            let mut mem_limit_reached = false;
            let mut header_len = 0usize;
            let check_each = req.num_passw > 1 && req.option_flags & pwtool_core::generate::PASSWOPTION_CHECKEACHPASSW != 0;
            let mut file_writer: Option<(std::fs::File, pwtool_core::text_file::CharEncoding)> = None;
            let result = {
                let mut gen_flags = 0;
                if req.chars_len != 0 {
                    gen_flags |= 1;
                }
                if req.num_words != 0 {
                    gen_flags |= 2;
                }
                if !req.format.is_empty() {
                    gen_flags |= 4;
                }
                let standalone = script.as_ref().map(|s| s.flags() & FLAG_STANDALONE_PASSW_GEN != 0).unwrap_or(false);
                let has_script = script.is_some();
                let g = &pg;
                let mut hook = |r: &mut dyn RandomGenerator, n: u64, p: Option<&str>, sec: f64| {
                    let s = script.as_mut().unwrap();
                    if !s.is_initialized() {
                        s.init_generate(g, r, num, dest as i32, gen_flags, req.option_flags, req.chars_len, req.num_words, &format_text)
                            .map_err(|e| e.to_string())?;
                    }
                    s.call_generate(g, r, n, p, sec).map_err(|e| e.to_string())
                };
                let hook_arg: Option<(&mut pwtool_core::generate::ScriptHook<'_>, bool)> =
                    if has_script { Some((&mut hook, standalone)) } else { None };
                let r = generate(&pg, &mut rng, &req, common.as_ref(), hook_arg, &c2, &p2, &mut |p, s| {
                    match dest {
                        GenDest::GuiList | GenDest::ClipboardList => {
                            let item_u16 = (p.passw.encode_utf16().count() + p.appendix.encode_utf16().count()) as u64;
                            if first_list_item {
                                first_list_item = false;
                                let mut hdr_u16 = 0u64;
                                if req.include_header {
                                    let total = if check_each { MAX_ENTROPY as f64 } else { s.total_security };
                                    let h = passw_list_header(req.num_passw, s.first_security, total);
                                    header_len = h.len();
                                    hdr_u16 = h.encode_utf16().count() as u64;
                                    list.push_str(&h);
                                    list_utf16_units = hdr_u16;
                                }
                                let est_bytes = hdr_u16
                                    .saturating_add(item_u16.saturating_mul(req.num_passw))
                                    .saturating_add(1)
                                    .saturating_mul(2);
                                if est_bytes > pwtool_core::generate::PASSWLIST_MAX_BYTES {
                                    return Err(trl("The estimated list size will probably exceed\nthe internal memory limit.\nPlease reduce the number of passwords"));
                                }
                            }
                            if list_utf16_units.saturating_add(item_u16).saturating_add(1).saturating_mul(2)
                                > pwtool_core::generate::PASSWLIST_MAX_BYTES
                            {
                                mem_limit_reached = true;
                                return Ok(SinkAction::Stop);
                            }
                            list.push_str(p.passw);
                            list.push_str(p.appendix);
                            list_utf16_units += item_u16;
                        }
                        GenDest::FileList => {
                            if file_writer.is_none() {
                                if let Some((path, append)) = &file {
                                    let opened = if *append {
                                        let enc = pwtool_core::text_file::detect_file_encoding(path, file_encoding)
                                            .unwrap_or(file_encoding);
                                        std::fs::OpenOptions::new().append(true).open(path).map(|f| (f, enc))
                                    } else {
                                        std::fs::File::create(path).and_then(|mut f| {
                                            use std::io::Write;
                                            f.write_all(pwtool_core::text_file::bom(file_encoding))?;
                                            Ok((f, file_encoding))
                                        })
                                    };
                                    match opened {
                                        Ok(x) => file_writer = Some(x),
                                        Err(e) => return Err(e.to_string()),
                                    }
                                }
                            }
                            if let Some((f, enc)) = file_writer.as_mut() {
                                use std::io::Write;
                                let mut t = Zeroizing::new(p.passw.to_string());
                                t.push_str(p.appendix);
                                f.write_all(&pwtool_core::text_file::encode_text(&t, *enc)).map_err(|e| e.to_string())?;
                            }
                        }
                        _ => {
                            single = Zeroizing::new(p.passw.to_string());
                        }
                    }
                    Ok(SinkAction::Continue)
                });
                if let Ok(s) = &r {
                    if matches!(dest, GenDest::GuiList | GenDest::ClipboardList) && s.count != 0 {
                        if req.include_header
                            && (s.count < req.num_passw || (check_each && s.total_security < MAX_ENTROPY as f64))
                        {
                            let h = passw_list_header(s.count, s.first_security, s.total_security);
                            if h.len() <= header_len {
                                let mut n = Zeroizing::new(h);
                                n.push_str(&list[header_len..]);
                                list = n;
                            }
                        }
                        let mut cut = 0;
                        for _ in 0..2 {
                            if let Some(c) = list[..list.len() - cut].chars().last() {
                                cut += c.len_utf8();
                            }
                        }
                        let nl = list.len() - cut;
                        list.truncate(nl);
                    }
                }
                r
            };
            let _ = tx.send_blocking((
                GenOutput { pg, rng, script, result, single, list, mem_limit_reached },
                common,
            ));
        });

        // Wait for the worker directly (`rx.recv()`): fast generations (< 1 ms)
        // finish immediately without sleeping or toggling `window.set_sensitive`,
        // which on GTK4/Wayland resets pointer-crossing state on hovered buttons
        // and can swallow the next click.
        let mut progress_win: Option<Rc<ProgressWindow>> = None;
        let mut win_disabled = false;
        let start = std::time::Instant::now();
        let mut last_prog = 0u64;
        let mut last_prog_change = std::time::Instant::now();
        let mut recv_fut = std::pin::pin!(rx.recv());
        let out = loop {
            let tick = std::pin::pin!(glib::timeout_future(std::time::Duration::from_millis(25)));
            match futures_util::future::select(recv_fut, tick).await {
                futures_util::future::Either::Left((res, _)) => {
                    break res.expect("generator thread exited unexpectedly");
                }
                futures_util::future::Either::Right((_, pending)) => {
                    recv_fut = pending;
                }
            }
            let elapsed = start.elapsed().as_millis();
            if !win_disabled && ((dest == GenDest::GuiList && num >= 100_000) || elapsed >= 200) {
                self.window.set_sensitive(false);
                win_disabled = true;
            }
            let cur_prog = progress.load(Ordering::Relaxed);
            if cur_prog != last_prog {
                last_prog = cur_prog;
                last_prog_change = std::time::Instant::now();
            }
            if progress_win.is_none()
                && ((dest == GenDest::GuiList && num >= 100_000) || elapsed >= 1000)
            {
                let title = trl(if num == 1 { "Generating password" } else { "Generating password list" });
                let pw = ProgressWindow::new(&win, &title, "");
                pw.win.present();
                progress_win = Some(pw);
            }
            if let Some(pw) = &progress_win {
                if pw.cancel.get() {
                    cancel.store(true, Ordering::Relaxed);
                    if let Some(sc) = &script_cancel {
                        sc.store(true, Ordering::Relaxed);
                    }
                }
                if script_cancel.is_some() && last_prog_change.elapsed().as_millis() >= 1000 {
                    pw.set_message(&trl("Waiting for script ..."));
                } else {
                    pw.set(cur_prog, num, "%1 of %2 passwords generated.");
                }
            }
        };
        if let Some(pw) = progress_win {
            pw.win.destroy();
        }
        if win_disabled {
            self.window.set_sensitive(true);
        }
        let (out, common) = out;
        {
            let mut core = self.core.borrow_mut();
            core.passw_gen = out.pg;
            core.common_passw = common;
        }
        drop(gen_guard);
        let cancelled = cancel.load(Ordering::Relaxed);
        *self.script.borrow_mut() = if cancelled && out.result.is_err() { None } else { out.script };
        if let GenRng::Det(g) = out.rng {
            // keep the state of the deterministic generator
            *self.key_seeded_prng.borrow_mut() = Some(g);
        }
        let summary = match out.result {
            Ok(s) => s,
            Err(e) => {
                if is_pool {
                    RandomPool::global().flush();
                    self.update_entropy_progress(false);
                }
                if cancelled && script_cancel.is_some() {
                    msg_box(
                        Some(&win),
                        &trl("Process was cancelled by the user"),
                        MsgIcon::Warning,
                        MsgButtons::Ok,
                        false,
                    )
                    .await;
                    return;
                }
                let mut m = trl_format("An error has occurred during password generation:\n%1.", &[&e]);
                if let (GenDest::FileList, Some((p, _))) = (dest, &file) {
                    m.push_str("\n\n");
                    m.push_str(&trl_format(
                        "%1 passwords written to file \"%2\".",
                        &[&progress.load(Ordering::Relaxed).to_string(), &p.file_name().unwrap_or_default().to_string_lossy()],
                    ));
                }
                msg(Some(&win), &m, MsgIcon::Error);
                return;
            }
        };
        if out.mem_limit_reached {
            let count_str = summary.count.to_string();
            self.entry("NumPasswBox").set_text(&count_str);
            msg_box(
                Some(&win),
                &trl_format(
                    "The memory limit of the password list has been reached. The number of passwords has been reduced to %1.",
                    &[&count_str],
                ),
                MsgIcon::Warning,
                MsgButtons::Ok,
                false,
            )
            .await;
        }
        if cancelled && num > 1 {
            msg_box(
                Some(&win),
                &format!(
                    "{}.\n\n{}",
                    trl("Process was cancelled by the user"),
                    trl_format("%1 passwords generated.", &[&summary.count.to_string()])
                ),
                MsgIcon::Warning,
                MsgButtons::Ok,
                false,
            )
            .await;
        }
        if dest == GenDest::GuiSingle || summary.count > 0 {
            self.label("FormatPasswInfoLbl").set_text(&summary.format_info);
        }
        if summary.count != 0 {
            match dest {
                GenDest::GuiSingle if gen_target.is_some() => {
                    if let Some(e) = &gen_target {
                        e.set_text(&out.single);
                    }
                }
                GenDest::GuiSingle => {
                    let e = self.entry("PasswBox");
                    let tag = self.passw_box_tag.get();
                    self.passw_box_tag.set(0);
                    e.set_text(&out.single);
                    self.passw_box_tag.set(tag | PASSWBOX_TAG_PASSW_GEN);
                    self.show_passw_info(summary.last_len, summary.last_security, summary.last_common_match, false);
                    e.grab_focus();
                    let core = self.core.borrow();
                    if core.config.auto_clear_passw {
                        self.auto_clear_passw_cnt.set(core.config.auto_clear_passw_time);
                    }
                }
                GenDest::GuiList => {
                    if self.passw_list.borrow().is_none() {
                        let w_enc = Rc::downgrade(&self);
                        let w_db = Rc::downgrade(&self);
                        let pl = PasswListWindow::new(
                            &win,
                            Box::new(move || {
                                w_enc
                                    .upgrade()
                                    .map(|w| {
                                        let c = w.core.borrow();
                                        (c.file_encoding, c.newline.clone())
                                    })
                                    .unwrap_or((pwtool_core::text_file::CharEncoding::Utf8, "\n".to_string()))
                            }),
                            Box::new(move || {
                                w_db.upgrade()
                                    .and_then(|w| w.passw_mng.borrow().as_ref().map(|m| m.is_db_open()))
                                    .unwrap_or(false)
                            }),
                        );
                        pl.load_config(&self.core.borrow().ini);
                        self.attach_entropy_events(&pl.win);
                        let w = Rc::downgrade(&self);
                        let pl2 = Rc::downgrade(&pl);
                        pl.b.action("PasswListMenu_EncryptCopy").connect_activate(move |_, _| {
                            if let (Some(w), Some(pl)) = (w.upgrade(), pl2.upgrade()) {
                                let text = pl.selected_text();
                                let parent = pl.win.clone();
                                glib::spawn_future_local(async move { w.crypt_text(true, Some(text), Some(parent)).await });
                            }
                        });
                        let w = Rc::downgrade(&self);
                        let pl2 = Rc::downgrade(&pl);
                        pl.b.action("PasswListMenu_AddToDb").connect_activate(move |_, _| {
                            if let (Some(w), Some(pl)) = (w.upgrade(), pl2.upgrade()) {
                                w.add_passw_to_db(&pl.selected_text(), true, None);
                            }
                        });
                        *self.passw_list.borrow_mut() = Some(pl);
                    }
                    self.passw_list.borrow().as_ref().unwrap().show_list(&out.list);
                }
                GenDest::ClipboardList => {
                    super::clipboard::set_data(&out.list);
                    info_box(&win, &trl("Password list copied to clipboard."));
                }
                GenDest::FileList => {
                    let name = file.as_ref().map(|(p, _)| p.file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();
                    msg(
                        Some(&win),
                        &trl_format("File \"%1\" successfully created.\n\n%2 passwords written.", &[&name, &summary.count.to_string()]),
                        MsgIcon::Info,
                    );
                }
                GenDest::Clipboard => {
                    super::clipboard::set_data(&out.single);
                    show_tray_info(&self, &trl("Password copied to clipboard."), false);
                }
                GenDest::Autotype => {
                    let delay = self.core.borrow().config.autotype_delay.clamp(0, 10000) as u32;
                    let mut strokes = Vec::new();
                    super::autotype::string_sequence(&out.single, delay, &mut strokes);
                    self.send_keys(&win, strokes, false);
                }
                GenDest::MsgBox => {
                    let len = out.single.chars().count();
                    if len > 1000 {
                        msg_box(
                            Some(&win),
                            &trl("Password is too long to show it in a\nmessage box like this.\n\nIt will be copied to the clipboard."),
                            MsgIcon::Warning,
                            MsgButtons::Ok,
                            false,
                        )
                        .await;
                        super::clipboard::set_data(&out.single);
                        show_tray_info(&self, &trl("Password copied to clipboard."), false);
                    } else {
                        let text = pwtool_core::unicode::format_w(
                            &trl("The generated password is:\n\n%1\n\nThe estimated security is %2 bits.\n\nYes -> copy password to clipboard,\nNo -> generate new password,\nCancel -> cancel process."),
                            &[&out.single, &format_float_1(summary.last_security)],
                        );
                        match msg_box(Some(&win), &text, MsgIcon::Info, MsgButtons::YesNoCancel, false).await {
                            MsgResult::Yes => {
                                super::clipboard::set_data(&out.single);
                                show_tray_info(&self, &trl("Password copied to clipboard."), false);
                            }
                            MsgResult::No => {
                                self.generate_passw(GenDest::MsgBox);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if is_pool {
            RandomPool::global().flush();
            EntropyManager::global().consume_entropy_bits(summary.total_security.ceil() as u32);
            self.update_entropy_progress(false);
        }
    }

    // ----------------------------------------------------------------------
    // entropy, timer

    /// `UpdateEntropyProgress()`.
    fn update_entropy_progress(&self, force: bool) {
        let _ = force;
        let status: gtk::Label = self.b.get("StatusBar.label");
        if !self.show_ent_progress.get() {
            status.set_text(&format!("{PROGRAM_NAME} {}", version_string()));
            return;
        }
        let em = EntropyManager::global();
        let bits = em.entropy_bits();
        let max = bits == pwtool_core::random::entropy::MAX_ENTROPYBITS;
        let pct = (100.0 * bits as f64 / MAX_ENTROPY as f64).round().min(100.0) as i32;
        let text = format!(
            "{}{}{} ({}%)",
            if max { ">" } else { "" },
            bits,
            if bits > MAX_ENTROPY { "+" } else { "" },
            pct
        );
        status.set_text(&format!("{PROGRAM_NAME} {}        E  {text}", version_string()));
    }

    fn setup_entropy_events(self: &Rc<Self>) {
        let win = self.window.clone();
        self.attach_entropy_events(&win);
    }

    /// Collect entropy from keyboard and mouse events of a window (the
    /// original does this application-wide in `AppMessage()`).
    pub fn attach_entropy_events(self: &Rc<Self>, window: &impl IsA<gtk::Widget>) {
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        key.connect_key_pressed(move |_, k, code, _| {
            if let Some(w) = w.upgrade() {
                w.notify_user_action();
                let mut em = EntropyManager::global();
                let bits = em.add_event(&mut RandomPool::global(), EntropyEventType::Keyboard, k.into_glib() as u64, code as u64, &[], ENTROPY_KEYBOARD);
                drop(em);
                if bits != 0 {
                    w.update_entropy_progress(false);
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key);
        let click = gtk::EventControllerLegacy::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        click.connect_event(move |_, ev| {
            if let Some(btn) = ev.downcast_ref::<gdk::ButtonEvent>() {
                if btn.event_type() == gdk::EventType::ButtonPress {
                    if let Some(w) = w.upgrade() {
                        w.notify_user_action();
                        let (x, y) = ev.position().unwrap_or((0.0, 0.0));
                        let pos = ((x as u64) << 32) | (y as u64 & 0xffff_ffff);
                        let bits = EntropyManager::global().add_event(
                            &mut RandomPool::global(),
                            EntropyEventType::MouseClick,
                            btn.button() as u64,
                            pos,
                            &[],
                            ENTROPY_MOUSECLICK,
                        );
                        if bits != 0 {
                            w.update_entropy_progress(false);
                        }
                    }
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        let w = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(w) = w.upgrade() {
                let c = w.mouse_move_cnt.get();
                if c == 0 {
                    let pos = ((x as u64) << 32) | (y as u64 & 0xffff_ffff);
                    let bits = EntropyManager::global().add_event(&mut RandomPool::global(), EntropyEventType::MouseMove, 0, pos, &[], ENTROPY_MOUSEMOVE);
                    w.mouse_move_cnt.set(50 + pwtool_core::random::fprng_rand_range(50) as i32);
                    if bits != 0 {
                        w.update_entropy_progress(false);
                    }
                } else {
                    w.mouse_move_cnt.set(c - 1);
                }
            }
        });
        window.add_controller(motion);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |_, dx, dy| {
            if let Some(w) = w.upgrade() {
                w.notify_user_action();
                let c = w.mouse_wheel_cnt.get();
                if c <= 0 {
                    let bits = EntropyManager::global().add_event(
                        &mut RandomPool::global(),
                        EntropyEventType::MouseWheel,
                        dx.to_bits(),
                        dy.to_bits(),
                        &[],
                        ENTROPY_MOUSECLICK,
                    );
                    w.mouse_wheel_cnt.set(pwtool_core::random::fprng_rand_range(10) as i32);
                    if bits != 0 {
                        w.update_entropy_progress(false);
                    }
                } else {
                    w.mouse_wheel_cnt.set(c - 1);
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(scroll);
    }

    /// `TimerTick()` (every second).
    fn setup_timer(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || {
            let Some(w) = w.upgrade() else { return glib::ControlFlow::Break };
            let mut c = w.timer_cnt.get();
            c[0] += 1;
            if c[0] == TIMER_TOUCHPOOL {
                RandomPool::global().touch_pool();
                c[0] = 0;
            }
            c[1] += 1;
            if c[1] == TIMER_RANDOMIZE {
                EntropyManager::global().add_system_entropy(&mut RandomPool::global());
                w.update_entropy_progress(false);
                c[1] = 0;
            }
            c[2] += 1;
            if c[2] == TIMER_MOVEPOOL {
                RandomPool::global().move_pool();
                c[2] = 0;
            }
            c[3] += 1;
            if c[3] == TIMER_WRITESEEDFILE {
                w.core.borrow().write_rand_seed_file();
                c[3] = 0;
            }
            w.timer_cnt.set(c);
            w.tools_timer();
            w.tray_timer();
            let n = w.auto_clear_clip_cnt.get();
            if n > 0 {
                w.auto_clear_clip_cnt.set(n - 1);
                if n == 1 {
                    super::clipboard::clear_data(false, |_| {});
                }
            }
            let n = w.auto_clear_passw_cnt.get();
            if n > 0 {
                w.auto_clear_passw_cnt.set(n - 1);
                if n == 1 {
                    let tag = w.passw_box_tag.get();
                    w.passw_box_tag.set(0);
                    w.entry("PasswBox").set_text("");
                    w.passw_box_tag.set(tag);
                    w.label("PasswInfoLbl").set_text(&trl("Click on \"Generate\""));
                    w.label("PasswInfoLbl").set_tooltip_text(None);
                    w.passw_bar.clear();
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn timer_info(&self) {
        let kind = pwtool_core::random::hrtimer::timer_kind();
        let name = match kind {
            pwtool_core::random::hrtimer::TimerKind::Rdtsc => "Time stamp counter (RDTSC instruction)",
            pwtool_core::random::hrtimer::TimerKind::Qpc => "CLOCK_MONOTONIC_RAW",
            pwtool_core::random::hrtimer::TimerKind::None => "N/A",
        };
        let val = pwtool_core::random::hrtimer::high_res_timer().to_string();
        msg(
            Some(&self.win()),
            &trl_format("High-resolution timer used:\n%1.\n\nCurrent value: %2.", &[name, &val]),
            MsgIcon::Info,
        );
    }

    fn add_entropy_from_file(self: &Rc<Self>) {
        let dlg = gtk::FileDialog::new();
        dlg.set_title(&trl("Select high-entropy file"));
        let w = self.clone();
        dlg.open(Some(&self.window), gio::Cancellable::NONE, move |r| {
            let Ok(f) = r else { return };
            let Some(p) = f.path() else { return };
            use std::io::Read;
            match std::fs::File::open(&p) {
                Ok(mut f) => {
                    const MAX_BYTES: u64 = 1_000_000_000;
                    let mut buf = Zeroizing::new(vec![0u8; 65_536]);
                    let mut total: u64 = 0;
                    let mut bits: u32 = 0;
                    let mut em = EntropyManager::global();
                    let mut pool = RandomPool::global();
                    while total < MAX_BYTES {
                        let to_read = (MAX_BYTES - total).min(buf.len() as u64) as usize;
                        match f.read(&mut buf[..to_read]) {
                            Ok(0) => break,
                            Ok(n) => {
                                bits = bits.saturating_add(em.add_data(&mut pool, &buf[..n], 4.0, 4.0));
                                total += n as u64;
                            }
                            Err(_) => break,
                        }
                    }
                    pool.flush();
                    drop(pool);
                    drop(em);
                    w.update_entropy_progress(true);
                    msg(
                        Some(&w.win()),
                        &trl_format("%1 bits of entropy have been added to the random pool.", &[&bits.to_string()]),
                        MsgIcon::Info,
                    );
                }
                Err(_) => msg(Some(&w.win()), &trl("Could not open file."), MsgIcon::Error),
            }
        });
    }

    fn deactivate_det_rand_gen(&self) {
        if self.key_seeded_prng.borrow_mut().take().is_some() {
            self.window.set_title(Some(&self.default_title));
            self.b.action("MainMenu_Tools_DetermRandGen_Reset").set_enabled(false);
            self.b.action("MainMenu_Tools_DetermRandGen_Deactivate").set_enabled(false);
            info_box(&self.win(), &trl("Deterministic random generator has been DEACTIVATED."));
        }
    }

    /// Activate the deterministic generator with a master password
    /// (`MainMenu_Tools_DetermRandGen_SetupClick` + `UseKeySeededPRNG`).
    pub fn use_key_seeded_prng(&self, passw: &str) {
        self.use_key_seeded_prng_with(pwtool_core::mp_passw_gen::deterministic_generator(passw));
    }

    /// Generate a formatted password using the deterministic PRNG if active,
    /// otherwise the global random pool (used by PassCube's FormatPW menu).
    pub fn format_passw_with_active_rng(&self, format: &str) -> Option<Zeroizing<String>> {
        let pg = PasswordGenerator::new();
        let f = pwtool_core::unicode::str_to_w32(format);
        let r = if let Some(det) = self.key_seeded_prng.borrow_mut().as_mut() {
            pg.get_format_passw(det, &f, 0, None, 16000, None)
        } else {
            pg.get_format_passw(&mut *RandomPool::global(), &f, 0, None, 16000, None)
        };
        if r.passw.is_empty() {
            None
        } else {
            Some(pwtool_core::unicode::w32_to_string_secure(&r.passw))
        }
    }

    /// `UseKeySeededPRNG()` with an already seeded generator.
    pub fn use_key_seeded_prng_with(&self, prng: AesCtrPrng) {
        *self.key_seeded_prng.borrow_mut() = Some(prng);
        let title = pwtool_core::unicode::format_w(
            "%1 (%2)",
            &[PROGRAM_NAME, &trl("WARNING: Random generator is deterministic (press F7 to deactivate)!")],
        );
        self.window.set_title(Some(&title));
        self.b.action("MainMenu_Tools_DetermRandGen_Reset").set_enabled(true);
        self.b.action("MainMenu_Tools_DetermRandGen_Deactivate").set_enabled(true);
        info_box(&self.win(), &trl("Deterministic random generator has been ACTIVATED."));
    }

    // ----------------------------------------------------------------------
    // configuration

    /// `SaveConfig()`.
    pub fn save_config(&self) -> bool {
        self.save_config_to(None)
    }

    /// `SaveConfig(sFileName)`: `path` = backup file.
    pub fn save_config_to(&self, path: Option<&std::path::Path>) -> bool {
        self.sync_settings();
        self.save_tools_config();
        let mut core = self.core.borrow_mut();
        let ini = &mut core.ini;
        ini.write_integer("Main", "WindowWidth", self.window.width() as i64);
        ini.write_integer("Main", "WindowHeight", self.window.height() as i64);
        ini.write_bool("Main", "AlwaysOnTop", self.action_checked("MainMenu_Options_AlwaysOnTop"));
        ini.write_bool("Main", "HidePassw", self.b.get::<gtk::ToggleButton>("TogglePasswBtn").is_active());
        ini.write_bool("Main", "HideEntropyProgress", self.action_checked("MainMenu_Options_HideEntProgress"));
        ini.write_bool("Main", "SaveSettingsOnExit", self.action_checked("MainMenu_Options_SaveSettingsOnExit"));
        ini.write_bool("Main", "PasswEditable", self.action_checked("PasswBoxMenu_Editable"));
        ini.write_bool("Main", "EnablePasswTest", self.action_checked("PasswBoxMenu_EnablePasswTest"));
        match core.save_config_to(path) {
            Ok(()) => true,
            Err(e) => {
                drop(core);
                msg(
                    Some(&self.win()),
                    &trl_format("Error while writing to configuration file:\n%1.", &[&e.to_string()]),
                    MsgIcon::Error,
                );
                false
            }
        }
    }

    fn on_close(&self) {
        self.abort_autotype();
        if (self.action_checked("MainMenu_Options_SaveSettingsOnExit") && !self.core.borrow().cmd.config_readonly)
            || crate::app::restart_requested()
        {
            self.save_config();
        }
        let seed_ok = self.core.borrow().write_rand_seed_file();
        if !seed_ok {
            let path = self
                .core
                .borrow()
                .paths
                .data_dir
                .join(crate::program::PROGRAM_RANDSEEDFILE)
                .to_string_lossy()
                .into_owned();
            eprintln!(
                "{}",
                trl_format("Could not write to random seed file\n\"%1\".", &[&path])
            );
        }
        super::autotype::close_session();
        if let Some(app) = self.window.application() {
            super::active_window::stop_watching(&app);
        }
        crate::ui::hotkeys::deactivate();
        let tag = self.passw_box_tag.get();
        self.passw_box_tag.set(0);
        self.entry("PasswBox").set_text("");
        self.passw_box_tag.set(tag);
    }

    /// Profile given on the command line or configured for start-up
    /// (`StartupAction()`).
    fn startup_profile(&self) {
        let (name, from_cmd) = {
            let core = self.core.borrow();
            if !core.cmd.profile_name.is_empty() {
                (core.cmd.profile_name.clone(), true)
            } else if core.config.load_profile_startup && !core.config.load_profile_name.is_empty() {
                (core.config.load_profile_name.clone(), false)
            } else {
                return;
            }
        };
        let idx = pwtool_core::config::find_profile(&self.core.borrow().profiles, &name);
        match idx {
            Some(i) => self.load_profile_inner(i, true),
            None => {
                let mut core = self.core.borrow_mut();
                core.startup_errors.push(trl_format("Profile \"%1\" not found.", &[&name]));
                if from_cmd {
                    core.cmd.profile_name.clear();
                } else {
                    core.config.load_profile_startup = false;
                    core.config.load_profile_name.clear();
                }
            }
        }
    }

    /// Open PassCube on startup if configured or requested on command line.
    pub fn startup_open_manager(self: &Rc<Self>) {
        let open_mng = {
            let core = self.core.borrow();
            core.config.database.open_window_on_startup || core.cmd.passw_db_file_name.is_some()
        };
        if open_mng {
            self.show_passw_manager();
        }
    }

    /// Show start-up errors and start the automatic update check
    /// (`FormActivate`, runs once when the main window is first shown).
    pub fn show_startup_errors(self: &Rc<Self>) {
        if self.startup_shown.replace(true) {
            return;
        }
        self.auto_check_updates();
        let errs: Vec<String> = self.core.borrow_mut().startup_errors.drain(..).collect();
        if !errs.is_empty() {
            let mut m = trl_format("%1 encountered %2 error(s) during startup:", &[PROGRAM_NAME, &errs.len().to_string()]);
            for e in errs {
                m.push_str("\n- ");
                m.push_str(&e);
            }
            msg(Some(&self.win()), &m, MsgIcon::Warning);
        }
    }
}

fn build_char_set_help() -> String {
    format_quick_help(&pwtool_core::unicode::format_w(
        "%1\n<AZ> = A..Z\t<base64>,<b64> = <AZ><az><09>, +, /\n<az> = a..z\t<easytoread>,<etr> = <AZ><az><09> %2\n<09> = 0..9\t<symbols>,<sym> = %3 (!\"#$%%...)\n<Hex> = 0..9, A..F\t<brackets>,<brac> = %4 (()[]{}<>)\n<hex> = 0..9, a..f\t<punctuation>,<punct> = %5 (,.;:)\n<highansi>,<high> = %6\n<phonetic> = %7\n<phoneticu> = %8\n<phoneticx> = %9\n\n%10\n<placeholder>:N[+]\n%11\n\n%12",
        &[
            &trl("You can use the following placeholders:"),
            &trl("without ambiguous characters"),
            &trl("Special symbols"),
            &trl("Brackets"),
            &trl("Punctuation marks"),
            &trl("Higher ANSI characters"),
            &trl("Generate phonetic password (lower-case letters)"),
            &trl("Generate phonetic password (upper-case letters)"),
            &trl("Generate phonetic password (mixed-case letters)"),
            &trl("You can use the following syntax to specify a (minimum) number N of characters\nfor a character set:"),
            &trl("N = Include exactly N characters. N+ = Include at least N characters."),
            &trl("Comments may be provided in square brackets \"[...]\"\nat the beginning of the sequence."),
        ],
    ))
}

fn build_format_help() -> String {
    let mut s = format_quick_help(&pwtool_core::unicode::format_w(
        "%1\n\n%2\n\nx = %3\tv = %4 (aeiou)\na = a..z, 0..9\tV = %5\nA = A..Z, a..z, 0..9\tZ = %6\nU = A..Z, 0..9\tc = %7 (bcdf...)\nd = 0..9\tC = %8\nh = 0..9, a..f\tz = %9\nH = 0..9, A..F\tp = %10 (,.;:)\nl = a..z\tb = %11 (()[]{}<>)\nL = A..Z, a..z\ts = %12 (!\"#$%%...)\nu = A..Z\tS = A + %13\ny = %14\tE = A %15\nq = %16\nQ = %17\nr = %18\n\n",
        &[
            &trl("Format specifiers have the form \"[*][N]x\" or \"[*][M-N]x\", where the\noptional argument \"N\" specifies the number of repetitions (between 1 and\n99999), \"M-N\" specifies a random number of repetitions in the range\nfrom M to N, and \"x\" is the actual format specifier/placeholder. The\noptional asterisk (*) means that each character must occur only once in\nthe sequence. Example: \"16u\" means \"Insert 16 random upper-case letters\ninto the password\"."),
            &trl("List of placeholders with optional argument \"N\":"),
            &trl("Custom character set"),
            &trl("Vowels, lower-case"),
            &trl("Vowels, mixed-case"),
            &trl("Vowels, upper-case"),
            &trl("Consonants, lower-case"),
            &trl("Consonants, mixed-case"),
            &trl("Consonants, upper-case"),
            &trl("Punctuation marks"),
            &trl("Brackets"),
            &trl("Special symbols"),
            &trl("Special symbols"),
            &trl("Higher ANSI characters"),
            &trl("without ambiguous characters"),
            &trl("Generate phonetic password (lower-case letters)"),
            &trl("Generate phonetic password (upper-case letters)"),
            &trl("Generate phonetic password (mixed-case letters)"),
        ],
    ));
    s.push_str(&pwtool_core::unicode::format_w(
        "%1\n\nP = %2\n{N}w = %3\n{N}W = %4\n{N}[...] = %5\n{N}{...} = %6\n{N}<<...>> = %7\n\n%8\n* %9\n* %10\n",
        &[
            &trl("List of special format specifiers\n(possible usage of argument \"N\" is indicated by \"{N}\"):"),
            &trl("Insert password generated via \"Include characters/words\""),
            &trl("Words from word list, multiple words are separated by spaces"),
            &trl("Words, multiple words are not separated at all"),
            &trl("Repeat input sequence in brackets N times"),
            &trl("Randomly permute formatted sequence in brackets, keep N characters"),
            &trl("Treat sequence as character set and insert N characters"),
            &trl("Additional notes:"),
            &trl("Text enclosed with quotation marks \"...\" is copied verbatim\nto the password."),
            &trl("Comments may be provided in square brackets \"[...]\"\nat the beginning of the sequence."),
        ],
    ));
    s
}

#[allow(dead_code)]
fn _unused(_: gdk::Texture) {}
