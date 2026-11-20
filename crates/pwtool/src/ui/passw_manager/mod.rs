//! PassCube password manager (`upstream/src/main/PasswManager.*` and the
//! dialogs `PasswMng*`).
//!
//! Differences to the original (deliberate):
//! - the main menu items "Rearrange Entry" call the function matching their
//!   caption (the original form connects "Move to Top"/"Move Up by
//!   One"/"Move Down by One" to the wrong handlers);
//! - the tag menu inserts the tag of the clicked item even in search mode
//!   (the original indexes the wrong list there);
//! - the password font is saved under the key it is read from
//!   (`PasswBoxFont`; the original writes `PasswFont`);
//! - window positions are not saved (not possible on Wayland), only sizes;
//! - autotype and "search database for keyword" (window title of the
//!   foreground window) are not available yet.

#![allow(deprecated)]

pub mod automation;
mod col_select;
mod db_prop;
mod db_settings;
mod helpers;
mod key_val_edit;
mod list;
mod main_hooks;
mod ops;
mod pw_history;
mod table;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::{Rc, Weak};
use std::time::Instant;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gio, glib};
use pwtool_core::config::{IniFile, parse_comma_text, to_comma_text};
use pwtool_core::language::trl;
use pwtool_core::passw::database::{NUM_FIELDS, PasswDatabase, PasswHistory};
use zeroize::Zeroizing;

use helpers::{DatePicker, DynCss};
use list::{DbList, RowKey, TagList};

use super::builder::{Built, build_form_with_prefix, mnemonic};
use super::form::load_form;
use super::main_window::MainWindow;
use super::passw_enter::PasswEnterDlg;
use super::quality_bar::QualityBar;

const CONFIG_ID: &str = "PasswManager";
const PASSW_MANAGER_NAME: &str = "PassCube";
const PWDB_FILE_EXT: &str = ".pwdb";
const PASSWORD_CHAR: char = '*';
const HIDE_PASSW_STR: &str = "********";
const ACTION_PREFIX: &str = "pm";

const DB_NUM_KEYVAL_KEYS: usize = 4;
const DB_KEYVAL_KEYS: [&str; DB_NUM_KEYVAL_KEYS] = ["Autotype", "Run", "Profile", "FormatPW"];
const DB_KEYVAL_UI_KEYS: [&str; DB_NUM_KEYVAL_KEYS] = ["Autotype", "Run", "Profile", "Format password"];
#[allow(dead_code)] // used by autotype
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

/// `g_config.Database` (owned by `AppCore`, edited in the configuration
/// dialog).
pub use crate::app_config::DatabaseConfig;

/// Filter of the entry list (View | Filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterType {
    None,
    Expired,
    ExpireSoon,
    WeakPassw,
}

impl FilterType {
    fn from_id(s: &str) -> Self {
        match s {
            "expired" => FilterType::Expired,
            "soon" => FilterType::ExpireSoon,
            "weak" => FilterType::WeakPassw,
            _ => FilterType::None,
        }
    }
}

