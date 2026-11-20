//! Event handlers and database operations of the password manager.

#![allow(deprecated)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use pwtech_core::language::{trl, trl_ctx, trl_format};
use pwtech_core::passw::PasswordGenerator;
use pwtech_core::passw::database::{
    self as pdb, DbError, FieldType, NUM_FIELDS, NUM_STRING_FIELDS, PasswDatabase, PasswHistory, Tag, VERSION_HIGH,
    VERSION_LOW, decode_expiry_date, encode_expiry_date,
};
use pwtech_core::random::RandomPool;
use pwtech_core::unicode::{str_to_w32, w32_to_string_secure};
use pwtech_core::util::floor_entropy_bits;
use zeroize::Zeroizing;

use super::db_prop::DbProperty;
use super::db_settings::DbSettings;
use super::helpers::{self, crlf_to_lf, lf_to_crlf, lower, str_find_fuzzy, text_view_text, today, today_plus_days};
use super::list::{RowData, RowKey, TagKind};
use super::*;
use crate::ui::passw_enter::{
    EnterResult, PASSWENTER_FLAG_ALLOWSKIP, PASSWENTER_FLAG_CONFIRMPASSW, PASSWENTER_FLAG_DECRYPT,
    PASSWENTER_FLAG_ENABLEKEYFILE, PASSWENTER_FLAG_ENABLEKEYFILECREATION, PASSWENTER_FLAG_ENCRYPT,
    PASSWENTER_FLAG_PASSWQUALITY,
};
use crate::ui::util::{MsgButtons, MsgIcon, MsgResult, combo_entry, combo_text, msg, msg_box, open_uri};

/// Guard for `ControlTagOverrider(this, FORM_TAG_ITEM_SELECTED)`.
struct Filling<'a>(&'a PasswManager, bool);

impl<'a> Filling<'a> {
    fn new(m: &'a PasswManager) -> Self {
        let old = m.filling.replace(true);
        Filling(m, old)
    }
}

impl Drop for Filling<'_> {
    fn drop(&mut self) {
        self.0.filling.set(self.1);
    }
}

/// Format password generated with the main random pool.
fn format_passw(format: &str) -> Option<Zeroizing<String>> {
    let pg = PasswordGenerator::new();
    let f = str_to_w32(format);
    let r = pg.get_format_passw(&mut *RandomPool::global(), &f, 0, None, 16000, None);
    if r.passw.is_empty() { None } else { Some(w32_to_string_secure(&r.passw)) }
}

fn file_filters(csv: bool) -> gio::ListStore {
    let store = gio::ListStore::new::<gtk::FileFilter>();
    let f = gtk::FileFilter::new();
    f.set_name(Some(&format!("{} (*.pwdb)", trl("Password databases"))));
    f.add_pattern("*.pwdb");
    store.append(&f);
    if csv {
        let f = gtk::FileFilter::new();
        f.set_name(Some(&format!("{} (*.csv)", trl("CSV files"))));
        f.add_pattern("*.csv");
        store.append(&f);
    }
    let f = gtk::FileFilter::new();
    f.set_name(Some(&format!("{} (*.*)", trl("All files"))));
    f.add_pattern("*");
    store.append(&f);
    store
}

impl PasswManager {
    // ------------------------------------------------------------------
    // signal connections

