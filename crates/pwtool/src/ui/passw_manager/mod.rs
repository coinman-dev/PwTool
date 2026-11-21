//! PassCube password manager (`upstream/src/main/PasswManager.*` and the
//! dialogs `PasswMng*`).
//!
//! Differences to the original (deliberate):
//! - the main menu items "Rearrange Entry" call the function matching their
//!   caption (the original form connects them to the wrong handlers);
//! - the tag menu inserts the tag of the clicked item even in search mode;
//! - the password font is saved under the key it is read from
//!   (`PasswBoxFont`; the original writes `PasswFont`);
//! - window positions are not saved, only sizes;
//! - the contents of the edit boxes cannot be dragged from their labels;
//! - "search database for keyword" uses the title of the foreground window
//!   on Windows only.

pub mod automation;
mod dialogs;
mod helpers;
mod list;
mod ops;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::{Rc, Weak};
use std::time::Instant;

use egui::{Ui, ViewportCommand, ViewportId};
use pwtool_core::config::{IniFile, parse_comma_text, to_comma_text};
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::database::{FieldType, NUM_FIELDS, PasswDatabase, PasswHistory};
use zeroize::Zeroizing;

use list::{RowKey, TagKind, TagList};

use super::main_window::MainWindow;
use super::passw_enter::PasswEnterDlg;
use super::quality_bar::QualityBar;
use crate::vcl::Form;
use crate::vcl::dialogs::MsgIcon;
use crate::vcl::fonts::{self, FontSpec};
use crate::vcl::list_view::{ListView, LvEvent};
use crate::windows::{self, Content};

pub use pwtool_app::app_config::DatabaseConfig;

const CONFIG_ID: &str = "PasswManager";
const WINDOW_ID: &str = "PasswManager";
const PASSW_MANAGER_NAME: &str = "PassCube";
const PWDB_FILE_EXT: &str = ".pwdb";
const HIDE_PASSW_STR: &str = "********";

const DB_NUM_KEYVAL_KEYS: usize = 4;
const DB_KEYVAL_KEYS: [&str; DB_NUM_KEYVAL_KEYS] = ["Autotype", "Run", "Profile", "FormatPW"];
const DB_KEYVAL_UI_KEYS: [&str; DB_NUM_KEYVAL_KEYS] = ["Autotype", "Run", "Profile", "Format password"];
const DB_KEYVAL_AUTOTYPE: usize = 0;
const DB_KEYVAL_RUN: usize = 1;
const DB_KEYVAL_PROFILE: usize = 2;
const DB_KEYVAL_FORMATPASSW: usize = 3;
const DB_MAX_TAG_LEN: usize = 30;

const SEARCH_MODE_OFF: i32 = 0;
const SEARCH_MODE_NORMAL: i32 = 1;
const SEARCH_MODE_FUZZY: i32 = 2;
const MOVE_TOP: i32 = 0;
const MOVE_UP: i32 = 1;
const MOVE_DOWN: i32 = 2;
const MOVE_BOTTOM: i32 = 3;
const ASK_SAVE_OK: i32 = 0;
const ASK_SAVE_CANCEL: i32 = 1;
const ASK_SAVE_ERROR: i32 = 2;
const RESET_COLUMNS: u32 = 1;
const RELOAD_TAGS: u32 = 2;
const DB_OPEN_FLAG_EXISTING: u32 = 1;
const DB_OPEN_FLAG_UNLOCK: u32 = 2;
const DB_OPEN_FLAG_READONLY: u32 = 4;
const DB_LOCK_MANUAL: i32 = 1;
const DB_LOCK_AUTO: i32 = 2;
const FORM_TAG_OPEN_DB: i32 = 1;
const FORM_TAG_UNLOCK_DB: i32 = 2;
const NOTES_TAG_HIDDEN: i32 = 1;
const NOTES_TAG_CHANGING: i32 = 2;
const WEAK_PASSW_THRESHOLD: i32 = 75;
const MAX_NUM_RECENT_FILES: usize = 10;
const STD_EXPIRY_DAYS: [i32; 6] = [7, 14, 30, 90, 180, 365];
const DB_FLAG_FOUND: u32 = 1;
const DB_FLAG_EXPIRED: u32 = 2;
const DB_FLAG_EXPIRES_SOON: u32 = 4;
const MAX_NOTES_LEN: usize = 200;

const UI_FIELD_NAMES: [&str; NUM_FIELDS] = [
    "Title",
    "User name",
    "Password",
    "URL",
    "Keyword",
    "Notes",
    "Key-value list",
    "Tags",
    "Creation time",
    "Last modification",
    "Password changed",
    "Password expiry",
    "Password history",
];

/// Filter of the entry list (View | Filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterType {
    None,
    Expired,
    ExpireSoon,
    WeakPassw,
}

