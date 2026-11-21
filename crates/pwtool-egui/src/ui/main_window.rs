//! Main window (`TMainForm`): the password generator. Port of the GTK
//! version's `main_window.rs` to the `vcl` forms; tray, hot keys, autotype,
//! tools and the password manager follow in later stages (their menu items
//! do nothing yet).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use egui::{Ui, ViewportCommand, ViewportId};
use pwtool_app::app::AppCore;
use pwtool_app::program::*;
use pwtool_core::config::{LISTS_MAX_ENTRIES, WLFNLIST_DEFAULT, remove_comment_and_trim};
use pwtool_core::generate::{
    GenDest, GenRequest, PASSW_MAX_CHARS, PASSW_MAX_NUM, PASSW_MAX_WORDS, PASSWOPTION_EACHCHARONLYONCE,
    PASSWOPTION_EACHWORDONLYONCE, PASSWOPTIONS_NUM, PASSWOPTIONS_STARRED, PasswOptions, SinkAction, generate,
    parse_num_passwords, parse_passphrase_length, passw_list_header, passwoptions_starred_bitfield,
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

use super::clipboard;
use super::dialogs::{self, PasswListWindow, ProgressWindow, format_quick_help, info_box, msg, quick_help};
use super::quality_bar::QualityBar;
use crate::runtime::spawn;
use crate::vcl::Form;
use crate::vcl::dialogs::{MsgButtons, MsgIcon, MsgResult};
use crate::windows::{self, Content};

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
const ROOT: ViewportId = ViewportId::ROOT;
const PROFILES_MENU_SHORTCUTS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Random generator of one generation run: a pool derived from the main
/// pool, or the deterministic generator.
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
    result: Result<pwtool_core::generate::GenSummary, String>,
    single: Zeroizing<String>,
    list: Zeroizing<String>,
    mem_limit_reached: bool,
}

pub struct MainWindow {
    pub form: Rc<Form>,
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
    pub generating: Cell<bool>,
    /// The window is disabled during a long generation.
    busy: Cell<bool>,
    passw_list: RefCell<Option<Rc<PasswListWindow>>>,
    char_set_help: String,
    format_passw_help: String,
    title: RefCell<String>,
    title_shown: RefCell<String>,
    mouse_move_cnt: Cell<i32>,
    mouse_wheel_cnt: Cell<i32>,
    close_confirmed: Cell<bool>,
    startup_shown: Cell<bool>,
    profile_list_guard: Cell<bool>,
    last_pointer: Cell<Option<egui::Pos2>>,
    /// Edit receiving the next single password instead of `PasswBox`.
    pub gen_target: RefCell<Option<(Rc<Form>, String)>>,
}

impl MainWindow {
    pub fn new(core: AppCore) -> Rc<Self> {
        let form = dialogs::load("Main");
        let w = Rc::new(MainWindow {
            form,
            core: RefCell::new(core),
            passw_bar: QualityBar::new(128),
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
            busy: Cell::new(false),
            passw_list: RefCell::new(None),
            char_set_help: build_char_set_help(),
            format_passw_help: build_format_help(),
            title: RefCell::new(PROGRAM_NAME.to_string()),
            title_shown: RefCell::new(String::new()),
            mouse_move_cnt: Cell::new(60),
            mouse_wheel_cnt: Cell::new(5),
            close_confirmed: Cell::new(false),
            startup_shown: Cell::new(false),
            profile_list_guard: Cell::new(false),
            last_pointer: Cell::new(None),
            gen_target: RefCell::new(None),
        });
        w.setup();
        w
    }

    fn f(&self) -> &Form {
        &self.form
    }

    fn weak(self: &Rc<Self>) -> Weak<Self> {
        Rc::downgrade(self)
    }

    fn menu(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        if !self.form.has_menu_item(name) {
            eprintln!("pwtool: menu item {name} not found");
            return;
        }
        let w = self.weak();
        self.form.on_menu(name, move |_| {
            if let Some(w) = w.upgrade() {
                f(&w);
            }
        });
    }

    /// Menu item with a check mark that toggles when chosen.
    fn check_menu(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        let w = self.weak();
        let n = name.to_string();
        self.form.on_menu(name, move |_| {
            if let Some(w) = w.upgrade() {
                let on = !w.form.menu_checked(&n);
                w.form.set_menu_checked(&n, on);
                f(&w);
            }
        });
    }

    fn click(self: &Rc<Self>, name: &str, f: impl Fn(&Rc<Self>) + 'static) {
        let w = self.weak();
        self.form.on_click(name, move |_| {
            if let Some(w) = w.upgrade() {
                f(&w);
            }
        });
    }

    fn on_event(self: &Rc<Self>, f: impl Fn(&Rc<Self>, &crate::vcl::Event) + 'static) {
        let w = self.weak();
        self.form.on_any(move |e| {
            if let Some(w) = w.upgrade() {
                f(&w, e);
            }
        });
    }

    pub fn set_title(&self, t: &str) {
        *self.title.borrow_mut() = t.to_string();
    }

    // ----------------------------------------------------------------------
    // set-up