    pub(super) fn connect_signals(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        let act = |name: &str, f: fn(&Rc<PasswManager>)| {
            let w = w.clone();
            if let Some(a) = self.action(name) {
                a.connect_activate(move |a, _| {
                    if let Some(s) = a.state() {
                        if let Some(v) = s.get::<bool>() {
                            a.set_state(&(!v).to_variant());
                        }
                    }
                    if let Some(m) = w.upgrade() {
                        f(&m);
                    }
                });
            }
        };
        act("MainMenu_File_New", |m| {
            m.notify_user_action();
            let m = m.clone();
            glib::spawn_future_local(async move {
                m.open_database(0, "").await;
            });
        });
        act("MainMenu_File_Open", |m| {
            m.notify_user_action();
            let m = m.clone();
            glib::spawn_future_local(async move {
                m.open_database(DB_OPEN_FLAG_EXISTING, "").await;
            });
        });
        act("MainMenu_File_OpenReadOnly", |m| {
            m.notify_user_action();
            let m = m.clone();
            glib::spawn_future_local(async move {
                m.open_database(DB_OPEN_FLAG_EXISTING | DB_OPEN_FLAG_READONLY, "").await;
            });
        });
        act("MainMenu_File_Close", |m| {
            m.notify_user_action();
            let m = m.clone();
            glib::spawn_future_local(async move {
                m.close_database(false, 0).await;
            });
        });
        act("MainMenu_File_Lock", |m| {
            m.notify_user_action();
            let m = m.clone();
            glib::spawn_future_local(async move { m.lock_or_unlock_database(false).await });
        });
        act("MainMenu_File_ClearRecentFiles", |m| {
            m.notify_user_action();
            m.recent_files.borrow_mut().clear();
            m.update_recent_files_menu();
        });
        act("MainMenu_File_Save", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.save_click().await });
        });
        act("MainMenu_File_SaveAs", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.save_as_click().await });
        });
        act("MainMenu_File_Export_CsvFile", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.export_csv_click().await });
        });
        act("MainMenu_File_DbSettings", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.db_settings_click().await });
        });
        act("MainMenu_File_ChangeMasterPassword", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.change_master_password_click().await });
        });
        act("MainMenu_File_SetRecoveryPassword", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.set_recovery_password_click().await });
        });
        act("MainMenu_File_Properties", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.properties_click().await });
        });
        act("MainMenu_File_Exit", |m| m.win.close());
        for name in ["MainMenu_Edit_CopyUserName", "DbViewMenu_CopyUserName"] {
            act(name, |m| m.copy_user_name_click());
        }
        for name in ["MainMenu_Edit_CopyPassw", "DbViewMenu_CopyPassw"] {
            act(name, |m| m.copy_passw_click());
        }
        for name in ["MainMenu_Edit_OpenUrl", "DbViewMenu_OpenUrl"] {
            act(name, |m| m.open_url_click());
        }
        for name in ["MainMenu_Edit_Run", "DbViewMenu_Run"] {
            act(name, |m| m.run_click());
        }
        for name in ["MainMenu_Edit_PerformAutotype", "DbViewMenu_PerformAutotype"] {
            act(name, |m| {
                m.notify_user_action();
                if let Some(i) = m.selected_entry_index() {
                    m.perform_autotype_for(i);
                }
            });
        }
        act("MainMenu_Edit_AddEntry", |m| m.add_entry_click());
        for name in ["MainMenu_Edit_DuplicateEntry", "DbViewMenu_DuplicateEntry"] {
            act(name, |m| m.duplicate_entry_click());
        }
        for name in ["MainMenu_Edit_DeleteEntry", "DbViewMenu_DeleteEntry"] {
            act(name, |m| {
                let m = m.clone();
                glib::spawn_future_local(async move { m.delete_click().await });
            });
        }
        // see the module documentation: actions follow the captions
        act("MainMenu_Edit_Rearrange_Down", |m| m.move_db_entries(MOVE_TOP));
        act("MainMenu_Edit_Rearrange_Top", |m| m.move_db_entries(MOVE_UP));
        act("MainMenu_Edit_Rearrange_Up", |m| m.move_db_entries(MOVE_DOWN));
        act("MainMenu_Edit_Rearrange_Bottom", |m| m.move_db_entries(MOVE_BOTTOM));
        act("DbViewMenu_Rearrange_Top", |m| m.move_db_entries(MOVE_TOP));
        act("DbViewMenu_Rearrange_Up", |m| m.move_db_entries(MOVE_UP));
        act("DbViewMenu_Rearrange_Down", |m| m.move_db_entries(MOVE_DOWN));
        act("DbViewMenu_Rearrange_Bottom", |m| m.move_db_entries(MOVE_BOTTOM));
        for name in ["MainMenu_Edit_SelectAll", "DbViewMenu_SelectAll"] {
            act(name, |m| {
                m.notify_user_action();
                m.list.select_all();
                m.trigger_apply();
            });
        }
        act("MainMenu_View_ShowPasswInList", |m| {
            m.notify_user_action();
            m.reset_list_view(0);
        });
        act("MainMenu_View_ChangeListFont", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.change_font(true).await });
        });
        act("MainMenu_View_ChangePasswFont", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.change_font(false).await });
        });
        act("MainMenu_View_ResetFonts", |m| {
            m.notify_user_action();
            *m.list_font.borrow_mut() = None;
            *m.passw_font.borrow_mut() = None;
            m.apply_fonts();
        });
        act("SearchMenu_CaseSensitive", |_| {});
        act("SearchMenu_FuzzySearch", |_| {});
        act("SearchMenu_SelectFields", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move {
                let _s = IdleSuspend::new(&m);
                if let Some(f) = m.col_dlg().execute(&m.ui_field_names, m.search_flags.get(), true).await {
                    m.search_flags.set(f);
                }
            });
        });
        for i in 0..NUM_FIELDS {
            let w = w.clone();
            if let Some(a) = self.action(&format!("col{i}")) {
                a.connect_activate(move |a, _| {
                    let v = a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
                    a.set_state(&(!v).to_variant());
                    if let Some(m) = w.upgrade() {
                        m.notify_user_action();
                        m.set_show_col_mask_from_actions();
                        m.reset_list_view(RESET_COLUMNS);
                    }
                });
            }
        }
        let radio = |name: &str, f: fn(&Rc<PasswManager>, &glib::Variant)| {
            let w = w.clone();
            if let Some(a) = self.action(name) {
                a.connect_activate(move |a, p| {
                    if let Some(p) = p {
                        a.set_state(p);
                        if let Some(m) = w.upgrade() {
                            f(&m, p);
                        }
                    }
                });
            }
        };
        radio("sortby", |m, p| {
            m.notify_user_action();
            let idx = p.get::<i32>().unwrap_or(-1);
            if idx != m.sort_by_idx.get() {
                m.sort_by_idx.set(idx);
                m.sort_list();
                m.set_list_view_sort_flag();
            }
        });
        radio("sortorder", |m, p| {
            m.notify_user_action();
            let f = p.get::<i32>().unwrap_or(1);
            if f != m.sort_order_factor.get() {
                m.sort_order_factor.set(f);
                if m.sort_by_idx.get() >= 0 {
                    m.sort_list();
                }
                m.set_list_view_sort_flag();
            }
        });
        radio("tagsortby", |m, p| {
            m.notify_user_action();
            let idx = p.get::<i32>().unwrap_or(0);
            if idx != m.tags_sort_by_idx.get() {
                m.tags_sort_by_idx.set(idx);
                m.tag_view.set_sort(idx, m.tags_sort_order_factor.get());
            }
        });
        radio("tagsortorder", |m, p| {
            m.notify_user_action();
            let f = p.get::<i32>().unwrap_or(1);
            if f != m.tags_sort_order_factor.get() {
                m.tags_sort_order_factor.set(f);
                m.tag_view.set_sort(m.tags_sort_by_idx.get(), f);
            }
        });
        radio("filter", |m, p| {
            m.filter.set(FilterType::from_id(&p.get::<String>().unwrap_or_default()));
            m.reset_list_view(0);
        });
        if let Some(a) = self.action("recent") {
            let w = w.clone();
            a.connect_activate(move |_, p| {
                let (Some(m), Some(i)) = (w.upgrade(), p.and_then(|p| p.get::<i32>())) else { return };
                m.notify_user_action();
                let Some(file) = m.recent_files.borrow().get(i as usize).cloned() else { return };
                if !m.is_db_open() || lower(&file) != lower(&m.db_file_name.borrow()) {
                    glib::spawn_future_local(async move {
                        m.open_database(DB_OPEN_FLAG_EXISTING, &file).await;
                    });
                }
            });
        }
        if let Some(a) = self.action("addtag") {
            let w = w.clone();
            a.connect_activate(move |_, p| {
                let (Some(m), Some(t)) = (w.upgrade(), p.and_then(|p| p.get::<String>())) else { return };
                m.on_tag_menu_item_click(&t);
            });
        }
        if let Some(a) = self.action("expirydays") {
            let w = w.clone();
            a.connect_activate(move |_, p| {
                let (Some(m), Some(d)) = (w.upgrade(), p.and_then(|p| p.get::<i32>())) else { return };
                m.notify_user_action();
                let date = today_plus_days(d);
                if date != m.expiry_picker.date() {
                    m.expiry_picker.set_date(date);
                    m.set_item_changed(true);
                }
            });
        }

        // tool bar
        let click = |name: &str, f: fn(&Rc<PasswManager>)| {
            let w = w.clone();
            if let Some(b) = self.b.try_get::<gtk::Button>(name) {
                b.connect_clicked(move |_| {
                    if let Some(m) = w.upgrade() {
                        f(&m);
                    }
                });
            }
        };
        click("NewBtn", |m| {
            let _ = m.activate("MainMenu_File_New");
        });
        click("OpenBtn", |m| {
            let _ = m.activate("MainMenu_File_Open");
        });
        click("SaveBtn", |m| {
            let _ = m.activate("MainMenu_File_Save");
        });
        click("LockBtn", |m| {
            let _ = m.activate("MainMenu_File_Lock");
        });
        click("AddEntryBtn", |m| m.add_entry_click());
        click("ClearSearchBtn", |m| m.clear_search_btn_click());
        click("SearchBtn", |m| m.search_btn_click());
        click("ClearFilterBtn", |m| {
            m.filter.set(FilterType::None);
            if let Some(a) = m.action("filter") {
                a.set_state(&"".to_variant());
            }
            m.reset_list_view(0);
        });
        if let Some(cb) = self.b.try_get::<gtk::ComboBoxText>("SearchBox") {
            if let Some(e) = combo_entry(&cb) {
                let w = w.clone();
                e.connect_activate(move |_| {
                    if let Some(m) = w.upgrade() {
                        m.notify_user_action();
                        m.search_btn_click();
                    }
                });
            }
            let w = w.clone();
            cb.connect_changed(move |cb| {
                // `SearchBoxSelect`: an item was chosen from the list
                if cb.active().is_some() {
                    if let Some(m) = w.upgrade() {
                        if m.filling.get() {
                            return;
                        }
                        m.notify_user_action();
                        let text = combo_text(cb);
                        m.search_database(&text, m.search_flags.get(), m.action_bool("SearchMenu_CaseSensitive"), m.action_bool("SearchMenu_FuzzySearch"));
                    }
                }
            });
        }

        // edit panel
        click("AddModifyBtn", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.add_modify_btn_click().await });
        });
        click("PrevBtn", |m| m.prev_btn_click());
        click("NextBtn", |m| m.next_btn_click());
        click("CancelBtn", |m| m.cancel_btn_click());
        click("DeleteBtn", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.delete_click().await });
        });
        click("AddTagBtn", |m| m.add_tag_btn_click());
        click("EditKeyValBtn", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.edit_key_val_btn_click().await });
        });
        click("ExpiryBtn", |m| {
            m.notify_user_action();
            if m.selected.get().is_some() && m.b.get::<gtk::CheckButton>("ExpiryCheck").is_active() {
                m.expiry_popover.popup();
            }
        });
        click("UrlBtn", |m| {
            m.notify_user_action();
            let url = m.entry("UrlBox").text().to_string();
            open_uri(Some(&m.gwin()), &url);
        });
        click("PasswHistoryBtn", |m| {
            let m = m.clone();
            glib::spawn_future_local(async move { m.passw_history_btn_click().await });
        });
        let tb = self.toggle("TogglePasswBtn");
        let ww = w.clone();
        tb.connect_toggled(move |_| {
            if let Some(m) = ww.upgrade() {
                m.toggle_passw_btn_click();
            }
        });
        let g = gtk::GestureClick::new();
        g.set_button(gdk::BUTTON_SECONDARY);
        let ww = w.clone();
        g.connect_released(move |_, _, _, _| {
            if let Some(m) = ww.upgrade() {
                m.toggle_passw_btn_right_click();
            }
        });
        tb.add_controller(g);
        let ww = w.clone();
        self.toggle("PasswQualityBtn").connect_toggled(move |_| {
            if let Some(m) = ww.upgrade() {
                m.notify_user_action();
                m.passw_quality_btn_click();
            }
        });
        let ww = w.clone();
        self.toggle("ToggleNotesBtn").connect_toggled(move |_| {
            if let Some(m) = ww.upgrade() {
                m.notify_user_action();
                m.toggle_notes_btn_click();
            }
        });
        for name in ["TitleBox", "UserNameBox", "UrlBox", "KeywordBox", "TagsBox", "KeyValueListBox"] {
            let ww = w.clone();
            self.entry(name).connect_changed(move |_| {
                if let Some(m) = ww.upgrade() {
                    m.title_box_change();
                }
            });
        }
        for name in ["TitleBox", "UserNameBox", "PasswBox", "UrlBox", "KeywordBox", "TagsBox"] {
            let ww = w.clone();
            self.entry(name).connect_activate(move |_| {
                if let Some(m) = ww.upgrade() {
                    if m.selected.get().is_some() {
                        glib::spawn_future_local(async move { m.add_modify_btn_click().await });
                    }
                }
            });
        }
        let ww = w.clone();
        self.entry("PasswBox").connect_changed(move |_| {
            if let Some(m) = ww.upgrade() {
                m.passw_box_change();
            }
        });
        let ww = w.clone();
        self.notes_view().buffer().connect_changed(move |_| {
            if let Some(m) = ww.upgrade() {
                if m.notes_tag.get() != NOTES_TAG_CHANGING {
                    m.title_box_change();
                }
            }
        });
        let fc = gtk::EventControllerFocus::new();
        let ww = w.clone();
        fc.connect_enter(move |_| {
            if let Some(m) = ww.upgrade() {
                m.notes_box_enter();
            }
        });
        let ww = w.clone();
        fc.connect_leave(move |_| {
            if let Some(m) = ww.upgrade() {
                m.notes_box_exit();
            }
        });
        self.notes_view().add_controller(fc);
        let ww = w.clone();
        self.b.get::<gtk::CheckButton>("ExpiryCheck").connect_toggled(move |_| {
            if let Some(m) = ww.upgrade() {
                m.expiry_check_click();
            }
        });
        let ww = w.clone();
        self.expiry_picker.connect_changed(move || {
            if let Some(m) = ww.upgrade() {
                m.notify_user_action();
                m.set_item_changed(true);
            }
        });
        // drag the contents of edit boxes from their labels
        for (lbl, src) in [
            ("TitleLbl", "TitleBox"),
            ("UserNameLbl", "UserNameBox"),
            ("PasswLbl", "PasswBox"),
            ("UrlLbl", "UrlBox"),
            ("KeywordLbl", "KeywordBox"),
            ("PasswSecurityLbl", "PasswBox"),
            ("PasswSecurityBarPanel", "PasswBox"),
        ] {
            let ds = gtk::DragSource::new();
            let e = self.entry(src);
            ds.connect_prepare(move |_, _, _| {
                let t = e.text();
                if t.is_empty() { None } else { Some(gdk::ContentProvider::for_value(&t.to_value())) }
            });
            self.b.w(lbl).add_controller(ds);
        }
        let ds = gtk::DragSource::new();
        let ww = w.clone();
        ds.connect_prepare(move |_, _, _| {
            let m = ww.upgrade()?;
            let t = text_view_text(&m.notes_view());
            if t.is_empty() { None } else { Some(gdk::ContentProvider::for_value(&t.to_value())) }
        });
        self.b.w("NotesLbl").add_controller(ds);

        // entry list
        let ww = w.clone();
        self.list.selection.connect_selection_changed(move |_, _, _| {
            if let Some(m) = ww.upgrade() {
                m.trigger_apply();
            }
        });
        let ww = w.clone();
        self.list.connect_context(move |x, y| {
            if let Some(m) = ww.upgrade() {
                m.notify_user_action();
                m.apply_now();
                m.update_menu_states();
                m.db_view_popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                m.db_view_popover.popup();
            }
        });
        let ww = w.clone();
        let wd = w.clone();
        self.list.connect_drag_drop(
            move || {
                // `DbViewMouseMove`: only unsorted, single real entry
                ww.upgrade().is_some_and(|m| {
                    m.sort_by_idx.get() < 0
                        && m.list.sel_count() == 1
                        && matches!(m.selected.get(), Some(RowKey::Entry(_)))
                        && !m.item_changed.get()
                })
            },
            move |target| {
                if let Some(m) = wd.upgrade() {
                    m.db_view_drag_drop(target);
                }
            },
        );
        let ww = w.clone();
        self.list.connect_sort(move |field, asc| {
            if let Some(m) = ww.upgrade() {
                m.db_view_column_click(field, asc);
            }
        });
        let ww = w.clone();
        self.tag_view.flow.connect_selected_children_changed(move |_| {
            if let Some(m) = ww.upgrade() {
                if !m.tag_view.suppress.get() {
                    m.trigger_tag_apply();
                }
            }
        });

        // window: close, activation, minimize
        let ww = w.clone();
        self.win.connect_close_request(move |win| {
            let Some(m) = ww.upgrade() else { return glib::Propagation::Proceed };
            m.notify_user_action();
            if m.busy.get() {
                return glib::Propagation::Stop;
            }
            let win = win.clone();
            glib::spawn_future_local(async move {
                if m.close_database(false, 0).await {
                    // `FormClose`
                    if let Some(cb) = m.b.try_get::<gtk::ComboBoxText>("SearchBox") {
                        cb.remove_all();
                        crate::ui::util::set_combo_text(&cb, "");
                    }
                    win.set_visible(false);
                }
            });
            glib::Propagation::Stop
        });
        let ww = w.clone();
        self.win.connect_is_active_notify(move |win| {
            if win.is_active() {
                if let Some(m) = ww.upgrade() {
                    m.form_activate();
                }
            }
        });
        let ww = w.clone();
        self.win.connect_realize(move |win| {
            let Some(surface) = win.surface() else { return };
            let Ok(top) = surface.downcast::<gdk::Toplevel>() else { return };
            let ww = ww.clone();
            let prev = std::cell::Cell::new(top.state());
            top.connect_state_notify(move |t| {
                let Some(m) = ww.upgrade() else { return };
                let st = t.state();
                let was_min = prev.get().contains(gdk::ToplevelState::MINIMIZED);
                let is_min = st.contains(gdk::ToplevelState::MINIMIZED);
                prev.set(st);
                if is_min && !was_min {
                    // `OnWindowPosChanging`: lock when minimized
                    if m.is_db_open() && m.db_config().lock_minimize && !m.busy.get() {
                        let m = m.clone();
                        glib::spawn_future_local(async move { m.lock_or_unlock_database(false).await });
                    }
                } else if !is_min && was_min && m.locked.get() {
                    // `WMSize(SIZE_RESTORED)`
                    m.form_tag.set(FORM_TAG_UNLOCK_DB);
                }
            });
        });
        // any key press or click in the window counts as user action
        let kc = gtk::EventControllerKey::new();
        kc.set_propagation_phase(gtk::PropagationPhase::Capture);
        let ww = w.clone();
        kc.connect_key_pressed(move |_, _, _, _| {
            if let Some(m) = ww.upgrade() {
                m.notify_user_action();
            }
            glib::Propagation::Proceed
        });
        self.win.add_controller(kc);
    }

    fn activate(&self, name: &str) -> bool {
        if self.action(name).is_some_and(|a| a.is_enabled()) {
            gio::prelude::ActionGroupExt::activate_action(&self.b.actions, name, None);
            true
        } else {
            false
        }
    }

    /// Activate a menu action by name (for automated tests).
    pub fn activate_action(&self, name: &str) -> bool {
        self.activate(name)
    }

    /// `FormActivate()`.
    fn form_activate(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        if self.form_tag.get() == FORM_TAG_OPEN_DB {
            self.form_tag.set(0);
            let file = self.db_file_name.borrow().clone();
            let m = self.clone();
            glib::spawn_future_local(async move {
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
                let m = self.clone();
                glib::spawn_future_local(async move { m.lock_or_unlock_database(false).await });
            }
        }
        self.form_tag.set(0);
    }

    pub(super) fn start_idle_timer(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || {
            let Some(m) = w.upgrade() else { return glib::ControlFlow::Break };
            m.idle_timer_tick();
            glib::ControlFlow::Continue
        });
    }

    /// `IdleTimerTimer()`.
    fn idle_timer_tick(self: &Rc<Self>) {
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
            let m = self.clone();
            glib::spawn_future_local(async move { m.lock_or_unlock_database(true).await });
        }
    }

    // ------------------------------------------------------------------
    // selection handling (`TSelectItemThread`: coalesce selection events)

    pub(super) fn trigger_apply(self: &Rc<Self>) {
        if self.apply_pending.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            if let Some(m) = w.upgrade() {
                m.apply_pending.borrow_mut().take();
                m.apply_db_view_item_selection(None);
            }
        });
        *self.apply_pending.borrow_mut() = Some(id);
    }

    /// `ApplyNow()`.
    pub(super) fn apply_now(self: &Rc<Self>) {
        if let Some(id) = self.apply_pending.borrow_mut().take() {
            id.remove();
            self.apply_db_view_item_selection(None);
        }
    }

    fn cancel_pending(&self) {
        if let Some(id) = self.apply_pending.borrow_mut().take() {
            id.remove();
        }
        if let Some(id) = self.tag_apply_pending.borrow_mut().take() {
            id.remove();
        }
    }

    fn trigger_tag_apply(self: &Rc<Self>) {
        if self.tag_apply_pending.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            if let Some(m) = w.upgrade() {
                m.tag_apply_pending.borrow_mut().take();
                m.apply_tag_view_item_selection();
            }
        });
        *self.tag_apply_pending.borrow_mut() = Some(id);
    }

    /// `ApplyTagViewItemSelection()`.
    fn apply_tag_view_item_selection(self: &Rc<Self>) {
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

    pub(super) fn set_show_col_mask_from_actions(&self) {
        let mut mask = 0;
        for i in 0..NUM_FIELDS {
            if self.action_bool(&format!("col{i}")) {
                mask |= 1 << i;
            }
        }
        self.show_col_mask.set(mask);
    }

    /// `SetListViewSortFlag()`.
    pub(super) fn set_list_view_sort_flag(&self) {
        let idx = self.sort_by_idx.get();
        self.list.set_sort_indicator(if idx >= 0 { Some(idx as usize) } else { None }, self.sort_order_factor.get() > 0);
    }

    /// Row of an entry (`AddModifyListViewEntry()`).
    fn make_row(&self, db: &PasswDatabase, idx: usize) -> RowData {
        let e = db.entry(idx);
        let image = if e.user_flags & DB_FLAG_EXPIRED != 0 {
            0
        } else if e.user_flags & DB_FLAG_EXPIRES_SOON != 0 {
            1
        } else {
            -1
        };
        let show_passw = self.action_bool("MainMenu_View_ShowPasswInList");
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
                    let mut s: String = if notes.encode_utf16().count() > MAX_NOTES_LEN {
                        let mut t: String = notes.chars().take(MAX_NOTES_LEN - 3).collect();
                        t.push_str("...");
                        t
                    } else {
                        notes.to_string()
                    };
                    s = s.replace(['\r', '\n'], " ");
                    s
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
        RowData { key: RowKey::Entry(e.id()), cols, image }
    }

    fn new_entry_row(&self) -> RowData {
        let n = self.show_col_mask.get().count_ones() as usize;
        let mut cols = vec![String::new(); n];
        if n > 0 {
            cols[0] = format!("<{}>", trl("New Entry"));
        }
        RowData { key: RowKey::New, cols, image: -1 }
    }

    /// `DbViewCompare()`.
    fn compare_rows(&self, db: &PasswDatabase, a: &RowKey, b: &RowKey) -> std::cmp::Ordering {
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
            x if (0..NUM_STRING_FIELDS as i32).contains(&x) => {
                apply(helpers::wcsicmp(&e1.strings[x as usize], &e2.strings[x as usize]))
            }
            _ => Equal,
        }
    }

    /// Re-sort the list (`DbView->AlphaSort()`), keeping the selection.
    pub(super) fn sort_list(self: &Rc<Self>) {
        let n = self.list.len();
        if n < 2 {
            return;
        }
        let db = self.db.borrow();
        let Some(db) = db.as_ref() else { return };
        let mut rows: Vec<RowData> = (0..n)
            .filter_map(|i| self.list.store.item(i).and_downcast::<glib::BoxedAnyObject>())
            .map(|o| o.borrow::<RowData>().clone())
            .collect();
        let selected: Vec<RowKey> = self.list.selected_indices().iter().filter_map(|&i| self.list.key_at(i)).collect();
        rows.sort_by(|a, b| self.compare_rows(db, &a.key, &b.key));
        self.list.set_rows(rows);
        for k in selected {
            if let Some(i) = self.list.index_of(k) {
                self.list.selection.select_item(i, false);
            }
        }
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
                self.list.set_columns(&[]);
            }
            return;
        }
        if flags & RESET_COLUMNS != 0 {
            let widths = {
                let mut w = self.list_col_widths.borrow_mut();
                if w.is_empty() { self.list.column_widths() } else { std::mem::take(&mut *w) }
            };
            let mut cols = Vec::new();
            let mut j = 0;
            for i in 0..NUM_FIELDS {
                if mask & (1 << i) != 0 {
                    let width = widths.get(j).copied().unwrap_or(self.b.sc(120));
                    j += 1;
                    cols.push((i, self.ui_field_names[i].clone(), width));
                }
            }
            self.list.set_columns(&cols);
            self.set_list_view_sort_flag();
        }
        if !self.is_db_open() {
            return;
        }
        let search_mode = self.search_mode.get();
        let (cur_date, soon_date) = helpers::expiry_check_dates(self.db_config().warn_expire_num_days);
        let filter = self.filter.get();
        let use_adv = self.main.upgrade().is_some_and(|m| m.use_advanced_passw_est());
        let mut rows = Vec::new();
        let mut prev_sel_idx = None;
        let mut user_names: BTreeMap<String, String> = BTreeMap::new();
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
                    self.tag_view.add(
                        TagKind::Untagged,
                        &format!("{} ({})", trl("Untagged"), base_untagged),
                        base_untagged,
                        sel,
                    );
                }
                *self.tag_filter.borrow_mut() = new_filter;
                let names: Vec<String> = tags.keys().cloned().collect();
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
                    floor_entropy_bits(pwtech_core::zxcvbn::zxcvbn_match(&p))
                } else {
                    PasswordGenerator::estimate_passw_security(&p) as i32
                };
                if bits >= WEAK_PASSW_THRESHOLD {
                    continue;
                }
            }
            if !tag_filter.is_empty() {
                let matched = if e.tags().is_empty() {
                    tag_filter.contains("")
                } else {
                    tag_filter.iter().any(|t| !t.is_empty() && e.check_tag(t))
                };
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
        if prev_sel_idx.is_some() {
            if let Some(i) = prev_sel.and_then(|id| self.list.index_of(RowKey::Entry(id))) {
                self.list.select_only(i);
                self.list.scroll_to(i);
            }
        } else if search_mode != SEARCH_MODE_OFF && self.list.len() > 0 {
            self.list.select_only(0);
        } else {
            self.cancel_apply_only();
            self.apply_db_view_item_selection(None);
        }
        self.set_user_name_completion(user_names.into_values().collect());
        if filter != FilterType::None {
            self.filter_panel.set_visible(true);
            let caption = match filter {
                FilterType::Expired => "Expired Entries",
                FilterType::ExpireSoon => "Entries that Will Expire Soon",
                _ => "Entries with Weak Passwords",
            };
            self.filter_panel.set_tooltip_text(Some(&trl(caption)));
        } else {
            self.filter_panel.set_visible(false);
        }
    }

    fn cancel_apply_only(&self) {
        if let Some(id) = self.apply_pending.borrow_mut().take() {
            id.remove();
        }
    }

    /// `InitAutoComplete()` / `DisableAutoComplete()` for the user name.
    fn set_user_name_completion(&self, names: Vec<String>) {
        let e = self.entry("UserNameBox");
        if names.is_empty() {
            e.set_completion(None);
            return;
        }
        let store = gtk::ListStore::new(&[glib::Type::STRING]);
        for n in &names {
            store.insert_with_values(None, &[(0, n)]);
        }
        let c = gtk::EntryCompletion::new();
        c.set_model(Some(&store));
        c.set_text_column(0);
        c.set_inline_completion(true);
        c.set_popup_completion(true);
        c.set_match_func(|c, key, iter| {
            let Some(m) = c.model() else { return false };
            let v: String = m.get(iter, 0);
            lower(&v).starts_with(&lower(key))
        });
        e.set_completion(Some(&c));
    }

    // ------------------------------------------------------------------
    // edit panel

    /// `ClearEditPanel()`.
    pub(super) fn clear_edit_panel(&self) {
        let _f = Filling::new(self);
        for name in ["TitleBox", "UserNameBox", "PasswBox", "UrlBox", "KeywordBox", "KeyValueListBox", "TagsBox"] {
            self.entry(name).set_text("");
        }
        self.notes_tag_changing(|| self.notes_view().buffer().set_text(""));
        self.set_expiry_checked(false);
        self.expiry_picker.set_date(today());
        for name in ["CreationTimeInfo", "LastModificationInfo", "PasswChangeInfo"] {
            self.label(name).set_text("");
        }
        self.passw_entropy_bits.set(0.0);
        if self.toggle("PasswQualityBtn").is_active() {
            self.estimate_passw_quality(None);
        }
        *self.temp_key_val.borrow_mut() = None;
        *self.temp_passw_history.borrow_mut() = None;
        *self.temp_notes.borrow_mut() = None;
    }

    fn notes_tag_changing(&self, f: impl FnOnce()) {
        let old = self.notes_tag.replace(NOTES_TAG_CHANGING);
        f();
        self.notes_tag.set(old);
    }

    /// Set the expiry check box without marking the entry as changed.
    fn set_expiry_checked(&self, on: bool) {
        let c: gtk::CheckButton = self.b.get("ExpiryCheck");
        self.suppress_expiry.set(true);
        c.set_active(on);
        self.suppress_expiry.set(false);
        self.expiry_picker.button.set_sensitive(on);
    }

    /// `ApplyDbViewItemSelection()`.
    pub(super) fn apply_db_view_item_selection(self: &Rc<Self>, item: Option<RowKey>) {
        self.notify_user_action();
        let n = self.list.sel_count();
        self.b.w("EditPanel").set_sensitive(n > 0);
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
        {
            let _f = Filling::new(self);
            if self.selected.get().is_some() {
                self.clear_edit_panel();
            }
            match item {
                Some(RowKey::Entry(id)) => self.fill_entry(id),
                Some(RowKey::New) => self.fill_new_entry(),
                None => {}
            }
            self.selected.set(item);
        }
        // `PasswHistoryBtn->Left = PasswChangeInfo->Left + PasswChangeInfo->Width + 4`
        if let Some(ep) = self.b.boxes.get("EditPanel") {
            let info = self.label("PasswChangeInfo");
            let btn = self.b.w("PasswHistoryBtn");
            if let (Some(ir), Some(br)) = (ep.child_rect(&info), ep.child_rect(&btn)) {
                let (_, nat, _, _) = info.measure(gtk::Orientation::Horizontal, -1);
                ep.move_child(&btn, ir.0 + nat + self.b.sc(4), br.1, br.2, br.3);
            }
        }
        if self.item_changed.get() {
            self.set_item_changed(false);
        } else {
            self.reset_nav_controls();
        }
        self.update_menu_states();
    }

    fn fill_entry(&self, id: u32) {
        let dbb = self.db.borrow();
        let Some(db) = dbb.as_ref() else { return };
        let Some(idx) = db.index_of_id(id) else { return };
        let e = db.entry(idx);
        self.entry("TitleBox").set_text(e.string(FieldType::Title));
        self.entry("UserNameBox").set_text(e.string(FieldType::UserName));
        self.entry("UrlBox").set_text(e.string(FieldType::Url));
        self.entry("KeywordBox").set_text(e.string(FieldType::Keyword));
        self.entry("KeyValueListBox").set_text(&self.build_transl_key_val_string(e.key_value_list()));
        let notes = crlf_to_lf(e.string(FieldType::Notes));
        if self.notes_tag.get() == NOTES_TAG_HIDDEN {
            *self.temp_notes.borrow_mut() = Some(Zeroizing::new(notes));
            self.notes_tag_changing(|| self.notes_view().buffer().set_text(&self.notes_hidden_text));
        } else {
            self.notes_tag_changing(|| self.notes_view().buffer().set_text(&notes));
        }
        let tags = Zeroizing::new(e.tags().iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(", "));
        self.entry("TagsBox").set_text(&tags);
        match decode_expiry_date(e.passw_expiry_date) {
            Some(d) if e.passw_expiry_date != 0 => {
                self.expiry_picker.set_date(d);
                self.set_expiry_checked(true);
            }
            _ => {
                self.set_expiry_checked(false);
                self.expiry_picker.set_date(today());
            }
        }
        self.label("CreationTimeInfo").set_text(&e.creation_time_string());
        self.label("LastModificationInfo").set_text(&e.modification_time_string());
        let pct = e.passw_change_time_string();
        self.label("PasswChangeInfo").set_text(if pct.is_empty() { "-" } else { &pct });
        let passw = db.get_entry_passw(idx).unwrap_or_default();
        drop(dbb);
        self.entry("PasswBox").set_text(&passw);
        if self.toggle("PasswQualityBtn").is_active() {
            self.estimate_passw_quality(Some(&passw));
        }
    }

    fn fill_new_entry(&self) {
        let (user, fmt, days) = {
            let dbb = self.db.borrow();
            let Some(db) = dbb.as_ref() else { return };
            (db.default_user_name.clone(), db.passw_format_seq.clone(), db.default_passw_expiry_days)
        };
        self.entry("UserNameBox").set_text(&user);
        if !fmt.is_empty() {
            if let Some(p) = format_passw(&fmt) {
                self.entry("PasswBox").set_text(&p);
                if self.toggle("PasswQualityBtn").is_active() {
                    self.estimate_passw_quality(Some(&p));
                }
            }
        }
        if days != 0 {
            self.expiry_picker.set_date(today_plus_days(days as i32));
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
        self.item_changed.set(changed);
        self.b.w("AddModifyBtn").set_visible(changed);
        self.b.w("CancelBtn").set_visible(changed);
        if self.is_db_open() {
            self.list.view.set_sensitive(!changed);
            self.tag_view.flow.set_sensitive(!changed);
            for name in ["AddEntryBtn", "SearchBox", "ClearSearchBtn", "SearchBtn"] {
                self.b.w(name).set_sensitive(!changed);
            }
        }
        if changed {
            self.b.w("PrevBtn").set_visible(false);
            self.b.w("NextBtn").set_visible(false);
            self.b.w("DeleteBtn").set_visible(false);
        } else {
            self.item_passw_change_confirm.set(false);
            self.reset_nav_controls();
        }
        self.toggle_shutdown_blocker(if changed {
            Some(trl("Password database entry is being edited."))
        } else if self.db_changed.get() && self.is_db_open() {
            Some(trl("Password database contains unsaved changes."))
        } else {
            None
        });
        self.update_menu_states();
    }

    /// `ResetNavControls()`.
    pub(super) fn reset_nav_controls(&self) {
        let n = self.list.sel_count();
        let first = self.list.first_selected();
        self.b.w("PrevBtn").set_visible(first.is_some_and(|i| i > 0));
        self.b.w("NextBtn").set_visible(first.is_some_and(|i| i + 1 < self.list.len()));
        self.b
            .w("DeleteBtn")
            .set_visible(n > 1 || matches!(self.selected.get(), Some(RowKey::Entry(_))));
    }

    /// `ResetDbOpenControls()`.
    pub(super) fn reset_db_open_controls(&self) {
        let open = self.is_db_open();
        self.list.view.set_sensitive(open);
        self.b.w("EditPanel").set_sensitive(open && self.list.sel_count() > 0);
        let ro = self.db_read_only.get();
        self.b.w("SaveBtn").set_sensitive(open && !ro);
        self.b.w("LockBtn").set_sensitive(open || self.locked.get());
        for name in ["AddEntryBtn", "SearchBox", "ClearSearchBtn", "SearchBtn"] {
            self.b.w(name).set_sensitive(open);
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
        let rearrange = ext && self.sort_by_idx.get() < 0 && (n_sel as usize) < db_len;
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
        self.set_enabled(
            "MainMenu_Edit_AddEntry",
            edit_ok && self.search_mode.get() == SEARCH_MODE_OFF && !self.filter_panel.is_visible(),
        );
        let view_ok = !changed;
        for i in 0..NUM_FIELDS {
            self.set_enabled(&format!("col{i}"), view_ok);
        }
        for n in [
            "MainMenu_View_ShowPasswInList",
            "sortby",
            "sortorder",
            "tagsortby",
            "tagsortorder",
            "filter",
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
        self.win.set_title(Some(&s));
    }

    /// `ToggleShutdownBlocker()`: inhibit logout while there are unsaved
    /// changes.
    fn toggle_shutdown_blocker(&self, reason: Option<String>) {
        let Some(app) = self.win.application() else { return };
        let cookie = self.inhibit_cookie.replace(0);
        if cookie != 0 {
            app.uninhibit(cookie);
        }
        if let Some(r) = reason {
            let c = app.inhibit(Some(&self.win), gtk::ApplicationInhibitFlags::LOGOUT, Some(&r));
            self.inhibit_cookie.set(c);
        }
    }

    /// `SetDbChanged()`.
    pub(super) fn set_db_changed(&self, changed: bool, entry_changed: bool) {
        let mut changed = changed;
        let c = self.db_config();
        let file = self.db_file_name.borrow().clone();
        if changed
            && c.auto_save
            && !file.is_empty()
            && (c.auto_save_option == 1 || (c.auto_save_option == 0 && entry_changed))
            && self.save_database(&file)
        {
            changed = false;
        }
        if changed != self.db_changed.get() {
            self.db_changed.set(changed);
            self.change_caption();
        }
        self.toggle_shutdown_blocker(if changed { Some(trl("Password database contains unsaved changes.")) } else { None });
    }

    // ------------------------------------------------------------------
    // edit box handlers

    /// `TitleBoxChange()`.
    fn title_box_change(&self) {
        if self.selected.get().is_some() && !self.filling.get() && !self.item_changed.get() {
            self.notify_user_action();
            self.set_item_changed(true);
        }
    }

    /// `PasswBoxChange()`.
    fn passw_box_change(&self) {
        if self.selected.get().is_some() && !self.filling.get() {
            if !self.item_changed.get() {
                self.notify_user_action();
                self.set_item_changed(true);
            }
            self.item_passw_change_confirm.set(!self.passw_box_gen.get());
            if self.toggle("PasswQualityBtn").is_active() {
                self.estimate_passw_quality(None);
            }
        }
    }

    /// `EstimatePasswQuality()`.
    fn estimate_passw_quality(&self, passw: Option<&str>) {
        let mut chars = 0;
        let mut bits = 0.0;
        let text;
        let p = match passw {
            Some(p) => Some(p),
            None => {
                text = Zeroizing::new(self.entry("PasswBox").text().to_string());
                if text.is_empty() { None } else { Some(text.as_str()) }
            }
        };
        if let Some(p) = p {
            chars = p.chars().count();
            let adv = self.main.upgrade().is_some_and(|m| m.use_advanced_passw_est());
            bits = if adv {
                pwtech_core::zxcvbn::zxcvbn_match(p)
            } else {
                PasswordGenerator::estimate_passw_security(p)
            };
        }
        self.passw_entropy_bits.set(bits);
        let caption = if chars > 0 {
            format!("{} / {}", trl_format("%1 ch.", &[&chars.to_string()]), floor_entropy_bits(bits))
        } else {
            "0".to_string()
        };
        self.label("PasswSecurityLbl").set_text(&caption);
        self.passw_bar.set_quality(bits);
    }

    /// `PasswQualityBtnClick()`.
    pub(super) fn passw_quality_btn_click(&self) {
        let on = self.toggle("PasswQualityBtn").is_active();
        self.passw_bar.area.set_visible(on);
        if let Some(cap) = self.b.try_get::<gtk::Label>("PasswSecurityBarPanel.caption") {
            cap.set_visible(!on);
        }
        if on {
            self.estimate_passw_quality(None);
        } else {
            self.label("PasswSecurityLbl").set_text("");
        }
    }

    /// `TogglePasswBtnClick()`.
    fn toggle_passw_btn_click(&self) {
        self.notify_user_action();
        let hide = self.toggle("TogglePasswBtn").is_active();
        self.entry("PasswBox").set_visibility(!hide);
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
                    msg(Some(&self.gwin()), &trl_format("Profile \"%1\" not found.", &[&p]), MsgIcon::Error);
                    return;
                }
            } else if let Some(f) = format {
                if let Some(passw) = format_passw(&f) {
                    self.set_generated_passw(&passw);
                }
                gen_main = false;
            }
        }
        if gen_main {
            let w = Rc::downgrade(self);
            main.generate_passw_for(move |p| {
                if let Some(m) = w.upgrade() {
                    m.set_generated_passw(p);
                }
            });
        }
        self.entry("PasswBox").grab_focus();
    }

    /// Put a generated password into the password box
    /// (`PASSWBOX_TAG_PASSW_GEN`: no confirmation needed).
    fn set_generated_passw(&self, p: &str) {
        self.passw_box_gen.set(true);
        self.entry("PasswBox").set_text(p);
        self.passw_box_gen.set(false);
    }

    /// `ToggleNotesBtnClick()`.
    pub(super) fn toggle_notes_btn_click(&self) {
        let hide = self.toggle("ToggleNotesBtn").is_active();
        self.notes_tag.set(if hide { NOTES_TAG_HIDDEN } else { 0 });
        if self.selected.get().is_some() && !self.notes_view().has_focus() {
            if hide {
                self.notes_box_exit();
            } else if let Some(n) = self.temp_notes.borrow_mut().take() {
                self.notes_tag_changing(|| self.notes_view().buffer().set_text(&n));
            } else if let Some(RowKey::Entry(id)) = self.selected.get() {
                let notes = self.entry_notes(id);
                self.notes_tag_changing(|| self.notes_view().buffer().set_text(&notes));
            }
        }
    }

    fn entry_notes(&self, id: u32) -> Zeroizing<String> {
        let dbb = self.db.borrow();
        Zeroizing::new(
            dbb.as_ref()
                .and_then(|db| db.index_of_id(id).map(|i| crlf_to_lf(db.entry(i).string(FieldType::Notes))))
                .unwrap_or_default(),
        )
    }

    /// `NotesBoxEnter()`.
    fn notes_box_enter(&self) {
        if self.selected.get().is_none() {
            return;
        }
        if let Some(n) = self.temp_notes.borrow_mut().take() {
            self.notes_tag_changing(|| self.notes_view().buffer().set_text(&n));
        } else if self.notes_tag.get() == NOTES_TAG_HIDDEN {
            if let Some(RowKey::Entry(id)) = self.selected.get() {
                let notes = self.entry_notes(id);
                self.notes_tag_changing(|| self.notes_view().buffer().set_text(&notes));
            }
        }
    }

    /// `NotesBoxExit()`.
    fn notes_box_exit(&self) {
        if self.selected.get().is_some() && self.notes_tag.get() == NOTES_TAG_HIDDEN {
            *self.temp_notes.borrow_mut() = Some(text_view_text(&self.notes_view()));
            self.notes_tag_changing(|| self.notes_view().buffer().set_text(&self.notes_hidden_text));
        }
    }

    /// `ExpiryCheckClick()`.
    fn expiry_check_click(&self) {
        if !self.suppress_expiry.get() {
            if self.selected.get().is_none() {
                return;
            }
            self.notify_user_action();
            self.set_item_changed(true);
        }
        let on = self.b.get::<gtk::CheckButton>("ExpiryCheck").is_active();
        self.expiry_picker.button.set_sensitive(on);
    }

    /// `AddTagBtnClick()`.
    fn add_tag_btn_click(&self) {
        self.notify_user_action();
        if self.selected.get().is_none() {
            return;
        }
        if self.tag_menu.n_items() > 0 {
            self.tag_popover.popup();
        } else {
            self.entry("TagsBox").grab_focus();
        }
    }

    /// `OnTagMenuItemClick()`.
    fn on_tag_menu_item_click(&self, tag: &str) {
        self.notify_user_action();
        let e = self.entry("TagsBox");
        let cur = Zeroizing::new(e.text().to_string());
        let mut s = Zeroizing::new(cur.trim_end_matches(' ').to_string());
        if !s.is_empty() {
            if !s.ends_with(',') && !s.ends_with(';') {
                s.push_str(", ");
            }
            s.push_str(tag);
        } else {
            s.push_str(tag);
        }
        e.set_text(&s);
    }

    // ------------------------------------------------------------------
    // navigation and editing

    /// `PrevBtnClick()`.
    fn prev_btn_click(&self) {
        self.notify_user_action();
        let Some(i) = self.list.first_selected() else { return };
        if i == 0 {
            return;
        }
        self.list.select_only(i - 1);
        self.list.scroll_to(i - 1);
        self.list.view.grab_focus();
    }

    /// `NextBtnClick()`.
    fn next_btn_click(&self) {
        self.notify_user_action();
        let Some(i) = self.list.first_selected() else { return };
        if i + 1 >= self.list.len() {
            return;
        }
        self.list.select_only(i + 1);
        self.list.scroll_to(i + 1);
        self.list.view.grab_focus();
    }

    /// `CancelBtnClick()`.
    fn cancel_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        self.clear_edit_panel();
        let sel = self.selected.get();
        self.apply_db_view_item_selection(sel);
    }

    /// `MainMenu_Edit_AddEntryClick()`.
    fn add_entry_click(self: &Rc<Self>) {
        self.notify_user_action();
        let n = self.list.len();
        if n > 0 && self.list.key_at(n - 1) == Some(RowKey::New) && !self.item_changed.get() {
            self.list.clear_selection();
            self.list.scroll_to(n - 1);
            self.list.select_only(n - 1);
            self.apply_now();
            self.apply_db_view_item_selection(None);
            self.entry("TitleBox").grab_focus();
        }
    }

    /// `AddModifyBtnClick()`.
    async fn add_modify_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        let Some(sel) = self.selected.get() else { return };
        if self.entry("TitleBox").text_length() == 0 && self.entry("PasswBox").text_length() == 0 {
            msg_box(Some(&self.gwin()), &trl("Please specify at least a title or password."), MsgIcon::Warning, MsgButtons::Ok, false)
                .await;
            return;
        }
        let passw = Zeroizing::new(self.entry("PasswBox").text().to_string());
        if !passw.is_empty() && self.item_passw_change_confirm.get() && self.toggle("TogglePasswBtn").is_active() {
            let dlg = self.passw_enter();
            let r = dlg.execute(PASSWENTER_FLAG_ALLOWSKIP, &trl("Confirm password"), Some(&self.gwin())).await;
            match r {
                EnterResult::Ok => {
                    let same = *dlg.get_passw(0) == *passw;
                    dlg.clear_and_flush();
                    if !same {
                        msg_box(Some(&self.gwin()), &trl("Passwords are not identical."), MsgIcon::Error, MsgButtons::Ok, false)
                            .await;
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
        let tags_text = Zeroizing::new(self.entry("TagsBox").text().to_string());
        let mut new_tags: BTreeMap<String, String> = BTreeMap::new();
        if !tags_text.is_empty() {
            for item in tags_text.split([',', ';']) {
                let t = item.trim_start_matches(' ');
                if t.is_empty() {
                    continue;
                }
                let mut chars: Vec<char> = t.chars().collect();
                if chars.len() > DB_MAX_TAG_LEN {
                    chars.truncate(DB_MAX_TAG_LEN);
                    num_overlong += 1;
                }
                let tag: String = chars.into_iter().collect::<String>().trim_end_matches(' ').to_string();
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
        let expiry = if self.b.get::<gtk::CheckButton>("ExpiryCheck").is_active() {
            let (y, m, d) = self.expiry_picker.date();
            encode_expiry_date(y, m, d)
        } else {
            0
        };
        let notes = match self.temp_notes.borrow_mut().take() {
            Some(n) => n,
            None => text_view_text(&self.notes_view()),
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
                e.set_string(FieldType::Title, &self.entry("TitleBox").text());
                e.set_string(FieldType::UserName, &self.entry("UserNameBox").text());
                e.set_string(FieldType::Url, &self.entry("UrlBox").text());
                e.set_string(FieldType::Keyword, &self.entry("KeywordBox").text());
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
            self.selected.set(Some(RowKey::Entry(new_id)));
        }
        self.reset_list_view(RELOAD_TAGS);
        self.set_item_changed(false);
        self.set_db_changed(true, true);
        if num_overlong > 0 {
            msg_box(
                Some(&self.gwin()),
                &trl_format(
                    "The maximum tag length is %1 characters.\nLonger tags have been shortened.",
                    &[&DB_MAX_TAG_LEN.to_string()],
                ),
                MsgIcon::Warning,
                MsgButtons::Ok,
                false,
            )
            .await;
        }
        self.apply_now();
        self.next_btn_click();
    }

    /// `DeleteBtnClick()`.
    async fn delete_click(self: &Rc<Self>) {
        self.notify_user_action();
        if self.list.sel_count() == 0 {
            return;
        }
        let _s = IdleSuspend::new(self);
        let ids: Vec<u32> = self
            .list
            .selected_indices()
            .iter()
            .filter_map(|&i| match self.list.key_at(i) {
                Some(RowKey::Entry(id)) => Some(id),
                _ => None,
            })
            .collect();
        let text = if ids.len() == 1 {
            trl("Delete 1 entry?")
        } else {
            trl_format("Delete %1 entries?", &[&ids.len().to_string()])
        };
        if msg_box(Some(&self.gwin()), &text, MsgIcon::Warning, MsgButtons::YesNo, true).await != MsgResult::Yes {
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
    fn duplicate_entry_click(self: &Rc<Self>) {
        self.notify_user_action();
        if self.list.sel_count() == 0 {
            return;
        }
        let copy_str = format!(" - {}", trl_ctx("Copy", "noun"));
        {
            let mut dbb = self.db.borrow_mut();
            let Some(db) = dbb.as_mut() else { return };
            for i in self.list.selected_indices() {
                if let Some(RowKey::Entry(id)) = self.list.key_at(i) {
                    if let Some(idx) = db.index_of_id(id) {
                        let title = format!("{}{}", db.entry(idx).string(FieldType::Title), copy_str);
                        if let Err(e) = db.duplicate_entry(idx, &title) {
                            msg(Some(&self.gwin()), &format!("{e}."), MsgIcon::Error);
                        }
                    }
                }
            }
        }
        self.set_db_changed(true, false);
        self.reset_list_view(RELOAD_TAGS);
    }

    /// `MoveDbEntries()`.
    fn move_db_entries(self: &Rc<Self>, dir: i32) {
        self.notify_user_action();
        let mut changed = false;
        {
            let mut dbb = self.db.borrow_mut();
            let Some(db) = dbb.as_mut() else { return };
            let size = db.len();
            let mut j = 0;
            let ids: Vec<u32> = self
                .list
                .selected_indices()
                .iter()
                .filter_map(|&i| match self.list.key_at(i) {
                    Some(RowKey::Entry(id)) => Some(id),
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
    pub(super) fn db_view_drag_drop(self: &Rc<Self>, target: u32) {
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

    fn copy_user_name_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let u = self.db.borrow().as_ref().map(|db| Zeroizing::new(db.entry(i).string(FieldType::UserName).to_string()));
        if let Some(u) = u.filter(|u| !u.is_empty()) {
            crate::ui::clipboard::set_data(&u);
        }
    }

    fn copy_passw_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let p = self.db.borrow().as_ref().and_then(|db| db.get_entry_passw(i).ok());
        if let Some(p) = p.filter(|p| !p.is_empty()) {
            crate::ui::clipboard::set_data(&p);
        }
    }

    fn open_url_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let u = self.db.borrow().as_ref().map(|db| db.entry(i).string(FieldType::Url).to_string());
        if let Some(u) = u.filter(|u| !u.is_empty()) {
            open_uri(Some(&self.gwin()), &u);
        }
    }

    /// `MainMenu_Edit_RunClick()`: run the command of the "Run" key.
    fn run_click(&self) {
        self.notify_user_action();
        let Some(i) = self.selected_entry_index() else { return };
        let cmd = self
            .db
            .borrow()
            .as_ref()
            .and_then(|db| db.entry(i).key_value(DB_KEYVAL_KEYS[DB_KEYVAL_RUN]).map(String::from));
        if let Some(cmd) = cmd {
            if glib::spawn_command_line_async(cmd.as_str()).is_err() {
                open_uri(Some(&self.gwin()), &cmd);
            }
        }
    }

    // ------------------------------------------------------------------
    // dialogs of the edit panel

    /// `EditKeyValBtnClick()`.
    async fn edit_key_val_btn_click(self: &Rc<Self>) {
        let Some(sel) = self.selected.get() else { return };
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        if self.temp_key_val.borrow().is_none() {
            let kv = match sel {
                RowKey::Entry(id) => self
                    .db
                    .borrow()
                    .as_ref()
                    .and_then(|db| db.index_of_id(id).map(|i| db.entry(i).key_value_list().to_vec()))
                    .unwrap_or_default(),
                RowKey::New => Vec::new(),
            };
            *self.temp_key_val.borrow_mut() = Some(kv);
        }
        let keys: Vec<String> = self.key_val_names.keys().cloned().collect();
        let mut values = vec![Zeroizing::new(String::new()); keys.len()];
        if let Some(kv) = self.temp_key_val.borrow().as_ref() {
            for (k, v) in kv {
                if let Some(row) = keys.iter().position(|x| x == k.as_str()) {
                    values[row] = v.clone();
                }
            }
        }
        if let Some(new) = self.key_val_dlg().execute(&values).await {
            let mut list = Vec::new();
            for (i, v) in new.into_iter().enumerate() {
                if !v.is_empty() {
                    list.push((Zeroizing::new(keys[i].clone()), v));
                }
            }
            let s = self.build_transl_key_val_string(&list);
            *self.temp_key_val.borrow_mut() = Some(list);
            self.entry("KeyValueListBox").set_text(&s);
        }
    }

    /// `PasswHistoryBtnClick()`.
    async fn passw_history_btn_click(self: &Rc<Self>) {
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
        if let Some(r) = self.history_dlg().execute(&hist).await {
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
        let Some(cb) = self.b.try_get::<gtk::ComboBoxText>("SearchBox") else { return };
        let text = combo_text(&cb);
        if text.is_empty() && self.search_mode.get() != SEARCH_MODE_OFF {
            self.search_mode.set(SEARCH_MODE_OFF);
            self.reset_list_view(RELOAD_TAGS);
            self.apply_fonts();
            self.search_label.set_text("");
        } else if !text.is_empty() {
            self.search_database(
                &text,
                self.search_flags.get(),
                self.action_bool("SearchMenu_CaseSensitive"),
                self.action_bool("SearchMenu_FuzzySearch"),
            );
            // search history
            let _f = Filling::new(self);
            let model = cb.model();
            let mut items: Vec<String> = Vec::new();
            if let Some(m) = model {
                if let Some(mut it) = m.iter_first() {
                    loop {
                        let v: String = m.get(&it, 0);
                        items.push(v);
                        if !m.iter_next(&mut it) {
                            break;
                        }
                    }
                }
            }
            items.retain(|x| *x != text);
            items.insert(0, text.clone());
            cb.remove_all();
            for s in &items {
                cb.append_text(s);
            }
            crate::ui::util::set_combo_text(&cb, &text);
        }
    }

    fn clear_search_btn_click(self: &Rc<Self>) {
        self.notify_user_action();
        let Some(cb) = self.b.try_get::<gtk::ComboBoxText>("SearchBox") else { return };
        if self.search_mode.get() != SEARCH_MODE_OFF || !combo_text(&cb).is_empty() {
            crate::ui::util::set_combo_text(&cb, "");
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
                for f in 0..NUM_STRING_FIELDS {
                    if flags & (1 << f) == 0 {
                        continue;
                    }
                    let src: Zeroizing<String> = if f == FieldType::Password as usize && !db.entry(i).has_plaintext_passw() {
                        db.get_entry_passw(i).unwrap_or_default()
                    } else {
                        Zeroizing::new(db.entry(i).strings[f].to_string())
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
        self.search_label.set_text(&if found == 1 {
            trl("1 entry found.")
        } else {
            trl_format("%1 entries found.", &[&found.to_string()])
        });
        self.update_menu_states();
    }

    /// `DbViewColumnClick()`.
    fn db_view_column_click(self: &Rc<Self>, field: usize, ascending: bool) {
        self.notify_user_action();
        let factor = if field as i32 == self.sort_by_idx.get() {
            if self.sort_order_factor.get() > 0 { -1 } else { 1 }
        } else {
            1
        };
        let _ = ascending;
        self.sort_by_idx.set(field as i32);
        self.sort_order_factor.set(factor);
        if let Some(a) = self.action("sortby") {
            a.set_state(&(field as i32).to_variant());
        }
        if let Some(a) = self.action("sortorder") {
            a.set_state(&factor.to_variant());
        }
        self.set_list_view_sort_flag();
        self.sort_list();
    }

    // ------------------------------------------------------------------
    // fonts

    async fn change_font(self: &Rc<Self>, list: bool) {
        let _s = IdleSuspend::new(self);
        let dlg = gtk::FontDialog::new();
        let cur = if list { self.list_font.borrow().clone() } else { self.passw_font.borrow().clone() };
        let Ok(f) = dlg.choose_font_future(Some(&self.win), cur.as_ref()).await else { return };
        if list {
            *self.list_font.borrow_mut() = Some(f);
        } else {
            *self.passw_font.borrow_mut() = Some(f);
        }
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
            msg(
                Some(&self.gwin()),
                &trl("Please finish editing the current entry\nbefore adding new entries."),
                MsgIcon::Warning,
            );
            return 0;
        }
        let mut n = 0;
        {
            let mut dbb = self.db.borrow_mut();
            let db = dbb.as_mut().unwrap();
            let normalized = passw.replace("\r\n", "\n");
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
            let m = self.clone();
            glib::spawn_future_local(async move {
                let _s = IdleSuspend::new(&m);
                msg_box(
                    Some(&m.gwin()),
                    &trl_format("%1 entries with passwords have been added.", &[&n.to_string()]),
                    MsgIcon::Info,
                    MsgButtons::Ok,
                    false,
                )
                .await;
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
        let win = self.gwin();
        let mut pe_flags = PASSWENTER_FLAG_ENABLEKEYFILE;
        let mut file = file_name.to_string();
        let caption;
        if flags & DB_OPEN_FLAG_EXISTING != 0 {
            if flags & DB_OPEN_FLAG_UNLOCK != 0 {
                file = self.db_file_name.borrow().clone();
                caption = trl_format("Unlock %1", &[&file_name_of(&file)]);
            } else {
                if file.is_empty() {
                    let dlg = gtk::FileDialog::new();
                    dlg.set_filters(Some(&file_filters(false)));
                    let Ok(f) = dlg.open_future(Some(&self.win)).await else { return false };
                    let Some(p) = f.path() else { return false };
                    file = p.to_string_lossy().to_string();
                    if self.db.borrow().is_some() && lower(&file) == lower(&self.db_file_name.borrow()) {
                        msg_box(Some(&win), &trl("The selected file is already opened."), MsgIcon::Warning, MsgButtons::Ok, false)
                            .await;
                        return false;
                    }
                } else if !Path::new(&file).is_file() {
                    msg_box(
                        Some(&win),
                        &trl_format("Could not find database file:\n\"%1\".", &[&file]),
                        MsgIcon::Error,
                        MsgButtons::Ok,
                        false,
                    )
                    .await;
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
            if dlg.execute(pe_flags, &caption, Some(&win)).await != EnterResult::Ok {
                dlg.clear_and_flush();
                self.busy.set(false);
                return false;
            }
            let mut db = PasswDatabase::new();
            let path = PathBuf::from(&file);
            let r: Result<(), DbError> = (|| {
                if flags & DB_OPEN_FLAG_EXISTING != 0 {
                    db.read_header(&path)?;
                }
                let key = pdb::combine_key_sources(&dlg.get_passw_binary(db.passw_utf8), dlg.key_file_name().as_deref())?;
                dlg.clear_and_flush();
                if flags & DB_OPEN_FLAG_EXISTING != 0 {
                    self.win.set_cursor_from_name(Some("wait"));
                    db.open(&key, &path)
                } else {
                    db.create(&key)
                }
            })();
            dlg.clear_and_flush();
            self.win.set_cursor_from_name(None);
            match r {
                Ok(()) => break db,
                Err(DbError::InvalidKey(m)) | Err(DbError::Error(m)) => {
                    msg_box(Some(&win), &format!("{m}."), MsgIcon::Error, MsgButtons::Ok, false).await;
                }
                Err(DbError::InvalidFormat(m)) => {
                    let v = db.version();
                    msg_box(
                        Some(&win),
                        &trl_format(
                            "Database format error:\n%1.\nDatabase version: %2.%3\nHighest supported version: %4.%5",
                            &[
                                &m,
                                &(v >> 8).to_string(),
                                &(v & 0xff).to_string(),
                                &VERSION_HIGH.to_string(),
                                &VERSION_LOW.to_string(),
                            ],
                        ),
                        MsgIcon::Error,
                        MsgButtons::Ok,
                        false,
                    )
                    .await;
                    self.busy.set(false);
                    return false;
                }
                Err(DbError::Io(e)) => {
                    msg_box(
                        Some(&win),
                        &trl_format("Error while opening database file:\n%1.", &[&e.to_string()]),
                        MsgIcon::Error,
                        MsgButtons::Ok,
                        false,
                    )
                    .await;
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
            if let Some(a) = self.action("filter") {
                a.set_state(&"".to_variant());
            }
        }
        if flags & DB_OPEN_FLAG_EXISTING != 0 {
            self.reset_list_view(RELOAD_TAGS);
            *self.db_file_name.borrow_mut() = file.clone();
            self.update_recent_files();
            let ro = flags & DB_OPEN_FLAG_READONLY != 0
                || std::fs::metadata(&file).map(|m| m.permissions().readonly()).unwrap_or(false);
            self.db_read_only.set(ro);
        } else {
            self.list.set_rows(vec![self.new_entry_row()]);
            self.db_file_name.borrow_mut().clear();
            self.update_recent_files();
            self.db_read_only.set(false);
        }
        set_menu_label(&self.main_menu, "MainMenu_File_Lock", &trl("Lock"));
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
                if let Some(id) = self.tag_apply_pending.borrow_mut().take() {
                    id.remove();
                }
                self.apply_tag_view_item_selection();
            }
            let idx = self.lock_sel_item_index.replace(-1);
            if idx >= 0 && (idx as u32) < self.list.len() {
                self.list.select_only(idx as u32);
                self.apply_now();
                self.apply_db_view_item_selection(None);
                self.list.scroll_to(idx as u32);
                self.list.view.grab_focus();
            }
        } else {
            let check = |flag: u32| self.db.borrow().as_ref().is_some_and(|db| db.entries().iter().any(|e| e.user_flags & flag != 0));
            let c = self.db_config();
            if c.warn_expired_entries
                && check(DB_FLAG_EXPIRED)
                && msg_box(
                    Some(&win),
                    &trl("The database contains expired entries.\nDo you want to filter these entries now?"),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    == MsgResult::Yes
            {
                self.filter.set(FilterType::Expired);
                if let Some(a) = self.action("filter") {
                    a.set_state(&"expired".to_variant());
                }
                self.reset_list_view(0);
            }
            if c.warn_entries_expire_soon
                && check(DB_FLAG_EXPIRES_SOON)
                && msg_box(
                    Some(&win),
                    &trl("The database contains entries that will expire soon.\nDo you want to filter these entries now?"),
                    MsgIcon::Warning,
                    MsgButtons::YesNo,
                    true,
                )
                .await
                    == MsgResult::Yes
            {
                self.filter.set(FilterType::ExpireSoon);
                if let Some(a) = self.action("filter") {
                    a.set_state(&"soon".to_variant());
                }
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
                    msg_box(Some(&win), &m, MsgIcon::Warning, MsgButtons::Ok, false).await;
                }
            }
            if self.list.len() == 1 {
                self.list.select_only(0);
            }
        }
        self.toggle("TogglePasswBtn").set_active(true);
        self.toggle_passw_btn_click();
        self.notify_user_action();
        self.idle_timer_enabled.set(true);
        true
    }

    /// `AskSaveChanges()`.
    async fn ask_save_changes(self: &Rc<Self>, lock: i32) -> i32 {
        let win = self.gwin();
        if self.item_changed.get() {
            match msg_box(
                Some(&win),
                &trl("The currently selected entry has been changed.\nDo you want to save the changes?"),
                MsgIcon::Question,
                MsgButtons::YesNoCancel,
                false,
            )
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
                match msg_box(
                    Some(&win),
                    &trl("The database has been changed.\nDo you want to save the changes?"),
                    MsgIcon::Question,
                    MsgButtons::YesNoCancel,
                    false,
                )
                .await
                {
                    MsgResult::Yes => {
                        self.save_click().await;
                        if self.db_changed.get() {
                            return ASK_SAVE_ERROR;
                        }
                    }
                    MsgResult::No => {
                        if lock != 0 && self.db_file_name.borrow().is_empty() {
                            msg_box(Some(&win), &trl("An untitled database cannot be locked."), MsgIcon::Warning, MsgButtons::Ok, false)
                                .await;
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

    fn close_database_impl(self: &Rc<Self>, lock: i32) {
        self.lock_sel_item_index.set(-1);
        self.lock_sel_tags.borrow_mut().clear();
        if lock != 0 {
            if let Some(k) = self.selected.get() {
                if let Some(i) = self.list.index_of(k) {
                    self.lock_sel_item_index.set(i as i32);
                }
            }
            let tags: Vec<usize> = self.tag_view.selected_display_indices().into_iter().filter(|&i| i >= 1).collect();
            *self.lock_sel_tags.borrow_mut() = tags;
        }
        self.cancel_pending();
        *self.db.borrow_mut() = None;
        self.set_item_changed(false);
        self.toggle_shutdown_blocker(None);
        self.search_mode.set(SEARCH_MODE_OFF);
        self.passw_entropy_bits.set(0.0);
        self.tags.borrow_mut().clear();
        self.global_casei_tags.borrow_mut().clear();
        self.tag_filter.borrow_mut().clear();
        self.clear_list_view();
        self.apply_fonts();
        self.tag_view.clear();
        self.reset_db_open_controls();
        self.tag_menu.remove_all();
        if let Some(d) = self.settings_dlg.borrow().as_ref() {
            d.clear();
        }
        self.entry("UserNameBox").set_completion(None);
        if lock == 0 {
            if !self.db_config().open_last_db_on_startup {
                self.db_file_name.borrow_mut().clear();
            }
            self.locked.set(false);
            set_menu_label(&self.main_menu, "MainMenu_File_Lock", &trl("Lock"));
            if let Some(cb) = self.b.try_get::<gtk::ComboBoxText>("SearchBox") {
                let _f = Filling::new(self);
                cb.remove_all();
                crate::ui::util::set_combo_text(&cb, "");
            }
        }
        self.change_caption();
        self.search_label.set_text("");
        self.filter_panel.set_visible(false);
        self.idle_timer_enabled.set(false);
        self.reset_db_open_controls();
        if self.db_config().clear_clip_close_lock {
            crate::ui::clipboard::clear_data(true, |_| {});
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
            set_menu_label(&self.main_menu, "MainMenu_File_Lock", &trl("Unlock"));
            self.reset_db_open_controls();
            self.change_caption();
            self.win.minimize();
        }
    }

    /// `SaveDatabase()`.
    pub(super) fn save_database(&self, file: &str) -> bool {
        let c = self.db_config();
        let path = Path::new(file);
        if c.create_backup && path.is_file() {
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
            if std::fs::copy(path, &backup).is_err() {
                msg(Some(&self.gwin()), &trl_format("Could not save backup file\n\"%1\".", &[&backup]), MsgIcon::Error);
            }
        }
        let r = match self.db.borrow_mut().as_mut() {
            Some(db) => db.save(path),
            None => return false,
        };
        match r {
            Ok(()) => true,
            Err(e) => {
                msg(Some(&self.gwin()), &trl_format("Error while saving database file:\n%1.", &[&e.to_string()]), MsgIcon::Error);
                false
            }
        }
    }

    /// `MainMenu_File_SaveClick()`.
    async fn save_click(self: &Rc<Self>) {
        self.notify_user_action();
        if !self.is_db_open() {
            return;
        }
        let file = self.db_file_name.borrow().clone();
        if file.is_empty() || self.db_read_only.get() {
            Box::pin(self.save_as_click()).await;
        } else if self.save_database(&file) {
            self.set_db_changed(false, false);
        }
    }

    /// `MainMenu_File_SaveAsClick()`.
    async fn save_as_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let dlg = gtk::FileDialog::new();
        dlg.set_filters(Some(&file_filters(true)));
        let cur = self.db_file_name.borrow().clone();
        if !cur.is_empty() {
            dlg.set_initial_name(Some(&file_name_of(&cur)));
        } else {
            dlg.set_initial_name(Some(&format!("{}{PWDB_FILE_EXT}", trl("Untitled"))));
        }
        let Ok(f) = dlg.save_future(Some(&self.win)).await else { return };
        let Some(p) = f.path() else { return };
        let file = p.to_string_lossy().to_string();
        if self.save_database(&file) {
            *self.db_file_name.borrow_mut() = file;
            self.update_recent_files();
            self.db_read_only.set(false);
            self.db_changed.set(false);
            self.set_db_changed(false, false);
            self.change_caption();
            self.reset_db_open_controls();
        }
    }

    /// `UpdateRecentFiles()`.
    fn update_recent_files(&self) {
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

    /// `RequestPasswAndCheck()`.
    async fn request_passw_and_check(
        self: &Rc<Self>,
        request: &str,
        invalid: &str,
        check: impl Fn(&PasswDatabase, &[u8]) -> Result<bool, DbError>,
        utf8: bool,
    ) -> Option<Zeroizing<Vec<u8>>> {
        let dlg = self.passw_enter();
        let win = self.dialog_parent();
        while dlg.execute(PASSWENTER_FLAG_ENABLEKEYFILE, request, Some(&win)).await == EnterResult::Ok {
            let key = pdb::combine_key_sources(&dlg.get_passw_binary(utf8), dlg.key_file_name().as_deref());
            dlg.clear_and_flush();
            match key {
                Ok(key) => {
                    self.win.set_cursor_from_name(Some("wait"));
                    let r = {
                        let dbb = self.db.borrow();
                        match dbb.as_ref() {
                            Some(db) => check(db, &key),
                            None => return None,
                        }
                    };
                    self.win.set_cursor_from_name(None);
                    match r {
                        Ok(true) => return Some(key),
                        Ok(false) => {
                            msg_box(Some(&win), invalid, MsgIcon::Error, MsgButtons::Ok, false).await;
                        }
                        Err(e) => {
                            msg_box(Some(&win), &format!("{e}."), MsgIcon::Error, MsgButtons::Ok, false).await;
                        }
                    }
                }
                Err(e) => {
                    msg_box(Some(&win), &format!("{e}."), MsgIcon::Error, MsgButtons::Ok, false).await;
                }
            }
        }
        dlg.clear_and_flush();
        None
    }

    /// Parent for dialogs: the database settings dialog while it is shown
    /// (the password is requested from there), else the manager window.
    fn dialog_parent(&self) -> gtk::Window {
        self.settings_dlg
            .borrow()
            .as_ref()
            .map(|d| d.win.clone())
            .filter(|w| w.is_visible())
            .unwrap_or_else(|| self.gwin())
    }

    /// `MainMenu_File_ChangeMasterPasswordClick()`.
    async fn change_master_password_click(self: &Rc<Self>) {
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
                PASSWENTER_FLAG_CONFIRMPASSW
                    | PASSWENTER_FLAG_ENABLEKEYFILE
                    | PASSWENTER_FLAG_ENABLEKEYFILECREATION
                    | PASSWENTER_FLAG_PASSWQUALITY,
                &trl("Enter NEW master password"),
                Some(&self.gwin()),
            )
            .await
            == EnterResult::Ok
        {
            let (has_rk, cur_utf8) =
                self.db.borrow().as_ref().map(|d| (d.has_recovery_key(), d.passw_utf8)).unwrap_or((false, true));
            let utf8 = cur_utf8 || !has_rk;
            let key = pdb::combine_key_sources(&dlg.get_passw_binary(utf8), dlg.key_file_name().as_deref());
            dlg.clear_and_flush();
            let r = key.and_then(|key| {
                self.win.set_cursor_from_name(Some("wait"));
                let mut dbb = self.db.borrow_mut();
                let db = dbb.as_mut().unwrap();
                let r = db.change_master_key(&key, 0, None, None);
                if r.is_ok() {
                    db.passw_utf8 = utf8;
                    db.master_passw_expiry_date = 0;
                }
                r
            });
            self.win.set_cursor_from_name(None);
            match r {
                Ok(()) => {
                    self.set_db_changed(true, false);
                    self.change_caption();
                    msg_box(Some(&self.gwin()), &trl("Master password successfully changed."), MsgIcon::Info, MsgButtons::Ok, false)
                        .await;
                }
                Err(e) => {
                    msg_box(Some(&self.gwin()), &format!("{e}."), MsgIcon::Error, MsgButtons::Ok, false).await;
                }
            }
        } else {
            dlg.clear_and_flush();
        }
    }

    /// `MainMenu_File_SetRecoveryPasswordClick()`.
    async fn set_recovery_password_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let (has_rk, utf8) = self.db.borrow().as_ref().map(|d| (d.has_recovery_key(), d.passw_utf8)).unwrap_or((false, true));
        let new_flags = PASSWENTER_FLAG_CONFIRMPASSW
            | PASSWENTER_FLAG_ENABLEKEYFILE
            | PASSWENTER_FLAG_ENABLEKEYFILECREATION
            | PASSWENTER_FLAG_PASSWQUALITY;
        let mut success = false;
        let win = self.gwin();
        if !has_rk {
            let Some(key) = self
                .request_passw_and_check(
                    &trl("Enter master password"),
                    &trl("Master password is invalid."),
                    |db, k| db.check_master_key(k),
                    utf8,
                )
                .await
            else {
                return;
            };
            let dlg = self.passw_enter();
            if dlg.execute(new_flags, &trl("Enter recovery password"), Some(&win)).await == EnterResult::Ok {
                let rk = pdb::combine_key_sources(&dlg.get_passw_binary(utf8), dlg.key_file_name().as_deref());
                dlg.clear_and_flush();
                self.win.set_cursor_from_name(Some("wait"));
                let r = rk.and_then(|rk| self.db.borrow_mut().as_mut().unwrap().set_recovery_key(&key, &rk));
                self.win.set_cursor_from_name(None);
                match r {
                    Ok(()) => {
                        success = true;
                        msg_box(Some(&win), &trl("Recovery password successfully set."), MsgIcon::Info, MsgButtons::Ok, false).await;
                    }
                    Err(e) => {
                        msg_box(Some(&win), &format!("{e}."), MsgIcon::Error, MsgButtons::Ok, false).await;
                    }
                }
            } else {
                dlg.clear_and_flush();
            }
        } else {
            if self
                .request_passw_and_check(
                    &trl("Enter recovery password"),
                    &trl("Recovery password is invalid."),
                    |db, k| db.check_recovery_key(k),
                    utf8,
                )
                .await
                .is_none()
            {
                return;
            }
            let dlg = self.passw_enter();
            if dlg.execute(new_flags, &trl("Enter NEW master password"), Some(&win)).await == EnterResult::Ok {
                let key = pdb::combine_key_sources(&dlg.get_passw_binary(true), dlg.key_file_name().as_deref());
                dlg.clear_and_flush();
                self.win.set_cursor_from_name(Some("wait"));
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
                self.win.set_cursor_from_name(None);
                match r {
                    Ok(()) => {
                        success = true;
                        msg_box(Some(&win), &trl("Recovery password successfully removed."), MsgIcon::Info, MsgButtons::Ok, false)
                            .await;
                    }
                    Err(e) => {
                        msg_box(Some(&win), &format!("{e}."), MsgIcon::Error, MsgButtons::Ok, false).await;
                    }
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
        set_menu_label(
            &self.main_menu,
            "MainMenu_File_SetRecoveryPassword",
            &trl(if has { "Remove Recovery Password..." } else { "Set Recovery Password..." }),
        );
        if let Some(d) = self.settings_dlg.borrow().as_ref() {
            d.set_recovery_dependencies(has);
        }
    }

    fn current_settings(&self) -> Option<DbSettings> {
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
    async fn db_settings_click(self: &Rc<Self>) {
        self.notify_user_action();
        let _s = IdleSuspend::new(self);
        let Some(old) = self.current_settings() else { return };
        let has_rk = self.db.borrow().as_ref().is_some_and(|d| d.has_recovery_key());
        let dlg = self.settings_dlg();
        dlg.set_settings(&old, has_rk);
        let w = Rc::downgrade(self);
        let ok = dlg
            .show_modal(Box::new(move |s| {
                let w = w.clone();
                Box::pin(async move {
                    match w.upgrade() {
                        Some(m) => m.apply_db_settings(s).await,
                        None => false,
                    }
                })
            }))
            .await;
        if ok && self.current_settings().is_some_and(|new| new != old) {
            self.set_db_changed(true, false);
            self.change_caption();
        }
    }

    /// `ApplyDbSettings()`.
    async fn apply_db_settings(self: &Rc<Self>, s: DbSettings) -> bool {
        let (has_rk, kdf, utf8) = match self.db.borrow().as_ref() {
            Some(d) => (d.has_recovery_key(), d.kdf_iterations(), d.passw_utf8),
            None => return false,
        };
        if !has_rk && s.num_kdf_rounds != kdf {
            let Some(key) = self
                .request_passw_and_check(
                    &trl("Enter master password again"),
                    &trl("Master password is invalid."),
                    |db, k| db.check_master_key(k),
                    utf8,
                )
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
    async fn change_kdf_rounds(self: &Rc<Self>, key: &[u8], rounds: u32) -> bool {
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
        let mut progress: Option<Rc<crate::ui::dialogs::ProgressWindow>> = None;
        let parent: gtk::Window = self
            .settings_dlg
            .borrow()
            .as_ref()
            .map(|d| d.win.clone())
            .unwrap_or_else(|| self.gwin());
        let (db, r) = loop {
            match rx.try_recv() {
                Ok(v) => break v,
                Err(_) => {
                    glib::timeout_future(std::time::Duration::from_millis(100)).await;
                    if progress.is_none() && start.elapsed().as_millis() >= 1000 {
                        let p = crate::ui::dialogs::ProgressWindow::new(
                            &parent,
                            &trl("Changing number of KDF iterations"),
                            &trl("Computing derived key ..."),
                        );
                        let c3 = cancel.clone();
                        let pc = p.cancel.clone();
                        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                            if pc.get() {
                                c3.store(true, Ordering::Relaxed);
                                return glib::ControlFlow::Break;
                            }
                            glib::ControlFlow::Continue
                        });
                        p.win.present();
                        progress = Some(p);
                    }
                }
            }
        };
        if let Some(p) = progress {
            p.win.destroy();
        }
        *self.db.borrow_mut() = Some(db);
        if cancel.load(Ordering::Relaxed) {
            msg_box(Some(&parent), &trl("Process cancelled by user"), MsgIcon::Error, MsgButtons::Ok, false).await;
            return false;
        }
        if let Err(e) = r {
            msg_box(Some(&parent), &format!("{e}."), MsgIcon::Error, MsgButtons::Ok, false).await;
            return false;
        }
        true
    }

    /// `MainMenu_File_PropertiesClick()`.
    async fn properties_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let dlg = self.prop_dlg();
        let file = self.db_file_name.borrow().clone();
        if !file.is_empty() {
            let p = Path::new(&file);
            dlg.set_property(DbProperty::FileName, &file_name_of(&file));
            dlg.set_property(
                DbProperty::FilePath,
                &p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default(),
            );
            match std::fs::metadata(p) {
                Ok(md) => {
                    dlg.set_property(DbProperty::FileSize, &format!("{} {}", helpers::format_thousands(md.len()), trl("bytes")));
                    let ct = md.created().or_else(|_| md.modified());
                    if let Ok(t) = ct {
                        dlg.set_property(
                            DbProperty::CreationTime,
                            &pwtech_core::util::time_stamp_to_string(helpers::system_time_to_local_filetime(t)),
                        );
                    }
                    if let Ok(t) = md.modified() {
                        dlg.set_property(
                            DbProperty::ModificationTime,
                            &pwtech_core::util::time_stamp_to_string(helpers::system_time_to_local_filetime(t)),
                        );
                    }
                }
                Err(e) => msg(Some(&self.gwin()), &format!("{e}."), MsgIcon::Error),
            }
        }
        {
            let dbb = self.db.borrow();
            let db = dbb.as_ref().unwrap();
            let desc: String = db.description.to_string();
            if !desc.is_empty() {
                const MAX_LEN: usize = 100;
                let d = match desc.find('\n') {
                    Some(0) => String::new(),
                    Some(pos) => {
                        let line = desc[..pos].trim_end_matches('\r');
                        let n = line.chars().count();
                        let mut s: String = line.chars().take(MAX_LEN).collect::<String>().trim().to_string();
                        if !s.is_empty() && n > MAX_LEN {
                            s.push_str("...");
                        }
                        s
                    }
                    None => {
                        let n = desc.chars().count();
                        let mut s: String = desc.chars().take(MAX_LEN).collect::<String>().trim().to_string();
                        if !s.is_empty() && n > MAX_LEN {
                            s.push_str("...");
                        }
                        s
                    }
                };
                dlg.set_property(DbProperty::Description, &d);
            }
            let v = db.version();
            dlg.set_property(
                DbProperty::FormatVersion,
                &format!("{}.{} (PW: {})", v >> 8, v & 0xff, if db.passw_utf8 { "UTF-8" } else { "UTF-16" }),
            );
            dlg.set_property(DbProperty::RecoveryKeySet, &trl(if db.has_recovery_key() { "Yes" } else { "No" }));
            dlg.set_property(DbProperty::NumEntries, &db.len().to_string());
            let expired = db.entries().iter().filter(|e| e.user_flags & DB_FLAG_EXPIRED != 0).count();
            dlg.set_property(DbProperty::NumExpiredEntries, &expired.to_string());
            dlg.set_property(DbProperty::NumTags, &self.global_casei_tags.borrow().len().to_string());
        }
        dlg.show_modal().await;
    }

    /// `MainMenu_File_Export_CsvFileClick()`.
    async fn export_csv_click(self: &Rc<Self>) {
        if !self.is_db_open() {
            return;
        }
        let _s = IdleSuspend::new(self);
        let dlg = gtk::FileDialog::new();
        let filters = file_filters(true);
        dlg.set_filters(Some(&filters));
        if let Some(f) = filters.item(1).and_downcast::<gtk::FileFilter>() {
            dlg.set_default_filter(Some(&f));
        }
        let Ok(f) = dlg.save_future(Some(&self.win)).await else { return };
        let Some(path) = f.path() else { return };
        let Some(mask) = self.col_dlg().execute(&self.ui_field_names, self.csv_col_mask.get(), false).await else { return };
        self.csv_col_mask.set(mask);
        let (enc, newline) = {
            let main = self.mainw();
            let core = main.core.borrow();
            (core.file_encoding, core.newline.clone())
        };
        let r = self
            .db
            .borrow()
            .as_ref()
            .map(|db| db.export_to_csv(&path, mask, &self.ui_field_names, enc, &newline));
        match r {
            Some(Ok(())) => {
                msg_box(Some(&self.gwin()), &trl("Database successfully exported to CSV file."), MsgIcon::Info, MsgButtons::Ok, false)
                    .await;
            }
            Some(Err(e)) => {
                msg_box(
                    Some(&self.gwin()),
                    &trl_format("Error while creating CSV file:\n%1.", &[&e.to_string()]),
                    MsgIcon::Error,
                    MsgButtons::Ok,
                    false,
                )
                .await;
            }
            None => {}
        }
    }
}