impl FilterType {
    fn menu_item(self) -> Option<&'static str> {
        match self {
            FilterType::None => None,
            FilterType::Expired => Some("MainMenu_View_Filter_Expired"),
            FilterType::ExpireSoon => Some("MainMenu_View_Filter_ExpireSoon"),
            FilterType::WeakPassw => Some("MainMenu_View_Filter_WeakPassw"),
        }
    }
}

fn win_id() -> ViewportId {
    ViewportId::from_hash_of(WINDOW_ID)
}

/// The password manager window (`TPasswMngForm`).
pub struct PasswManager {
    main: Weak<MainWindow>,
    pub form: Rc<Form>,
    window: RefCell<Option<Rc<windows::Window>>>,
    size: Cell<egui::Vec2>,
    db: RefCell<Option<PasswDatabase>>,
    db_file_name: RefCell<String>,
    recent_files: RefCell<Vec<String>>,
    db_read_only: Cell<bool>,
    /// `m_pSelectedItem`: `None` = no item, else the key of the item.
    selected: Cell<Option<RowKey>>,
    show_col_mask: Cell<u32>,
    /// Fields of the shown columns.
    col_fields: RefCell<Vec<usize>>,
    sort_by_idx: Cell<i32>,
    tags_sort_by_idx: Cell<i32>,
    sort_order_factor: Cell<i32>,
    tags_sort_order_factor: Cell<i32>,
    search_mode: Cell<i32>,
    lock_sel_item_index: Cell<i32>,
    passw_entropy_bits: Cell<f64>,
    lock_sel_tags: RefCell<Vec<usize>>,
    item_changed: Cell<bool>,
    item_passw_change_confirm: Cell<bool>,
    db_changed: Cell<bool>,
    locked: Cell<bool>,
    unlock_tried: Cell<bool>,
    last_user_action: Cell<Instant>,
    search_flags: Cell<u32>,
    list_col_widths: RefCell<Vec<f32>>,
    ui_field_names: [String; NUM_FIELDS],
    /// Key → displayed (translated) name, ordered by key (`m_keyValNames`).
    key_val_names: BTreeMap<String, String>,
    /// Tags shown in the tag list with their frequency (`m_tags`).
    tags: RefCell<Vec<(String, u32)>>,
    tag_menu_items: RefCell<Vec<String>>,
    global_casei_tags: RefCell<BTreeMap<String, String>>,
    tag_filter: RefCell<BTreeSet<String>>,
    temp_key_val: RefCell<Option<Vec<(Zeroizing<String>, Zeroizing<String>)>>>,
    temp_passw_history: RefCell<Option<PasswHistory>>,
    temp_notes: RefCell<Option<Zeroizing<String>>>,
    notes_hidden_text: String,
    notes_tag: Cell<i32>,
    /// `Tag` of the form (`FORM_TAG_*`).
    form_tag: Cell<i32>,
    passw_box_gen: Cell<bool>,
    filter: Cell<FilterType>,
    csv_col_mask: Cell<u32>,
    idle_suspend: Cell<i32>,
    idle_timer_enabled: Cell<bool>,
    busy: Cell<bool>,
    first_activation: Cell<bool>,
    unlock_tick: Cell<Option<Instant>>,
    apply_pending: Cell<bool>,
    tag_apply_pending: Cell<bool>,
    /// Viewport state seen in the last frame: (minimized, focused).
    last_viewport: Cell<(bool, bool)>,
    list: Rc<ListView<RowKey>>,
    tag_view: Rc<TagList>,
    passw_bar: QualityBar,
    passw_enter: RefCell<Option<Rc<PasswEnterDlg>>>,
    dialogs: dialogs::Dialogs,
}

/// `SuspendIdleTimer`: no automatic locking while dialogs are shown.
struct IdleSuspend(Rc<PasswManager>);

impl IdleSuspend {
    fn new(m: &Rc<PasswManager>) -> Self {
        m.idle_suspend.set(m.idle_suspend.get() + 1);
        IdleSuspend(m.clone())
    }
}

impl Drop for IdleSuspend {
    fn drop(&mut self) {
        let n = self.0.idle_suspend.get() - 1;
        self.0.idle_suspend.set(n);
        if n == 0 {
            self.0.notify_user_action();
        }
    }
}

/// What the manager window draws each frame.
struct PmContent(Rc<PasswManager>);

impl Content for PmContent {
    fn ui(&self, ui: &mut Ui) {
        let m = &self.0;
        if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: true, .. })))
        {
            m.notify_user_action();
        }
        m.update_frame_states();
        let busy = m.busy.get();
        ui.add_enabled_ui(!busy, |ui| Content::ui(&*m.form, ui));
        m.after_frame(ui);
    }
}