/// The password manager window (`TPasswMngForm`).
pub struct PasswManager {
    main: Weak<MainWindow>,
    pub win: gtk::ApplicationWindow,
    b: Built,
    main_menu: gio::Menu,
    db: RefCell<Option<PasswDatabase>>,
    db_file_name: RefCell<String>,
    recent_files: RefCell<Vec<String>>,
    db_read_only: Cell<bool>,
    /// `m_pSelectedItem`: `None` = no item, else the key of the item.
    selected: Cell<Option<RowKey>>,
    show_col_mask: Cell<u32>,
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
    list_col_widths: RefCell<Vec<i32>>,
    ui_field_names: [String; NUM_FIELDS],
    /// Key → displayed (translated) name, ordered by key (`m_keyValNames`).
    key_val_names: BTreeMap<String, String>,
    /// Tags shown in the tag list with their frequency (`m_tags`).
    tags: RefCell<Vec<(String, u32)>>,
    global_casei_tags: RefCell<BTreeMap<String, String>>,
    tag_filter: RefCell<BTreeSet<String>>,
    temp_key_val: RefCell<Option<Vec<(Zeroizing<String>, Zeroizing<String>)>>>,
    temp_passw_history: RefCell<Option<PasswHistory>>,
    temp_notes: RefCell<Option<Zeroizing<String>>>,
    notes_hidden_text: String,
    notes_tag: Cell<i32>,
    /// `Tag` of the form (`FORM_TAG_*`).
    form_tag: Cell<i32>,
    /// Set while the edit panel is filled programmatically
    /// (`FORM_TAG_ITEM_SELECTED`).
    filling: Cell<bool>,
    passw_box_gen: Cell<bool>,
    suppress_expiry: Cell<bool>,
    filter: Cell<FilterType>,
    csv_col_mask: Cell<u32>,
    idle_suspend: Cell<i32>,
    idle_timer_enabled: Cell<bool>,
    busy: Cell<bool>,
    first_activation: Cell<bool>,
    unlock_tick: Cell<Option<Instant>>,
    apply_pending: RefCell<Option<glib::SourceId>>,
    tag_apply_pending: RefCell<Option<glib::SourceId>>,
    inhibit_cookie: Cell<u32>,
    /// Stand-in for `AppCore.config.database` (see [`Self::db_config`]).
    list: Rc<DbList>,
    tag_view: Rc<TagList>,
    passw_bar: QualityBar,
    expiry_picker: Rc<DatePicker>,
    list_font: RefCell<Option<gtk::pango::FontDescription>>,
    passw_font: RefCell<Option<gtk::pango::FontDescription>>,
    passw_css: DynCss,
    search_label: gtk::Label,
    filter_panel: gtk::Widget,
    tag_menu: gio::Menu,
    tag_popover: gtk::PopoverMenu,
    expiry_popover: gtk::PopoverMenu,
    db_view_popover: gtk::PopoverMenu,
    hpaned: gtk::Paned,
    vpaned: gtk::Paned,
    passw_enter: RefCell<Option<Rc<PasswEnterDlg>>>,
    col_dlg: RefCell<Option<Rc<col_select::ColSelectDlg>>>,
    prop_dlg: RefCell<Option<Rc<db_prop::DbPropDlg>>>,
    settings_dlg: RefCell<Option<Rc<db_settings::DbSettingsDlg>>>,
    key_val_dlg: RefCell<Option<Rc<key_val_edit::KeyValDlg>>>,
    history_dlg: RefCell<Option<Rc<pw_history::PwHistoryDlg>>>,
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

/// Find a menu item by its action (`prefix.name`) or submenu name; returns
/// the containing menu and the position.
fn find_menu_item(menu: &gio::Menu, name: &str) -> Option<(gio::Menu, i32)> {
    let action = format!("{ACTION_PREFIX}.{name}");
    for i in 0..menu.n_items() {
        let a = menu.item_attribute_value(i, "action", None).and_then(|v| v.get::<String>());
        let n = menu.item_attribute_value(i, "pwtool-name", None).and_then(|v| v.get::<String>());
        if a.as_deref() == Some(action.as_str()) || n.as_deref() == Some(name) {
            return Some((menu.clone(), i));
        }
        for link in ["section", "submenu"] {
            if let Some(sub) = menu.item_link(i, link).and_downcast::<gio::Menu>() {
                if let Some(r) = find_menu_item(&sub, name) {
                    return Some(r);
                }
            }
        }
    }
    None
}

/// Replace an item by a submenu (keeping its label).
fn replace_with_submenu(menu: &gio::Menu, name: &str, sub: &gio::Menu) {
    if let Some((m, i)) = find_menu_item(menu, name) {
        let label = m.item_attribute_value(i, "label", None).and_then(|v| v.get::<String>()).unwrap_or_default();
        m.remove(i);
        let mi = gio::MenuItem::new_submenu(Some(&label), sub);
        mi.set_attribute_value("pwtool-name", Some(&name.to_variant()));
        m.insert_item(i, &mi);
    }
}

/// Change the label of a menu item.
fn set_menu_label(menu: &gio::Menu, name: &str, label: &str) {
    if let Some((m, i)) = find_menu_item(menu, name) {
        let item = gio::MenuItem::from_model(&m, i);
        item.set_label(Some(&mnemonic(label)));
        m.remove(i);
        m.insert_item(i, &item);
    }
}

impl PasswManager {
    pub fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let b = build_form_with_prefix(load_form("PasswManager"), "PasswManager", ACTION_PREFIX);
        let app = main.window.application().expect("application");
        let win = gtk::ApplicationWindow::new(&app);
        win.set_title(Some(PASSW_MANAGER_NAME));
        win.set_hide_on_close(false);

        // --- layout: tool bar, tag list, splitters, entry list, edit panel
        let root = b.root.clone();
        let toolbar: gtk::Widget = b.w("ToolBar");
        let tag_sw: gtk::ScrolledWindow = b.get("TagView");
        let db_sw: gtk::ScrolledWindow = b.get("DbView");
        let edit_panel = b.boxes.get("EditPanel").cloned().expect("EditPanel");
        for name in ["ToolBar", "TagView", "DbView", "EditPanel", "Splitter1", "Splitter2"] {
            root.remove(&b.w(name));
        }
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let main_menu = b.menus.get("MainMenu").cloned().unwrap_or_default();
        let menubar = gtk::PopoverMenuBar::from_model(Some(&main_menu));
        vbox.append(&menubar);
        toolbar.set_margin_start(2);
        toolbar.set_margin_top(2);
        toolbar.set_margin_bottom(2);
        vbox.append(&toolbar);
        let hpaned = gtk::Paned::new(gtk::Orientation::Horizontal);
        hpaned.set_start_child(Some(&db_sw));
        hpaned.set_end_child(Some(&edit_panel));
        hpaned.set_resize_start_child(true);
        hpaned.set_resize_end_child(true);
        hpaned.set_shrink_start_child(false);
        hpaned.set_shrink_end_child(false);
        let vpaned = gtk::Paned::new(gtk::Orientation::Vertical);
        vpaned.set_start_child(Some(&tag_sw));
        vpaned.set_end_child(Some(&hpaned));
        vpaned.set_resize_start_child(false);
        vpaned.set_shrink_start_child(false);
        vpaned.set_vexpand(true);
        vbox.append(&vpaned);
        win.set_child(Some(&vbox));
        // the notes box (anchored on all sides) may shrink
        let (ew, eh) = edit_panel.design_size();
        edit_panel.set_min_size(ew, eh - b.sc(70));
        db_sw.set_size_request(b.sc(150), -1);
        tag_sw.set_size_request(-1, b.sc(24));
        tag_sw.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        // natural height: menu and tool bar are higher than in the original
        win.set_default_size(b.client.0, -1);
        win.set_size_request(b.min.0.max(b.sc(500)), b.min.1.max(b.sc(400)));
        win.insert_action_group(ACTION_PREFIX, Some(&b.actions));

