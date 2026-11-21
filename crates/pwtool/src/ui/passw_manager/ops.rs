//! Event handlers and database operations of the password manager.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use pwtool_core::language::{trl, trl_ctx, trl_format};
use pwtool_core::passw::PasswordGenerator;
use pwtool_core::passw::database::{
    self as pdb, DbError, FieldType, NUM_FIELDS, NUM_STRING_FIELDS, PasswDatabase, PasswHistory, Tag, VERSION_HIGH, VERSION_LOW, decode_expiry_date,
    encode_expiry_date,
};
use pwtool_core::random::RandomPool;
use pwtool_core::unicode::{str_to_w32, w32_to_string_secure};
use pwtool_core::util::floor_entropy_bits;
use zeroize::Zeroizing;

use super::dialogs::{DbProperty, DbSettings};
use super::helpers::{self, crlf_to_lf, lf_to_crlf, lower, str_find_fuzzy, today, today_plus_days};
use super::list::{RowKey, TagKind};
use super::*;
use crate::ui::dialogs as udlg;
use crate::ui::passw_enter::{
    EnterResult, PASSWENTER_FLAG_ALLOWSKIP, PASSWENTER_FLAG_CONFIRMPASSW, PASSWENTER_FLAG_DECRYPT, PASSWENTER_FLAG_ENABLEKEYFILE,
    PASSWENTER_FLAG_ENABLEKEYFILECREATION, PASSWENTER_FLAG_ENCRYPT, PASSWENTER_FLAG_PASSWQUALITY,
};
use crate::vcl::Event;
use crate::vcl::dialogs::{MsgButtons, MsgResult};
use crate::vcl::list_view::LvRow;

/// Format password generated with the active RNG (deterministic if active, else pool).
fn format_passw(main: Option<&Rc<MainWindow>>, format: &str) -> Option<Zeroizing<String>> {
    if let Some(m) = main {
        return m.format_passw_with_active_rng(format);
    }
    let pg = PasswordGenerator::new();
    let f = str_to_w32(format);
    let r = pg.get_format_passw(&mut *RandomPool::global(), &f, 0, None, 16000, None);
    if r.passw.is_empty() { None } else { Some(w32_to_string_secure(&r.passw)) }
}

/// File type filters of the file dialogs.
fn file_filters(csv: bool) -> Vec<(String, &'static [&'static str])> {
    let mut v: Vec<(String, &'static [&'static str])> = vec![(format!("{} (*.pwdb)", trl("Password databases")), &["pwdb"])];
    if csv {
        v.push((format!("{} (*.csv)", trl("CSV files")), &["csv"]));
    }
    v.push((format!("{} (*.*)", trl("All files")), &["*"]));
    v
}