    fn setup(self: &Rc<Self>) {
        let f = self.f();
        for n in [
            "MainMenu_Options_HideEntProgress",
            "MainMenu_Options_AlwaysOnTop",
            "MainMenu_Options_SaveSettingsOnExit",
            "PasswBoxMenu_Editable",
            "PasswBoxMenu_EnablePasswTest",
            "GenerateMenu_IncludeHeader",
        ] {
            f.set_menu_checked(n, false);
        }
        EntropyManager::global().set_system_entropy_bits(ENTROPY_SYSTEM);
        f.set_range("CharsLengthSpinBtn", 1, PASSW_MAX_CHARS as i64);
        f.set_range("WordsNumSpinBtn", 1, PASSW_MAX_WORDS as i64);
        f.set_text("LogoLbl", "PW GEN");
        f.set_label_align("LogoLbl", 0.5, true);
        if f.has("LogoImg") {
            f.set_image("LogoImg", Some(include_bytes!("../../../../assets/about_logo.png").to_vec()));
        }
        f.set_text("PasswInfoLbl", &trl("Click on \"Generate\""));
        f.set_mono("PasswBox", true);
        f.set_secret("PasswBox");
        f.set_popup_menu("PasswBox", "PasswBoxMenu");
        f.set_popup_menu("GenerateBtn2", "GenerateMenu");
        f.set_popup_menu("AdvancedBtn", "AdvancedOptionsMenu");
        f.set_text("StatusBar", &format!("{PROGRAM_NAME} {}", version_string()));
        let bar = self.passw_bar.clone();
        f.set_painter("PasswGauge", move |ui, rect| bar.paint(ui, rect));

        self.load_config_to_controls();
        self.startup_profile();
        self.connect_events();
        self.setup_timer();
        self.update_entropy_progress(true);
        self.set_advanced_btn_caption();
        let w = self.weak();
        windows::set_input_hook(move |ctx| {
            if let Some(w) = w.upgrade() {
                w.collect_entropy(ctx);
            }
        });
        let w = self.weak();
        clipboard::on_set_data(move || {
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
        let f = self.f();
        let core = self.core.borrow();
        let s = &core.settings;
        f.set_checked("IncludeCharsCheck", s.include_chars);
        f.set_position("CharsLengthSpinBtn", s.chars_length as i64);
        f.set_items("CharSetList", s.char_set_list.clone());
        f.set_text("CharSetList", &s.char_set_text);
        *self.char_set_input.borrow_mut() = s.char_set_text.clone();
        f.set_checked("IncludeWordsCheck", s.include_words);
        f.set_position("WordsNumSpinBtn", s.words_num as i64);
        f.set_items("WLFNList", s.wlfn_list.clone());
        f.set_text("WLFNList", &s.wlfn_text);
        if core.word_list_error.is_none() {
            *self.wl_file_name.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
            self.wl_file_name_err.borrow_mut().clear();
        } else {
            *self.wl_file_name_err.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
        }
        f.set_checked("CombineWordsCharsCheck", s.combine_words_chars);
        f.set_checked("SpecifyLengthCheck", s.passphr_spec_len);
        f.set_text("SpecifyLengthBox", &s.passphr_spec_len_string);
        f.set_checked("FormatPasswCheck", s.format_passw);
        f.set_items("FormatList", s.format_list.clone());
        f.set_text("FormatList", &s.format_text);
        f.set_checked("RunScriptCheck", s.run_script);
        f.set_items("ScriptList", s.script_list.clone());
        f.set_text("ScriptList", &s.script_text);
        f.set_text("NumPasswBox", &s.num_passw);
        f.set_menu_checked("GenerateMenu_IncludeHeader", s.include_header);
        let ini = &core.ini;
        let hide = ini.read_bool("Main", "HidePassw", false);
        f.set_checked("TogglePasswBtn", hide);
        f.set_password_char("PasswBox", hide);
        self.show_ent_progress.set(!ini.read_bool("Main", "HideEntropyProgress", false));
        f.set_menu_checked("MainMenu_Options_HideEntProgress", !self.show_ent_progress.get());
        f.set_menu_checked("MainMenu_Options_SaveSettingsOnExit", ini.read_bool("Main", "SaveSettingsOnExit", true));
        f.set_menu_checked("MainMenu_Options_AlwaysOnTop", ini.read_bool("Main", "AlwaysOnTop", false));
        let editable = ini.read_bool("Main", "PasswEditable", true);
        f.set_menu_checked("PasswBoxMenu_Editable", editable);
        f.set_menu_checked("PasswBoxMenu_EnablePasswTest", ini.read_bool("Main", "EnablePasswTest", true));
        f.set_read_only("PasswBox", !editable);
        let tag = if f.menu_checked("PasswBoxMenu_EnablePasswTest") { PASSWBOX_TAG_PASSW_TEST } else { 0 };
        self.passw_box_tag.set(tag);
        drop(core);
        self.update_char_set_info();
        self.update_word_list_info();
        self.update_profile_controls();
        self.include_chars_check_click();
    }

    fn update_char_set_info(&self) {
        let core = self.core.borrow();
        self.f().set_text("CharSetInfoLbl", &core.char_set_info);
        self.f().set_font_color("CharSetInfoLbl", core.char_set_error.then_some(egui::Color32::from_rgb(0xe0, 0x20, 0x20)));
    }

    fn update_word_list_info(&self) {
        let core = self.core.borrow();
        let red = egui::Color32::from_rgb(0xe0, 0x20, 0x20);
        match &core.word_list_error {
            Some(e) => {
                self.f().set_text("WordListInfoLbl", e);
                self.f().set_font_color("WordListInfoLbl", Some(red));
            }
            None => {
                self.f().set_text("WordListInfoLbl", &core.word_list_info);
                self.f().set_font_color("WordListInfoLbl", None);
            }
        }
    }

    /// `UpdateProfileControls()`.
    fn update_profile_controls(&self) {
        let names: Vec<String> = self.core.borrow().profiles.iter().map(|p| p.profile_name.clone()).collect();
        self.profile_list_guard.set(true);
        self.f().set_items("ProfileList", names.clone());
        self.profile_list_guard.set(false);
        // File | Profile menu: profiles with Alt+Shift+0..Z
        let shortcuts: Vec<char> = PROFILES_MENU_SHORTCUTS.chars().collect();
        let items: Vec<(String, String, Option<u16>)> = names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let (caption, sc) = match shortcuts.get(i) {
                    Some(&c) => (format!("&{c} {}", n.replace('&', "&&")), Some(0x8000 | 0x2000 | c.to_ascii_uppercase() as u16)),
                    None => (n.replace('&', "&&"), None),
                };
                (format!("profile{i}"), caption, sc)
            })
            .collect();
        if self.f().has_menu_item("MainMenu_File_Profile") {
            self.f().set_dynamic_items("MainMenu_File_Profile", &items);
        }
    }

    /// `IncludeCharsCheckClick()`: enable dependent controls.
    fn include_chars_check_click(&self) {
        let f = self.f();
        let c1 = f.checked("IncludeCharsCheck");
        let c2 = f.checked("IncludeWordsCheck");
        let c3 = f.checked("FormatPasswCheck");
        let c4 = f.checked("RunScriptCheck");
        for n in ["CharsLengthLbl", "CharsLengthBox", "CharsLengthSpinBtn"] {
            f.set_enabled(n, c1);
        }
        f.set_enabled("CombineWordsCharsCheck", c1 && c2);
        for n in ["WordsNumLbl", "WordsNumBox", "WordsNumSpinBtn", "SpecifyLengthCheck"] {
            f.set_enabled(n, c2);
        }
        f.set_enabled("SpecifyLengthBox", c2 && f.checked("SpecifyLengthCheck"));
        f.set_enabled("FormatList", c3);
        f.set_enabled("FormatPasswInfoLbl", c3);
        for n in ["ScriptList", "ReloadScriptBtn", "BrowseBtn2"] {
            f.set_enabled(n, c4);
        }
        let en = c1 || c2 || c3 || c4;
        for n in ["NumPasswBox", "GenerateBtn", "GenerateBtn2"] {
            f.set_enabled(n, en);
        }
    }

    // ----------------------------------------------------------------------
    // events