        // --- lists
        let img16 = |i: i64| b.image("ImageList16", i);
        let icon16 = b.image_size("ImageList16");
        let list = DbList::new([img16(9), img16(10)], icon16);
        db_sw.set_child(Some(&list.view));
        let tag_view = TagList::new([img16(6), img16(7), img16(8)], icon16);
        tag_sw.set_child(Some(&tag_view.flow));

        let passw_bar = QualityBar::new(b.get("PasswGauge"), 128);
        let expiry_picker = DatePicker::new(b.get("ExpiryDatePicker"), b.get("ExpiryDatePicker.calendar"));
        let passw_box: gtk::Entry = b.get("PasswBox");
        passw_box.set_buffer(&gtk::PasswordEntryBuffer::new().upcast::<gtk::EntryBuffer>());
        passw_box.set_invisible_char(Some(PASSWORD_CHAR));
        let passw_css = DynCss::new(&passw_box);
        passw_css.set("font-family: monospace;");
        if let Some(tv) = b.try_get::<gtk::TextView>("NotesBox.view") {
            tv.set_wrap_mode(gtk::WrapMode::WordChar);
        }

        // search result / filter info panels
        let search_label = gtk::Label::new(None);
        search_label.set_xalign(0.0);
        search_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        if let Some(p) = b.boxes.get("SearchResultPanel") {
            let (w, h) = p.design_size();
            p.put(&search_label, b.sc(4), 0, w - b.sc(4), h, 15, super::layout::Align::None);
        }
        let filter_panel: gtk::Widget = b.w("FilterInfoPanel");
        if let Some(p) = b.boxes.get("FilterInfoPanel") {
            let l = gtk::Label::new(Some(&trl("Filtered")));
            l.set_xalign(0.0);
            super::builder::add_css(&l, "color: blue;");
            let (_, h) = p.design_size();
            p.put(&l, 0, 0, b.sc(70), h, 3, super::layout::Align::None);
        }
        if let Some(btn) = b.try_get::<gtk::Button>("ClearFilterBtn") {
            btn.set_label("✕");
        }
        if let Some(btn) = b.try_get::<gtk::Button>("ClearSearchBtn") {
            btn.add_css_class("flat");
        }
        if let Some(cb) = b.try_get::<gtk::ComboBoxText>("SearchBox") {
            if let Some(e) = super::util::combo_entry(&cb) {
                e.set_placeholder_text(Some(&trl("Search...")));
                e.set_width_chars(12);
                e.set_max_width_chars(12);
            }
            cb.set_hexpand(false);
        }
        if let Some(p) = b.try_get::<gtk::Widget>("SearchResultPanel") {
            p.set_hexpand(false);
        }

        // drop-down part of the search button (`tbsDropDown`)
        if let (Some(sb), Some(menu)) = (b.try_get::<gtk::Widget>("SearchBtn"), b.menus.get("SearchMenu")) {
            if let Some(bx) = sb.parent().and_downcast::<gtk::Box>() {
                let mb = gtk::MenuButton::new();
                mb.set_menu_model(Some(menu));
                mb.add_css_class("flat");
                bx.insert_child_after(&mb, Some(&sb));
            }
        }

        let tag_menu = gio::Menu::new();
        let tag_popover = gtk::PopoverMenu::from_model(Some(&tag_menu));
        tag_popover.set_parent(&b.w("AddTagBtn"));
        let expiry_menu = gio::Menu::new();
        for d in STD_EXPIRY_DAYS {
            let mi = gio::MenuItem::new(
                Some(&pwtool_core::language::trl_format("%1 days", &[&d.to_string()])),
                None,
            );
            mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.expirydays")), Some(&d.to_variant()));
            expiry_menu.append_item(&mi);
        }
        let expiry_popover = gtk::PopoverMenu::from_model(Some(&expiry_menu));
        expiry_popover.set_parent(&b.w("ExpiryBtn"));
        let db_view_popover =
            gtk::PopoverMenu::from_model(Some(b.menus.get("DbViewMenu").cloned().unwrap_or_default().upcast_ref::<gio::MenuModel>()));
        db_view_popover.set_parent(&list.view);
        db_view_popover.set_has_arrow(false);

        // key names (`m_keyValNames`), translated if unique
        let mut key_val_names: BTreeMap<String, String> = DB_KEYVAL_KEYS
            .iter()
            .zip(DB_KEYVAL_UI_KEYS.iter())
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        if pwtool_core::language::has_translation() {
            let transl: BTreeMap<String, String> = key_val_names.iter().map(|(k, v)| (k.clone(), trl(v))).collect();
            let uniq: BTreeSet<&String> = transl.values().collect();
            if uniq.len() == transl.len() {
                key_val_names = transl;
            } else {
                super::util::msg(
                    Some(main.window.upcast_ref()),
                    "Password manager: Translations of key names not unique!",
                    super::util::MsgIcon::Warning,
                );
            }
        }