impl PasswManager {
    pub fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let form = super::dialogs::load("PasswManager");
        let f = &form;
        let icon16 = |i: usize| f.image("ImageList16", i);
        let list = ListView::new("pm-dbview", true);
        let tag_view = TagList::new([icon16(6), icon16(7), icon16(8)]);
        let passw_bar = QualityBar::new(128);
        let b = passw_bar.clone();
        f.set_painter("PasswGauge", move |ui, rect| b.paint(ui, rect));
        let l = list.clone();
        f.set_painter("DbView", move |ui, rect| l.paint(ui, rect));
        let t = tag_view.clone();
        f.set_painter("TagView", move |ui, rect| t.paint(ui, rect));
        f.set_secret("PasswBox");
        f.set_password_char("PasswBox", true);
        f.set_font_family("PasswBox", Some(fonts::PM_PASSW));
        f.set_control_caption("ClearFilterBtn", "✕");
        f.set_control_caption("FilterInfoPanel", &trl("Filtered"));
        f.set_font_color("FilterInfoPanel", Some(egui::Color32::from_rgb(0x20, 0x60, 0xe0)));
        f.set_hint("SearchBox", &trl("Search..."));
        // the tool bar buttons of the edit panel: AddModify/Cancel at the
        // places of Prev/Next (`AddModifyBtn->Left = PrevBtn->Left`)
        for (src, dst) in [("PrevBtn", "AddModifyBtn"), ("NextBtn", "CancelBtn")] {
            let (x, _, _, _) = f.bounds(src);
            let (_, y, w, h) = f.bounds(dst);
            f.set_bounds(dst, x, y, w, h);
        }