fn filter_refs<'a>(v: &'a [(String, &'static [&'static str])]) -> Vec<(&'a str, &'a [&'a str])> {
    v.iter().map(|(n, e)| (n.as_str(), *e)).collect()
}

async fn ask(parent: ViewportId, text: &str, icon: MsgIcon, buttons: MsgButtons, default_second: bool) -> MsgResult {
    udlg::ask(parent, text, icon, buttons, default_second).await
}

async fn info(parent: ViewportId, text: &str, icon: MsgIcon) {
    udlg::ask(parent, text, icon, MsgButtons::Ok, false).await;
}

fn open_uri(uri: &str) {
    let target = if uri.contains("://") || Path::new(uri).exists() { uri.to_string() } else { format!("https://{uri}") };
    pwtool_app::platform::sys::system_open(std::ffi::OsStr::new(&target));
}

impl PasswManager {
    // ------------------------------------------------------------------
    // events

    /// Run a function with the manager in a task (for `async` handlers).
    pub(super) fn spawn_with<F, Fut>(self: &Rc<Self>, f: F)
    where
        F: FnOnce(Rc<Self>) -> Fut + 'static,
        Fut: std::future::Future<Output = ()> + 'static,
    {
        let m = self.clone();
        crate::runtime::spawn(async move { f(m).await });
    }

    pub(super) fn connect_events(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        self.form.on_any(move |e| {
            if let Some(m) = w.upgrade() {
                m.on_event(e);
            }
        });
    }

    /// Activate a menu item by name if it is enabled (also tool buttons and
    /// the automation).
    pub fn activate(self: &Rc<Self>, name: &str) -> bool {
        if !self.f().has_menu_item(name) || !self.f().menu_enabled(name) {
            return false;
        }
        self.on_menu(name);
        true
    }

    pub(super) fn on_event(self: &Rc<Self>, e: &Event) {
        match e {
            Event::Menu(n) => self.on_menu(n),
            Event::Click(n) => self.on_click(n),
            Event::Change(n) => match n.as_str() {
                "TitleBox" | "UserNameBox" | "UrlBox" | "KeywordBox" | "TagsBox" | "KeyValueListBox" => self.title_box_change(),
                "PasswBox" => self.passw_box_change(),
                "NotesBox" => {
                    if self.notes_tag.get() != NOTES_TAG_CHANGING {
                        self.title_box_change();
                    }
                }
                "ExpiryDatePicker" => {
                    self.notify_user_action();
                    self.set_item_changed(true);
                }
                // `SearchBoxSelect`: an item was chosen from the list
                "SearchBox" if self.f().item_index("SearchBox").is_some() => {
                    self.notify_user_action();
                    let text = self.f().text("SearchBox");
                    self.search_database(&text, self.search_flags.get(), self.menu_bool("SearchMenu_CaseSensitive"), self.menu_bool("SearchMenu_FuzzySearch"));
                }
                _ => {}
            },
            Event::Enter(n) => match n.as_str() {
                "SearchBox" => {
                    self.notify_user_action();
                    self.search_btn_click();
                }
                "TitleBox" | "UserNameBox" | "PasswBox" | "UrlBox" | "KeywordBox" | "TagsBox" => {
                    if self.selected.get().is_some() {
                        self.spawn_with(|m| async move { m.add_modify_btn_click().await });
                    }
                }
                _ => {}
            },
            Event::FocusIn(n) if n == "NotesBox" => self.notes_box_enter(),
            Event::Exit(n) if n == "NotesBox" => self.notes_box_exit(),
            _ => {}
        }
    }

    pub(super) fn on_click(self: &Rc<Self>, n: &str) {
        match n {
            "NewBtn" => {
                self.activate("MainMenu_File_New");
            }
            "OpenBtn" => {
                self.activate("MainMenu_File_Open");
            }
            "SaveBtn" => {
                self.activate("MainMenu_File_Save");
            }
            "LockBtn" => {
                self.activate("MainMenu_File_Lock");
            }
            "AddEntryBtn" => self.add_entry_click(),
            "ClearSearchBtn" => self.clear_search_btn_click(),
            "SearchBtn" => self.search_btn_click(),
            "SearchBtn#dropdown" => {
                let r = self.f().rect("SearchBtn");
                self.f().popup_menu("SearchMenu", Some(r.left_bottom()));
            }
            "ClearFilterBtn" => {
                self.filter.set(FilterType::None);
                self.update_view_checks();
                self.reset_list_view(0);
            }
            "AddModifyBtn" => self.spawn_with(|m| async move { m.add_modify_btn_click().await }),
            "PrevBtn" => self.prev_btn_click(),
            "NextBtn" => self.next_btn_click(),
            "CancelBtn" => self.cancel_btn_click(),
            "DeleteBtn" => self.spawn_with(|m| async move { m.delete_click().await }),
            "AddTagBtn" => self.add_tag_btn_click(),
            "EditKeyValBtn" => self.spawn_with(|m| async move { m.edit_key_val_btn_click().await }),
            "ExpiryBtn" => {
                self.notify_user_action();
                if self.selected.get().is_some() && self.f().checked("ExpiryCheck") {
                    let r = self.f().rect("ExpiryBtn");
                    self.f().popup_menu("ExpiryMenu", Some(r.left_bottom()));
                }
            }
            "UrlBtn" => {
                self.notify_user_action();
                let url = self.f().text("UrlBox");
                if !url.is_empty() {
                    open_uri(&url);
                }
            }
            "PasswHistoryBtn" => self.spawn_with(|m| async move { m.passw_history_btn_click().await }),
            "TogglePasswBtn" => self.toggle_passw_btn_click(),
            "PasswQualityBtn" => {
                self.notify_user_action();
                self.passw_quality_btn_click();
            }
            "ToggleNotesBtn" => {
                self.notify_user_action();
                self.toggle_notes_btn_click();
            }
            "ExpiryCheck" => self.expiry_check_click(),
            _ => {}
        }
    }

    pub(super) fn on_menu(self: &Rc<Self>, name: &str) {
        if let Some(i) = name.strip_prefix("dyn:col").and_then(|s| s.parse::<usize>().ok()) {
            let on = !self.menu_bool(name);
            self.set_menu_bool(name, on);
            let _ = i;
            self.notify_user_action();
            self.set_show_col_mask_from_menu();
            self.reset_list_view(RESET_COLUMNS);
            return;
        }
        if let Some(i) = name.strip_prefix("dyn:sort").and_then(|s| s.parse::<i32>().ok()) {
            self.sort_by(i);
            return;
        }
        if let Some(i) = name.strip_prefix("dyn:recent").and_then(|s| s.parse::<usize>().ok()) {
            self.notify_user_action();
            let Some(file) = self.recent_files.borrow().get(i).cloned() else { return };
            if !self.is_db_open() || lower(&file) != lower(&self.db_file_name.borrow()) {
                self.spawn_with(move |m| async move {
                    m.open_database(DB_OPEN_FLAG_EXISTING, &file).await;
                });
            }
            return;
        }
        if let Some(i) = name.strip_prefix("dyn:tag").and_then(|s| s.parse::<usize>().ok()) {
            let tag = self.tag_menu_items.borrow().get(i).cloned();
            if let Some(t) = tag {
                self.on_tag_menu_item_click(&t);
            }
            return;
        }
        if let Some(d) = name.strip_prefix("dyn:days").and_then(|s| s.parse::<i32>().ok()) {
            self.notify_user_action();
            let date = today_plus_days(d);
            if date != self.f().date("ExpiryDatePicker") {
                self.f().set_date("ExpiryDatePicker", date.0, date.1, date.2);
                self.set_item_changed(true);
            }
            return;
        }
        match name {
            "MainMenu_File_New" => {
                self.notify_user_action();
                self.spawn_with(|m| async move {
                    m.open_database(0, "").await;
                });
            }
            "MainMenu_File_Open" => {
                self.notify_user_action();
                self.spawn_with(|m| async move {
                    m.open_database(DB_OPEN_FLAG_EXISTING, "").await;
                });
            }
            "MainMenu_File_OpenReadOnly" => {
                self.notify_user_action();
                self.spawn_with(|m| async move {
                    m.open_database(DB_OPEN_FLAG_EXISTING | DB_OPEN_FLAG_READONLY, "").await;
                });
            }
            "MainMenu_File_Close" => {
                self.notify_user_action();
                self.spawn_with(|m| async move {
                    m.close_database(false, 0).await;
                });
            }
            "MainMenu_File_Lock" => {
                self.notify_user_action();
                self.spawn_with(|m| async move { m.lock_or_unlock_database(false).await });
            }
            "MainMenu_File_ClearRecentFiles" => {
                self.notify_user_action();
                self.recent_files.borrow_mut().clear();
                self.update_recent_files_menu();
            }
            "MainMenu_File_Save" => self.spawn_with(|m| async move { m.save_click().await }),
            "MainMenu_File_SaveAs" => self.spawn_with(|m| async move { m.save_as_click().await }),
            "MainMenu_File_Export_CsvFile" => self.spawn_with(|m| async move { m.export_csv_click().await }),
            "MainMenu_File_DbSettings" => self.spawn_with(|m| async move { m.db_settings_click().await }),
            "MainMenu_File_ChangeMasterPassword" => self.spawn_with(|m| async move { m.change_master_password_click().await }),
            "MainMenu_File_SetRecoveryPassword" => self.spawn_with(|m| async move { m.set_recovery_password_click().await }),
            "MainMenu_File_Properties" => self.spawn_with(|m| async move { m.properties_click().await }),
            "MainMenu_File_Exit" => {
                if let Some(w) = self.window.borrow().as_ref() {
                    if let Some(f) = w.on_close.borrow().as_ref() {
                        f();
                    }
                }
            }
            "MainMenu_Edit_CopyUserName" | "DbViewMenu_CopyUserName" => self.copy_user_name_click(),
            "MainMenu_Edit_CopyPassw" | "DbViewMenu_CopyPassw" => self.copy_passw_click(),
            "MainMenu_Edit_OpenUrl" | "DbViewMenu_OpenUrl" => self.open_url_click(),
            "MainMenu_Edit_Run" | "DbViewMenu_Run" => self.run_click(),
            "MainMenu_Edit_PerformAutotype" | "DbViewMenu_PerformAutotype" => {
                self.notify_user_action();
                if let Some(i) = self.selected_entry_index() {
                    self.perform_autotype_for(i);
                }
            }
            "MainMenu_Edit_AddEntry" => self.add_entry_click(),
            "MainMenu_Edit_DuplicateEntry" | "DbViewMenu_DuplicateEntry" => self.duplicate_entry_click(),
            "MainMenu_Edit_DeleteEntry" | "DbViewMenu_DeleteEntry" => self.spawn_with(|m| async move { m.delete_click().await }),
            // see the module documentation: actions follow the captions
            "MainMenu_Edit_Rearrange_Down" => self.move_db_entries(MOVE_TOP),
            "MainMenu_Edit_Rearrange_Top" => self.move_db_entries(MOVE_UP),
            "MainMenu_Edit_Rearrange_Up" => self.move_db_entries(MOVE_DOWN),
            "MainMenu_Edit_Rearrange_Bottom" => self.move_db_entries(MOVE_BOTTOM),
            "DbViewMenu_Rearrange_Top" => self.move_db_entries(MOVE_TOP),
            "DbViewMenu_Rearrange_Up" => self.move_db_entries(MOVE_UP),
            "DbViewMenu_Rearrange_Down" => self.move_db_entries(MOVE_DOWN),
            "DbViewMenu_Rearrange_Bottom" => self.move_db_entries(MOVE_BOTTOM),
            "MainMenu_Edit_SelectAll" | "DbViewMenu_SelectAll" => {
                self.notify_user_action();
                self.list.select_all();
                self.trigger_apply();
            }
            "MainMenu_View_ShowPasswInList" => {
                self.set_menu_bool(name, !self.menu_bool(name));
                self.notify_user_action();
                self.reset_list_view(0);
            }
            "MainMenu_View_ChangeListFont" => self.spawn_with(|m| async move { m.change_font(true).await }),
            "MainMenu_View_ChangePasswFont" => self.spawn_with(|m| async move { m.change_font(false).await }),
            "MainMenu_View_ResetFonts" => {
                self.notify_user_action();
                let ctx = windows::ctx();
                fonts::set_named_font(&ctx, fonts::PM_LIST, None);
                fonts::set_named_font(&ctx, fonts::PM_PASSW, None);
                self.apply_fonts();
            }
            "SearchMenu_CaseSensitive" | "SearchMenu_FuzzySearch" => self.set_menu_bool(name, !self.menu_bool(name)),
            "SearchMenu_SelectFields" => self.spawn_with(|m| async move {
                let _s = IdleSuspend::new(&m);
                if let Some(f) = m.dialogs.col_select(&m.ui_field_names, m.search_flags.get(), true, win_id()).await {
                    m.search_flags.set(f);
                }
            }),
            "MainMenu_View_SortBy_Unsorted" => self.sort_by(-1),
            "MainMenu_View_SortBy_Ascending" | "MainMenu_View_SortBy_Descending" => {
                self.notify_user_action();
                let f = if name.ends_with("Ascending") { 1 } else { -1 };
                if f != self.sort_order_factor.get() {
                    self.sort_order_factor.set(f);
                    if self.sort_by_idx.get() >= 0 {
                        self.sort_list();
                    }
                    self.set_list_view_sort_flag();
                }
                self.update_view_checks();
            }
            "MainMenu_View_SortTagsBy_Name" | "MainMenu_View_SortTagsBy_Freq" => {
                self.notify_user_action();
                let idx = if name.ends_with("Name") { 0 } else { 1 };
                if idx != self.tags_sort_by_idx.get() {
                    self.tags_sort_by_idx.set(idx);
                    self.tag_view.set_sort(idx, self.tags_sort_order_factor.get());
                }
                self.update_view_checks();
            }
            "MainMenu_View_SortTagsBy_Ascending" | "MainMenu_View_SortTagsBy_Descending" => {
                self.notify_user_action();
                let f = if name.ends_with("Ascending") { 1 } else { -1 };
                if f != self.tags_sort_order_factor.get() {
                    self.tags_sort_order_factor.set(f);
                    self.tag_view.set_sort(self.tags_sort_by_idx.get(), f);
                }
                self.update_view_checks();
            }
            "MainMenu_View_Filter_Expired" | "MainMenu_View_Filter_ExpireSoon" | "MainMenu_View_Filter_WeakPassw" => {
                let clicked = match name {
                    "MainMenu_View_Filter_Expired" => FilterType::Expired,
                    "MainMenu_View_Filter_ExpireSoon" => FilterType::ExpireSoon,
                    _ => FilterType::WeakPassw,
                };
                let next = if clicked == self.filter.get() { FilterType::None } else { clicked };
                self.filter.set(next);
                self.update_view_checks();
                self.reset_list_view(0);
            }
            _ => {}
        }
    }

    pub(super) fn sort_by(self: &Rc<Self>, idx: i32) {
        self.notify_user_action();
        if idx != self.sort_by_idx.get() {
            self.sort_by_idx.set(idx);
            self.sort_list();
            self.set_list_view_sort_flag();
        }
        self.update_view_checks();
    }

    /// Before drawing: nothing to do yet (states are updated on changes).
    pub(super) fn update_frame_states(&self) {}

    /// After drawing: events of the lists, coalesced selection handling,
    /// window state, right click on the password toggle button.
    pub(super) fn after_frame(self: &Rc<Self>, ui: &Ui) {
        for e in self.list.take_events() {
            match e {
                LvEvent::Selection => self.trigger_apply(),
                LvEvent::DblClick(_) => {}
                LvEvent::Context(pos) => {
                    self.notify_user_action();
                    self.apply_now();
                    self.update_menu_states();
                    self.f().popup_menu("DbViewMenu", Some(pos));
                }
                LvEvent::HeaderClick(ci) => {
                    let field = self.col_fields.borrow().get(ci).copied();
                    if let Some(f) = field {
                        self.db_view_column_click(f);
                    }
                }
                LvEvent::Drop(target) => self.db_view_drag_drop(target),
            }
        }
        // `DbViewMouseMove`: only unsorted, single real entry
        self.list.set_can_drag(
            self.sort_by_idx.get() < 0
                && self.list.sel_count() == 1
                && matches!(self.selected.get(), Some(RowKey::Entry(_)))
                && !self.item_changed.get(),
        );
        if self.tag_view.take_changed() {
            self.tag_apply_pending.set(true);
        }
        if self.apply_pending.replace(false) {
            self.apply_db_view_item_selection(None);
        }
        if self.tag_apply_pending.replace(false) {
            self.apply_tag_view_item_selection();
        }
        // shortcuts of the list's context menu work in the list (Ctrl+C
        // arrives as a copy event)
        if self.list.has_focus() && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
            self.activate("DbViewMenu_CopyPassw");
        }
        if self.list.has_focus() {
            const ITEMS: [&str; 9] = [
                "DbViewMenu_CopyUserName",
                "DbViewMenu_CopyPassw",
                "DbViewMenu_OpenUrl",
                "DbViewMenu_Run",
                "DbViewMenu_PerformAutotype",
                "DbViewMenu_DuplicateEntry",
                "DbViewMenu_DeleteEntry",
                "DbViewMenu_Rearrange_Top",
                "DbViewMenu_Rearrange_Bottom",
            ];
            for name in ITEMS {
                let Some((mods, key)) = self.f().menu_shortcut(name).and_then(crate::vcl::render::shortcut_key) else { continue };
                if ui.input_mut(|i| i.consume_key(mods, key)) {
                    self.activate(name);
                }
            }
        }
        // right click on the password toggle button: generate a password
        let r = self.f().rect("TogglePasswBtn");
        if ui.input(|i| i.pointer.secondary_released() && i.pointer.interact_pos().is_some_and(|p| r.contains(p))) && self.f().visible("TogglePasswBtn") {
            self.toggle_passw_btn_right_click();
        }
        self.watch_viewport(ui);
    }

    /// `FormActivate()`.
    pub(super) fn form_activate(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        if self.form_tag.get() == FORM_TAG_OPEN_DB {
            self.form_tag.set(0);
            let file = self.db_file_name.borrow().clone();
            self.spawn_with(move |m| async move {
                m.open_database(DB_OPEN_FLAG_EXISTING, &file).await;
            });
            return;
        }
        if self.locked.get() {
            let mut unlock = false;
            if self.form_tag.get() == FORM_TAG_UNLOCK_DB {
                if self.unlock_tick.get().is_none_or(|t| t.elapsed().as_millis() > 3000) {
                    unlock = true;
                }
            } else if !self.unlock_tried.get() {
                unlock = true;
            }
            self.unlock_tried.set(true);
            self.unlock_tick.set(Some(Instant::now()));
            if unlock {
                self.spawn_with(|m| async move { m.lock_or_unlock_database(false).await });
            }
        }
        self.form_tag.set(0);
    }

    pub(super) fn start_idle_timer(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        crate::runtime::spawn(async move {
            loop {
                pwtool_app::rt::sleep(Duration::from_secs(1)).await;
                let Some(m) = w.upgrade() else { return };
                m.idle_timer_tick();
            }
        });
    }

    /// `IdleTimerTimer()`.
    pub(super) fn idle_timer_tick(self: &Rc<Self>) {
        if !self.idle_timer_enabled.get() || self.busy.get() || self.idle_suspend.get() > 0 {
            return;
        }
        let c = self.db_config();
        if self.is_db_open()
            && c.lock_idle
            && !self.item_changed.get()
            && !self.locked.get()
            && self.last_user_action.get().elapsed().as_secs() >= c.lock_idle_time as u64
        {
            self.spawn_with(|m| async move { m.lock_or_unlock_database(true).await });
        }
    }

    // ------------------------------------------------------------------
    // selection handling (`TSelectItemThread`: coalesce selection events)

    pub(super) fn trigger_apply(&self) {
        self.apply_pending.set(true);
        windows::ctx().request_repaint();
    }

    /// `ApplyNow()`.
    pub(super) fn apply_now(self: &Rc<Self>) {
        if self.apply_pending.replace(false) {
            self.apply_db_view_item_selection(None);
        }
    }

    pub(super) fn cancel_pending(&self) {
        self.apply_pending.set(false);
        self.tag_apply_pending.set(false);
    }

    /// `ApplyTagViewItemSelection()`.
    pub(super) fn apply_tag_view_item_selection(self: &Rc<Self>) {
        let mut filter = BTreeSet::new();
        for k in self.tag_view.selected() {
            match k {
                TagKind::All => break,
                TagKind::Tag(t) => {
                    filter.insert(t);
                }
                TagKind::Untagged => {
                    filter.insert(String::new());
                }
            }
        }
        if filter != *self.tag_filter.borrow() {
            *self.tag_filter.borrow_mut() = filter;
            self.reset_list_view(0);
        }
    }

    // ------------------------------------------------------------------
    // list view

    pub(super) fn set_show_col_mask_from_menu(&self) {
        let mut mask = 0;
        for i in 0..NUM_FIELDS {
            if self.menu_bool(&format!("dyn:col{i}")) {
                mask |= 1 << i;
            }
        }
        self.show_col_mask.set(mask);
    }

    /// `SetListViewSortFlag()`.
    pub(super) fn set_list_view_sort_flag(&self) {
        let idx = self.sort_by_idx.get();
        let col = if idx >= 0 { self.col_fields.borrow().iter().position(|&f| f == idx as usize) } else { None };
        self.list.set_sort_indicator(col, self.sort_order_factor.get() > 0);
    }

    /// Row of an entry (`AddModifyListViewEntry()`).
    pub(super) fn make_row(&self, db: &PasswDatabase, idx: usize) -> LvRow<RowKey> {
        let e = db.entry(idx);
        let icon = if e.user_flags & DB_FLAG_EXPIRED != 0 {
            self.f().image("ImageList16", 9)
        } else if e.user_flags & DB_FLAG_EXPIRES_SOON != 0 {
            self.f().image("ImageList16", 10)
        } else {
            None
        };
        let show_passw = self.menu_bool("MainMenu_View_ShowPasswInList");
        let mask = self.show_col_mask.get();
        let mut cols = Vec::new();
        for i in 0..NUM_FIELDS {
            if mask & (1 << i) == 0 {
                continue;
            }
            let s: String = match i {
                x if x == FieldType::Password as usize => {
                    if !show_passw {
                        HIDE_PASSW_STR.to_string()
                    } else {
                        db.get_entry_passw(idx).map(|p| p.to_string()).unwrap_or_default()
                    }
                }
                x if x == FieldType::Notes as usize => {
                    let notes = e.string(FieldType::Notes);
                    let s: String = if notes.encode_utf16().count() > MAX_NOTES_LEN {
                        let mut t: String = notes.chars().take(MAX_NOTES_LEN - 3).collect();
                        t.push_str("...");
                        t
                    } else {
                        notes.to_string()
                    };
                    s.replace(['\r', '\n'], " ")
                }
                x if x == FieldType::CreationTime as usize => e.creation_time_string(),
                x if x == FieldType::ModificationTime as usize => e.modification_time_string(),
                x if x == FieldType::PasswChangeTime as usize => e.passw_change_time_string(),
                x if x == FieldType::PasswExpiryDate as usize => e.passw_expiry_date_string(),
                x if x == FieldType::PasswHistory as usize => e.passw_history().len().to_string(),
                x if x < NUM_STRING_FIELDS => e.strings[x].to_string(),
                _ => String::new(),
            };
            if cols.is_empty() && self.search_mode.get() == SEARCH_MODE_FUZZY {
                cols.push(format!("[{}%] {}", e.user_tag.min(100), s));
            } else {
                cols.push(s);
            }
        }
        LvRow { key: RowKey::Entry(e.id()), cells: cols, icon, header: false }
    }

    pub(super) fn new_entry_row(&self) -> LvRow<RowKey> {
        let n = self.show_col_mask.get().count_ones() as usize;
        let mut cols = vec![String::new(); n];
        if n > 0 {
            cols[0] = format!("<{}>", trl("New Entry"));
        }
        LvRow::new(RowKey::New, cols)
    }

    /// `DbViewCompare()`.
    pub(super) fn compare_rows(&self, db: &PasswDatabase, a: &RowKey, b: &RowKey) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let (RowKey::Entry(ia), RowKey::Entry(ib)) = (a, b) else {
            return match (a, b) {
                (RowKey::New, RowKey::New) => Equal,
                (RowKey::New, _) => Greater,
                _ => Less,
            };
        };
        let (Some(xa), Some(xb)) = (db.index_of_id(*ia), db.index_of_id(*ib)) else { return Equal };
        let (e1, e2) = (db.entry(xa), db.entry(xb));
        if self.search_mode.get() == SEARCH_MODE_FUZZY {
            return e2.user_tag.cmp(&e1.user_tag);
        }
        let f = self.sort_order_factor.get();
        let apply = |o: std::cmp::Ordering| if f < 0 { o.reverse() } else { o };
        let idx = self.sort_by_idx.get();
        match idx {
            -1 => e1.index().cmp(&e2.index()),
            x if x == FieldType::Password as i32 => {
                let p1 = db.get_entry_passw(xa).unwrap_or_default();
                let p2 = db.get_entry_passw(xb).unwrap_or_default();
                apply(helpers::wcscmp(&p1, &p2))
            }
            x if x == FieldType::CreationTime as i32 => apply(e1.creation_time.cmp(&e2.creation_time)),
            x if x == FieldType::ModificationTime as i32 => apply(e1.modification_time.cmp(&e2.modification_time)),
            x if x == FieldType::PasswChangeTime as i32 => apply(e1.passw_change_time.cmp(&e2.passw_change_time)),
            x if x == FieldType::PasswExpiryDate as i32 => apply(e1.passw_expiry_date.cmp(&e2.passw_expiry_date)),
            x if x == FieldType::PasswHistory as i32 => apply(e1.passw_history().len().cmp(&e2.passw_history().len())),
            x if (0..NUM_STRING_FIELDS as i32).contains(&x) => apply(helpers::wcsicmp(&e1.strings[x as usize], &e2.strings[x as usize])),
            _ => Equal,
        }
    }

    /// Re-sort the list (`DbView->AlphaSort()`), keeping the selection.
    pub(super) fn sort_list(self: &Rc<Self>) {
        if self.list.len() < 2 {
            return;
        }
        let db = self.db.borrow();
        let Some(db) = db.as_ref() else { return };
        let mut rows = self.list.rows();
        rows.sort_by(|a, b| self.compare_rows(db, &a.key, &b.key));
        self.list.set_rows_keep_selection(rows);
        if let Some(i) = self.list.first_selected() {
            self.list.scroll_to(i);
        }
    }

    /// `ClearListView()`.
    pub(super) fn clear_list_view(self: &Rc<Self>) {
        self.selected.set(None);
        self.list.clear();
        self.clear_edit_panel();
    }

    /// `ResetListView()`.
    pub(super) fn reset_list_view(self: &Rc<Self>, flags: u32) {
        let prev_sel = if self.search_mode.get() == SEARCH_MODE_OFF {
            match self.selected.get() {
                Some(RowKey::Entry(id)) => Some(id),
                _ => None,
            }
        } else {
            None
        };
        self.clear_list_view();
        let mask = self.show_col_mask.get();
        if mask == 0 {
            if flags & RESET_COLUMNS != 0 {
                self.list.set_columns(Vec::new());
                self.col_fields.borrow_mut().clear();
            }
            return;
        }
        if flags & RESET_COLUMNS != 0 {
            let widths = {
                let mut w = self.list_col_widths.borrow_mut();
                if w.is_empty() { self.list.column_widths() } else { std::mem::take(&mut *w) }
            };
            let mut cols = Vec::new();
            let mut fields = Vec::new();
            let mut j = 0;
            for i in 0..NUM_FIELDS {
                if mask & (1 << i) != 0 {
                    let width = widths.get(j).copied().unwrap_or(self.f().sc(120));
                    j += 1;
                    cols.push((self.ui_field_names[i].clone(), width));
                    fields.push(i);
                }
            }
            self.list.set_columns(cols);
            *self.col_fields.borrow_mut() = fields;
            self.set_list_view_sort_flag();
        }
        if !self.is_db_open() {
            return;
        }
        let search_mode = self.search_mode.get();
        let (cur_date, soon_date) = helpers::expiry_check_dates(self.db_config().warn_expire_num_days);
        let filter = self.filter.get();
        let use_adv = self.main.upgrade().is_some_and(|m| m.core.borrow().use_advanced_passw_est);
        let mut rows = Vec::new();
        let mut prev_sel_idx = None;
        {
            let mut dbb = self.db.borrow_mut();
            let db = dbb.as_mut().unwrap();
            if flags & RELOAD_TAGS != 0 {
                let mut tags: BTreeMap<String, u32> = BTreeMap::new();
                let mut search_tags: BTreeMap<String, u32> = BTreeMap::new();
                let (mut num_untagged, mut num_untagged_search, mut num_search) = (0u32, 0u32, 0u32);
                for e in db.entries() {
                    if e.tags().is_empty() {
                        num_untagged += 1;
                    } else {
                        for t in e.tags() {
                            *tags.entry(t.0.clone()).or_insert(0) += 1;
                        }
                    }
                    if search_mode != SEARCH_MODE_OFF && e.user_flags & DB_FLAG_FOUND != 0 {
                        num_search += 1;
                        if e.tags().is_empty() {
                            num_untagged_search += 1;
                        } else {
                            for t in e.tags() {
                                *search_tags.entry(t.0.clone()).or_insert(0) += 1;
                            }
                        }
                    }
                }
                {
                    let mut g = self.global_casei_tags.borrow_mut();
                    g.clear();
                    for t in tags.keys() {
                        g.entry(lower(t)).or_insert_with(|| t.clone());
                    }
                }
                let src = if search_mode == SEARCH_MODE_OFF { &tags } else { &search_tags };
                let mut m_tags = self.tags.borrow_mut();
                m_tags.clear();
                self.tag_view.clear();
                let all_caption = format!(
                    "{} ({})",
                    if search_mode == SEARCH_MODE_OFF { trl_ctx("All", "tags") } else { trl("All search results") },
                    if search_mode == SEARCH_MODE_OFF { db.len() as u32 } else { num_search }
                );
                self.tag_view.add(TagKind::All, &all_caption, 0, false);
                let old_filter = self.tag_filter.borrow().clone();
                let mut new_filter = BTreeSet::new();
                for (t, n) in src {
                    m_tags.push((t.clone(), *n));
                    let sel = old_filter.contains(t);
                    if sel {
                        new_filter.insert(t.clone());
                    }
                    self.tag_view.add(TagKind::Tag(t.clone()), &format!("{t} ({n})"), *n, sel);
                }
                let (base_untagged, show_untagged) = if search_mode == SEARCH_MODE_OFF {
                    (num_untagged, num_untagged > 0 && (num_untagged as usize) < db.len())
                } else {
                    (num_untagged_search, num_untagged_search > 0 && num_untagged_search < num_search)
                };
                if show_untagged {
                    m_tags.push((String::new(), base_untagged));
                    let sel = old_filter.contains("");
                    if sel {
                        new_filter.insert(String::new());
                    }
                    self.tag_view.add(TagKind::Untagged, &format!("{} ({})", trl("Untagged"), base_untagged), base_untagged, sel);
                }
                *self.tag_filter.borrow_mut() = new_filter;
                let names: Vec<String> = tags.keys().cloned().collect();
                drop(m_tags);
                self.update_tag_menu(&names);
            }
            // expiry flags
            for i in 0..db.len() {
                let e = db.entry_mut(i);
                e.user_flags &= !(DB_FLAG_EXPIRED | DB_FLAG_EXPIRES_SOON);
                if e.passw_expiry_date != 0 {
                    if cur_date >= e.passw_expiry_date {
                        e.user_flags |= DB_FLAG_EXPIRED;
                    } else if soon_date >= e.passw_expiry_date {
                        e.user_flags |= DB_FLAG_EXPIRES_SOON;
                    }
                }
            }
        }
        let dbb = self.db.borrow();
        let db = dbb.as_ref().unwrap();
        let tag_filter = self.tag_filter.borrow().clone();
        // `InitAutoComplete()`: user names of all entries, once each (ignoring case)
        let mut user_names: BTreeMap<String, String> = BTreeMap::new();
        for i in 0..db.len() {
            let e = db.entry(i);
            let un = e.string(FieldType::UserName);
            if !un.is_empty() {
                user_names.entry(lower(un)).or_insert_with(|| un.to_string());
            }
            if (filter == FilterType::Expired && e.user_flags & DB_FLAG_EXPIRED == 0)
                || (filter == FilterType::ExpireSoon && e.user_flags & DB_FLAG_EXPIRES_SOON == 0)
                || (search_mode != SEARCH_MODE_OFF && e.user_flags & DB_FLAG_FOUND == 0)
            {
                continue;
            }
            if filter == FilterType::WeakPassw {
                let p = db.get_entry_passw(i).unwrap_or_default();
                if p.is_empty() {
                    continue;
                }
                let bits = if use_adv {
                    floor_entropy_bits(pwtool_core::zxcvbn::zxcvbn_match(&p))
                } else {
                    PasswordGenerator::estimate_passw_security(&p) as i32
                };
                if bits >= WEAK_PASSW_THRESHOLD {
                    continue;
                }
            }
            if !tag_filter.is_empty() {
                let matched = if e.tags().is_empty() { tag_filter.contains("") } else { tag_filter.iter().any(|t| !t.is_empty() && e.check_tag(t)) };
                if !matched {
                    continue;
                }
            }
            if prev_sel == Some(e.id()) {
                prev_sel_idx = Some(rows.len());
            }
            rows.push(self.make_row(db, i));
        }
        if search_mode == SEARCH_MODE_OFF && filter == FilterType::None {
            rows.push(self.new_entry_row());
        }
        if rows.len() >= 2 && (self.sort_by_idx.get() >= 0 || search_mode == SEARCH_MODE_FUZZY) {
            rows.sort_by(|a, b| self.compare_rows(db, &a.key, &b.key));
        }
        drop(dbb);
        self.list.set_rows(rows);
        self.f().set_completion("UserNameBox", user_names.into_values().collect());
        if prev_sel_idx.is_some() {
            if let Some(i) = prev_sel.and_then(|id| self.list.index_of(&RowKey::Entry(id))) {
                self.list.select_only(i);
                self.list.scroll_to(i);
                self.trigger_apply();
            }
        } else if search_mode != SEARCH_MODE_OFF && !self.list.is_empty() {
            self.list.select_only(0);
            self.trigger_apply();
        } else {
            self.apply_pending.set(false);
            self.apply_db_view_item_selection(None);
        }
        if filter != FilterType::None {
            self.f().set_visible("FilterInfoPanel", true);
            let caption = match filter {
                FilterType::Expired => "Expired Entries",
                FilterType::ExpireSoon => "Entries that Will Expire Soon",
                _ => "Entries with Weak Passwords",
            };
            self.f().set_hint("FilterInfoPanel", &trl(caption));
        } else {
            self.f().set_visible("FilterInfoPanel", false);
        }
    }

    // ------------------------------------------------------------------
    // edit panel

    /// `ClearEditPanel()`.
    pub(super) fn clear_edit_panel(&self) {
        let f = self.f();
        for name in ["TitleBox", "UserNameBox", "PasswBox", "UrlBox", "KeywordBox", "KeyValueListBox", "TagsBox", "NotesBox"] {
            f.set_text(name, "");
        }
        self.set_expiry_checked(false);
        let (y, m, d) = today();
        f.set_date("ExpiryDatePicker", y, m, d);
        for name in ["CreationTimeInfo", "LastModificationInfo", "PasswChangeInfo"] {
            f.set_text(name, "");
        }
        self.passw_entropy_bits.set(0.0);
        if f.checked("PasswQualityBtn") {
            self.estimate_passw_quality(None);
        }
        *self.temp_key_val.borrow_mut() = None;
        *self.temp_passw_history.borrow_mut() = None;
        *self.temp_notes.borrow_mut() = None;
    }

    /// Set the expiry check box without marking the entry as changed.
    pub(super) fn set_expiry_checked(&self, on: bool) {
        self.f().set_checked("ExpiryCheck", on);
        self.f().set_enabled("ExpiryDatePicker", on);
    }

    /// `ApplyDbViewItemSelection()`.
    pub(super) fn apply_db_view_item_selection(self: &Rc<Self>, item: Option<RowKey>) {
        self.notify_user_action();
        let n = self.list.sel_count();
        self.f().set_enabled("EditPanel", n > 0);
        let mut item = item;
        if n == 1 {
            if item.is_none() {
                item = self.list.first_selected().and_then(|i| self.list.key_at(i));
                if item == self.selected.get() {
                    self.update_menu_states();
                    return;
                }
            }
        } else {
            item = None;
        }
        if self.selected.get().is_some() {
            self.clear_edit_panel();
        }
        match item {
            Some(RowKey::Entry(id)) => self.fill_entry(id),
            Some(RowKey::New) => self.fill_new_entry(),
            None => {}
        }
        self.selected.set(item);
        if self.item_changed.get() {
            self.set_item_changed(false);
        } else {
            self.reset_nav_controls();
        }
        self.update_menu_states();
    }

    pub(super) fn fill_entry(&self, id: u32) {
        let f = self.f();
        let dbb = self.db.borrow();
        let Some(db) = dbb.as_ref() else { return };
        let Some(idx) = db.index_of_id(id) else { return };
        let e = db.entry(idx);
        f.set_text("TitleBox", e.string(FieldType::Title));
        f.set_text("UserNameBox", e.string(FieldType::UserName));
        f.set_text("UrlBox", e.string(FieldType::Url));
        f.set_text("KeywordBox", e.string(FieldType::Keyword));
        f.set_text("KeyValueListBox", &self.build_transl_key_val_string(e.key_value_list()));
        let notes = crlf_to_lf(e.string(FieldType::Notes));
        if self.notes_tag.get() == NOTES_TAG_HIDDEN {
            *self.temp_notes.borrow_mut() = Some(Zeroizing::new(notes));
            f.set_text("NotesBox", &self.notes_hidden_text);
        } else {
            f.set_text("NotesBox", &notes);
        }
        let tags = Zeroizing::new(e.tags().iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(", "));
        f.set_text("TagsBox", &tags);
        match decode_expiry_date(e.passw_expiry_date) {
            Some((y, m, d)) if e.passw_expiry_date != 0 => {
                f.set_date("ExpiryDatePicker", y, m, d);
                self.set_expiry_checked(true);
            }
            _ => {
                self.set_expiry_checked(false);
                let (y, m, d) = today();
                f.set_date("ExpiryDatePicker", y, m, d);
            }
        }
        f.set_text("CreationTimeInfo", &e.creation_time_string());
        f.set_text("LastModificationInfo", &e.modification_time_string());
        let pct = e.passw_change_time_string();
        f.set_text("PasswChangeInfo", if pct.is_empty() { "-" } else { &pct });
        let passw = db.get_entry_passw(idx).unwrap_or_default();
        drop(dbb);
        f.set_text("PasswBox", &passw);
        if f.checked("PasswQualityBtn") {
            self.estimate_passw_quality(Some(&passw));
        }
    }

    pub(super) fn fill_new_entry(&self) {
        let (user, fmt, days) = {
            let dbb = self.db.borrow();
            let Some(db) = dbb.as_ref() else { return };
            (db.default_user_name.clone(), db.passw_format_seq.clone(), db.default_passw_expiry_days)
        };
        let f = self.f();
        f.set_text("UserNameBox", &user);
        if !fmt.is_empty() {
            if let Some(p) = format_passw(self.main.upgrade().as_ref(), &fmt) {
                f.set_text("PasswBox", &p);
                if f.checked("PasswQualityBtn") {
                    self.estimate_passw_quality(Some(&p));
                }
            }
        }
        if days != 0 {
            let (y, m, d) = today_plus_days(days as i32);
            f.set_date("ExpiryDatePicker", y, m, d);
            self.set_expiry_checked(true);
        } else {
            self.set_expiry_checked(false);
        }
    }

    /// `BuildTranslKeyValString()`.
    pub(super) fn build_transl_key_val_string(&self, list: &[(Zeroizing<String>, Zeroizing<String>)]) -> Zeroizing<String> {
        let mut s = Zeroizing::new(String::new());
        for (k, v) in list {
            if let Some(name) = self.key_val_names.get(k.as_str()) {
                if !s.is_empty() {
                    s.push_str(", ");
                }
                s.push_str(name);
                s.push('=');
                s.push_str(v);
            }
        }
        s
    }

    /// `SetItemChanged()`.
    pub(super) fn set_item_changed(&self, changed: bool) {
        let f = self.f();
        self.item_changed.set(changed);
        f.set_visible("AddModifyBtn", changed);
        f.set_visible("CancelBtn", changed);
        if self.is_db_open() {
            self.list.set_enabled(!changed);
            self.tag_view.set_enabled(!changed);
            for name in ["AddEntryBtn", "SearchBox", "ClearSearchBtn", "SearchBtn"] {
                f.set_enabled(name, !changed);
            }
        }
        if changed {
            f.set_visible("PrevBtn", false);
            f.set_visible("NextBtn", false);
            f.set_visible("DeleteBtn", false);
        } else {
            self.item_passw_change_confirm.set(false);
            self.reset_nav_controls();
        }
        self.update_menu_states();
    }

    /// `ResetNavControls()`.
    pub(super) fn reset_nav_controls(&self) {
        let n = self.list.sel_count();
        let first = self.list.first_selected();
        let f = self.f();
        f.set_visible("PrevBtn", first.is_some_and(|i| i > 0));
        f.set_visible("NextBtn", first.is_some_and(|i| i + 1 < self.list.len()));
        f.set_visible("DeleteBtn", n > 1 || matches!(self.selected.get(), Some(RowKey::Entry(_))));
    }

    /// `ResetDbOpenControls()`.
    pub(super) fn reset_db_open_controls(&self) {
        let open = self.is_db_open();
        let f = self.f();
        self.list.set_enabled(open);
        f.set_enabled("EditPanel", open && self.list.sel_count() > 0);
        let ro = self.db_read_only.get();
        f.set_enabled("SaveBtn", open && !ro);
        f.set_enabled("LockBtn", open || self.locked.get());
        for name in ["AddEntryBtn", "SearchBox", "ClearSearchBtn", "SearchBtn"] {
            f.set_enabled(name, open);
        }
        self.update_menu_states();
    }

    /// Enabled state of the menu items (`ResetDbOpenControls()`,
    /// `MainMenu_EditClick()`, `DbViewMenuPopup()`, `SetItemChanged()`).
    pub(super) fn update_menu_states(&self) {
        let open = self.is_db_open();
        let locked = self.locked.get();
        let ro = self.db_read_only.get();
        let changed = self.item_changed.get();
        self.set_enabled("MainMenu_File_Close", open || locked);
        self.set_enabled("MainMenu_File_Lock", open || locked);
        self.set_enabled("MainMenu_File_Save", open && !ro);
        for n in [
            "MainMenu_File_SaveAs",
            "MainMenu_File_Export_CsvFile",
            "MainMenu_File_DbSettings",
            "MainMenu_File_Properties",
            "MainMenu_File_ChangeMasterPassword",
            "MainMenu_File_SetRecoveryPassword",
        ] {
            self.set_enabled(n, open);
        }
        let is_entry = matches!(self.selected.get(), Some(RowKey::Entry(_)));
        let n_sel = self.list.sel_count();
        let ext = is_entry || n_sel >= 2;
        let db_len = self.db.borrow().as_ref().map(|d| d.len()).unwrap_or(0);
        let rearrange = ext && self.sort_by_idx.get() < 0 && n_sel < db_len;
        let edit_ok = open && !changed;
        for p in ["MainMenu_Edit_", "DbViewMenu_"] {
            for n in ["CopyUserName", "CopyPassw", "OpenUrl", "Run", "PerformAutotype"] {
                self.set_enabled(&format!("{p}{n}"), edit_ok && is_entry);
            }
            for n in ["DuplicateEntry", "DeleteEntry"] {
                self.set_enabled(&format!("{p}{n}"), edit_ok && ext);
            }
            for n in ["Rearrange_Top", "Rearrange_Up", "Rearrange_Down", "Rearrange_Bottom"] {
                self.set_enabled(&format!("{p}{n}"), edit_ok && rearrange);
            }
            self.set_enabled(&format!("{p}SelectAll"), edit_ok);
        }
        self.set_enabled("MainMenu_Edit_AddEntry", edit_ok && self.search_mode.get() == SEARCH_MODE_OFF && !self.f().visible("FilterInfoPanel"));
        let view_ok = !changed;
        for i in 0..NUM_FIELDS {
            self.set_enabled(&format!("dyn:col{i}"), view_ok);
            self.set_enabled(&format!("dyn:sort{i}"), view_ok);
        }
        for n in [
            "MainMenu_View_ShowPasswInList",
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
            "MainMenu_View_ChangeListFont",
            "MainMenu_View_ChangePasswFont",
            "MainMenu_View_ResetFonts",
        ] {
            self.set_enabled(n, view_ok);
        }
    }

    /// `ChangeCaption()`.
    pub(super) fn change_caption(&self) {
        let mut s = String::new();
        if self.is_db_open() || self.locked.get() {
            if self.db_changed.get() {
                s.push('*');
            }
            let f = self.db_file_name.borrow();
            s.push_str(&if f.is_empty() { trl("Untitled") } else { file_name_of(&f) });
            if self.db_read_only.get() {
                s.push_str(" (R)");
            }
            if self.locked.get() {
                s.push_str(&format!(" ({})", trl("Locked")));
            } else {
                let exp = self.db.borrow().as_ref().map(|d| d.master_passw_expiry_date).unwrap_or(0);
                if exp != 0 {
                    let (cur, _) = helpers::expiry_check_dates(self.db_config().warn_expire_num_days);
                    if cur >= exp {
                        s.push_str(&format!(" - {}", trl("WARNING: Master password expired!")));
                    }
                }
            }
            s.push_str(" - ");
        }
        s.push_str(PASSW_MANAGER_NAME);
        self.set_title(&s);
    }

    /// `SetDbChanged()`.
    pub(super) fn set_db_changed(&self, changed: bool, entry_changed: bool) {
        let mut changed = changed;
        let c = self.db_config();
        let file = self.db_file_name.borrow().clone();
        if changed && c.auto_save && !file.is_empty() && (c.auto_save_option == 1 || (c.auto_save_option == 0 && entry_changed)) && self.save_database(&file) {
            changed = false;
        }
        if changed != self.db_changed.get() {
            self.db_changed.set(changed);
            self.change_caption();
        }
    }

    // ------------------------------------------------------------------
    // edit box handlers

    /// `TitleBoxChange()`.
    pub(super) fn title_box_change(&self) {
        if self.selected.get().is_some() && !self.item_changed.get() {
            self.notify_user_action();
            self.set_item_changed(true);
        }
    }

    /// `PasswBoxChange()`.
    pub(super) fn passw_box_change(&self) {
        if self.selected.get().is_some() {
            if !self.item_changed.get() {
                self.notify_user_action();
                self.set_item_changed(true);
            }
            self.item_passw_change_confirm.set(!self.passw_box_gen.get());
            if self.f().checked("PasswQualityBtn") {
                self.estimate_passw_quality(None);
            }
        }
    }

    /// `EstimatePasswQuality()`.
    pub(super) fn estimate_passw_quality(&self, passw: Option<&str>) {
        let mut chars = 0;
        let mut bits = 0.0;
        let text;
        let p = match passw {
            Some(p) => Some(p),
            None => {
                text = self.f().secret_text("PasswBox");
                if text.is_empty() { None } else { Some(text.as_str()) }
            }
        };
        if let Some(p) = p {
            chars = p.chars().count();
            let adv = self.main.upgrade().is_some_and(|m| m.core.borrow().use_advanced_passw_est);
            bits = if adv { pwtool_core::zxcvbn::zxcvbn_match(p) } else { PasswordGenerator::estimate_passw_security(p) };
        }
        self.passw_entropy_bits.set(bits);
        let caption =
            if chars > 0 { format!("{} / {}", trl_format("%1 ch.", &[&chars.to_string()]), floor_entropy_bits(bits)) } else { "0".to_string() };
        self.f().set_text("PasswSecurityLbl", &caption);
        self.passw_bar.set_quality(bits);
    }

    /// `PasswQualityBtnClick()`.
    pub(super) fn passw_quality_btn_click(&self) {
        let on = self.f().checked("PasswQualityBtn");
        self.f().set_visible("PasswGauge", on);
        let caption = if on { String::new() } else { trl("Password quality estimation is disabled") };
        self.f().set_control_caption("PasswSecurityBarPanel", &caption);
        if on {
            self.estimate_passw_quality(None);
        } else {
            self.f().set_text("PasswSecurityLbl", "");
        }
    }

    /// `TogglePasswBtnClick()`.
    pub(super) fn toggle_passw_btn_click(&self) {
        self.notify_user_action();
        let hide = self.f().checked("TogglePasswBtn");
        self.f().set_password_char("PasswBox", hide);
    }

    /// `TogglePasswBtnMouseUp()` with the right button: generate a password.
    pub(super) fn toggle_passw_btn_right_click(self: &Rc<Self>) {
        self.notify_user_action();
        let Some(main) = self.main.upgrade() else { return };
        let mut gen_main = true;
        if let Some(RowKey::Entry(id)) = self.selected.get() {
            let (profile, format) = {
                let dbb = self.db.borrow();
                let e = dbb.as_ref().and_then(|db| db.index_of_id(id).map(|i| db.entry(i)));
                (
                    e.and_then(|e| e.key_value(DB_KEYVAL_KEYS[DB_KEYVAL_PROFILE]).map(String::from)),
                    e.and_then(|e| e.key_value(DB_KEYVAL_KEYS[DB_KEYVAL_FORMATPASSW]).map(|s| Zeroizing::new(s.to_string()))),
                )
            };
            if let Some(p) = profile {
                if !main.load_profile_by_name(&p) {
                    udlg::msg(win_id(), &trl_format("Profile \"%1\" not found.", &[&p]), MsgIcon::Error);
                    return;
                }
            } else if let Some(f) = format {
                if let Some(passw) = format_passw(Some(&main), &f) {
                    self.set_generated_passw(&passw);
                }
                gen_main = false;
            }
        }
        if gen_main {
            let w = Rc::downgrade(self);
            main.generate_passw_for(Rc::new(move |p: &str| {
                if let Some(m) = w.upgrade() {
                    m.set_generated_passw(p);
                }
            }));
        }
        self.f().focus("PasswBox");
    }

    /// Put a generated password into the password box
    /// (`PASSWBOX_TAG_PASSW_GEN`: no confirmation needed).
    pub(super) fn set_generated_passw(&self, p: &str) {
        self.passw_box_gen.set(true);
        self.f().set_text("PasswBox", p);
        self.passw_box_change();
        self.passw_box_gen.set(false);
    }

    /// `ToggleNotesBtnClick()`.
    pub(super) fn toggle_notes_btn_click(&self) {
        let hide = self.f().checked("ToggleNotesBtn");
        self.notes_tag.set(if hide { NOTES_TAG_HIDDEN } else { 0 });
        if self.selected.get().is_some() && !self.f().has_focus("NotesBox") {
            if hide {
                self.notes_box_exit();
            } else if let Some(n) = self.temp_notes.borrow_mut().take() {
                self.f().set_text("NotesBox", &n);
            } else if let Some(RowKey::Entry(id)) = self.selected.get() {
                let notes = self.entry_notes(id);
                self.f().set_text("NotesBox", &notes);
            }
        }
    }

    pub(super) fn entry_notes(&self, id: u32) -> Zeroizing<String> {
        let dbb = self.db.borrow();
        Zeroizing::new(dbb.as_ref().and_then(|db| db.index_of_id(id).map(|i| crlf_to_lf(db.entry(i).string(FieldType::Notes)))).unwrap_or_default())
    }

    /// `NotesBoxEnter()`.
    pub(super) fn notes_box_enter(&self) {
        if self.selected.get().is_none() {
            return;
        }
        if let Some(n) = self.temp_notes.borrow_mut().take() {
            self.f().set_text("NotesBox", &n);
        } else if self.notes_tag.get() == NOTES_TAG_HIDDEN {
            if let Some(RowKey::Entry(id)) = self.selected.get() {
                let notes = self.entry_notes(id);
                self.f().set_text("NotesBox", &notes);
            }
        }
    }

    /// `NotesBoxExit()`.
    pub(super) fn notes_box_exit(&self) {
        if self.selected.get().is_some() && self.notes_tag.get() == NOTES_TAG_HIDDEN {
            *self.temp_notes.borrow_mut() = Some(Zeroizing::new(self.f().text("NotesBox")));
            self.f().set_text("NotesBox", &self.notes_hidden_text);
        }
    }

    /// `ExpiryCheckClick()` (a click of the user).
    pub(super) fn expiry_check_click(&self) {
        if self.selected.get().is_none() {
            return;
        }
        self.notify_user_action();
        self.set_item_changed(true);
        let on = self.f().checked("ExpiryCheck");
        self.f().set_enabled("ExpiryDatePicker", on);
    }

    /// `AddTagBtnClick()`.
    pub(super) fn add_tag_btn_click(&self) {
        self.notify_user_action();
        if self.selected.get().is_none() {
            return;
        }
        if !self.tag_menu_items.borrow().is_empty() {
            let r = self.f().rect("AddTagBtn");
            self.f().popup_menu("TagMenu", Some(r.left_bottom()));
        } else {
            self.f().focus("TagsBox");
        }
    }

    /// `OnTagMenuItemClick()`.
    pub(super) fn on_tag_menu_item_click(&self, tag: &str) {
        self.notify_user_action();
        let cur = Zeroizing::new(self.f().text("TagsBox"));
        let mut s = Zeroizing::new(cur.trim_end_matches(' ').to_string());
        if !s.is_empty() {
            if !s.ends_with(',') && !s.ends_with(';') {
                s.push_str(", ");
            }
            s.push_str(tag);
        } else {
            s.push_str(tag);
        }
        self.f().set_text("TagsBox", &s);
        self.title_box_change();
    }

    // ------------------------------------------------------------------
    // navigation and editing

    /// `PrevBtnClick()`.
    pub(super) fn prev_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        let Some(i) = self.list.first_selected() else { return };
        if i == 0 {
            return;
        }
        self.list.select_only(i - 1);
        self.list.scroll_to(i - 1);
        self.list.focus();
        self.trigger_apply();
    }

    /// `NextBtnClick()`.
    pub(super) fn next_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        let Some(i) = self.list.first_selected() else { return };
        if i + 1 >= self.list.len() {
            return;
        }
        self.list.select_only(i + 1);
        self.list.scroll_to(i + 1);
        self.list.focus();
        self.trigger_apply();
    }

    /// `CancelBtnClick()`.
    pub(super) fn cancel_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        self.clear_edit_panel();
        let sel = self.selected.get();
        self.apply_db_view_item_selection(sel);
    }

    /// `MainMenu_Edit_AddEntryClick()`.
    pub(super) fn add_entry_click(self: &Rc<Self>) {
        self.notify_user_action();
        let n = self.list.len();
        if n > 0 && self.list.key_at(n - 1) == Some(RowKey::New) && !self.item_changed.get() {
            self.list.clear_selection();
            self.list.scroll_to(n - 1);
            self.list.select_only(n - 1);
            self.apply_pending.set(false);
            self.apply_db_view_item_selection(None);
            self.f().focus("TitleBox");
        }
    }

    /// `AddModifyBtnClick()`.
    pub(super) async fn add_modify_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        let Some(sel) = self.selected.get() else { return };
        let f = self.form.clone();
        if f.text("TitleBox").is_empty() && f.secret_text("PasswBox").is_empty() {
            info(win_id(), &trl("Please specify at least a title or password."), MsgIcon::Warning).await;
            return;
        }
        let passw = f.secret_text("PasswBox");
        if !passw.is_empty() && self.item_passw_change_confirm.get() && f.checked("TogglePasswBtn") {
            let dlg = self.passw_enter();
            let r = dlg.execute(PASSWENTER_FLAG_ALLOWSKIP, &trl("Confirm password"), win_id()).await;
            match r {
                EnterResult::Ok => {
                    let same = *dlg.get_passw(0) == *passw;
                    dlg.clear_and_flush();
                    if !same {
                        info(win_id(), &trl("Passwords are not identical."), MsgIcon::Error).await;
                        return;
                    }
                }
                EnterResult::Skip => dlg.clear_and_flush(),
                EnterResult::Cancel => {
                    dlg.clear_and_flush();
                    return;
                }
            }
        }
        let mut num_overlong = 0;
        let tags_text = Zeroizing::new(f.text("TagsBox"));
        let mut new_tags: BTreeMap<String, String> = BTreeMap::new();
        if !tags_text.is_empty() {
            for item in tags_text.split([',', ';']) {
                let t = item.trim_start_matches(' ');
                if t.is_empty() {
                    continue;
                }
                let mut u16_len = 0usize;
                let mut truncated = String::new();
                let mut over = false;
                for c in t.chars() {
                    let clen = c.len_utf16();
                    if u16_len + clen > DB_MAX_TAG_LEN {
                        over = true;
                        break;
                    }
                    truncated.push(c);
                    u16_len += clen;
                }
                if over {
                    num_overlong += 1;
                }
                let tag = truncated.trim_end_matches(' ').to_string();
                let ci = lower(&tag);
                match self.global_casei_tags.borrow().get(&ci) {
                    Some(g) => {
                        new_tags.entry(ci).or_insert_with(|| g.clone());
                    }
                    None => {
                        new_tags.entry(ci).or_insert(tag);
                    }
                }
            }
        }
        let expiry = if f.checked("ExpiryCheck") {
            let (y, m, d) = f.date("ExpiryDatePicker");
            encode_expiry_date(y, m, d)
        } else {
            0
        };
        let notes = match self.temp_notes.borrow_mut().take() {
            Some(n) => n,
            None => Zeroizing::new(f.text("NotesBox")),
        };
        {
            let mut dbb = self.db.borrow_mut();
            let Some(db) = dbb.as_mut() else { return };
            let idx = match sel {
                RowKey::Entry(id) => match db.index_of_id(id) {
                    Some(i) => i,
                    None => return,
                },
                RowKey::New => db.new_entry(),
            };
            let new_id = db.entry(idx).id();
            let old_passw = db.get_entry_passw(idx).unwrap_or_default();
            {
                let e = db.entry_mut(idx);
                e.set_string(FieldType::Title, &f.text("TitleBox"));
                e.set_string(FieldType::UserName, &f.text("UserNameBox"));
                e.set_string(FieldType::Url, &f.text("UrlBox"));
                e.set_string(FieldType::Keyword, &f.text("KeywordBox"));
                e.set_string(FieldType::Notes, &lf_to_crlf(&notes));
                if let Some(kv) = self.temp_key_val.borrow_mut().take() {
                    e.set_key_value_list(kv);
                }
                if let Some(h) = self.temp_passw_history.borrow_mut().take() {
                    e.passw_history_mut().adopt_from(&h);
                }
                let tags: BTreeSet<Tag> = new_tags.into_values().map(Tag).collect();
                e.set_tags(tags);
            }
            let passw_changed = *passw != *old_passw;
            if passw_changed {
                db.set_entry_passw(idx, &passw);
                db.entry_mut(idx).add_current_passw_to_history(old_passw);
            }
            let e = db.entry_mut(idx);
            e.passw_expiry_date = expiry;
            e.update_modification_time(passw_changed);
            let (cur_date, soon_date) = helpers::expiry_check_dates(self.db_config().warn_expire_num_days);
            e.user_flags &= !(DB_FLAG_EXPIRED | DB_FLAG_EXPIRES_SOON);
            if e.passw_expiry_date != 0 {
                if cur_date >= e.passw_expiry_date {
                    e.user_flags |= DB_FLAG_EXPIRED;
                } else if soon_date >= e.passw_expiry_date {
                    e.user_flags |= DB_FLAG_EXPIRES_SOON;
                }
            }
            self.selected.set(Some(RowKey::Entry(new_id)));
        }
        // in-place list row update when no tags/filter/search/sort are active
        let can_in_place = self.tags.borrow().is_empty()
            && self.filter.get() == FilterType::None
            && self.search_mode.get() == SEARCH_MODE_OFF
            && self.sort_by_idx.get() < 0
            && self.show_col_mask.get() != 0
            && self.db.borrow().as_ref().is_some_and(|db| {
                self.selected
                    .get()
                    .and_then(|k| match k {
                        RowKey::Entry(id) => db.index_of_id(id).map(|i| db.entry(i).tags().is_empty()),
                        _ => None,
                    })
                    .unwrap_or(false)
            });
        let mut done = false;
        if can_in_place {
            let row = {
                let dbb = self.db.borrow();
                let db = dbb.as_ref().unwrap();
                match self.selected.get() {
                    Some(RowKey::Entry(id)) => db.index_of_id(id).map(|i| self.make_row(db, i)),
                    _ => None,
                }
            };
            if let Some(row) = row {
                let key = row.key;
                match sel {
                    RowKey::Entry(id) => {
                        if let Some(pos) = self.list.index_of(&RowKey::Entry(id)) {
                            self.list.replace_row(pos, row);
                            self.list.select_only(pos);
                            done = true;
                        }
                    }
                    RowKey::New => {
                        if let Some(pos) = self.list.index_of(&RowKey::New) {
                            self.list.replace_row(pos, row);
                            self.list.append_row(self.new_entry_row());
                            if let Some(p) = self.list.index_of(&key) {
                                self.list.select_only(p);
                            }
                            done = true;
                        }
                    }
                }
            }
        }
        if !done {
            self.reset_list_view(RELOAD_TAGS);
        }
        self.set_item_changed(false);
        self.set_db_changed(true, true);
        if num_overlong > 0 {
            info(
                win_id(),
                &trl_format("The maximum tag length is %1 characters.\nLonger tags have been shortened.", &[&DB_MAX_TAG_LEN.to_string()]),
                MsgIcon::Warning,
            )
            .await;
        }
        self.apply_now();
        self.next_btn_click();
        self.apply_now();
    }

    /// `DeleteBtnClick()`.
    pub(super) async fn delete_click(self: &Rc<Self>) {
        self.notify_user_action();
        if self.list.sel_count() == 0 {
            return;
        }
        let _s = IdleSuspend::new(self);
        let ids: Vec<u32> = self
            .list
            .selected_keys()
            .iter()
            .filter_map(|k| match k {
                RowKey::Entry(id) => Some(*id),
                _ => None,
            })
            .collect();
        let text = if ids.len() == 1 { trl("Delete 1 entry?") } else { trl_format("Delete %1 entries?", &[&ids.len().to_string()]) };
        if ask(win_id(), &text, MsgIcon::Warning, MsgButtons::YesNo, true).await != MsgResult::Yes {
            return;
        }
        {
            let mut dbb = self.db.borrow_mut();
            let Some(db) = dbb.as_mut() else { return };
            for id in ids {
                if let Some(i) = db.index_of_id(id) {
                    db.delete_entry(i);
                }
            }
        }
        self.reset_list_view(RELOAD_TAGS);
        self.reset_nav_controls();
        self.set_db_changed(true, false);
    }

    /// `MainMenu_Edit_DuplicateEntryClick()`.
    pub(super) fn duplicate_entry_click(self: &Rc<Self>) {
        self.notify_user_action();
        if self.list.sel_count() == 0 {
            return;
        }
        let copy_str = format!(" - {}", trl_ctx("Copy", "noun"));
        {
            let mut dbb = self.db.borrow_mut();
            let Some(db) = dbb.as_mut() else { return };
            for k in self.list.selected_keys() {
                if let RowKey::Entry(id) = k {
                    if let Some(idx) = db.index_of_id(id) {
                        let title = format!("{}{}", db.entry(idx).string(FieldType::Title), copy_str);
                        if let Err(e) = db.duplicate_entry(idx, &title) {
                            udlg::msg(win_id(), &format!("{e}."), MsgIcon::Error);
                        }
                    }
                }
            }
        }
        self.set_db_changed(true, false);
        self.reset_list_view(RELOAD_TAGS);
    }

    /// `MoveDbEntries()`.
    pub(super) fn move_db_entries(self: &Rc<Self>, dir: i32) {
        self.notify_user_action();
        let mut changed = false;
        {
            let mut dbb = self.db.borrow_mut();
            let Some(db) = dbb.as_mut() else { return };
            let size = db.len();
            let mut j = 0;
            let ids: Vec<u32> = self
                .list
                .selected_keys()
                .iter()
                .filter_map(|k| match k {
                    RowKey::Entry(id) => Some(*id),
                    _ => None,
                })
                .collect();
            for id in ids {
                let Some(pos) = db.index_of_id(id) else { continue };
                let new_pos = match dir {
                    MOVE_TOP => {
                        j += 1;
                        Some(j - 1)
                    }
                    MOVE_UP if pos > 0 => Some(pos - 1),
                    MOVE_DOWN if pos + 1 < size => Some(pos + 1),
                    MOVE_BOTTOM => Some(size - 1),
                    _ => None,
                };
                if let Some(np) = new_pos {
                    if np != pos {
                        db.move_entry(pos, np);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.set_db_changed(true, false);
            self.reset_list_view(0);
        }
    }

    /// `DbViewDragDrop()`: move the selected entry to the position of the
    /// entry it was dropped on (to the end if dropped on "<New Entry>").
    pub(super) fn db_view_drag_drop(self: &Rc<Self>, target: usize) {
        self.notify_user_action();
        let Some(RowKey::Entry(id)) = self.selected.get() else { return };
        let Some(tkey) = self.list.key_at(target) else { return };
        if tkey == RowKey::Entry(id) {
            return;
        }
        {
            let mut dbb = self.db.borrow_mut();
            let Some(db) = dbb.as_mut() else { return };
            let Some(cur) = db.index_of_id(id) else { return };
            let new_pos = match tkey {
                RowKey::Entry(tid) => match db.index_of_id(tid) {
                    Some(i) => i,
                    None => return,
                },
                RowKey::New => db.len() - 1,
            };
            db.move_entry(cur, new_pos);
        }
        self.reset_list_view(0);
        self.set_db_changed(true, false);
    }

    pub(super) fn selected_entry_index(&self) -> Option<usize> {
        let Some(RowKey::Entry(id)) = self.selected.get() else { return None };
        self.db.borrow().as_ref().and_then(|db| db.index_of_id(id))
    }

    pub(super) fn copy_user_name_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let u = self.db.borrow().as_ref().map(|db| Zeroizing::new(db.entry(i).string(FieldType::UserName).to_string()));
        if let Some(u) = u.filter(|u| !u.is_empty()) {
            crate::ui::clipboard::set_data(&u);
        }
    }

    pub(super) fn copy_passw_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let p = self.db.borrow().as_ref().and_then(|db| db.get_entry_passw(i).ok());
        if let Some(p) = p.filter(|p| !p.is_empty()) {
            crate::ui::clipboard::set_data(&p);
        }
    }

    pub(super) fn open_url_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let u = self.db.borrow().as_ref().map(|db| db.entry(i).string(FieldType::Url).to_string());
        if let Some(u) = u.filter(|u| !u.is_empty()) {
            open_uri(&u);
        }
    }

    /// `MainMenu_Edit_RunClick()`: run the command of the "Run" key.
    pub(super) fn run_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let cmd = self.db.borrow().as_ref().and_then(|db| db.entry(i).key_value(DB_KEYVAL_KEYS[DB_KEYVAL_RUN]).map(String::from));
        if let Some(cmd) = cmd {
            // Linux: command line; Windows: ShellExecute as in the original
            #[cfg(unix)]
            let started = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn().is_ok();
            #[cfg(windows)]
            let started = pwtool_app::platform::sys::system_open(std::ffi::OsStr::new(&cmd));
            if !started {
                open_uri(&cmd);
            }
        }
    }

    // ------------------------------------------------------------------
    // dialogs of the edit panel

    /// `EditKeyValBtnClick()`.
    pub(super) async fn edit_key_val_btn_click(self: &Rc<Self>) {
        let Some(sel) = self.selected.get() else { return };
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        if self.temp_key_val.borrow().is_none() {
            let kv = match sel {
                RowKey::Entry(id) => {
                    self.db.borrow().as_ref().and_then(|db| db.index_of_id(id).map(|i| db.entry(i).key_value_list().to_vec())).unwrap_or_default()
                }
                RowKey::New => Vec::new(),
            };
            *self.temp_key_val.borrow_mut() = Some(kv);
        }
        let keys: Vec<String> = self.key_val_names.keys().cloned().collect();
        let names: Vec<String> = self.key_val_names.values().cloned().collect();
        let mut values = vec![Zeroizing::new(String::new()); keys.len()];
        if let Some(kv) = self.temp_key_val.borrow().as_ref() {
            for (k, v) in kv {
                if let Some(row) = keys.iter().position(|x| x == k.as_str()) {
                    values[row] = v.clone();
                }
            }
        }
        if let Some(new) = self.dialogs.key_val(&names, &values, win_id()).await {
            let mut list = Vec::new();
            for (i, v) in new.into_iter().enumerate() {
                if !v.is_empty() {
                    list.push((Zeroizing::new(keys[i].clone()), v));
                }
            }
            let s = self.build_transl_key_val_string(&list);
            *self.temp_key_val.borrow_mut() = Some(list);
            self.f().set_text("KeyValueListBox", &s);
            self.title_box_change();
        }
    }

    /// `PasswHistoryBtnClick()`.
    pub(super) async fn passw_history_btn_click(self: &Rc<Self>) {
        let Some(sel) = self.selected.get() else {
            self.notify_user_action();
            return;
        };
        if !self.is_db_open() {
            self.notify_user_action();
            return;
        }
        let _s = IdleSuspend::new(self);
        if self.temp_passw_history.borrow().is_none() {
            let h = {
                let dbb = self.db.borrow();
                let db = dbb.as_ref().unwrap();
                match sel {
                    RowKey::Entry(id) => db.index_of_id(id).map(|i| db.entry(i).passw_history().clone()).unwrap_or_default(),
                    RowKey::New => PasswHistory::new(db.default_max_passw_history_size, db.default_max_passw_history_size > 0),
                }
            };
            *self.temp_passw_history.borrow_mut() = Some(h);
        }
        let hist = self.temp_passw_history.borrow().clone().unwrap();
        if let Some(r) = self.dialogs.history(&hist, win_id()).await {
            if let Some(h) = self.temp_passw_history.borrow_mut().as_mut() {
                h.set_active(r.active);
                h.set_max_size(r.max_size);
                if r.cleared {
                    h.clear();
                }
            }
            self.set_item_changed(true);
        }
    }

    // ------------------------------------------------------------------
    // search

    /// `SearchBtnClick()`.
    pub(super) fn search_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        let text = self.f().text("SearchBox");
        if text.is_empty() && self.search_mode.get() != SEARCH_MODE_OFF {
            self.search_mode.set(SEARCH_MODE_OFF);
            self.reset_list_view(RELOAD_TAGS);
            self.apply_fonts();
            self.f().set_control_caption("SearchResultPanel", "");
        } else if !text.is_empty() {
            self.search_database(&text, self.search_flags.get(), self.menu_bool("SearchMenu_CaseSensitive"), self.menu_bool("SearchMenu_FuzzySearch"));
            // search history
            let mut items = self.f().items("SearchBox");
            items.retain(|x| !x.eq_ignore_ascii_case(&text));
            items.insert(0, text.clone());
            self.f().set_items("SearchBox", items);
            self.f().set_text("SearchBox", &text);
        }
    }

    pub(super) fn clear_search_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        if self.search_mode.get() != SEARCH_MODE_OFF || !self.f().text("SearchBox").is_empty() {
            self.f().set_text("SearchBox", "");
            self.search_btn_click();
        }
    }

    /// `SearchDatabase()`.
    pub(super) fn search_database(self: &Rc<Self>, text: &str, flags: u32, case_sensitive: bool, fuzzy: bool) {
        if !self.is_db_open() || text.is_empty() || flags == 0 {
            return;
        }
        let mut case_sensitive = case_sensitive;
        let mut fuzzy = fuzzy;
        if fuzzy {
            case_sensitive = false;
        }
        if text.encode_utf16().count() < 4 {
            fuzzy = false;
        }
        let pattern = if case_sensitive { text.to_string() } else { lower(text) };
        let pat16: Vec<u16> = pattern.encode_utf16().collect();
        self.search_mode.set(if fuzzy { SEARCH_MODE_FUZZY } else { SEARCH_MODE_NORMAL });
        let mut found = 0;
        {
            let mut dbb = self.db.borrow_mut();
            let db = dbb.as_mut().unwrap();
            for i in 0..db.len() {
                let mut score_found: Option<f32> = None;
                for fld in 0..NUM_STRING_FIELDS {
                    if flags & (1 << fld) == 0 {
                        continue;
                    }
                    let src: Zeroizing<String> = if fld == FieldType::Password as usize && !db.entry(i).has_plaintext_passw() {
                        db.get_entry_passw(i).unwrap_or_default()
                    } else {
                        Zeroizing::new(db.entry(i).strings[fld].to_string())
                    };
                    if src.is_empty() {
                        continue;
                    }
                    let src = if case_sensitive { src } else { Zeroizing::new(lower(&src)) };
                    let score = if fuzzy {
                        let s16: Zeroizing<Vec<u16>> = Zeroizing::new(src.encode_utf16().collect());
                        str_find_fuzzy(&s16, &pat16)
                    } else if src.contains(pattern.as_str()) {
                        1.0
                    } else {
                        0.0
                    };
                    if score >= 0.5 {
                        score_found = Some(score);
                        break;
                    }
                }
                let e = db.entry_mut(i);
                e.user_flags &= !DB_FLAG_FOUND;
                if let Some(s) = score_found {
                    e.user_flags |= DB_FLAG_FOUND;
                    e.user_tag = (s * 100.0) as i32;
                    found += 1;
                }
            }
        }
        self.apply_fonts();
        self.reset_list_view(RELOAD_TAGS);
        self.f().set_control_caption(
            "SearchResultPanel",
            &if found == 1 { trl("1 entry found.") } else { trl_format("%1 entries found.", &[&found.to_string()]) },
        );
        self.update_menu_states();
    }

    /// `DbViewColumnClick()`.
    pub(super) fn db_view_column_click(self: &Rc<Self>, field: usize) {
        self.notify_user_action();
        let factor = if field as i32 == self.sort_by_idx.get() {
            if self.sort_order_factor.get() > 0 { -1 } else { 1 }
        } else {
            1
        };
        self.sort_by_idx.set(field as i32);
        self.sort_order_factor.set(factor);
        self.update_view_checks();
        self.set_list_view_sort_flag();
        self.sort_list();
    }

    // ------------------------------------------------------------------
    // fonts

    pub(super) async fn change_font(self: &Rc<Self>, list: bool) {
        let _s = IdleSuspend::new(self);
        let fam = if list { fonts::PM_LIST } else { fonts::PM_PASSW };
        let cur = fonts::named_font(fam);
        let Some(f) = crate::ui::font_dialog::choose_font(win_id(), cur.as_ref(), !list).await else { return };
        fonts::set_named_font(&windows::ctx(), fam, Some(f));
        self.apply_fonts();
    }

    // ------------------------------------------------------------------
    // database: open, close, save, lock

    /// `AddPassw()`.
    pub(super) fn add_passw(self: &Rc<Self>, passw: &str, show_num_passw: bool, param: Option<&str>) -> usize {
        if passw.is_empty() || !self.is_db_open() {
            return 0;
        }
        if self.item_changed.get() {
            udlg::msg(win_id(), &trl("Please finish editing the current entry\nbefore adding new entries."), MsgIcon::Warning);
            return 0;
        }
        let mut n = 0;
        {
            let mut dbb = self.db.borrow_mut();
            let db = dbb.as_mut().unwrap();
            let normalized = Zeroizing::new(passw.replace("\r\n", "\n"));
            for line in normalized.split('\n') {
                if n >= 1000 {
                    break;
                }
                if line.is_empty() {
                    continue;
                }
                let idx = db.new_entry();
                db.set_entry_passw(idx, line);
                if let Some(p) = param.filter(|p| !p.is_empty()) {
                    db.entry_mut(idx).set_string(FieldType::Title, p);
                }
                n += 1;
            }
        }
        if n > 0 {
            self.set_db_changed(true, true);
            self.reset_list_view(RELOAD_TAGS);
        }
        if show_num_passw {
            self.spawn_with(move |m| async move {
                let _s = IdleSuspend::new(&m);
                info(win_id(), &trl_format("%1 entries with passwords have been added.", &[&n.to_string()]), MsgIcon::Info).await;
            });
        }
        n
    }

    /// `OpenDatabase()`.
    pub(super) async fn open_database(self: &Rc<Self>, flags: u32, file_name: &str) -> bool {
        if self.busy.get() {
            return false;
        }
        let _s = IdleSuspend::new(self);
        if flags & DB_OPEN_FLAG_UNLOCK == 0 && self.is_db_open() && self.ask_save_changes(0).await != ASK_SAVE_OK {
            return false;
        }
        let win = win_id();
        let mut pe_flags = PASSWENTER_FLAG_ENABLEKEYFILE;
        let mut file = file_name.to_string();
        let caption;
        if flags & DB_OPEN_FLAG_EXISTING != 0 {
            if flags & DB_OPEN_FLAG_UNLOCK != 0 {
                file = self.db_file_name.borrow().clone();
                caption = trl_format("Unlock %1", &[&file_name_of(&file)]);
            } else {
                if file.is_empty() {
                    let fl = file_filters(false);
                    let Some(p) = crate::vcl::dialogs::open_file(&trl("Open database"), None, &filter_refs(&fl)).await else { return false };
                    file = p.to_string_lossy().to_string();
                    if self.db.borrow().is_some() && lower(&file) == lower(&self.db_file_name.borrow()) {
                        info(win, &trl("The selected file is already opened."), MsgIcon::Warning).await;
                        return false;
                    }
                } else if !Path::new(&file).is_file() {
                    info(win, &trl_format("Could not find database file:\n\"%1\".", &[&file]), MsgIcon::Error).await;
                    return false;
                }
                caption = trl_format("Open %1", &[&file_name_of(&file)]);
            }
            pe_flags |= PASSWENTER_FLAG_DECRYPT;
        } else {
            caption = trl("Create new database");
            pe_flags |= PASSWENTER_FLAG_ENCRYPT | PASSWENTER_FLAG_CONFIRMPASSW | PASSWENTER_FLAG_ENABLEKEYFILECREATION;
        }
        self.busy.set(true);
        let dlg = self.passw_enter();
        let db = loop {
            if dlg.execute(pe_flags, &caption, win).await != EnterResult::Ok {
                dlg.clear_and_flush();
                self.busy.set(false);
                return false;
            }
            let mut db = PasswDatabase::new();
            let path = PathBuf::from(&file);
            let existing = flags & DB_OPEN_FLAG_EXISTING != 0;
            let header = if existing { db.read_header(&path) } else { Ok(()) };
            let key = header.and_then(|_| pdb::combine_key_sources(&dlg.get_passw_binary(db.passw_utf8), dlg.key_file_name().as_deref()));
            dlg.clear_and_flush();
            // key derivation and decryption take time: a worker thread
            let r = match key {
                Ok(key) => {
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let r = if existing { db.open(&key, &path) } else { db.create(&key) };
                        let _ = tx.send_blocking((db, r));
                    });
                    match rx.recv().await {
                        Ok((d, r)) => {
                            db = d;
                            r
                        }
                        Err(_) => {
                            db = PasswDatabase::new();
                            Err(DbError::Error("internal error".into()))
                        }
                    }
                }
                Err(e) => Err(e),
            };
            match r {
                Ok(()) => break db,
                Err(DbError::InvalidKey(m)) | Err(DbError::Error(m)) => {
                    info(win, &format!("{m}."), MsgIcon::Error).await;
                }
                Err(DbError::InvalidFormat(m)) => {
                    let v = db.version();
                    info(
                        win,
                        &trl_format(
                            "Database format error:\n%1.\nDatabase version: %2.%3\nHighest supported version: %4.%5",
                            &[&m, &(v >> 8).to_string(), &(v & 0xff).to_string(), &VERSION_HIGH.to_string(), &VERSION_LOW.to_string()],
                        ),
                        MsgIcon::Error,
                    )
                    .await;
                    self.busy.set(false);
                    return false;
                }
                Err(DbError::Io(e)) => {
                    info(win, &trl_format("Error while opening database file:\n%1.", &[&e.to_string()]), MsgIcon::Error).await;
                    self.busy.set(false);
                    return false;
                }
            }
        };
        self.busy.set(false);
        if self.db.borrow().is_some() {
            self.close_database_impl(0);
        }
        *self.db.borrow_mut() = Some(db);
        self.selected.set(None);
        self.search_mode.set(SEARCH_MODE_OFF);
        self.db_changed.set(false);
        self.item_changed.set(false);
        self.item_passw_change_confirm.set(false);
        self.locked.set(false);
        if flags & DB_OPEN_FLAG_UNLOCK == 0 {
            self.filter.set(FilterType::None);
            self.update_view_checks();
        }
        if flags & DB_OPEN_FLAG_EXISTING != 0 {
            self.reset_list_view(RELOAD_TAGS);
            *self.db_file_name.borrow_mut() = file.clone();
            self.update_recent_files();
            let ro = flags & DB_OPEN_FLAG_READONLY != 0 || std::fs::metadata(&file).map(|m| m.permissions().readonly()).unwrap_or(false);
            self.db_read_only.set(ro);
        } else {
            if self.show_col_mask.get() != 0 {
                self.list.set_rows(vec![self.new_entry_row()]);
            } else {
                self.list.clear();
            }
            self.db_file_name.borrow_mut().clear();
            self.update_recent_files();
            self.db_read_only.set(false);
        }
        self.f().set_menu_caption("MainMenu_File_Lock", &trl("Lock"));
        self.change_caption();
        self.reset_db_open_controls();
        self.set_recovery_key_dependencies();
        if flags & DB_OPEN_FLAG_UNLOCK != 0 {
            let tags = std::mem::take(&mut *self.lock_sel_tags.borrow_mut());
            if !tags.is_empty() {
                for i in tags {
                    if i < self.tag_view.len() {
                        self.tag_view.select_display_index(i);
                    }
                }
                self.tag_apply_pending.set(false);
                self.apply_tag_view_item_selection();
            }
            let idx = self.lock_sel_item_index.replace(-1);
            if idx >= 0 && (idx as usize) < self.list.len() {
                self.list.select_only(idx as usize);
                self.apply_pending.set(false);
                self.apply_db_view_item_selection(None);
                self.list.scroll_to(idx as usize);
                self.list.focus();
            }
        } else {
            let check = |flag: u32| self.db.borrow().as_ref().is_some_and(|db| db.entries().iter().any(|e| e.user_flags & flag != 0));
            let c = self.db_config();
            if c.warn_expired_entries
                && check(DB_FLAG_EXPIRED)
                && ask(win, &trl("The database contains expired entries.\nDo you want to filter these entries now?"), MsgIcon::Warning, MsgButtons::YesNo, true)
                    .await
                    == MsgResult::Yes
            {
                self.filter.set(FilterType::Expired);
                self.update_view_checks();
                self.reset_list_view(0);
            }
            if c.warn_entries_expire_soon
                && check(DB_FLAG_EXPIRES_SOON)
                && ask(
                    win,
                    &trl("The database contains entries that will expire soon.\nDo you want to filter these entries now?"),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    == MsgResult::Yes
            {
                self.filter.set(FilterType::ExpireSoon);
                self.update_view_checks();
                self.reset_list_view(0);
            }
            let exp = self.db.borrow().as_ref().map(|d| d.master_passw_expiry_date).unwrap_or(0);
            if exp != 0 {
                let (cur, soon) = helpers::expiry_check_dates(c.warn_expire_num_days);
                let mut m = String::new();
                if cur >= exp {
                    m = trl("The master password has expired!");
                } else if soon >= exp {
                    m = trl("The master password will expire soon.");
                }
                if !m.is_empty() {
                    m.push('\n');
                    m.push_str(&trl("Set a new master password via File |\nChange Master Password."));
                    info(win, &m, MsgIcon::Warning).await;
                }
            }
            if self.list.len() == 1 {
                self.list.select_only(0);
                self.trigger_apply();
            }
        }
        self.f().set_checked("TogglePasswBtn", true);
        self.toggle_passw_btn_click();
        self.notify_user_action();
        self.idle_timer_enabled.set(true);
        true
    }

    /// `AskSaveChanges()`.
    pub(super) async fn ask_save_changes(self: &Rc<Self>, lock: i32) -> i32 {
        let win = win_id();
        if self.item_changed.get() {
            match ask(win, &trl("The currently selected entry has been changed.\nDo you want to save the changes?"), MsgIcon::Question, MsgButtons::YesNoCancel, false)
                .await
            {
                MsgResult::Yes => Box::pin(self.add_modify_btn_click()).await,
                MsgResult::Cancel => return ASK_SAVE_CANCEL,
                _ => {}
            }
        }
        if self.db_changed.get() {
            if lock == DB_LOCK_AUTO && self.db_config().lock_auto_save {
                self.save_click().await;
                if self.db_changed.get() {
                    return ASK_SAVE_ERROR;
                }
            } else {
                match ask(win, &trl("The database has been changed.\nDo you want to save the changes?"), MsgIcon::Question, MsgButtons::YesNoCancel, false).await {
                    MsgResult::Yes => {
                        self.save_click().await;
                        if self.db_changed.get() {
                            return ASK_SAVE_ERROR;
                        }
                    }
                    MsgResult::No => {
                        if lock != 0 && self.db_file_name.borrow().is_empty() {
                            info(win, &trl("An untitled database cannot be locked."), MsgIcon::Warning).await;
                            return ASK_SAVE_ERROR;
                        }
                    }
                    _ => return ASK_SAVE_CANCEL,
                }
            }
        }
        ASK_SAVE_OK
    }

    /// `CloseDatabase()`.
    pub(super) async fn close_database(self: &Rc<Self>, force: bool, lock: i32) -> bool {
        if self.is_db_open() && !force {
            let _s = IdleSuspend::new(self);
            if self.ask_save_changes(lock).await != ASK_SAVE_OK {
                return false;
            }
        }
        self.close_database_impl(lock);
        true
    }

    pub(super) fn close_database_impl(self: &Rc<Self>, lock: i32) {
        self.lock_sel_item_index.set(-1);
        self.lock_sel_tags.borrow_mut().clear();
        if lock != 0 {
            if let Some(k) = self.selected.get() {
                if let Some(i) = self.list.index_of(&k) {
                    self.lock_sel_item_index.set(i as i32);
                }
            }
            let tags: Vec<usize> = self.tag_view.selected_display_indices().into_iter().filter(|&i| i >= 1).collect();
            *self.lock_sel_tags.borrow_mut() = tags;
        }
        self.cancel_pending();
        *self.db.borrow_mut() = None;
        self.set_item_changed(false);
        self.search_mode.set(SEARCH_MODE_OFF);
        self.passw_entropy_bits.set(0.0);
        self.tags.borrow_mut().clear();
        self.global_casei_tags.borrow_mut().clear();
        self.tag_filter.borrow_mut().clear();
        self.clear_list_view();
        self.f().set_completion("UserNameBox", Vec::new());
        self.apply_fonts();
        self.tag_view.clear();
        self.reset_db_open_controls();
        self.update_tag_menu(&[]);
        self.dialogs.clear_settings();
        if lock == 0 {
            if !self.db_config().open_last_db_on_startup {
                self.db_file_name.borrow_mut().clear();
            }
            self.locked.set(false);
            self.f().set_menu_caption("MainMenu_File_Lock", &trl("Lock"));
            self.f().set_items("SearchBox", Vec::new());
            self.f().set_text("SearchBox", "");
        }
        self.change_caption();
        self.f().set_control_caption("SearchResultPanel", "");
        self.f().set_visible("FilterInfoPanel", false);
        self.idle_timer_enabled.set(false);
        self.reset_db_open_controls();
        if self.db_config().clear_clip_close_lock {
            crate::ui::clipboard::clear_data(true);
        }
    }

    /// `LockOrUnlockDatabase()`.
    pub(super) async fn lock_or_unlock_database(self: &Rc<Self>, auto: bool) {
        if self.busy.get() {
            return;
        }
        if self.locked.get() {
            self.open_database(DB_OPEN_FLAG_EXISTING | DB_OPEN_FLAG_UNLOCK, "").await;
        } else if self.close_database(false, if auto { DB_LOCK_AUTO } else { DB_LOCK_MANUAL }).await {
            self.locked.set(true);
            self.unlock_tried.set(false);
            self.f().set_menu_caption("MainMenu_File_Lock", &trl("Unlock"));
            self.reset_db_open_controls();
            self.change_caption();
            if self.is_shown() {
                windows::ctx().send_viewport_cmd_to(win_id(), ViewportCommand::Minimized(true));
                self.last_viewport.set((true, false));
            }
        }
    }

    pub(super) fn create_backup_if_enabled(&self, path: &Path) -> Option<String> {
        let c = self.db_config();
        if !c.create_backup || !path.is_file() {
            return None;
        }
        const BACKUP_EXT: &str = ".bak";
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mut base = path.with_file_name(&stem).to_string_lossy().to_string();
        let backup = if c.number_backups {
            let max = c.max_num_backups;
            if max > 1 {
                base.push('_');
                let mut num = 1;
                let mut oldest_num = 0;
                let mut oldest_age: Option<std::time::SystemTime> = None;
                while num <= max {
                    let check = format!("{base}{num:03}{BACKUP_EXT}");
                    match std::fs::metadata(&check).and_then(|m| m.modified()) {
                        Ok(age) => {
                            if oldest_num == 0 || oldest_age.is_some_and(|o| age < o) {
                                oldest_age = Some(age);
                                oldest_num = num;
                            }
                        }
                        Err(_) => break,
                    }
                    num += 1;
                }
                if num > max {
                    num = oldest_num.max(1);
                }
                format!("{base}{num:03}{BACKUP_EXT}")
            } else {
                format!("{base}{BACKUP_EXT}")
            }
        } else {
            let t = format!("{base}_{}{BACKUP_EXT}", helpers::time_stamp_string(false));
            if Path::new(&t).exists() { format!("{base}_{}{BACKUP_EXT}", helpers::time_stamp_string(true)) } else { t }
        };
        if let Some(db) = self.db.borrow_mut().as_mut() {
            db.release_file();
        }
        if std::fs::copy(path, &backup).is_err() { Some(backup) } else { None }
    }

    /// `SaveDatabase()`.
    pub(super) fn save_database(&self, file: &str) -> bool {
        let path = Path::new(file);
        if let Some(backup) = self.create_backup_if_enabled(path) {
            udlg::msg(win_id(), &trl_format("Could not save backup file\n\"%1\".", &[&backup]), MsgIcon::Error);
        }
        let r = match self.db.borrow_mut().as_mut() {
            Some(db) => db.save(path),
            None => return false,
        };
        match r {
            Ok(()) => true,
            Err(e) => {
                udlg::msg(win_id(), &trl_format("Error while saving database file:\n%1.", &[&e.to_string()]), MsgIcon::Error);
                false
            }
        }
    }

    /// Modal-reporting version of `save_database` for Save / Save As.
    pub(super) async fn save_database_modal(&self, file: &str) -> bool {
        let path = Path::new(file);
        if let Some(backup) = self.create_backup_if_enabled(path) {
            info(win_id(), &trl_format("Could not save backup file\n\"%1\".", &[&backup]), MsgIcon::Error).await;
        }
        let r = match self.db.borrow_mut().as_mut() {
            Some(db) => db.save(path),
            None => return false,
        };
        match r {
            Ok(()) => true,
            Err(e) => {
                info(win_id(), &trl_format("Error while saving database file:\n%1.", &[&e.to_string()]), MsgIcon::Error).await;
                false
            }
        }
    }

    /// `MainMenu_File_SaveClick()`.
    pub(super) async fn save_click(self: &Rc<Self>) {
        self.notify_user_action();
        if !self.is_db_open() {
            return;
        }
        let file = self.db_file_name.borrow().clone();
        if file.is_empty() || self.db_read_only.get() {
            Box::pin(self.save_as_click()).await;
        } else if self.save_database_modal(&file).await {
            self.set_db_changed(false, false);
        }
    }

    /// `MainMenu_File_SaveAsClick()`.
    pub(super) async fn save_as_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let cur = self.db_file_name.borrow().clone();
        let name = if !cur.is_empty() { file_name_of(&cur) } else { format!("{}{PWDB_FILE_EXT}", trl("Untitled")) };
        let fl = file_filters(true);
        let Some(mut p) = crate::vcl::dialogs::save_file(&trl("Save database as"), None, Some(&name), &filter_refs(&fl)).await else { return };
        if p.extension().is_none() {
            p.set_extension("pwdb");
        }
        let file = p.to_string_lossy().to_string();
        self.save_as(&file).await;
    }

    /// Save under a new name (without the file dialog; also automation).
    pub(super) async fn save_as(self: &Rc<Self>, file: &str) -> bool {
        if self.save_database_modal(file).await {
            *self.db_file_name.borrow_mut() = file.to_string();
            self.update_recent_files();
            self.db_read_only.set(false);
            self.db_changed.set(false);
            self.set_db_changed(false, false);
            self.change_caption();
            self.reset_db_open_controls();
            true
        } else {
            false
        }
    }

    /// `UpdateRecentFiles()`.
    pub(super) fn update_recent_files(&self) {
        let file = self.db_file_name.borrow().clone();
        if !self.db_config().keep_recent_files || file.is_empty() {
            return;
        }
        let mut r = self.recent_files.borrow_mut();
        match r.iter().position(|f| lower(f) == lower(&file)) {
            None => {
                r.insert(0, file);
                r.truncate(MAX_NUM_RECENT_FILES);
            }
            Some(0) => return,
            Some(i) => {
                let f = r.remove(i);
                r.insert(0, f);
            }
        }
        drop(r);
        self.update_recent_files_menu();
    }

    // ------------------------------------------------------------------
    // master password, recovery password, settings, properties, export

    /// Parent for dialogs: the database settings dialog while it is shown
    /// (the password is requested from there), else the manager window.
    pub(super) fn dialog_parent(&self) -> ViewportId {
        if self.dialogs.settings_open() { dialogs::settings_id() } else { win_id() }
    }

    /// `RequestPasswAndCheck()`.
    pub(super) async fn request_passw_and_check(
        self: &Rc<Self>,
        request: &str,
        invalid: &str,
        check: impl Fn(&PasswDatabase, &[u8]) -> Result<bool, DbError>,
        utf8: bool,
    ) -> Option<Zeroizing<Vec<u8>>> {
        let dlg = self.passw_enter();
        let win = self.dialog_parent();
        while dlg.execute(PASSWENTER_FLAG_ENABLEKEYFILE, request, win).await == EnterResult::Ok {
            let key = pdb::combine_key_sources(&dlg.get_passw_binary(utf8), dlg.key_file_name().as_deref());
            dlg.clear_and_flush();
            match key {
                Ok(key) => {
                    let r = {
                        let dbb = self.db.borrow();
                        match dbb.as_ref() {
                            Some(db) => check(db, &key),
                            None => return None,
                        }
                    };
                    match r {
                        Ok(true) => return Some(key),
                        Ok(false) => info(win, invalid, MsgIcon::Error).await,
                        Err(e) => info(win, &format!("{e}."), MsgIcon::Error).await,
                    }
                }
                Err(e) => info(win, &format!("{e}."), MsgIcon::Error).await,
            }
        }
        dlg.clear_and_flush();
        None
    }

    /// `MainMenu_File_ChangeMasterPasswordClick()`.
    pub(super) async fn change_master_password_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let utf8 = self.db.borrow().as_ref().map(|d| d.passw_utf8).unwrap_or(true);
        let key = self
            .request_passw_and_check(
                &trl("Enter OLD master password"),
                &trl("Old master password is invalid."),
                |db, k| Ok(db.check_master_key(k)? || (db.has_recovery_key() && db.check_recovery_key(k)?)),
                utf8,
            )
            .await;
        if key.is_none() {
            return;
        }
        let dlg = self.passw_enter();
        if dlg
            .execute(
                PASSWENTER_FLAG_CONFIRMPASSW | PASSWENTER_FLAG_ENABLEKEYFILE | PASSWENTER_FLAG_ENABLEKEYFILECREATION | PASSWENTER_FLAG_PASSWQUALITY,
                &trl("Enter NEW master password"),
                win_id(),
            )
            .await
            == EnterResult::Ok
        {
            let (has_rk, cur_utf8) = self.db.borrow().as_ref().map(|d| (d.has_recovery_key(), d.passw_utf8)).unwrap_or((false, true));
            let utf8 = cur_utf8 || !has_rk;
            let key = pdb::combine_key_sources(&dlg.get_passw_binary(utf8), dlg.key_file_name().as_deref());
            dlg.clear_and_flush();
            let r = key.and_then(|key| {
                let mut dbb = self.db.borrow_mut();
                let db = dbb.as_mut().unwrap();
                let r = db.change_master_key(&key, 0, None, None);
                if r.is_ok() {
                    db.passw_utf8 = utf8;
                    db.master_passw_expiry_date = 0;
                }
                r
            });
            match r {
                Ok(()) => {
                    self.set_db_changed(true, false);
                    self.change_caption();
                    info(win_id(), &trl("Master password successfully changed."), MsgIcon::Info).await;
                }
                Err(e) => info(win_id(), &format!("{e}."), MsgIcon::Error).await,
            }
        } else {
            dlg.clear_and_flush();
        }
    }

    /// `MainMenu_File_SetRecoveryPasswordClick()`.
    pub(super) async fn set_recovery_password_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let (has_rk, utf8) = self.db.borrow().as_ref().map(|d| (d.has_recovery_key(), d.passw_utf8)).unwrap_or((false, true));
        let new_flags = PASSWENTER_FLAG_CONFIRMPASSW | PASSWENTER_FLAG_ENABLEKEYFILE | PASSWENTER_FLAG_ENABLEKEYFILECREATION | PASSWENTER_FLAG_PASSWQUALITY;
        let mut success = false;
        let win = win_id();
        if !has_rk {
            let Some(key) = self
                .request_passw_and_check(&trl("Enter master password"), &trl("Master password is invalid."), |db, k| db.check_master_key(k), utf8)
                .await
            else {
                return;
            };
            let dlg = self.passw_enter();
            if dlg.execute(new_flags, &trl("Enter recovery password"), win).await == EnterResult::Ok {
                let rk = pdb::combine_key_sources(&dlg.get_passw_binary(utf8), dlg.key_file_name().as_deref());
                dlg.clear_and_flush();
                let r = rk.and_then(|rk| self.db.borrow_mut().as_mut().unwrap().set_recovery_key(&key, &rk));
                match r {
                    Ok(()) => {
                        success = true;
                        info(win, &trl("Recovery password successfully set."), MsgIcon::Info).await;
                    }
                    Err(e) => info(win, &format!("{e}."), MsgIcon::Error).await,
                }
            } else {
                dlg.clear_and_flush();
            }
        } else {
            if self
                .request_passw_and_check(&trl("Enter recovery password"), &trl("Recovery password is invalid."), |db, k| db.check_recovery_key(k), utf8)
                .await
                .is_none()
            {
                return;
            }
            let dlg = self.passw_enter();
            if dlg.execute(new_flags, &trl("Enter NEW master password"), win).await == EnterResult::Ok {
                let key = pdb::combine_key_sources(&dlg.get_passw_binary(true), dlg.key_file_name().as_deref());
                dlg.clear_and_flush();
                let r = key.and_then(|key| {
                    let mut dbb = self.db.borrow_mut();
                    let db = dbb.as_mut().unwrap();
                    let r = db.remove_recovery_key(&key);
                    if r.is_ok() {
                        db.passw_utf8 = true;
                        db.master_passw_expiry_date = 0;
                    }
                    r
                });
                match r {
                    Ok(()) => {
                        success = true;
                        info(win, &trl("Recovery password successfully removed."), MsgIcon::Info).await;
                    }
                    Err(e) => info(win, &format!("{e}."), MsgIcon::Error).await,
                }
            } else {
                dlg.clear_and_flush();
            }
        }
        if success {
            self.set_db_changed(true, false);
            self.set_recovery_key_dependencies();
        }
    }

    /// `SetRecoveryKeyDependencies()`.
    pub(super) fn set_recovery_key_dependencies(&self) {
        let has = self.db.borrow().as_ref().is_some_and(|d| d.has_recovery_key());
        self.f()
            .set_menu_caption("MainMenu_File_SetRecoveryPassword", &trl(if has { "Remove Recovery Password..." } else { "Set Recovery Password..." }));
        self.dialogs.set_recovery_dependencies(has);
    }

    pub(super) fn current_settings(&self) -> Option<DbSettings> {
        let dbb = self.db.borrow();
        let db = dbb.as_ref()?;
        Some(DbSettings {
            default_user_name: db.default_user_name.clone(),
            passw_format_seq: db.passw_format_seq.clone(),
            description: db.description.clone(),
            default_expiry_days: db.default_passw_expiry_days,
            master_passw_expiry_date: db.master_passw_expiry_date,
            default_max_passw_history_size: db.default_max_passw_history_size,
            cipher_type: db.cipher_type() as u32,
            num_kdf_rounds: db.kdf_iterations(),
            compressed: db.compressed,
            compression_level: db.compression_level,
        })
    }

    /// `MainMenu_File_DbSettingsClick()`.
    pub(super) async fn db_settings_click(self: &Rc<Self>) {
        self.notify_user_action();
        let _s = IdleSuspend::new(self);
        let Some(old) = self.current_settings() else { return };
        let has_rk = self.db.borrow().as_ref().is_some_and(|d| d.has_recovery_key());
        let w = Rc::downgrade(self);
        let ok = self
            .dialogs
            .db_settings(
                &old,
                has_rk,
                win_id(),
                Rc::new(move |s| {
                    let w = w.clone();
                    Box::pin(async move {
                        match w.upgrade() {
                            Some(m) => m.apply_db_settings(s).await,
                            None => false,
                        }
                    })
                }),
            )
            .await;
        if ok && self.current_settings().is_some_and(|new| new != old) {
            self.set_db_changed(true, false);
            self.change_caption();
        }
    }

    /// `ApplyDbSettings()`.
    pub(super) async fn apply_db_settings(self: &Rc<Self>, s: DbSettings) -> bool {
        let (has_rk, kdf, utf8) = match self.db.borrow().as_ref() {
            Some(d) => (d.has_recovery_key(), d.kdf_iterations(), d.passw_utf8),
            None => return false,
        };
        if !has_rk && s.num_kdf_rounds != kdf {
            let Some(key) = self
                .request_passw_and_check(&trl("Enter master password again"), &trl("Master password is invalid."), |db, k| db.check_master_key(k), utf8)
                .await
            else {
                return false;
            };
            if !self.change_kdf_rounds(&key, s.num_kdf_rounds).await {
                return false;
            }
        }
        let mut dbb = self.db.borrow_mut();
        let Some(db) = dbb.as_mut() else { return false };
        db.default_user_name = s.default_user_name.clone();
        db.passw_format_seq = s.passw_format_seq.clone();
        db.description = s.description.clone();
        db.default_passw_expiry_days = s.default_expiry_days;
        db.default_max_passw_history_size = s.default_max_passw_history_size;
        db.master_passw_expiry_date = s.master_passw_expiry_date;
        if !db.has_recovery_key() {
            let _ = db.set_cipher_type(s.cipher_type as u8);
        }
        db.compressed = s.compressed;
        db.compression_level = s.compression_level;
        true
    }

    /// Change the number of KDF iterations in a worker thread, with a
    /// progress window after 1 s (cancellable).
    pub(super) async fn change_kdf_rounds(self: &Rc<Self>, key: &[u8], rounds: u32) -> bool {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let Some(mut db) = self.db.borrow_mut().take() else { return false };
        let cancel = Arc::new(AtomicBool::new(false));
        let key = Zeroizing::new(key.to_vec());
        let c2 = cancel.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let r = db.change_master_key(&key, rounds, Some(&c2), None);
            let _ = tx.send_blocking((db, r));
        });
        let start = Instant::now();
        let mut progress: Option<Rc<udlg::ProgressWindow>> = None;
        let parent = self.dialog_parent();
        let (db, r) = loop {
            match rx.try_recv() {
                Ok(v) => break v,
                Err(_) => {
                    pwtool_app::rt::sleep(Duration::from_millis(100)).await;
                    if progress.is_none() && start.elapsed().as_millis() >= 1000 {
                        progress = Some(udlg::ProgressWindow::new(parent, &trl("Changing number of KDF iterations"), &trl("Computing derived key ...")));
                    }
                    if let Some(p) = &progress {
                        if p.cancel.get() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        p.set_message(&trl("Computing derived key ..."));
                    }
                }
            }
        };
        if let Some(p) = progress {
            p.close();
        }
        *self.db.borrow_mut() = Some(db);
        if cancel.load(Ordering::Relaxed) {
            info(parent, &trl("Process was cancelled by the user"), MsgIcon::Error).await;
            return false;
        }
        if let Err(e) = r {
            info(parent, &format!("{e}."), MsgIcon::Error).await;
            return false;
        }
        true
    }

    /// `MainMenu_File_PropertiesClick()`.
    pub(super) async fn properties_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let mut props: [String; 11] = Default::default();
        let mut set = |p: DbProperty, v: &str| {
            if !v.is_empty() {
                props[p as usize] = v.to_string();
            }
        };
        let file = self.db_file_name.borrow().clone();
        if !file.is_empty() {
            let p = Path::new(&file);
            set(DbProperty::FileName, &file_name_of(&file));
            set(DbProperty::FilePath, &p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default());
            match std::fs::metadata(p) {
                Ok(md) => {
                    set(DbProperty::FileSize, &format!("{} {}", helpers::format_thousands(md.len()), trl("bytes")));
                    if let Ok(t) = md.created().or_else(|_| md.modified()) {
                        set(DbProperty::CreationTime, &pwtool_core::util::time_stamp_to_long_string(helpers::system_time_to_local_filetime(t)));
                    }
                    if let Ok(t) = md.modified() {
                        set(DbProperty::ModificationTime, &pwtool_core::util::time_stamp_to_long_string(helpers::system_time_to_local_filetime(t)));
                    }
                }
                Err(e) => udlg::msg(win_id(), &format!("{e}."), MsgIcon::Error),
            }
        }
        {
            let dbb = self.db.borrow();
            let db = dbb.as_ref().unwrap();
            let desc: String = db.description.to_string();
            if !desc.is_empty() {
                const MAX_LEN: usize = 100;
                let first = |line: &str| {
                    let n = line.chars().count();
                    let mut s: String = line.chars().take(MAX_LEN).collect::<String>().trim().to_string();
                    if !s.is_empty() && n > MAX_LEN {
                        s.push_str("...");
                    }
                    s
                };
                let d = match desc.find('\n') {
                    Some(0) => String::new(),
                    Some(pos) => first(desc[..pos].trim_end_matches('\r')),
                    None => first(&desc),
                };
                set(DbProperty::Description, &d);
            }
            let v = db.version();
            set(DbProperty::FormatVersion, &format!("{}.{} (PW: {})", v >> 8, v & 0xff, if db.passw_utf8 { "UTF-8" } else { "UTF-16" }));
            set(DbProperty::RecoveryKeySet, &trl(if db.has_recovery_key() { "Yes" } else { "No" }));
            set(DbProperty::NumEntries, &db.len().to_string());
            let expired = db.entries().iter().filter(|e| e.user_flags & DB_FLAG_EXPIRED != 0).count();
            set(DbProperty::NumExpiredEntries, &expired.to_string());
            set(DbProperty::NumTags, &self.global_casei_tags.borrow().len().to_string());
        }
        self.dialogs.db_prop(props, win_id()).await;
    }

    /// `MainMenu_File_Export_CsvFileClick()`.
    pub(super) async fn export_csv_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let fl = file_filters(true);
        // the CSV filter first (default filter of the original)
        let mut refs = filter_refs(&fl);
        refs.swap(0, 1);
        let Some(mut path) = crate::vcl::dialogs::save_file(&trl("Export to CSV file"), None, None, &refs).await else { return };
        if path.extension().is_none() {
            path.set_extension("csv");
        }
        let Some(mask) = self.dialogs.col_select(&self.ui_field_names, self.csv_col_mask.get(), false, win_id()).await else { return };
        self.csv_col_mask.set(mask);
        let (enc, newline) = {
            let main = self.mainw();
            let core = main.core.borrow();
            (core.file_encoding, core.newline.clone())
        };
        let r = self.db.borrow().as_ref().map(|db| db.export_to_csv(&path, mask, &self.ui_field_names, enc, &newline));
        match r {
            Some(Ok(())) => info(win_id(), &trl("Database successfully exported to CSV file."), MsgIcon::Info).await,
            Some(Err(e)) => info(win_id(), &trl_format("Error while creating CSV file:\n%1.", &[&e.to_string()]), MsgIcon::Error).await,
            None => {}
        }
    }
}