        let m = Rc::new(PasswManager {
            main: Rc::downgrade(main),
            win,
            main_menu,
            db: RefCell::new(None),
            db_file_name: RefCell::new(String::new()),
            recent_files: RefCell::new(Vec::new()),
            db_read_only: Cell::new(false),
            selected: Cell::new(None),
            show_col_mask: Cell::new(7),
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
            global_casei_tags: RefCell::new(BTreeMap::new()),
            tag_filter: RefCell::new(BTreeSet::new()),
            temp_key_val: RefCell::new(None),
            temp_passw_history: RefCell::new(None),
            temp_notes: RefCell::new(None),
            notes_hidden_text: format!("[{}]", trl("Notes hidden - click here to display")),
            notes_tag: Cell::new(0),
            form_tag: Cell::new(0),
            filling: Cell::new(false),
            passw_box_gen: Cell::new(false),
            suppress_expiry: Cell::new(false),
            filter: Cell::new(FilterType::None),
            csv_col_mask: Cell::new(u32::MAX >> 1),
            idle_suspend: Cell::new(0),
            idle_timer_enabled: Cell::new(false),
            busy: Cell::new(false),
            first_activation: Cell::new(true),
            unlock_tick: Cell::new(None),
            apply_pending: RefCell::new(None),
            tag_apply_pending: RefCell::new(None),
            inhibit_cookie: Cell::new(0),
            list,
            tag_view,
            passw_bar,
            expiry_picker,
            list_font: RefCell::new(None),
            passw_font: RefCell::new(None),
            passw_css,
            search_label,
            filter_panel,
            tag_menu,
            tag_popover,
            expiry_popover,
            db_view_popover,
            hpaned,
            vpaned,
            passw_enter: RefCell::new(None),
            col_dlg: RefCell::new(None),
            prop_dlg: RefCell::new(None),
            settings_dlg: RefCell::new(None),
            key_val_dlg: RefCell::new(None),
            history_dlg: RefCell::new(None),
            b,
        });
        m.setup_menus(&app);
        m.connect_signals();
        {
            let main = m.main.upgrade().unwrap();
            let core = main.core.borrow();
            m.load_config(&core.ini);
        }
        // `AddModifyBtn->Left = PrevBtn->Left; CancelBtn->Left = NextBtn->Left`
        if let Some(ep) = m.b.boxes.get("EditPanel") {
            if let (Some(p), Some(a)) = (ep.child_rect(&m.b.w("PrevBtn")), ep.child_rect(&m.b.w("AddModifyBtn"))) {
                ep.move_child(&m.b.w("AddModifyBtn"), p.0, a.1, a.2, a.3);
            }
            if let (Some(n), Some(c)) = (ep.child_rect(&m.b.w("NextBtn")), ep.child_rect(&m.b.w("CancelBtn"))) {
                ep.move_child(&m.b.w("CancelBtn"), n.0, c.1, c.2, c.3);
            }
        }
        m.set_show_col_mask_from_actions();
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

    fn gwin(&self) -> gtk::Window {
        self.win.clone().upcast()
    }

    fn entry(&self, name: &str) -> gtk::Entry {
        self.b.get(name)
    }

    fn label(&self, name: &str) -> gtk::Label {
        self.b.get(name)
    }

    fn notes_view(&self) -> gtk::TextView {
        self.b.get("NotesBox.view")
    }

    fn toggle(&self, name: &str) -> gtk::ToggleButton {
        self.b.get(name)
    }

    fn action(&self, name: &str) -> Option<gio::SimpleAction> {
        self.b.actions.lookup_action(name).and_downcast::<gio::SimpleAction>()
    }

    fn set_enabled(&self, name: &str, on: bool) {
        if let Some(a) = self.action(name) {
            a.set_enabled(on);
        }
    }

    fn action_bool(&self, name: &str) -> bool {
        self.action(name).and_then(|a| a.state()).and_then(|s| s.get::<bool>()).unwrap_or(false)
    }

    fn set_action_bool(&self, name: &str, v: bool) {
        if let Some(a) = self.action(name) {
            a.set_state(&v.to_variant());
        }
    }

    /// Password entry dialog shared with the main window (if set), else an
    /// own instance.
    fn passw_enter(&self) -> Rc<PasswEnterDlg> {
        if let Some(d) = self.passw_enter.borrow().as_ref() {
            return d.clone();
        }
        let main = self.mainw();
        let d = PasswEnterDlg::new(&self.gwin(), &main.core.borrow().ini);
        d.use_advanced_passw_est.set(main.use_advanced_passw_est());
        *self.passw_enter.borrow_mut() = Some(d.clone());
        d
    }

    /// Use the main window's password dialog (`PasswEnterDlg` is a single
    /// instance in the original).
    pub fn set_passw_enter_dlg(&self, dlg: Rc<PasswEnterDlg>) {
        *self.passw_enter.borrow_mut() = Some(dlg);
    }

    fn col_dlg(&self) -> Rc<col_select::ColSelectDlg> {
        self.col_dlg.borrow_mut().get_or_insert_with(|| col_select::ColSelectDlg::new(&self.gwin())).clone()
    }