        // key names (`m_keyValNames`), translated if unique
        let mut key_val_names: BTreeMap<String, String> =
            DB_KEYVAL_KEYS.iter().zip(DB_KEYVAL_UI_KEYS.iter()).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        if pwtool_core::language::has_translation() {
            let transl: BTreeMap<String, String> = key_val_names.iter().map(|(k, v)| (k.clone(), trl(v))).collect();
            let uniq: BTreeSet<&String> = transl.values().collect();
            if uniq.len() == transl.len() {
                key_val_names = transl;
            } else {
                super::dialogs::msg(ViewportId::ROOT, "Password manager: Translations of key names not unique!", MsgIcon::Warning);
            }
        }
        super::dialogs::register_form(WINDOW_ID, f);
        let size = crate::vcl::window_size(f, &windows::ctx());
        let m = Rc::new(PasswManager {
            main: Rc::downgrade(main),
            form,
            window: RefCell::new(None),
            size: Cell::new(size),
            db: RefCell::new(None),
            db_file_name: RefCell::new(String::new()),
            recent_files: RefCell::new(Vec::new()),
            db_read_only: Cell::new(false),
            selected: Cell::new(None),
            show_col_mask: Cell::new(7),
            col_fields: RefCell::new(Vec::new()),
            sort_by_idx: Cell::new(-1),
            tags_sort_by_idx: Cell::new(0),
            sort_order_factor: Cell::new(1),
            tags_sort_order_factor: Cell::new(1),
            search_mode: Cell::new(SEARCH_MODE_OFF),
            lock_sel_item_index: Cell::new(-1),
            passw_entropy_bits: Cell::new(0.0),
            lock_sel_tags: RefCell::new(Vec::new()),
            item_changed: Cell::new(false),
            item_passw_change_confirm: Cell::new(false),
            db_changed: Cell::new(false),
            locked: Cell::new(false),
            unlock_tried: Cell::new(false),
            last_user_action: Cell::new(Instant::now()),
            search_flags: Cell::new(u32::MAX >> 1),
            list_col_widths: RefCell::new(Vec::new()),
            ui_field_names: UI_FIELD_NAMES.map(trl),
            key_val_names,
            tags: RefCell::new(Vec::new()),
            tag_menu_items: RefCell::new(Vec::new()),
            global_casei_tags: RefCell::new(BTreeMap::new()),
            tag_filter: RefCell::new(BTreeSet::new()),
            temp_key_val: RefCell::new(None),
            temp_passw_history: RefCell::new(None),
            temp_notes: RefCell::new(None),
            notes_hidden_text: format!("[{}]", trl("Notes hidden - click here to display")),
            notes_tag: Cell::new(0),
            form_tag: Cell::new(0),
            passw_box_gen: Cell::new(false),
            filter: Cell::new(FilterType::None),
            csv_col_mask: Cell::new(u32::MAX >> 1),
            idle_suspend: Cell::new(0),
            idle_timer_enabled: Cell::new(false),
            busy: Cell::new(false),
            first_activation: Cell::new(true),
            unlock_tick: Cell::new(None),
            apply_pending: Cell::new(false),
            tag_apply_pending: Cell::new(false),
            last_viewport: Cell::new((false, false)),
            list,
            tag_view,
            passw_bar,
            passw_enter: RefCell::new(None),
            dialogs: dialogs::Dialogs::default(),
        });
        m.setup_menus();
        m.connect_events();
        {
            let main = m.mainw();
            let core = main.core.borrow();
            m.load_config(&core.ini);
        }
        m.set_show_col_mask_from_menu();
        m.reset_list_view(RESET_COLUMNS);
        m.set_item_changed(false);
        m.reset_db_open_controls();
        m.change_caption();
        m.start_idle_timer();
        m
    }

    fn mainw(&self) -> Rc<MainWindow> {
        self.main.upgrade().expect("main window")
    }

    fn f(&self) -> &Form {
        &self.form
    }

    /// Password entry dialog shared with the main window.
    fn passw_enter(&self) -> Rc<PasswEnterDlg> {
        if let Some(d) = self.passw_enter.borrow().as_ref() {
            return d.clone();
        }
        let d = self.mainw().passw_enter();
        *self.passw_enter.borrow_mut() = Some(d.clone());
        d
    }

    /// Enabled state of a menu item.
    fn set_enabled(&self, name: &str, on: bool) {
        if self.f().has_menu_item(name) {
            self.f().set_menu_enabled(name, on);
        }
    }

    fn menu_bool(&self, name: &str) -> bool {
        self.f().menu_checked(name)
    }

    fn set_menu_bool(&self, name: &str, v: bool) {
        if self.f().has_menu_item(name) {
            self.f().set_menu_checked(name, v);
        }
    }

    // ------------------------------------------------------------------
    // menus

    fn setup_menus(&self) {
        let f = self.f();
        // View | Show Columns
        let cols: Vec<(String, String, Option<u16>)> =
            self.ui_field_names.iter().enumerate().map(|(i, n)| (format!("col{i}"), n.clone(), None)).collect();
        f.set_dynamic_items_before("MainMenu_View_ShowColumns", None, &cols);
        // View | Sort Entries By: the fields between the separators
        let sort: Vec<(String, String, Option<u16>)> =
            self.ui_field_names.iter().enumerate().map(|(i, n)| (format!("sort{i}"), n.clone(), None)).collect();
        f.set_dynamic_items_before("MainMenu_View_SortBy", Some("MainMenu_View_SortBy_N2"), &sort);
        // expiry days
        let days: Vec<(String, String, Option<u16>)> =
            STD_EXPIRY_DAYS.iter().map(|d| (format!("days{d}"), trl_format("%1 days", &[&d.to_string()]), None)).collect();
        f.set_dynamic_items_before("ExpiryMenu", None, &days);
        for n in [
            "MainMenu_View_ShowPasswInList",
            "SearchMenu_CaseSensitive",
            "SearchMenu_FuzzySearch",
            "MainMenu_View_SortBy_Unsorted",
            "MainMenu_View_SortBy_Ascending",
            "MainMenu_View_SortBy_Descending",
            "MainMenu_View_SortTagsBy_Name",
            "MainMenu_View_SortTagsBy_Freq",
            "MainMenu_View_SortTagsBy_Ascending",
            "MainMenu_View_SortTagsBy_Descending",
            "MainMenu_View_Filter_Expired",
            "MainMenu_View_Filter_ExpireSoon",
            "MainMenu_View_Filter_WeakPassw",
        ] {
            self.set_menu_bool(n, false);
        }
    }

    /// Check marks of the View menu (sort orders, filter).
    fn update_view_checks(&self) {
        let by = self.sort_by_idx.get();
        self.set_menu_bool("MainMenu_View_SortBy_Unsorted", by < 0);
        for i in 0..NUM_FIELDS {
            self.set_menu_bool(&format!("dyn:sort{i}"), by == i as i32);
        }
        let asc = self.sort_order_factor.get() > 0;
        self.set_menu_bool("MainMenu_View_SortBy_Ascending", asc);
        self.set_menu_bool("MainMenu_View_SortBy_Descending", !asc);
        let tby = self.tags_sort_by_idx.get();
        self.set_menu_bool("MainMenu_View_SortTagsBy_Name", tby == 0);
        self.set_menu_bool("MainMenu_View_SortTagsBy_Freq", tby == 1);
        let tasc = self.tags_sort_order_factor.get() > 0;
        self.set_menu_bool("MainMenu_View_SortTagsBy_Ascending", tasc);
        self.set_menu_bool("MainMenu_View_SortTagsBy_Descending", !tasc);
        let filt = self.filter.get().menu_item();
        for n in ["MainMenu_View_Filter_Expired", "MainMenu_View_Filter_ExpireSoon", "MainMenu_View_Filter_WeakPassw"] {
            self.set_menu_bool(n, filt == Some(n));
        }
    }

    /// Recently opened files in the File menu (`UpdateRecentFilesMenu()`).
    fn update_recent_files_menu(&self) {
        let items: Vec<(String, String, Option<u16>)> = self
            .recent_files
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, f)| (format!("recent{i}"), format!("&{}: {}", i + 1, helpers::shorten_file_name(f, 60).replace('&', "&&")), None))
            .collect();
        self.f().set_dynamic_items_before("MainMenu_File", Some("MainMenu_File_ClearRecentFiles"), &items);
        self.set_enabled("MainMenu_File_ClearRecentFiles", !self.recent_files.borrow().is_empty());
    }

    /// Rebuild the tag menu (`TagMenu`).
    fn update_tag_menu(&self, tags: &[String]) {
        let items: Vec<(String, String, Option<u16>)> =
            tags.iter().enumerate().map(|(i, t)| (format!("tag{i}"), t.replace('&', "&&"), None)).collect();
        self.f().set_dynamic_items_before("TagMenu", None, &items);
        *self.tag_menu_items.borrow_mut() = tags.to_vec();
    }

    // ------------------------------------------------------------------
    // configuration

    fn load_config(self: &Rc<Self>, ini: &IniFile) {
        let f = self.f();
        let tag_h = ini.read_int(CONFIG_ID, "TagListHeight", -1);
        if tag_h > 1 {
            let (x, y, w, _) = f.bounds("TagView");
            f.set_bounds("TagView", x, y, w, tag_h as i64);
        }
        let ctx = windows::ctx();
        fonts::set_named_font(&ctx, fonts::PM_LIST, FontSpec::parse(&ini.read_string(CONFIG_ID, "ListFont", "")));
        fonts::set_named_font(&ctx, fonts::PM_PASSW, FontSpec::parse(&ini.read_string(CONFIG_ID, "PasswBoxFont", "")));
        self.apply_fonts();
        let mask = ini.read_int(CONFIG_ID, "ShowColMask", 7) as u32;
        for i in 0..NUM_FIELDS {
            self.set_menu_bool(&format!("dyn:col{i}"), mask & (1 << i) != 0);
        }
        let sort_by = ini.read_int(CONFIG_ID, "SortByIdx", -1);
        if (-1..NUM_FIELDS as i32).contains(&sort_by) {
            self.sort_by_idx.set(sort_by);
        }
        let order = ini.read_int(CONFIG_ID, "SortOrder", 1);
        if order == 1 || order == -1 {
            self.sort_order_factor.set(order);
        }
        let tsort = ini.read_int(CONFIG_ID, "TagsSortByIdx", 0);
        if tsort == 0 || tsort == 1 {
            self.tags_sort_by_idx.set(tsort);
        }
        let torder = ini.read_int(CONFIG_ID, "TagsSortOrder", 1);
        if torder == 1 || torder == -1 {
            self.tags_sort_order_factor.set(torder);
        }
        self.tag_view.set_sort(self.tags_sort_by_idx.get(), self.tags_sort_order_factor.get());
        self.set_menu_bool("MainMenu_View_ShowPasswInList", ini.read_bool(CONFIG_ID, "ShowPasswInList", false));
        let widths = ini.read_string(CONFIG_ID, "ListColWidths", "");
        if !widths.is_empty() {
            let mut v = Vec::new();
            for s in widths.split(';').filter(|s| !s.is_empty()) {
                v.push(s.trim().parse::<i32>().unwrap_or(0).max(10) as f32);
                if v.len() == NUM_FIELDS {
                    break;
                }
            }
            *self.list_col_widths.borrow_mut() = v;
        }
        // the list ends with its last column (the splitter right of it);
        // a wider list would only show empty space
        let cols = {
            let w = self.list_col_widths.borrow();
            if w.is_empty() { self.list.column_widths() } else { w.clone() }
        };
        let bar = windows::ctx().global_style().spacing.scroll.allocated_width();
        let cols_width = ((cols.iter().sum::<f32>() + 2.0 + bar) / f.scale()).ceil() as i64;
        let saved = ini.read_int(CONFIG_ID, "ListWidth", -1);
        let list_width = if saved > 10 { (saved as i64).min(cols_width) } else { cols_width };
        if list_width > 10 {
            let (x, y, _, h) = f.bounds("DbView");
            f.set_bounds("DbView", x, y, list_width, h);
        }
        f.set_checked("ToggleNotesBtn", ini.read_bool(CONFIG_ID, "HideNotes", false));
        self.toggle_notes_btn_click();
        f.set_checked("PasswQualityBtn", ini.read_bool(CONFIG_ID, "EstimatePasswQuality", true));
        self.passw_quality_btn_click();
        self.set_menu_bool("SearchMenu_CaseSensitive", ini.read_bool(CONFIG_ID, "FindCaseSensitive", false));
        self.set_menu_bool("SearchMenu_FuzzySearch", ini.read_bool(CONFIG_ID, "FindFuzzy", false));
        if self.db_config().open_last_db_on_startup {
            *self.db_file_name.borrow_mut() = ini.read_string(CONFIG_ID, "LastDatabase", "");
        }
        let mut recent = parse_comma_text(&ini.read_string(CONFIG_ID, "RecentFiles", ""));
        recent.truncate(MAX_NUM_RECENT_FILES);
        *self.recent_files.borrow_mut() = recent;
        self.update_recent_files_menu();
        let h = ini.read_int(CONFIG_ID, "WindowHeight", -1);
        let w = ini.read_int(CONFIG_ID, "WindowWidth", -1);
        if w > 0 && h > 0 {
            self.size.set(egui::vec2(w as f32, h as f32).max(self.f().min_size()));
        }
        // room for the list, the splitter and the edit panel at its design
        // width (narrower, its edit boxes would shrink)
        let need = f.sc(f.bounds("DbView").2 + f.bounds("Splitter2").2 + f.bounds("EditPanel").2);
        let size = self.size.get();
        if size.x < need {
            self.size.set(egui::vec2(need.ceil(), size.y));
        }
        self.update_view_checks();
        self.dialogs.load_config(ini);
    }

    /// Database options (`g_config.Database`).
    fn db_config(&self) -> DatabaseConfig {
        self.main.upgrade().map(|m| m.core.borrow().config.database.clone()).unwrap_or_default()
    }

    /// `SaveConfig()` (also of the dialogs).
    pub fn save_config(&self, ini: &mut IniFile) {
        let size = windows::last_size(win_id()).unwrap_or(self.size.get());
        self.size.set(size);
        ini.write_integer(CONFIG_ID, "WindowHeight", size.y as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", size.x as i64);
        let f = self.f();
        ini.write_integer(CONFIG_ID, "ListWidth", f.bounds("DbView").2);
        if let Some(lf) = fonts::named_font(fonts::PM_LIST) {
            ini.write_string(CONFIG_ID, "ListFont", &lf.to_font_string());
        }
        ini.write_integer(CONFIG_ID, "TagListHeight", f.bounds("TagView").3);
        ini.write_string(CONFIG_ID, "PasswBoxFont", &fonts::named_font(fonts::PM_PASSW).map(|f| f.to_font_string()).unwrap_or_default());
        ini.write_integer(CONFIG_ID, "ShowColMask", self.show_col_mask.get() as i64);
        ini.write_bool(CONFIG_ID, "ShowPasswInList", self.menu_bool("MainMenu_View_ShowPasswInList"));
        let widths: String = self.list.column_widths().iter().map(|w| format!("{};", w.round() as i32)).collect();
        ini.write_string(CONFIG_ID, "ListColWidths", &widths);
        ini.write_bool(CONFIG_ID, "HideNotes", f.checked("ToggleNotesBtn"));
        ini.write_bool(CONFIG_ID, "EstimatePasswQuality", f.checked("PasswQualityBtn"));
        ini.write_bool(CONFIG_ID, "FindCaseSensitive", self.menu_bool("SearchMenu_CaseSensitive"));
        ini.write_bool(CONFIG_ID, "FindFuzzy", self.menu_bool("SearchMenu_FuzzySearch"));
        ini.write_integer(CONFIG_ID, "SortByIdx", self.sort_by_idx.get() as i64);
        ini.write_integer(CONFIG_ID, "SortOrder", self.sort_order_factor.get() as i64);
        ini.write_integer(CONFIG_ID, "TagsSortByIdx", self.tags_sort_by_idx.get() as i64);
        ini.write_integer(CONFIG_ID, "TagsSortOrder", self.tags_sort_order_factor.get() as i64);
        ini.write_string(CONFIG_ID, "LastDatabase", &self.db_file_name.borrow());
        ini.write_string(CONFIG_ID, "RecentFiles", &to_comma_text(&self.recent_files.borrow()));
        self.dialogs.save_config(ini);
    }

    fn apply_fonts(&self) {
        let font = fonts::named_size(fonts::PM_LIST).map(|s| egui::FontId::new(s, egui::FontFamily::Name(fonts::PM_LIST.into())));
        self.list.set_font(font);
        self.list.set_text_color((self.search_mode.get() != SEARCH_MODE_OFF).then_some(egui::Color32::from_rgb(0x20, 0x60, 0xe0)));
    }

    // ------------------------------------------------------------------
    // window

    fn is_shown(&self) -> bool {
        windows::is_open(win_id())
    }

    fn open_window(self: &Rc<Self>) {
        if self.is_shown() {
            windows::send_cmd(win_id(), ViewportCommand::Minimized(false));
            windows::focus(win_id());
            return;
        }
        let min = self.f().min_size().max(egui::vec2(500.0, 400.0));
        let content: Rc<dyn Content> = Rc::new(PmContent(self.clone()));
        let w = Rc::new(
            windows::Window::new(WINDOW_ID, PASSW_MANAGER_NAME, content, ViewportId::ROOT, self.size.get().max(min)).resizable(true, min),
        );
        let mw = Rc::downgrade(self);
        *w.on_close.borrow_mut() = Some(Box::new(move || {
            let Some(m) = mw.upgrade() else { return true };
            m.notify_user_action();
            if m.busy.get() {
                return false;
            }
            crate::runtime::spawn(async move {
                if m.close_database(false, 0).await {
                    // `FormClose`
                    m.f().set_items("SearchBox", Vec::new());
                    m.f().set_text("SearchBox", "");
                    if let Some(s) = windows::last_size(win_id()) {
                        m.size.set(s);
                    }
                    windows::close(win_id());
                }
            });
            false
        }));
        *self.window.borrow_mut() = Some(w.clone());
        windows::open(w);
        self.change_caption();
    }

    fn set_title(&self, t: &str) {
        if let Some(w) = self.window.borrow().as_ref() {
            *w.title.borrow_mut() = t.to_string();
        }
        windows::send_cmd(win_id(), ViewportCommand::Title(t.to_string()));
    }

    /// Minimize/restore and activation of the window (`OnWindowPosChanging`,
    /// `WMSize`, `FormActivate`), each frame.
    fn watch_viewport(self: &Rc<Self>, ui: &Ui) {
        let (min, foc) = ui.input(|i| (i.viewport().minimized.unwrap_or(false), i.viewport().focused.unwrap_or(false)));
        let (was_min, was_foc) = self.last_viewport.replace((min, foc));
        if min && !was_min {
            let autotyping = self.main.upgrade().is_some_and(|mw| mw.is_autotype_active());
            if self.is_db_open() && self.db_config().lock_minimize && !self.busy.get() && !autotyping {
                let m = self.clone();
                crate::runtime::spawn(async move { m.lock_or_unlock_database(false).await });
            }
        } else if !min && was_min && self.locked.get() {
            self.form_tag.set(FORM_TAG_UNLOCK_DB);
        }
        if foc && !was_foc {
            self.form_activate();
        }
    }

    // ------------------------------------------------------------------
    // public interface for the main window

    /// Show the window (`Show()` + `FormShow`/`FormActivate` on start-up:
    /// opens the last database if configured).
    pub fn show(self: &Rc<Self>) {
        let first = self.first_activation.get();
        if !self.is_db_open() && !self.locked.get() && self.db_config().open_last_db_on_startup && !self.db_file_name.borrow().is_empty() {
            self.form_tag.set(FORM_TAG_OPEN_DB);
        }
        let cmd_file = if first { self.mainw().core.borrow().cmd.passw_db_file_name.clone() } else { None };
        if cmd_file.is_some() && !self.is_db_open() {
            self.form_tag.set(FORM_TAG_OPEN_DB);
        }
        self.first_activation.set(false);
        self.open_window();
        if self.form_tag.get() == FORM_TAG_OPEN_DB {
            self.form_tag.set(0);
            let file = cmd_file.map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| self.db_file_name.borrow().clone());
            let m = self.clone();
            crate::runtime::spawn(async move {
                m.open_database(DB_OPEN_FLAG_EXISTING, &file).await;
            });
        }
    }

    pub fn is_db_open(&self) -> bool {
        self.db.borrow().as_ref().is_some_and(|d| d.is_open())
    }

    /// "Add to Database" of the main window, MP generator, and password list
    /// (`AddPassw(passw, blShowNumPassw, param)`): one entry per line.
    pub fn add_passw_to_db(self: &Rc<Self>, passw: &str, show_num_passw: bool, param: Option<&str>) -> usize {
        self.add_passw(passw, show_num_passw, param)
    }

    /// `CloseQuery()` of the main window: close the database (asking to
    /// save changes); `false` if the user cancelled.
    pub async fn close_query(self: &Rc<Self>) -> bool {
        self.close_database(false, 0).await
    }

    /// `NotifyUserAction()`.
    pub fn notify_user_action(&self) {
        self.last_user_action.set(Instant::now());
    }

    /// Lock the database when the main window is minimized
    /// (`LockMinimize` handling of the main window).
    pub fn lock_if_open(self: &Rc<Self>) {
        if self.is_db_open() && !self.locked.get() {
            let m = self.clone();
            crate::runtime::spawn(async move { m.lock_or_unlock_database(false).await });
        }
    }

    /// `SaveDbOnShutdown()`: save unsaved changes when the session ends
    /// (only with "LockAutoSave"); `false` if the database could not be
    /// saved.
    pub fn save_db_on_shutdown(self: &Rc<Self>) -> bool {
        if !self.is_db_open() {
            return true;
        }
        if self.item_changed.get() {
            return false;
        }
        if !self.db_changed.get() {
            return true;
        }
        if !self.db_config().lock_auto_save {
            return false;
        }
        let file = self.db_file_name.borrow().clone();
        let save_name = if file.is_empty() {
            format!("Untitled_{}{}", helpers::time_stamp_string(false), PWDB_FILE_EXT)
        } else if !self.db_read_only.get() {
            file
        } else {
            let p = std::path::Path::new(&file);
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            p.with_file_name(format!("{stem}_session_{}{ext}", helpers::time_stamp_string(false))).to_string_lossy().to_string()
        };
        self.save_database(&save_name)
    }

    /// Search the database for the keyword contained in the title of the
    /// active window (`SearchDbForKeyword()`); with `autotype` the first
    /// matching entry is typed.
    pub fn search_db_for_keyword(self: &Rc<Self>, autotype: bool) {
        if !self.is_db_open() || self.item_changed.get() {
            return;
        }
        let m = self.clone();
        crate::runtime::spawn(async move {
            let Some(main) = m.main.upgrade() else { return };
            let title = pwtool_app::platform::active_window::active_window_title().await;
            let Some(title) = title else {
                let text = trl("This function is not available on this system.");
                if autotype {
                    main.show_tray_info(&text, true);
                } else {
                    m.show();
                    super::dialogs::msg(win_id(), &text, MsgIcon::Warning);
                }
                return;
            };
            let title_lc = helpers::lower(&title);
            let mut found: Option<usize> = None;
            {
                let mut dbb = m.db.borrow_mut();
                let Some(db) = dbb.as_mut() else { return };
                for i in 0..db.len() {
                    if !autotype {
                        db.entry_mut(i).user_flags &= !DB_FLAG_FOUND;
                    }
                    let kw = db.entry(i).string(FieldType::Keyword).to_string();
                    if !kw.is_empty() && title_lc.contains(helpers::lower(&kw).as_str()) {
                        if found.is_none() {
                            found = Some(i);
                        }
                        if autotype {
                            break;
                        }
                        db.entry_mut(i).user_flags |= DB_FLAG_FOUND;
                    }
                }
            }
            let not_found = trl_format("No matching keyword found for window text\n\"%1\".", &[&title]);
            if autotype {
                match found {
                    Some(i) => m.perform_autotype_for_inner(i, false),
                    None => main.show_tray_info(&not_found, true),
                }
                return;
            }
            if let Some(i) = found {
                m.search_mode.set(SEARCH_MODE_NORMAL);
                m.apply_fonts();
                m.reset_list_view(RELOAD_TAGS);
                m.reset_nav_controls();
                let kw = m.db.borrow().as_ref().map(|db| db.entry(i).string(FieldType::Keyword).to_string()).unwrap_or_default();
                m.f().set_text("SearchBox", &kw);
                m.f().set_control_caption("SearchResultPanel", &trl("Search via hot key."));
                m.update_menu_states();
            }
            m.show();
            if found.is_none() {
                super::dialogs::msg(win_id(), &not_found, MsgIcon::Warning);
            }
        });
    }

    /// `MainMenu_Edit_PerformAutotypeClick()`: autotype sequence of the
    /// selected entry (key-value "Autotype" or the default sequence).
    fn perform_autotype_for(self: &Rc<Self>, index: usize) {
        let Some(main) = self.main.upgrade() else { return };
        let minimize = main.core.borrow().config.minimize_autotype;
        self.perform_autotype_for_inner(index, minimize);
    }

    fn perform_autotype_for_inner(self: &Rc<Self>, index: usize, minimize: bool) {
        let Some(main) = self.main.upgrade() else { return };
        let delay = main.core.borrow().config.autotype_delay.clamp(0, 10000) as u32;
        let default_seq = self.db_config().default_autotype_sequence;
        let strokes = {
            let db = self.db.borrow();
            let Some(db) = db.as_ref() else { return };
            if index >= db.len() {
                return;
            }
            let e = db.entry(index);
            let seq = e.key_value(DB_KEYVAL_KEYS[DB_KEYVAL_AUTOTYPE]).map(String::from).unwrap_or(default_seq);
            let field = |f: usize| -> Option<Zeroizing<String>> {
                if f == FieldType::Password as usize { db.get_entry_passw(index).ok() } else { Some(Zeroizing::new(e.strings[f].to_string())) }
            };
            pwtool_app::autotype::build_sequence(&seq, Some(&field), None, None, delay)
        };
        main.send_keys(strokes, minimize);
    }
}

fn file_name_of(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

/// Tag kinds for the automation output.
fn tag_name(k: &TagKind) -> String {
    match k {
        TagKind::All => "All".to_string(),
        TagKind::Tag(t) => t.clone(),
        TagKind::Untagged => "Untagged".to_string(),
    }
}