    fn connect_events(self: &Rc<Self>) {
        for n in ["IncludeCharsCheck", "IncludeWordsCheck", "FormatPasswCheck", "RunScriptCheck", "SpecifyLengthCheck"] {
            let is_spec = n == "SpecifyLengthCheck";
            let name = n.to_string();
            self.click(n, move |w| {
                w.include_chars_check_click();
                if is_spec && w.f().checked(&name) {
                    w.f().focus("SpecifyLengthBox");
                }
            });
        }
        self.on_event(|w, e| {
            use crate::vcl::Event::*;
            match e {
                Exit(n) => match n.as_str() {
                    "CharSetList" => w.char_set_list_exit(),
                    "WLFNList" => w.wlfn_list_exit(),
                    "FormatList" => w.format_list_exit(),
                    "CharsLengthBox" => w.f().set_position("CharsLengthSpinBtn", w.f().position("CharsLengthSpinBtn")),
                    "WordsNumBox" => w.f().set_position("WordsNumSpinBtn", w.f().position("WordsNumSpinBtn")),
                    _ => {}
                },
                Enter(n) => match n.as_str() {
                    "CharSetList" => w.char_set_info_btn_click(),
                    "WLFNList" => w.word_list_info_btn_click(),
                    "NumPasswBox" => w.generate_passw(GenDest::GuiList),
                    "PasswBox" => w.generate_passw(GenDest::GuiSingle),
                    _ => {}
                },
                Change(n) => match n.as_str() {
                    // an entry chosen from the list
                    "CharSetList" if w.f().item_index(n).is_some() => w.char_set_list_exit(),
                    "WLFNList" if w.f().item_index(n).is_some() => w.wlfn_list_exit(),
                    "PasswBox" => w.passw_box_change(),
                    "ProfileList" => {
                        if let (false, Some(i)) = (w.profile_list_guard.get(), w.f().item_index(n)) {
                            w.load_profile(i);
                        }
                    }
                    _ => {}
                },
                Menu(n) => {
                    if let Some(i) = n.strip_prefix("dyn:profile").and_then(|i| i.parse::<usize>().ok()) {
                        w.profile_menu_click(i);
                    }
                }
                _ => {}
            }
        });
        self.click("TogglePasswBtn", |w| w.f().set_password_char("PasswBox", w.f().checked("TogglePasswBtn")));
        self.click("GenerateBtn", |w| w.generate_passw(GenDest::GuiSingle));
        self.click("GenerateBtn2", |w| w.generate_passw(GenDest::GuiList));
        self.menu("GenerateMenu_Clipboard", |w| w.generate_passw(GenDest::ClipboardList));
        self.menu("GenerateMenu_File", |w| w.generate_passw(GenDest::FileList));
        self.check_menu("GenerateMenu_IncludeHeader", |_| {});
        self.click("AdvancedBtn", |w| w.advanced_btn_click());
        self.menu("AdvancedOptionsMenu_DeactivateAll", |w| w.deactivate_options(u32::MAX));
        self.menu("AdvancedOptionsMenu_DeactivateAllStarred", |w| {
            w.f().set_checked("SpecifyLengthCheck", false);
            w.include_chars_check_click();
            w.deactivate_options(passwoptions_starred_bitfield());
            w.set_advanced_btn_caption();
        });
        self.click("CharSetInfoBtn", |w| w.char_set_info_btn_click());
        self.click("WordListInfoBtn", |w| w.word_list_info_btn_click());
        self.click("CharSetHelpBtn", |w| quick_help(ROOT, &w.char_set_help));
        self.click("FormatPasswHelpBtn", |w| quick_help(ROOT, &w.format_passw_help));
        self.click("BrowseBtn", |w| w.browse_file("WLFNList", &trl("Select word list file"), true));
        self.click("BrowseBtn2", |w| w.browse_file("ScriptList", &trl("Select Lua script file"), false));
        self.click("ReloadScriptBtn", |w| {
            *w.script.borrow_mut() = None;
        });
        self.click("HelpBtn", |w| w.help_btn_click());
        self.click("ClearClipBtn", |w| w.clear_clip_btn_click());
        self.click("ReloadProfileBtn", |w| {
            if let Some(i) = w.f().item_index("ProfileList") {
                w.load_profile(i);
            }
        });

        self.menu("MainMenu_File_Exit", |_| windows::ctx().send_viewport_cmd(ViewportCommand::Close));
        self.menu("MainMenu_Tools_ClearClipboard", |w| w.clear_clip_btn_click());
        self.menu("MainMenu_Help_OpenManual", |w| w.help_btn_click());
        self.menu("MainMenu_Help_CmdLineArgs", |_| msg(ROOT, &AppCore::cmd_line_info(), MsgIcon::Info));
        self.menu("MainMenu_Help_Donate", |_| open_uri(PROGRAM_URL_DONATE));
        for n in ["MainMenu_Help_VisitWebsite", "MainMenu_Help_GetTranslations"] {
            self.menu(n, |_| open_uri(if PORT_REPOSITORY.is_empty() { PROGRAM_URL_ORIGINAL } else { PORT_REPOSITORY }));
        }
        self.menu("MainMenu_Help_TimerInfo", |_| timer_info());
        self.menu("MainMenu_Options_SaveSettingsNow", |w| {
            if w.save_config() {
                info_box(ROOT, &trl("Settings saved."));
            }
        });
        self.check_menu("MainMenu_Options_HideEntProgress", |w| {
            w.show_ent_progress.set(!w.f().menu_checked("MainMenu_Options_HideEntProgress"));
            w.update_entropy_progress(true);
        });
        self.check_menu("MainMenu_Options_SaveSettingsOnExit", |_| {});
        self.check_menu("MainMenu_Options_AlwaysOnTop", |w| {
            let on = w.f().menu_checked("MainMenu_Options_AlwaysOnTop");
            windows::ctx().send_viewport_cmd(ViewportCommand::WindowLevel(if on {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
        });
        self.check_menu("PasswBoxMenu_Editable", |w| {
            let editable = w.f().menu_checked("PasswBoxMenu_Editable");
            w.f().set_read_only("PasswBox", !editable);
        });
        self.check_menu("PasswBoxMenu_EnablePasswTest", |w| {
            let on = w.f().menu_checked("PasswBoxMenu_EnablePasswTest");
            let t = w.passw_box_tag.get();
            w.passw_box_tag.set(if on { t | PASSWBOX_TAG_PASSW_TEST } else { t & !PASSWBOX_TAG_PASSW_TEST });
        });
        self.menu("PasswBoxMenu_Copy", |w| w.passw_box_copy());
        self.menu("PasswBoxMenu_SelectAll", |w| w.f().select_all("PasswBox"));
        self.menu("PasswBoxMenu_Cut", |w| {
            if w.passw_box_editable() && w.f().selection("PasswBox").is_some() {
                w.passw_box_copy();
                w.f().delete_selection("PasswBox");
            }
        });
        self.menu("PasswBoxMenu_Delete", |w| {
            if w.passw_box_editable() {
                w.f().delete_selection("PasswBox");
            }
        });
        self.menu("PasswBoxMenu_Paste", |w| {
            if w.passw_box_editable() {
                if let Some(t) = clipboard::read_text() {
                    w.f().insert_text("PasswBox", &t);
                }
            }
        });
        self.menu("PasswBoxMenu_Undo", |w| {
            if w.passw_box_editable() {
                w.f().undo("PasswBox");
            }
        });
        self.menu("PasswBoxMenu_SaveAsFile", |w| w.passw_box_save_as_file());
        self.menu("MainMenu_Tools_ProvideAddEntropy_FromFile", |w| w.add_entropy_from_file());
        self.menu("MainMenu_Tools_DetermRandGen_Reset", |w| {
            if let Some(g) = w.key_seeded_prng.borrow_mut().as_mut() {
                g.reset();
            }
            info_box(ROOT, &trl("Deterministic random generator has been RESET."));
        });
        self.menu("MainMenu_Tools_DetermRandGen_Deactivate", |w| w.deactivate_det_rand_gen());
    }

    fn passw_box_editable(&self) -> bool {
        self.f().menu_checked("PasswBoxMenu_Editable")
    }

    /// `AddEntryToList()`.
    fn add_entry_to_list(&self, combo: &str, entry: &str, case_sensitive: bool) {
        let mut items = self.f().items(combo);
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
        let text = self.f().text(combo);
        self.f().set_items(combo, items);
        self.f().set_text(combo, &text);
    }

    /// `CharSetListExit()` → `LoadCharSet()`.
    fn char_set_list_exit(&self) {
        let text = self.f().text("CharSetList");
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
        let text = self.f().text("WLFNList");
        let input = if text.is_empty() { WLFNLIST_DEFAULT.to_string() } else { text };
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
        let text = self.f().text("FormatList");
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
                s.push_str(&trl_format("(phonetic, based on trigram frequencies\nfrom the following source:\n<%1>)", &[&src]));
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
        msg(ROOT, &m, MsgIcon::Info);
    }

    /// `WordListInfoBtnClick()`.
    fn word_list_info_btn_click(&self) {
        self.wlfn_list_exit();
        let core = self.core.borrow();
        let mut m = String::new();
        if !self.wl_file_name_err.borrow().is_empty() {
            m = trl_format(
                "Error while loading the file\n\"%1\":\n%2",
                &[&self.wl_file_name_err.borrow(), &self.f().text("WordListInfoLbl")],
            ) + "\n\n";
        }
        let name = self.wl_file_name.borrow().clone();
        let wl = if name.is_empty() || name == WLFNLIST_DEFAULT { format!("<{}>", trl("default (English) word list")) } else { format!("\"{name}\"") };
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
        msg(ROOT, &m, MsgIcon::Info);
    }

    fn browse_file(self: &Rc<Self>, combo: &str, title: &str, word_list: bool) {
        let (w, combo, title) = (self.weak(), combo.to_string(), title.to_string());
        spawn(async move {
            let Some(p) = crate::vcl::dialogs::open_file(&title, None, &[]).await else { return };
            let Some(w) = w.upgrade() else { return };
            w.f().set_text(&combo, &p.to_string_lossy());
            if word_list {
                w.wlfn_list_exit();
            }
            w.f().focus(&combo);
        });
    }

    fn help_btn_click(&self) {
        let p = self.core.borrow().help_file_name.clone();
        let alt = self.core.borrow().paths.resources.join("manual").join("manual.pdf");
        let file = if p.exists() { p } else if alt.exists() { alt } else { p };
        open_uri(&file.to_string_lossy());
    }

    /// `ClearClipBtnClick()`.
    fn clear_clip_btn_click(&self) {
        let ok = clipboard::clear_data(true);
        let m = trl(if ok { "Clipboard cleared." } else { "Could not open clipboard." });
        if ok {
            info_box(ROOT, &m);
        } else {
            msg(ROOT, &m, MsgIcon::Error);
        }
    }

    /// `PasswBoxMenuPopup()`: enabled state of the PasswBoxMenu items
    /// (updated every frame).
    fn update_passw_box_menu(&self) {
        let f = self.f();
        let editable = self.passw_box_editable();
        let selected = f.selection("PasswBox").is_some();
        let has_text = !f.secret_text("PasswBox").is_empty();
        let set = |n: &str, on: bool| {
            if f.has_menu_item(n) {
                f.set_menu_enabled(n, on);
            }
        };
        set("PasswBoxMenu_Undo", editable);
        set("PasswBoxMenu_Cut", editable && selected);
        set("PasswBoxMenu_Copy", selected);
        set("PasswBoxMenu_EncryptCopy", false);
        set("PasswBoxMenu_Paste", editable);
        set("PasswBoxMenu_PerformAutotype", false);
        set("PasswBoxMenu_Delete", editable && selected);
        set("PasswBoxMenu_AddToDb", false);
        set("PasswBoxMenu_SelectAll", has_text);
        set("PasswBoxMenu_SaveAsFile", has_text);
        set("PasswBoxMenu_EnablePasswTest", editable);
    }

    fn passw_box_copy(&self) {
        let text = self.f().selected_text("PasswBox");
        if !text.is_empty() {
            clipboard::set_data(&text);
        }
    }

    fn passw_box_save_as_file(self: &Rc<Self>) {
        let w = self.weak();
        spawn(async move {
            let Some(p) = crate::vcl::dialogs::save_file(&trl("Save password"), None, None, &[]).await else { return };
            let Some(w) = w.upgrade() else { return };
            let text = w.f().secret_text("PasswBox");
            let enc = w.core.borrow().file_encoding;
            let res = pwtool_core::text_file::TextFileWriter::create(&p, enc, true).and_then(|mut f| f.write_str(&text));
            match res {
                Ok(()) => msg(ROOT, &trl_format("File \"%1\" successfully created.", &[&p.file_name().unwrap_or_default().to_string_lossy()]), MsgIcon::Info),
                Err(e) => msg(ROOT, &trl_format("Error while creating file\n\"%1\":\n%2.", &[&p.to_string_lossy(), &e.to_string()]), MsgIcon::Error),
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
        let text = self.f().secret_text("PasswBox");
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
                bits = if core.use_advanced_passw_est { pwtool_core::zxcvbn::zxcvbn_match(&text) } else { PasswordGenerator::estimate_passw_security(&text) };
            }
        }
        drop(core);
        self.show_passw_info(len, bits, common, true);
    }

    /// `ShowPasswInfo()`.
    fn show_passw_info(&self, len: usize, bits: f64, common: bool, estimated: bool) {
        self.passw_bar.set_quality(bits);
        let mut caption = String::new();
        if common {
            caption.push_str("**");
            self.f().set_hint("PasswInfoLbl", &trl("Matches common password!"));
        } else {
            if estimated {
                caption.push('*');
            }
            self.f().set_hint("PasswInfoLbl", "");
        }
        caption.push_str(&trl_format("%1 bits / %2 characters", &[&format_float_1(bits), &len.to_string()]));
        self.f().set_text("PasswInfoLbl", &caption);
    }

    /// `SetAdvancedBtnCaption()`.
    fn set_advanced_btn_caption(&self) {
        let flags = self.core.borrow().settings.passw_options.flags;
        let num = flags.count_ones();
        let mut hint = String::new();
        if num > 0 {
            for i in 0..PASSWOPTIONS_NUM {
                if flags & (1 << i) != 0 {
                    let mut opt = String::new();
                    if PASSWOPTIONS_STARRED[i] {
                        opt.push('*');
                    }
                    opt.push_str(&dialogs::option_name(i));
                    if num > 5 && opt.chars().count() > 40 {
                        opt = opt.chars().take(37).collect::<String>() + "...";
                    }
                    hint.push('\n');
                    hint.push_str(&opt);
                }
            }
        }
        let mut caption = trl("Advanced");
        let hint = format!("{} - {}{hint}", trl("Advanced password options"), trl_format("%1 option(s) active", &[&num.to_string()]));
        if num > 0 {
            caption.push_str(&format!("|{num}"));
            if flags & passwoptions_starred_bitfield() != 0 {
                caption.push('*');
            }
        }
        self.f().set_control_caption("AdvancedBtn", &caption);
        self.f().set_hint("AdvancedBtn", &hint);
    }

    /// `AdvancedBtnClick()`.
    fn advanced_btn_click(self: &Rc<Self>) {
        let w = self.clone();
        spawn(async move {
            let old = w.core.borrow().settings.passw_options.clone();
            if let Some(new) = dialogs::passw_options(ROOT, &old).await {
                w.apply_passw_options(old, new);
            }
        });
    }

    fn apply_passw_options(&self, old: PasswOptions, new: PasswOptions) {
        self.core.borrow_mut().settings.passw_options = new.clone();
        if new.min_word_len != old.min_word_len
            || new.max_word_len != old.max_word_len
            || (new.flags ^ old.flags) & pwtool_core::generate::PASSWOPTION_LOWERCASEWORDS != 0
        {
            let wl = self.f().text("WLFNList");
            self.load_word_list_file(&wl);
        }
        if new.trigram_file_name != old.trigram_file_name {
            let ok = self.core.borrow_mut().load_trigram_file(&new.trigram_file_name) > 0;
            let errs: Vec<String> = self.core.borrow_mut().startup_errors.drain(..).collect();
            for e in errs {
                msg(ROOT, &e, MsgIcon::Error);
            }
            if !ok {
                self.core.borrow_mut().settings.passw_options.trigram_file_name = old.trigram_file_name.clone();
            }
        }
        let cs = self.f().text("CharSetList");
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

    /// `MainMenu_File_ProfileClick()`.
    fn profile_menu_click(&self, i: usize) {
        let Some(name) = self.core.borrow().profiles.get(i).map(|p| p.profile_name.clone()) else { return };
        self.load_profile(i);
        info_box(ROOT, &trl_format("Profile \"%1\" activated.", &[&name]));
    }

    /// `LoadProfile(index)`.
    pub fn load_profile(&self, i: usize) {
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
                msg(ROOT, &e, MsgIcon::Error);
            }
        }
        let (s, wl_err) = {
            let core = self.core.borrow();
            (core.settings.clone(), core.word_list_error.is_some())
        };
        let f = self.f();
        f.set_checked("IncludeCharsCheck", s.include_chars);
        f.set_position("CharsLengthSpinBtn", s.chars_length as i64);
        f.set_text("CharSetList", &s.char_set_text);
        *self.char_set_input.borrow_mut() = s.char_set_text.clone();
        f.set_checked("IncludeWordsCheck", s.include_words);
        f.set_position("WordsNumSpinBtn", s.words_num as i64);
        f.set_text("WLFNList", &s.wlfn_text);
        if !wl_err {
            *self.wl_file_name.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
            self.wl_file_name_err.borrow_mut().clear();
        } else {
            *self.wl_file_name_err.borrow_mut() = remove_comment_and_trim(&s.wlfn_text);
        }
        f.set_checked("CombineWordsCharsCheck", s.combine_words_chars);
        f.set_checked("SpecifyLengthCheck", s.passphr_spec_len);
        f.set_text("SpecifyLengthBox", &s.passphr_spec_len_string);
        f.set_checked("FormatPasswCheck", s.format_passw);
        f.set_text("FormatList", &s.format_text);
        f.set_text("FormatPasswInfoLbl", "");
        f.set_checked("RunScriptCheck", s.run_script);
        f.set_text("ScriptList", &s.script_text);
        f.set_text("NumPasswBox", &s.num_passw);
        f.set_menu_checked("GenerateMenu_IncludeHeader", s.include_header);
        self.update_char_set_info();
        self.update_word_list_info();
        self.set_advanced_btn_caption();
        self.include_chars_check_click();
        self.profile_list_guard.set(true);
        f.set_item_index("ProfileList", Some(i));
        self.profile_list_guard.set(false);
    }

    /// Copy the controls into the generator settings.
    fn sync_settings(&self) {
        let f = self.f();
        let mut core = self.core.borrow_mut();
        let s = &mut core.settings;
        s.include_chars = f.checked("IncludeCharsCheck");
        s.chars_length = f.position("CharsLengthSpinBtn") as i32;
        s.char_set_text = f.text("CharSetList");
        s.char_set_list = f.items("CharSetList");
        if s.char_set_list.is_empty() {
            s.char_set_list.push(pwtool_core::config::CHARSETLIST_DEFAULTENTRIES[0].into());
        }
        s.include_words = f.checked("IncludeWordsCheck");
        s.words_num = f.position("WordsNumSpinBtn") as i32;
        s.wlfn_text = f.text("WLFNList");
        s.wlfn_list = f.items("WLFNList");
        if s.wlfn_list.is_empty() {
            s.wlfn_list.push(WLFNLIST_DEFAULT.into());
        }
        s.combine_words_chars = f.checked("CombineWordsCharsCheck");
        s.passphr_spec_len = f.checked("SpecifyLengthCheck");
        s.passphr_spec_len_string = f.text("SpecifyLengthBox");
        s.format_passw = f.checked("FormatPasswCheck");
        s.format_text = f.text("FormatList");
        s.format_list = f.items("FormatList");
        s.run_script = f.checked("RunScriptCheck");
        s.script_text = f.text("ScriptList");
        s.script_list = f.items("ScriptList");
        s.num_passw = f.text("NumPasswBox");
        s.include_header = f.menu_checked("GenerateMenu_IncludeHeader");
    }

    // ----------------------------------------------------------------------
    // generation

    /// `GeneratePassw()`.
    pub fn generate_passw(self: &Rc<Self>, dest: GenDest) {
        if self.generating.get() {
            self.gen_target.borrow_mut().take();
            return;
        }
        self.generating.set(true);
        self.char_set_list_exit();
        self.wlfn_list_exit();
        self.sync_settings();
        let w = self.clone();
        spawn(async move { w.generate_passw_async(dest).await });
    }

    async fn generate_passw_async(self: Rc<Self>, dest: GenDest) {
        struct GenGuard(Rc<MainWindow>);
        impl Drop for GenGuard {
            fn drop(&mut self) {
                self.0.generating.set(false);
                self.0.busy.set(false);
            }
        }
        let gen_guard = GenGuard(self.clone());
        let gen_target = self.gen_target.borrow_mut().take();
        let mut num: u64 = 1;
        let mut file: Option<(PathBuf, bool)> = None;
        if dest.is_list() {
            match parse_num_passwords(&self.f().text("NumPasswBox")) {
                Some(n) => num = n,
                None => {
                    dialogs::ask(
                        ROOT,
                        &trl_format("Invalid number of passwords.\nAllowed range is %1-%2.", &["2", &PASSW_MAX_NUM.to_string()]),
                        MsgIcon::Error,
                        MsgButtons::Ok,
                        false,
                    )
                    .await;
                    self.f().focus("NumPasswBox");
                    return;
                }
            }
            if dest == GenDest::FileList {
                let Some(path) = crate::vcl::dialogs::save_file(&trl("Save password list"), None, None, &[]).await else { return };
                let mut append = false;
                if path.exists() {
                    match dialogs::ask(
                        ROOT,
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
                pl.close();
            }
        }

        {
            let core = self.core.borrow();
            let s = &core.settings;
            if s.include_chars && s.passw_options.flags & PASSWOPTION_EACHCHARONLYONCE != 0 && s.chars_length > core.passw_gen.custom_char_set_unique_size() {
                let n = core.passw_gen.custom_char_set_unique_size();
                drop(core);
                dialogs::ask(
                    ROOT,
                    &trl_format("If the option \"Each character must occur\nonly once\" is activated, the password length\nis limited to %1 unique characters.", &[&n.to_string()]),
                    MsgIcon::Warning,
                    MsgButtons::Ok,
                    false,
                )
                .await;
                self.f().set_position("CharsLengthSpinBtn", n as i64);
                self.core.borrow_mut().settings.chars_length = n;
            }
        }
        if self.core.borrow().settings.include_words && self.core.borrow().settings.passphr_spec_len {
            let t = self.core.borrow().settings.passphr_spec_len_string.clone();
            if let Err(e) = parse_passphrase_length(&t) {
                dialogs::ask(ROOT, &e, MsgIcon::Error, MsgButtons::Ok, false).await;
                return;
            }
        }
        {
            let core = self.core.borrow();
            let s = &core.settings;
            if s.include_words && s.passw_options.flags & PASSWOPTION_EACHWORDONLYONCE != 0 && s.words_num > core.passw_gen.word_list_size() {
                let n = core.passw_gen.word_list_size();
                drop(core);
                dialogs::ask(
                    ROOT,
                    &trl_format("If the option \"Each word must occur only\nonce\" is activated, the number of words\nis limited to the size of the word list (%1).", &[&n.to_string()]),
                    MsgIcon::Warning,
                    MsgButtons::Ok,
                    false,
                )
                .await;
                self.f().set_position("WordsNumSpinBtn", n as i64);
                self.core.borrow_mut().settings.words_num = n;
            }
        }
        let mut req: GenRequest = match self.core.borrow().gen_request(dest, num) {
            Ok(r) => r,
            Err(e) => {
                msg(ROOT, &e, MsgIcon::Error);
                return;
            }
        };

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
                        msg(ROOT, &trl_format("Error while loading script:\n%1.", &[&e.to_string()]), MsgIcon::Error);
                        return;
                    }
                    s
                }
            };
            let text = self.f().text("ScriptList");
            self.add_entry_to_list("ScriptList", &text, true);
            script = Some(s);
        }
        if req.chars_len == 0 && req.num_words == 0 && req.format.is_empty() && script.is_none() {
            return;
        }
        if dest == GenDest::FileList {
            req.file_newline = self.core.borrow().newline.clone();
        }

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
                let hook_arg: Option<(&mut pwtool_core::generate::ScriptHook<'_>, bool)> = if has_script { Some((&mut hook, standalone)) } else { None };
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
                                let est_bytes = hdr_u16.saturating_add(item_u16.saturating_mul(req.num_passw)).saturating_add(1).saturating_mul(2);
                                if est_bytes > pwtool_core::generate::PASSWLIST_MAX_BYTES {
                                    return Err(trl("The estimated list size will probably exceed\nthe internal memory limit.\nPlease reduce the number of passwords"));
                                }
                            }
                            if list_utf16_units.saturating_add(item_u16).saturating_add(1).saturating_mul(2) > pwtool_core::generate::PASSWLIST_MAX_BYTES {
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
                                        let enc = pwtool_core::text_file::detect_file_encoding(path, file_encoding).unwrap_or(file_encoding);
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
                        if req.include_header && (s.count < req.num_passw || (check_each && s.total_security < MAX_ENTROPY as f64)) {
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
            let _ = tx.send_blocking((GenOutput { pg, rng, script, result, single, list, mem_limit_reached }, common));
        });

        let mut progress_win: Option<Rc<ProgressWindow>> = None;
        let start = std::time::Instant::now();
        let mut last_prog = 0u64;
        let mut last_prog_change = std::time::Instant::now();
        let mut recv_fut = std::pin::pin!(rx.recv());
        let out = loop {
            let tick = std::pin::pin!(pwtool_app::rt::sleep(Duration::from_millis(25)));
            match futures_util::future::select(recv_fut, tick).await {
                futures_util::future::Either::Left((res, _)) => break res.expect("generator thread exited unexpectedly"),
                futures_util::future::Either::Right((_, pending)) => recv_fut = pending,
            }
            let elapsed = start.elapsed().as_millis();
            if !self.busy.get() && ((dest == GenDest::GuiList && num >= 100_000) || elapsed >= 200) {
                self.busy.set(true);
            }
            let cur_prog = progress.load(Ordering::Relaxed);
            if cur_prog != last_prog {
                last_prog = cur_prog;
                last_prog_change = std::time::Instant::now();
            }
            if progress_win.is_none() && ((dest == GenDest::GuiList && num >= 100_000) || elapsed >= 1000) {
                let title = trl(if num == 1 { "Generating password" } else { "Generating password list" });
                progress_win = Some(ProgressWindow::new(ROOT, &title, ""));
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
            pw.close();
        }
        self.busy.set(false);
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
                    dialogs::ask(ROOT, &trl("Process was cancelled by the user"), MsgIcon::Warning, MsgButtons::Ok, false).await;
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
                msg(ROOT, &m, MsgIcon::Error);
                return;
            }
        };
        if out.mem_limit_reached {
            let count_str = summary.count.to_string();
            self.f().set_text("NumPasswBox", &count_str);
            dialogs::ask(
                ROOT,
                &trl_format("The memory limit of the password list has been reached. The number of passwords has been reduced to %1.", &[&count_str]),
                MsgIcon::Warning,
                MsgButtons::Ok,
                false,
            )
            .await;
        }
        if cancelled && num > 1 {
            dialogs::ask(
                ROOT,
                &format!("{}.\n\n{}", trl("Process was cancelled by the user"), trl_format("%1 passwords generated.", &[&summary.count.to_string()])),
                MsgIcon::Warning,
                MsgButtons::Ok,
                false,
            )
            .await;
        }
        if dest == GenDest::GuiSingle || summary.count > 0 {
            self.f().set_text("FormatPasswInfoLbl", &summary.format_info);
        }
        if summary.count != 0 {
            match dest {
                GenDest::GuiSingle if gen_target.is_some() => {
                    if let Some((form, name)) = &gen_target {
                        form.set_text(name, &out.single);
                    }
                }
                GenDest::GuiSingle => {
                    let tag = self.passw_box_tag.get();
                    self.passw_box_tag.set(0);
                    self.f().set_text("PasswBox", &out.single);
                    self.passw_box_tag.set(tag | PASSWBOX_TAG_PASSW_GEN);
                    self.show_passw_info(summary.last_len, summary.last_security, summary.last_common_match, false);
                    self.f().focus("PasswBox");
                    let core = self.core.borrow();
                    if core.config.auto_clear_passw {
                        self.auto_clear_passw_cnt.set(core.config.auto_clear_passw_time);
                    }
                }
                GenDest::GuiList => {
                    if self.passw_list.borrow().is_none() {
                        let w_enc = self.weak();
                        let pl = PasswListWindow::new(
                            ROOT,
                            Rc::new(move || {
                                w_enc
                                    .upgrade()
                                    .map(|w| {
                                        let c = w.core.borrow();
                                        (c.file_encoding, c.newline.clone())
                                    })
                                    .unwrap_or((pwtool_core::text_file::CharEncoding::Utf8, "\n".to_string()))
                            }),
                        );
                        pl.load_config(&self.core.borrow().ini);
                        *self.passw_list.borrow_mut() = Some(pl);
                    }
                    self.passw_list.borrow().as_ref().unwrap().show_list(ROOT, &out.list);
                }
                GenDest::ClipboardList => {
                    clipboard::set_data(&out.list);
                    info_box(ROOT, &trl("Password list copied to clipboard."));
                }
                GenDest::FileList => {
                    let name = file.as_ref().map(|(p, _)| p.file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();
                    msg(ROOT, &trl_format("File \"%1\" successfully created.\n\n%2 passwords written.", &[&name, &summary.count.to_string()]), MsgIcon::Info);
                }
                GenDest::Clipboard => {
                    clipboard::set_data(&out.single);
                    msg(ROOT, &trl("Password copied to clipboard."), MsgIcon::Info);
                }
                GenDest::MsgBox => {
                    let len = out.single.chars().count();
                    if len > 1000 {
                        dialogs::ask(
                            ROOT,
                            &trl("Password is too long to show it in a\nmessage box like this.\n\nIt will be copied to the clipboard."),
                            MsgIcon::Warning,
                            MsgButtons::Ok,
                            false,
                        )
                        .await;
                        clipboard::set_data(&out.single);
                    } else {
                        let text = pwtool_core::unicode::format_w(
                            &trl("The generated password is:\n\n%1\n\nThe estimated security is %2 bits.\n\nYes -> copy password to clipboard,\nNo -> generate new password,\nCancel -> cancel process."),
                            &[&out.single, &format_float_1(summary.last_security)],
                        );
                        match dialogs::ask(ROOT, &text, MsgIcon::Info, MsgButtons::YesNoCancel, false).await {
                            MsgResult::Yes => clipboard::set_data(&out.single),
                            MsgResult::No => self.generate_passw(GenDest::MsgBox),
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
    fn update_entropy_progress(&self, _force: bool) {
        if !self.show_ent_progress.get() {
            self.f().set_text("StatusBar", &format!("{PROGRAM_NAME} {}", version_string()));
            return;
        }
        let bits = EntropyManager::global().entropy_bits();
        let max = bits == pwtool_core::random::entropy::MAX_ENTROPYBITS;
        let pct = (100.0 * bits as f64 / MAX_ENTROPY as f64).round().min(100.0) as i32;
        let text = format!("{}{}{} ({}%)", if max { ">" } else { "" }, bits, if bits > MAX_ENTROPY { "+" } else { "" }, pct);
        self.f().set_text("StatusBar", &format!("{PROGRAM_NAME} {}        E  {text}", version_string()));
    }

    /// Entropy from the keyboard and mouse events of a window (the original
    /// does this application-wide in `AppMessage()`).
    fn collect_entropy(&self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        let mut changed = false;
        for e in events {
            let mut em = EntropyManager::global();
            let bits = match e {
                egui::Event::Key { key, pressed: true, physical_key, .. } => em.add_event(
                    &mut RandomPool::global(),
                    EntropyEventType::Keyboard,
                    key as u64,
                    physical_key.map_or(0, |k| k as u64),
                    &[],
                    ENTROPY_KEYBOARD,
                ),
                egui::Event::PointerButton { pos, button, pressed: true, .. } => {
                    let p = ((pos.x as u64) << 32) | (pos.y as u64 & 0xffff_ffff);
                    em.add_event(&mut RandomPool::global(), EntropyEventType::MouseClick, button as u64, p, &[], ENTROPY_MOUSECLICK)
                }
                egui::Event::PointerMoved(pos) => {
                    if self.last_pointer.replace(Some(pos)) == Some(pos) {
                        0
                    } else {
                        let c = self.mouse_move_cnt.get();
                        if c == 0 {
                            self.mouse_move_cnt.set(50 + pwtool_core::random::fprng_rand_range(50) as i32);
                            let p = ((pos.x as u64) << 32) | (pos.y as u64 & 0xffff_ffff);
                            em.add_event(&mut RandomPool::global(), EntropyEventType::MouseMove, 0, p, &[], ENTROPY_MOUSEMOVE)
                        } else {
                            self.mouse_move_cnt.set(c - 1);
                            0
                        }
                    }
                }
                egui::Event::MouseWheel { delta, .. } => {
                    let c = self.mouse_wheel_cnt.get();
                    if c <= 0 {
                        self.mouse_wheel_cnt.set(pwtool_core::random::fprng_rand_range(10) as i32);
                        em.add_event(
                            &mut RandomPool::global(),
                            EntropyEventType::MouseWheel,
                            delta.x.to_bits() as u64,
                            delta.y.to_bits() as u64,
                            &[],
                            ENTROPY_MOUSECLICK,
                        )
                    } else {
                        self.mouse_wheel_cnt.set(c - 1);
                        0
                    }
                }
                _ => 0,
            };
            changed |= bits != 0;
        }
        if changed {
            self.update_entropy_progress(false);
        }
    }

    /// `TimerTick()` (every second).
    fn setup_timer(self: &Rc<Self>) {
        let w = self.weak();
        spawn(async move {
            loop {
                pwtool_app::rt::sleep(Duration::from_secs(1)).await;
                let Some(w) = w.upgrade() else { return };
                w.timer_tick();
            }
        });
    }

    fn timer_tick(&self) {
        let mut c = self.timer_cnt.get();
        c[0] += 1;
        if c[0] == TIMER_TOUCHPOOL {
            RandomPool::global().touch_pool();
            c[0] = 0;
        }
        c[1] += 1;
        if c[1] == TIMER_RANDOMIZE {
            EntropyManager::global().add_system_entropy(&mut RandomPool::global());
            self.update_entropy_progress(false);
            c[1] = 0;
        }
        c[2] += 1;
        if c[2] == TIMER_MOVEPOOL {
            RandomPool::global().move_pool();
            c[2] = 0;
        }
        c[3] += 1;
        if c[3] == TIMER_WRITESEEDFILE {
            self.core.borrow().write_rand_seed_file();
            c[3] = 0;
        }
        self.timer_cnt.set(c);
        let n = self.auto_clear_clip_cnt.get();
        if n > 0 {
            self.auto_clear_clip_cnt.set(n - 1);
            if n == 1 {
                clipboard::clear_data(false);
            }
        }
        let n = self.auto_clear_passw_cnt.get();
        if n > 0 {
            self.auto_clear_passw_cnt.set(n - 1);
            if n == 1 {
                let tag = self.passw_box_tag.get();
                self.passw_box_tag.set(0);
                self.f().set_text("PasswBox", "");
                self.passw_box_tag.set(tag);
                self.f().set_text("PasswInfoLbl", &trl("Click on \"Generate\""));
                self.f().set_hint("PasswInfoLbl", "");
                self.passw_bar.clear();
            }
        }
    }

    fn add_entropy_from_file(self: &Rc<Self>) {
        let w = self.weak();
        spawn(async move {
            let Some(p) = crate::vcl::dialogs::open_file(&trl("Select high-entropy file"), None, &[]).await else { return };
            let Some(w) = w.upgrade() else { return };
            use std::io::Read;
            match std::fs::File::open(&p) {
                Ok(mut f) => {
                    const MAX_BYTES: u64 = 1_000_000_000;
                    let mut buf = Zeroizing::new(vec![0u8; 65_536]);
                    let mut total: u64 = 0;
                    let mut bits: u32 = 0;
                    {
                        let mut em = EntropyManager::global();
                        let mut pool = RandomPool::global();
                        while total < MAX_BYTES {
                            let to_read = (MAX_BYTES - total).min(buf.len() as u64) as usize;
                            match f.read(&mut buf[..to_read]) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    bits = bits.saturating_add(em.add_data(&mut pool, &buf[..n], 4.0, 4.0));
                                    total += n as u64;
                                }
                            }
                        }
                        pool.flush();
                    }
                    w.update_entropy_progress(true);
                    msg(ROOT, &trl_format("%1 bits of entropy have been added to the random pool.", &[&bits.to_string()]), MsgIcon::Info);
                }
                Err(_) => msg(ROOT, &trl("Could not open file."), MsgIcon::Error),
            }
        });
    }

    fn deactivate_det_rand_gen(&self) {
        if self.key_seeded_prng.borrow_mut().take().is_some() {
            self.set_title(PROGRAM_NAME);
            self.f().set_menu_enabled("MainMenu_Tools_DetermRandGen_Reset", false);
            self.f().set_menu_enabled("MainMenu_Tools_DetermRandGen_Deactivate", false);
            info_box(ROOT, &trl("Deterministic random generator has been DEACTIVATED."));
        }
    }

    /// `UseKeySeededPRNG()` with an already seeded generator.
    pub fn use_key_seeded_prng_with(&self, prng: AesCtrPrng) {
        *self.key_seeded_prng.borrow_mut() = Some(prng);
        self.set_title(&pwtool_core::unicode::format_w(
            "%1 (%2)",
            &[PROGRAM_NAME, &trl("WARNING: Random generator is deterministic (press F7 to deactivate)!")],
        ));
        self.f().set_menu_enabled("MainMenu_Tools_DetermRandGen_Reset", true);
        self.f().set_menu_enabled("MainMenu_Tools_DetermRandGen_Deactivate", true);
        info_box(ROOT, &trl("Deterministic random generator has been ACTIVATED."));
    }

    // ----------------------------------------------------------------------
    // configuration, closing

    /// `SaveConfig()`.
    pub fn save_config(&self) -> bool {
        self.save_config_to(None)
    }

    pub fn save_config_to(&self, path: Option<&std::path::Path>) -> bool {
        self.sync_settings();
        let f = self.f();
        {
            let mut core = self.core.borrow_mut();
            if let Some(pl) = self.passw_list.borrow().as_ref() {
                pl.save_config(&mut core.ini);
            }
            let ini = &mut core.ini;
            ini.write_bool("Main", "AlwaysOnTop", f.menu_checked("MainMenu_Options_AlwaysOnTop"));
            ini.write_bool("Main", "HidePassw", f.checked("TogglePasswBtn"));
            ini.write_bool("Main", "HideEntropyProgress", f.menu_checked("MainMenu_Options_HideEntProgress"));
            ini.write_bool("Main", "SaveSettingsOnExit", f.menu_checked("MainMenu_Options_SaveSettingsOnExit"));
            ini.write_bool("Main", "PasswEditable", f.menu_checked("PasswBoxMenu_Editable"));
            ini.write_bool("Main", "EnablePasswTest", f.menu_checked("PasswBoxMenu_EnablePasswTest"));
        }
        let r = self.core.borrow_mut().save_config_to(path);
        match r {
            Ok(()) => true,
            Err(e) => {
                msg(ROOT, &trl_format("Error while writing to configuration file:\n%1.", &[&e.to_string()]), MsgIcon::Error);
                false
            }
        }
    }

    /// `FormCloseQuery()`: whether the window may close now; asks if needed
    /// and closes it again afterwards.
    pub fn close_query(self: &Rc<Self>) -> bool {
        if self.close_confirmed.get() || !self.core.borrow().config.confirm_exit || pwtool_app::app::restart_requested() {
            return true;
        }
        let w = self.clone();
        spawn(async move {
            if dialogs::ask(ROOT, &trl_format("Are you sure you want to exit\n%1?", &[PROGRAM_NAME]), MsgIcon::Question, MsgButtons::YesNo, true).await
                == MsgResult::Yes
            {
                w.close_confirmed.set(true);
                windows::ctx().send_viewport_cmd(ViewportCommand::Close);
            }
        });
        false
    }

    /// `FormClose()`.
    pub fn on_close(&self) {
        if (self.f().menu_checked("MainMenu_Options_SaveSettingsOnExit") && !self.core.borrow().cmd.config_readonly)
            || pwtool_app::app::restart_requested()
        {
            self.save_config();
        }
        if !self.core.borrow().write_rand_seed_file() {
            let path = self.core.borrow().paths.data_dir.join(PROGRAM_RANDSEEDFILE).to_string_lossy().into_owned();
            eprintln!("{}", trl_format("Could not write to random seed file\n\"%1\".", &[&path]));
        }
        let tag = self.passw_box_tag.get();
        self.passw_box_tag.set(0);
        self.f().set_text("PasswBox", "");
        self.passw_box_tag.set(tag);
    }

    /// Profile given on the command line or configured for start-up.
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

    /// Show start-up errors (`FormActivate`, once).
    pub fn show_startup_errors(&self) {
        if self.startup_shown.replace(true) {
            return;
        }
        let errs: Vec<String> = self.core.borrow_mut().startup_errors.drain(..).collect();
        if !errs.is_empty() {
            let mut m = trl_format("%1 encountered %2 error(s) during startup:", &[PROGRAM_NAME, &errs.len().to_string()]);
            for e in errs {
                m.push_str("\n- ");
                m.push_str(&e);
            }
            msg(ROOT, &m, MsgIcon::Warning);
        }
    }
}

/// What the main window shows each frame.
pub struct MainContent(pub Rc<MainWindow>);

impl Content for MainContent {
    fn ui(&self, ui: &mut Ui) {
        let w = &self.0;
        w.update_passw_box_menu();
        if let Some(pl) = w.passw_list.borrow().as_ref() {
            pl.update_menu(false);
        }
        let title = w.title.borrow().clone();
        if *w.title_shown.borrow() != title {
            ui.ctx().send_viewport_cmd(ViewportCommand::Title(title.clone()));
            *w.title_shown.borrow_mut() = title;
        }
        let busy = w.busy.get();
        ui.add_enabled_ui(!busy, |ui| Content::ui(&*w.form, ui));
    }
}

fn open_uri(uri: &str) {
    let target = if uri.contains("://") || std::path::Path::new(uri).exists() { uri.to_string() } else { format!("https://{uri}") };
    if !pwtool_app::platform::sys::system_open(std::ffi::OsStr::new(&target)) {
        msg(ROOT, &trl_format("Could not open file/execute operation\n\"%1\".", &[&target]), MsgIcon::Error);
    }
}

fn timer_info() {
    let kind = pwtool_core::random::hrtimer::timer_kind();
    let name = match kind {
        pwtool_core::random::hrtimer::TimerKind::Rdtsc => "Time stamp counter (RDTSC instruction)",
        pwtool_core::random::hrtimer::TimerKind::Qpc => {
            if cfg!(windows) {
                "QueryPerformanceCounter"
            } else {
                "CLOCK_MONOTONIC_RAW"
            }
        }
        pwtool_core::random::hrtimer::TimerKind::None => "N/A",
    };
    let val = pwtool_core::random::hrtimer::high_res_timer().to_string();
    msg(ROOT, &trl_format("High-resolution timer used:\n%1.\n\nCurrent value: %2.", &[name, &val]), MsgIcon::Info);
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