    fn prop_dlg(&self) -> Rc<db_prop::DbPropDlg> {
        if let Some(d) = self.prop_dlg.borrow().as_ref() {
            return d.clone();
        }
        let d = db_prop::DbPropDlg::new(&self.gwin(), &self.mainw().core.borrow().ini);
        *self.prop_dlg.borrow_mut() = Some(d.clone());
        d
    }

    fn settings_dlg(&self) -> Rc<db_settings::DbSettingsDlg> {
        if let Some(d) = self.settings_dlg.borrow().as_ref() {
            return d.clone();
        }
        let d = db_settings::DbSettingsDlg::new(&self.gwin(), &self.mainw().core.borrow().ini);
        *self.settings_dlg.borrow_mut() = Some(d.clone());
        d
    }

    fn key_val_dlg(&self) -> Rc<key_val_edit::KeyValDlg> {
        if let Some(d) = self.key_val_dlg.borrow().as_ref() {
            return d.clone();
        }
        let names: Vec<String> = self.key_val_names.values().cloned().collect();
        let d = key_val_edit::KeyValDlg::new(&self.gwin(), &self.mainw().core.borrow().ini, &names);
        *self.key_val_dlg.borrow_mut() = Some(d.clone());
        d
    }

    fn history_dlg(&self) -> Rc<pw_history::PwHistoryDlg> {
        if let Some(d) = self.history_dlg.borrow().as_ref() {
            return d.clone();
        }
        let d = pw_history::PwHistoryDlg::new(&self.gwin(), &self.mainw().core.borrow().ini);
        *self.history_dlg.borrow_mut() = Some(d.clone());
        d
    }

    // ------------------------------------------------------------------
    // menus

    fn setup_menus(self: &Rc<Self>, app: &gtk::Application) {
        let actions = &self.b.actions;
        // main menu accelerators (application-wide, only for actions of this
        // window); the shortcuts of the list's context menu work only in
        // the list
        for (action, accel) in &self.b.accels {
            if action.starts_with(&format!("{ACTION_PREFIX}.MainMenu_")) {
                app.set_accels_for_action(action, &[accel.as_str()]);
            }
        }
        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Local);
        for (action, accel) in &self.b.accels {
            if action.starts_with(&format!("{ACTION_PREFIX}.DbViewMenu_")) {
                if let Some(t) = gtk::ShortcutTrigger::parse_string(accel) {
                    sc.add_shortcut(gtk::Shortcut::new(Some(t), Some(gtk::NamedAction::new(action))));
                }
            }
        }
        self.list.view.add_controller(sc);

        // View | Show Columns
        let cols = gio::Menu::new();
        for (i, name) in self.ui_field_names.iter().enumerate() {
            let a = gio::SimpleAction::new_stateful(&format!("col{i}"), None, &false.to_variant());
            actions.add_action(&a);
            cols.append(Some(&mnemonic(name)), Some(&format!("{ACTION_PREFIX}.col{i}")));
        }
        replace_with_submenu(&self.main_menu, "MainMenu_View_ShowColumns", &cols);

        // View | Sort Entries By
        let sort = gio::Menu::new();
        let s1 = gio::Menu::new();
        let mi = gio::MenuItem::new(Some(&mnemonic(&trl("Unsorted"))), None);
        mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.sortby")), Some(&(-1i32).to_variant()));
        s1.append_item(&mi);
        sort.append_section(None, &s1);
        let s2 = gio::Menu::new();
        for (i, name) in self.ui_field_names.iter().enumerate() {
            let mi = gio::MenuItem::new(Some(&mnemonic(name)), None);
            mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.sortby")), Some(&(i as i32).to_variant()));
            s2.append_item(&mi);
        }
        sort.append_section(None, &s2);
        let s3 = gio::Menu::new();
        for (label, v) in [("Ascending", 1i32), ("Descending", -1)] {
            let mi = gio::MenuItem::new(Some(&mnemonic(&trl(label))), None);
            mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.sortorder")), Some(&v.to_variant()));
            s3.append_item(&mi);
        }
        sort.append_section(None, &s3);
        replace_with_submenu(&self.main_menu, "MainMenu_View_SortBy", &sort);
        let a = gio::SimpleAction::new_stateful("sortby", Some(glib::VariantTy::INT32), &(-1i32).to_variant());
        actions.add_action(&a);
        let a = gio::SimpleAction::new_stateful("sortorder", Some(glib::VariantTy::INT32), &1i32.to_variant());
        actions.add_action(&a);

        // View | Sort Tags By
        let tsort = gio::Menu::new();
        let t1 = gio::Menu::new();
        for (label, v) in [("Name", 0i32), ("Frequency", 1)] {
            let mi = gio::MenuItem::new(Some(&mnemonic(&trl(label))), None);
            mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.tagsortby")), Some(&v.to_variant()));
            t1.append_item(&mi);
        }
        tsort.append_section(None, &t1);
        let t2 = gio::Menu::new();
        for (label, v) in [("Ascending", 1i32), ("Descending", -1)] {
            let mi = gio::MenuItem::new(Some(&mnemonic(&trl(label))), None);
            mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.tagsortorder")), Some(&v.to_variant()));
            t2.append_item(&mi);
        }
        tsort.append_section(None, &t2);
        replace_with_submenu(&self.main_menu, "MainMenu_View_SortTagsBy", &tsort);
        let a = gio::SimpleAction::new_stateful("tagsortby", Some(glib::VariantTy::INT32), &0i32.to_variant());
        actions.add_action(&a);
        let a = gio::SimpleAction::new_stateful("tagsortorder", Some(glib::VariantTy::INT32), &1i32.to_variant());
        actions.add_action(&a);

        // View | Filter
        let filt = gio::Menu::new();
        for (label, id) in [
            ("Expired Entries", "expired"),
            ("Entries that Will Expire Soon", "soon"),
            ("Entries with Weak Passwords", "weak"),
        ] {
            let mi = gio::MenuItem::new(Some(&mnemonic(&trl(label))), None);
            mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.filter")), Some(&id.to_variant()));
            filt.append_item(&mi);
        }
        replace_with_submenu(&self.main_menu, "MainMenu_View_Filter", &filt);
        let a = gio::SimpleAction::new_stateful("filter", Some(glib::VariantTy::STRING), &"".to_variant());
        actions.add_action(&a);

        // dynamic menus: recent files, tags, expiry days
        let a = gio::SimpleAction::new("recent", Some(glib::VariantTy::INT32));
        actions.add_action(&a);
        let a = gio::SimpleAction::new("addtag", Some(glib::VariantTy::STRING));
        actions.add_action(&a);
        let a = gio::SimpleAction::new("expirydays", Some(glib::VariantTy::INT32));
        actions.add_action(&a);
    }

    /// Rebuild the recently opened files in the File menu
    /// (`UpdateRecentFilesMenu()`).
    fn update_recent_files_menu(&self) {
        if let Some((m, _)) = find_menu_item(&self.main_menu, "MainMenu_File_ClearRecentFiles") {
            let clear_item = (0..m.n_items())
                .find(|&i| {
                    m.item_attribute_value(i, "action", None).and_then(|v| v.get::<String>()).as_deref()
                        == Some(&format!("{ACTION_PREFIX}.MainMenu_File_ClearRecentFiles"))
                })
                .map(|i| gio::MenuItem::from_model(&m, i));
            m.remove_all();
            for (i, f) in self.recent_files.borrow().iter().enumerate() {
                let label = format!("{}: {}", i + 1, helpers::shorten_file_name(f, 60)).replace('_', "__");
                let mi = gio::MenuItem::new(Some(&label), None);
                mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.recent")), Some(&(i as i32).to_variant()));
                m.append_item(&mi);
            }
            if let Some(ci) = clear_item {
                m.append_item(&ci);
            }
        }
        self.set_enabled("MainMenu_File_ClearRecentFiles", !self.recent_files.borrow().is_empty());
    }

    /// Rebuild the tag menu (`TagMenu`).
    fn update_tag_menu(&self, tags: &[String]) {
        self.tag_menu.remove_all();
        for t in tags {
            let mi = gio::MenuItem::new(Some(&t.replace('_', "__")), None);
            mi.set_action_and_target_value(Some(&format!("{ACTION_PREFIX}.addtag")), Some(&t.to_variant()));
            self.tag_menu.append_item(&mi);
        }
    }

    // ------------------------------------------------------------------
    // configuration

    fn load_config(self: &Rc<Self>, ini: &IniFile) {
        let list_width = ini.read_int(CONFIG_ID, "ListWidth", -1);
        self.hpaned.set_position(if list_width > 10 { list_width } else { self.b.sc(375) });
        let font = ini.read_string(CONFIG_ID, "ListFont", "");
        if let Some(f) = helpers::string_to_font(&font) {
            *self.list_font.borrow_mut() = Some(f);
        }
        let tag_h = ini.read_int(CONFIG_ID, "TagListHeight", -1);
        self.vpaned.set_position(if tag_h > 1 { tag_h } else { self.b.sc(42) });
        let pf = ini.read_string(CONFIG_ID, "PasswBoxFont", "");
        if let Some(f) = helpers::string_to_font(&pf) {
            *self.passw_font.borrow_mut() = Some(f);
        }
        self.apply_fonts();
        let mask = ini.read_int(CONFIG_ID, "ShowColMask", 7) as u32;
        for i in 0..NUM_FIELDS {
            self.set_action_bool(&format!("col{i}"), mask & (1 << i) != 0);
        }
        let sort_by = ini.read_int(CONFIG_ID, "SortByIdx", -1);
        if (-1..NUM_FIELDS as i32).contains(&sort_by) {
            self.sort_by_idx.set(sort_by);
            if let Some(a) = self.action("sortby") {
                a.set_state(&sort_by.to_variant());
            }
        }
        let order = ini.read_int(CONFIG_ID, "SortOrder", 1);
        if order == 1 || order == -1 {
            self.sort_order_factor.set(order);
            if let Some(a) = self.action("sortorder") {
                a.set_state(&order.to_variant());
            }
        }
        let tsort = ini.read_int(CONFIG_ID, "TagsSortByIdx", 0);
        if tsort == 0 || tsort == 1 {
            self.tags_sort_by_idx.set(tsort);
            if let Some(a) = self.action("tagsortby") {
                a.set_state(&tsort.to_variant());
            }
        }
        let torder = ini.read_int(CONFIG_ID, "TagsSortOrder", 1);
        if torder == 1 || torder == -1 {
            self.tags_sort_order_factor.set(torder);
            if let Some(a) = self.action("tagsortorder") {
                a.set_state(&torder.to_variant());
            }
        }
        self.tag_view.set_sort(self.tags_sort_by_idx.get(), self.tags_sort_order_factor.get());
        self.set_action_bool("MainMenu_View_ShowPasswInList", ini.read_bool(CONFIG_ID, "ShowPasswInList", false));
        let widths = ini.read_string(CONFIG_ID, "ListColWidths", "");
        if !widths.is_empty() {
            let mut v = Vec::new();
            for s in widths.split(';').filter(|s| !s.is_empty()) {
                v.push(s.trim().parse::<i32>().unwrap_or(0).max(10));
                if v.len() == NUM_FIELDS {
                    break;
                }
            }
            *self.list_col_widths.borrow_mut() = v;
        }
        self.toggle("ToggleNotesBtn").set_active(ini.read_bool(CONFIG_ID, "HideNotes", false));
        self.toggle_notes_btn_click();
        self.toggle("PasswQualityBtn").set_active(ini.read_bool(CONFIG_ID, "EstimatePasswQuality", true));
        self.passw_quality_btn_click();
        self.set_action_bool("SearchMenu_CaseSensitive", ini.read_bool(CONFIG_ID, "FindCaseSensitive", false));
        self.set_action_bool("SearchMenu_FuzzySearch", ini.read_bool(CONFIG_ID, "FindFuzzy", false));
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
            self.win.set_default_size(w, h);
        }
    }

    /// Database options (`g_config.Database`).
    pub(super) fn db_config(&self) -> DatabaseConfig {
        self.main.upgrade().map(|m| m.core.borrow().config.database.clone()).unwrap_or_default()
    }

    /// `SaveConfig()` (also of the dialogs).
    pub fn save_config(&self, ini: &mut IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.default_height().max(self.win.height()) as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.default_width().max(self.win.width()) as i64);
        ini.write_integer(CONFIG_ID, "ListWidth", self.hpaned.position() as i64);
        if let Some(f) = self.list_font.borrow().as_ref() {
            ini.write_string(CONFIG_ID, "ListFont", &helpers::font_to_string(f));
        }
        ini.write_integer(CONFIG_ID, "TagListHeight", self.vpaned.position() as i64);
        ini.write_string(
            CONFIG_ID,
            "PasswBoxFont",
            &self.passw_font.borrow().as_ref().map(helpers::font_to_string).unwrap_or_default(),
        );
        ini.write_integer(CONFIG_ID, "ShowColMask", self.show_col_mask.get() as i64);
        ini.write_bool(CONFIG_ID, "ShowPasswInList", self.action_bool("MainMenu_View_ShowPasswInList"));
        let widths: String = self.list.column_widths().iter().map(|w| format!("{w};")).collect();
        ini.write_string(CONFIG_ID, "ListColWidths", &widths);
        ini.write_bool(CONFIG_ID, "HideNotes", self.toggle("ToggleNotesBtn").is_active());
        ini.write_bool(CONFIG_ID, "EstimatePasswQuality", self.toggle("PasswQualityBtn").is_active());
        ini.write_bool(CONFIG_ID, "FindCaseSensitive", self.action_bool("SearchMenu_CaseSensitive"));
        ini.write_bool(CONFIG_ID, "FindFuzzy", self.action_bool("SearchMenu_FuzzySearch"));
        ini.write_integer(CONFIG_ID, "SortByIdx", self.sort_by_idx.get() as i64);
        ini.write_integer(CONFIG_ID, "SortOrder", self.sort_order_factor.get() as i64);
        ini.write_integer(CONFIG_ID, "TagsSortByIdx", self.tags_sort_by_idx.get() as i64);
        ini.write_integer(CONFIG_ID, "TagsSortOrder", self.tags_sort_order_factor.get() as i64);
        ini.write_string(CONFIG_ID, "LastDatabase", &self.db_file_name.borrow());
        ini.write_string(CONFIG_ID, "RecentFiles", &to_comma_text(&self.recent_files.borrow()));
        if let Some(d) = self.prop_dlg.borrow().as_ref() {
            d.save_config(ini);
        }
        if let Some(d) = self.settings_dlg.borrow().as_ref() {
            d.save_config(ini);
        }
        if let Some(d) = self.key_val_dlg.borrow().as_ref() {
            d.save_config(ini);
        }
        if let Some(d) = self.history_dlg.borrow().as_ref() {
            d.save_config(ini);
        }
    }

    fn apply_fonts(&self) {
        let mut css = String::new();
        if let Some(f) = self.list_font.borrow().as_ref() {
            css.push_str(&helpers::font_css(f));
        }
        if self.search_mode.get() != SEARCH_MODE_OFF {
            css.push_str("color: blue;");
        }
        self.list.css.set(&css);
        match self.passw_font.borrow().as_ref() {
            Some(f) => self.passw_css.set(&helpers::font_css(f)),
            None => self.passw_css.set("font-family: monospace;"),
        }
    }

    // ------------------------------------------------------------------
    // public interface for the main window

    /// Show the window (`Show()` + `FormShow`/`FormActivate` on start-up:
    /// opens the last database if configured).
    pub fn show(self: &Rc<Self>) {
        let first = self.first_activation.get();
        if !self.is_db_open()
            && !self.locked.get()
            && self.db_config().open_last_db_on_startup
            && !self.db_file_name.borrow().is_empty()
        {
            self.form_tag.set(FORM_TAG_OPEN_DB);
        }
        let cmd_file = if first { self.mainw().core.borrow().cmd.passw_db_file_name.clone() } else { None };
        if cmd_file.is_some() && !self.is_db_open() {
            self.form_tag.set(FORM_TAG_OPEN_DB);
        }
        self.first_activation.set(false);
        self.win.unminimize();
        self.win.present();
        if self.form_tag.get() == FORM_TAG_OPEN_DB {
            self.form_tag.set(0);
            let file = cmd_file.map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| self.db_file_name.borrow().clone());
            let m = self.clone();
            glib::spawn_future_local(async move {
                m.open_database(DB_OPEN_FLAG_EXISTING, &file).await;
            });
        }
    }

    pub fn is_db_open(&self) -> bool {
        self.db.borrow().as_ref().is_some_and(|d| d.is_open())
    }

    pub fn is_db_open_or_locked(&self) -> bool {
        self.is_db_open() || self.locked.get()
    }

    /// Unsaved changes in the database or the current entry.
    pub fn has_unsaved_changes(&self) -> bool {
        self.is_db_open() && (self.db_changed.get() || self.item_changed.get())
    }

    /// "Add to Database" of the main window and the password list
    /// (`AddPassw(passw, blShowNumPassw)`): one entry per line.
    pub fn add_passw_to_db(self: &Rc<Self>, passw: &str, show_num_passw: bool) -> usize {
        self.add_passw(passw, show_num_passw, None)
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
            glib::spawn_future_local(async move { m.lock_or_unlock_database(false).await });
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
            p.with_file_name(format!("{stem}_session_{}{ext}", helpers::time_stamp_string(false)))
                .to_string_lossy()
                .to_string()
        };
        self.save_database(&save_name)
    }

    /// Search the database for the keyword contained in the title of the
    /// active window (`SearchDbForKeyword()`); with `autotype` the first
    /// matching entry is typed. The title comes from KWin (KDE Plasma).
    pub fn search_db_for_keyword(self: &Rc<Self>, autotype: bool) {
        if !self.is_db_open() || self.item_changed.get() {
            return;
        }
        let m = self.clone();
        glib::spawn_future_local(async move {
            let Some(main) = m.main.upgrade() else { return };
            let Some(app) = main.window.application() else { return };
            let title = crate::ui::active_window::active_window_title(&app).await;
            let Some(title) = title else {
                let text = trl("This function is not available on this system.");
                if autotype {
                    crate::ui::util::msg(Some(&main.window.clone().upcast()), &text, super::util::MsgIcon::Warning);
                } else {
                    m.show();
                    super::util::msg(Some(&m.gwin()), &text, super::util::MsgIcon::Warning);
                }
                return;
            };
            use helpers::lower;
            use pwtool_core::language::trl_format;
            use pwtool_core::passw::database::FieldType;
            let title_lc = lower(&title);
            let mut found: Option<usize> = None;
            {
                let mut dbb = m.db.borrow_mut();
                let Some(db) = dbb.as_mut() else { return };
                for i in 0..db.len() {
                    if !autotype {
                        db.entry_mut(i).user_flags &= !DB_FLAG_FOUND;
                    }
                    let kw = db.entry(i).string(FieldType::Keyword).to_string();
                    if !kw.is_empty() && title_lc.contains(lower(&kw).as_str()) {
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
                    Some(i) => m.perform_autotype_for(i),
                    None => crate::ui::main_window::show_tray_info(&main, &not_found, true),
                }
                return;
            }
            if let Some(i) = found {
                m.search_mode.set(SEARCH_MODE_NORMAL);
                m.apply_fonts();
                m.reset_list_view(RELOAD_TAGS);
                m.reset_nav_controls();
                let kw = m.db.borrow().as_ref().map(|db| db.entry(i).string(pwtool_core::passw::database::FieldType::Keyword).to_string()).unwrap_or_default();
                if let Some(sb) = m.b.try_get::<gtk::ComboBoxText>("SearchBox") {
                    crate::ui::util::set_combo_text(&sb, &kw);
                }
                m.search_label.set_text(&trl("Search via hot key."));
                m.update_menu_states();
            }
            m.show();
            if found.is_none() {
                super::util::msg(Some(&m.gwin()), &not_found, super::util::MsgIcon::Warning);
            }
        });
    }
}

impl PasswManager {
    /// `MainMenu_Edit_PerformAutotypeClick()`: autotype sequence of the
    /// selected entry (key-value "Autotype" or the default sequence).
    pub(super) fn perform_autotype_for(self: &Rc<Self>, index: usize) {
        let Some(main) = self.main.upgrade() else { return };
        let (delay, minimize) = {
            let c = &main.core.borrow().config;
            (c.autotype_delay.clamp(0, 10000) as u32, c.minimize_autotype)
        };
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
                if f == pwtool_core::passw::database::FieldType::Password as usize {
                    db.get_entry_passw(index).ok()
                } else {
                    Some(Zeroizing::new(e.strings[f].to_string()))
                }
            };
            crate::ui::autotype::build_sequence(&seq, Some(&field), None, None, delay)
        };
        main.send_keys_from(&self.gwin(), strokes, minimize);
    }
}

fn file_name_of(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}
