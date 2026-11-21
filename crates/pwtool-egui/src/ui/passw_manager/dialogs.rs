//! Dialogs of the password manager: column/field selection
//! (`PasswMngColSelect`), database properties (`PasswMngDbProp`),
//! database settings (`PasswMngDbSettings`), key-value list editor
//! (`PasswMngKeyValEdit`) and password history (`PasswMngPwHistory`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use egui::{Vec2, ViewportId, vec2};
use pwtool_core::config::IniFile;
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::PasswordGenerator;
use pwtool_core::passw::database::{MAX_PASSW_HISTORY_SIZE, NUM_FIELDS, NUM_STRING_FIELDS, PasswHistory, decode_expiry_date, encode_expiry_date};
use pwtool_core::random::fast::Jsf32;
use pwtool_core::unicode::{str_to_w32, w32_to_string_secure};
use zeroize::Zeroizing;

use super::helpers::{crlf_to_lf, lf_to_crlf, today};
use crate::ui::dialogs as udlg;
use crate::vcl::Form;
use crate::vcl::dialogs::{MsgButtons, MsgIcon};
use crate::vcl::list_view::{ListView, LvRow};
use crate::windows;

const CIPHER_NAMES: [&str; 2] = ["Advanced Encryption Standard (AES-CBC)", "ChaCha20"];
const CIPHER_KEY_SIZES: [u32; 2] = [256, 256];

/// `DbProperty`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbProperty {
    FileName,
    FilePath,
    FileSize,
    CreationTime,
    ModificationTime,
    Description,
    FormatVersion,
    RecoveryKeySet,
    NumEntries,
    NumExpiredEntries,
    NumTags,
}

/// Captions of the properties (items of `PropView`) and their group.
const PROPS: [(&str, usize); 11] = [
    ("Name", 0),
    ("Location", 0),
    ("Size", 0),
    ("Creation time", 0),
    ("Last modification", 0),
    ("Description", 1),
    ("Format version", 1),
    ("Recovery password set", 1),
    ("Number of entries", 1),
    ("Number of expired entries", 1),
    ("Number of tags", 1),
];
const GROUPS: [&str; 2] = ["File", "Database"];

/// `PasswDbSettings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbSettings {
    pub default_user_name: Zeroizing<String>,
    pub passw_format_seq: Zeroizing<String>,
    /// With CRLF line breaks (as stored in the database).
    pub description: Zeroizing<String>,
    pub default_expiry_days: u32,
    pub master_passw_expiry_date: u32,
    pub default_max_passw_history_size: u32,
    pub cipher_type: u32,
    pub num_kdf_rounds: u32,
    pub compressed: bool,
    pub compression_level: i32,
}

/// Callback applying the settings (`TPasswMngForm::ApplyDbSettings()`);
/// returns `false` to keep the dialog open.
pub type ApplyFn = Rc<dyn Fn(DbSettings) -> Pin<Box<dyn Future<Output = bool>>>>;

/// Result of the password history dialog.
pub struct HistoryResult {
    pub active: bool,
    pub max_size: u32,
    /// The list was cleared with the "Clear" button.
    pub cleared: bool,
}

/// Window id of the database settings dialog.
pub fn settings_id() -> ViewportId {
    ViewportId::from_hash_of("PasswMngDbSettings")
}

/// State of the dialogs: sizes and column widths, the open settings form.
#[derive(Default)]
pub struct Dialogs {
    sizes: RefCell<HashMap<&'static str, Vec2>>,
    col_widths: RefCell<HashMap<&'static str, (f32, f32)>>,
    settings: RefCell<Option<Rc<Form>>>,
}

fn read_widths(ini: &IniFile, section: &str) -> Option<(f32, f32)> {
    let s = ini.read_string(section, "ColWidths", "");
    let (a, c) = s.split_once(';')?;
    Some((a.trim().parse::<i32>().ok()?.max(20) as f32, c.trim().parse::<i32>().ok()?.max(20) as f32))
}

impl Dialogs {
    pub fn load_config(&self, ini: &IniFile) {
        for id in ["PasswMngDbProp", "PasswMngDbSettings", "PasswMngKeyValEdit", "PasswHistoryDlg"] {
            let (w, h) = (ini.read_int(id, "WindowWidth", 0), ini.read_int(id, "WindowHeight", 0));
            if w > 0 && h > 0 {
                self.sizes.borrow_mut().insert(id, vec2(w as f32, h as f32));
            }
        }
        for id in ["PasswMngDbProp", "PasswMngKeyValEdit", "PasswHistoryDlg"] {
            if let Some(w) = read_widths(ini, id) {
                self.col_widths.borrow_mut().insert(id, w);
            }
        }
    }

    pub fn save_config(&self, ini: &mut IniFile) {
        for (id, s) in self.sizes.borrow().iter() {
            ini.write_integer(id, "WindowHeight", s.y as i64);
            ini.write_integer(id, "WindowWidth", s.x as i64);
        }
        for (id, (a, b)) in self.col_widths.borrow().iter() {
            ini.write_string(id, "ColWidths", &format!("{};{}", a.round() as i32, b.round() as i32));
        }
    }

    /// Show `form` modally in window `id` (size remembered under `key`);
    /// returns the name of the button that closed it.
    async fn run(&self, form: Rc<Form>, id: &str, key: &'static str, parent: ViewportId, buttons: &[&str]) -> Option<String> {
        let (tx, rx) = async_channel::bounded::<Option<String>>(1);
        let win = udlg::form_window(id, form.clone(), parent, true);
        let min = win.size.get();
        if let Some(s) = self.sizes.borrow().get(key) {
            win.size.set(s.max(min));
        }
        let win = Rc::new(win.resizable(true, min));
        let vid = win.id;
        for b in buttons {
            let tx = tx.clone();
            let name = b.to_string();
            form.on_click(b, move |_| {
                let _ = tx.try_send(Some(name.clone()));
            });
        }
        let tx2 = tx.clone();
        *win.on_close.borrow_mut() = Some(Box::new(move || {
            let _ = tx2.try_send(None);
            false
        }));
        windows::open(win.clone());
        let r = rx.recv().await.ok().flatten();
        if let Some(s) = win.last_size.get() {
            self.sizes.borrow_mut().insert(key, s);
        }
        windows::close(vid);
        r
    }

    // ------------------------------------------------------------------
    // column / field selection

    /// `TPasswMngColDlg::Execute()`: the new bit mask, or `None` if cancelled.
    pub async fn col_select(&self, names: &[String; NUM_FIELDS], flags: u32, strings_only: bool, parent: ViewportId) -> Option<u32> {
        let n = if strings_only { NUM_STRING_FIELDS } else { NUM_FIELDS };
        let f = udlg::load("PasswMngColSelect");
        f.set_items("OptionsList", names.iter().take(n).cloned().collect());
        for i in 0..n {
            f.set_list_checked("OptionsList", i, flags & (1 << i) != 0);
        }
        loop {
            // OK keeps the dialog open until at least one item is checked
            let (tx, rx) = async_channel::bounded::<bool>(1);
            let win = Rc::new(udlg::form_window("PasswMngColSelect", f.clone(), parent, true));
            for (b, ok) in [("OKBtn", true), ("CancelBtn", false)] {
                let tx = tx.clone();
                f.on_click(b, move |_| {
                    let _ = tx.try_send(ok);
                });
            }
            let tx2 = tx.clone();
            *win.on_close.borrow_mut() = Some(Box::new(move || {
                let _ = tx2.try_send(false);
                false
            }));
            windows::open(win.clone());
            let ok = rx.recv().await.unwrap_or(false);
            if !ok {
                windows::close(win.id);
                return None;
            }
            let mut mask = 0u32;
            for (i, c) in f.list_checked("OptionsList").iter().enumerate() {
                if *c {
                    mask |= 1 << i;
                }
            }
            if mask != 0 {
                windows::close(win.id);
                return Some(mask);
            }
            udlg::ask(win.id, &trl("Select at least one column."), MsgIcon::Warning, MsgButtons::Ok, false).await;
            windows::close(win.id);
        }
    }

    // ------------------------------------------------------------------
    // database properties

    /// `ShowModal()` of the properties dialog with the values of
    /// [`DbProperty`] (empty values are left empty).
    pub async fn db_prop(&self, values: [String; 11], parent: ViewportId) {
        let f = udlg::load("PasswMngDbProp");
        let (w0, w1) = self.col_widths.borrow().get("PasswMngDbProp").copied().unwrap_or((f.sc(188), f.sc(300)));
        let table = ListView::new("pm-dbprop", true);
        table.set_columns(vec![(trl("Property"), w0), (trl("Value"), w1)]);
        let mut rows = Vec::new();
        for (g, gname) in GROUPS.iter().enumerate() {
            rows.push(LvRow { key: 100 + g, cells: vec![trl(gname)], icon: None, header: true });
            for (i, (name, grp)) in PROPS.iter().enumerate() {
                if *grp == g {
                    rows.push(LvRow::new(i, vec![trl(name), values[i].clone()]));
                }
            }
        }
        table.set_rows(rows);
        let t = table.clone();
        f.set_painter("PropView", move |ui, rect| t.paint(ui, rect));
        self.run(f, "PasswMngDbProp", "PasswMngDbProp", parent, &["CloseBtn"]).await;
        let w = table.column_widths();
        self.col_widths.borrow_mut().insert("PasswMngDbProp", (w[0], w[1]));
    }

    // ------------------------------------------------------------------
    // database settings

    pub fn settings_open(&self) -> bool {
        self.settings.borrow().is_some() && windows::is_open(settings_id())
    }

    /// Controls that cannot be changed while a recovery password is set.
    pub fn set_recovery_dependencies(&self, has_recovery: bool) {
        if let Some(f) = self.settings.borrow().as_ref() {
            for n in ["EncryptionAlgoList", "NumKdfRoundsBox", "CalcRoundsBtn"] {
                f.set_enabled(n, !has_recovery);
            }
        }
    }

    /// `Clear()` (when the database is closed).
    pub fn clear_settings(&self) {
        if let Some(f) = self.settings.borrow().as_ref() {
            for n in ["DefUserNameBox", "PasswFormatSeqBox", "DescriptionBox", "PasswGenTestBox"] {
                f.set_text(n, "");
            }
        }
    }

    /// `ShowModal()` of the settings dialog: OK validates the KDF rounds and
    /// calls `apply` (the dialog stays open if it returns `false`).
    pub async fn db_settings(&self, s: &DbSettings, has_recovery: bool, parent: ViewportId, apply: ApplyFn) -> bool {
        let f = udlg::load("PasswMngDbSettings");
        if f.has("Label1") {
            // a leftover of the form designer without function
            f.set_visible("Label1", false);
        }
        f.set_items(
            "EncryptionAlgoList",
            CIPHER_NAMES.iter().enumerate().map(|(i, n)| trl_format("%1 with %2-bit key", &[n, &CIPHER_KEY_SIZES[i].to_string()])).collect(),
        );
        f.set_range("PasswHistorySpinBtn", 0, MAX_PASSW_HISTORY_SIZE as i64);
        f.set_font_color("ExpiryInfoLbl", Some(egui::Color32::from_rgb(0xe0, 0x20, 0x20)));
        // set the settings (`SetSettings()`)
        f.set_text("DefUserNameBox", &s.default_user_name);
        f.set_text("PasswFormatSeqBox", &s.passw_format_seq);
        f.set_text("DescriptionBox", &crlf_to_lf(&s.description));
        f.set_position("DefaultExpirySpinBtn", s.default_expiry_days as i64);
        f.set_position("PasswHistorySpinBtn", s.default_max_passw_history_size as i64);
        match decode_expiry_date(s.master_passw_expiry_date) {
            Some((y, m, d)) => {
                f.set_checked("MasterPasswExpiryCheck", true);
                f.set_date("ExpiryDatePicker", y, m, d);
                let expired = today() >= (y, m, d);
                f.set_text("ExpiryInfoLbl", &if expired { trl("WARNING: Master password expired!") } else { String::new() });
            }
            None => {
                f.set_checked("MasterPasswExpiryCheck", false);
                let (y, m, d) = today();
                f.set_date("ExpiryDatePicker", y, m, d);
                f.set_text("ExpiryInfoLbl", "");
            }
        }
        f.set_enabled("ExpiryDatePicker", f.checked("MasterPasswExpiryCheck"));
        f.set_item_index("EncryptionAlgoList", Some(s.cipher_type as usize));
        f.set_text("NumKdfRoundsBox", &s.num_kdf_rounds.to_string());
        f.set_checked("EnableCompressionCheck", s.compressed);
        f.set_position("CompressionLevelBar", if s.compressed { s.compression_level as i64 } else { 6 });
        let compression = |f: &Form| {
            let on = f.checked("EnableCompressionCheck");
            f.set_enabled("CompressionLevelBar", on);
            f.set_enabled("CompressionLevelLbl", on);
        };
        compression(&f);
        f.set_item_index("ConfigPages", Some(0));
        f.set_text("PasswGenTestBox", &trl("Test:"));
        f.set_read_only("PasswGenTestBox", true);
        *self.settings.borrow_mut() = Some(f.clone());
        self.set_recovery_dependencies(has_recovery);

        let (tx, rx) = async_channel::bounded::<bool>(1);
        let busy = Rc::new(Cell::new(false));
        let ff = Rc::downgrade(&f);
        let txc = tx.clone();
        f.on_any(move |e| {
            let Some(f) = ff.upgrade() else { return };
            let crate::vcl::Event::Click(n) = e else { return };
            match n.as_str() {
                "EnableCompressionCheck" => compression(&f),
                "MasterPasswExpiryCheck" => f.set_enabled("ExpiryDatePicker", f.checked("MasterPasswExpiryCheck")),
                "CalcRoundsBtn" => f.set_text("NumKdfRoundsBox", &calc_rounds().to_string()),
                "PasswGenTestBtn" => {
                    let mut s = trl("Test:");
                    let fmt = Zeroizing::new(f.text("PasswFormatSeqBox"));
                    if !fmt.is_empty() {
                        let pg = PasswordGenerator::new();
                        let mut rng = Jsf32::new();
                        let r = pg.get_format_passw(&mut rng, &str_to_w32(&fmt), 0, None, 16000, None);
                        if !r.passw.is_empty() {
                            s.push(' ');
                            s.push_str(&w32_to_string_secure(&r.passw));
                        }
                    }
                    f.set_text("PasswGenTestBox", &s);
                }
                "CancelBtn" => {
                    let _ = txc.try_send(false);
                }
                "OKBtn" => {
                    if busy.get() {
                        return;
                    }
                    let (f, busy, apply, txc) = (f.clone(), busy.clone(), apply.clone(), txc.clone());
                    crate::runtime::spawn(async move {
                        let rounds: u32 = f.text("NumKdfRoundsBox").trim().parse().unwrap_or(0);
                        if rounds == 0 {
                            udlg::ask(settings_id(), &trl("Invalid number of key derivation rounds."), MsgIcon::Error, MsgButtons::Ok, false).await;
                            return;
                        }
                        busy.set(true);
                        let ok = apply(get_settings(&f)).await;
                        busy.set(false);
                        if ok {
                            let _ = txc.try_send(true);
                        }
                    });
                }
                _ => {}
            }
        });
        let win = udlg::form_window("PasswMngDbSettings", f.clone(), parent, true);
        let min = win.size.get();
        if let Some(s) = self.sizes.borrow().get("PasswMngDbSettings") {
            win.size.set(s.max(min));
        }
        let win = Rc::new(win.resizable(true, min));
        let tx2 = tx.clone();
        *win.on_close.borrow_mut() = Some(Box::new(move || {
            let _ = tx2.try_send(false);
            false
        }));
        windows::open(win.clone());
        f.focus("DefUserNameBox");
        let ok = rx.recv().await.unwrap_or(false);
        if let Some(s) = win.last_size.get() {
            self.sizes.borrow_mut().insert("PasswMngDbSettings", s);
        }
        windows::close(win.id);
        // `FormClose()`
        for n in ["DefUserNameBox", "PasswFormatSeqBox", "PasswGenTestBox", "DescriptionBox"] {
            f.set_text(n, "");
        }
        self.settings.borrow_mut().take();
        ok
    }

    // ------------------------------------------------------------------
    // key-value list

    /// Show the editor with the values (one per key row); the edited
    /// values if OK was pressed.
    pub async fn key_val(&self, names: &[String], values: &[Zeroizing<String>], parent: ViewportId) -> Option<Vec<Zeroizing<String>>> {
        let f = udlg::load("PasswMngKeyValEdit");
        let (w0, _) = self.col_widths.borrow().get("PasswMngKeyValEdit").copied().unwrap_or((f.sc(150), f.sc(300)));
        let grid = Rc::new(RefCell::new(KeyValGrid {
            names: names.to_vec(),
            values: values.iter().map(|v| v.to_string()).collect(),
            key_width: w0,
            focus_first: true,
        }));
        let g = grid.clone();
        f.set_painter("KeyValueGrid", move |ui, rect| g.borrow_mut().paint(ui, rect));
        let r = self.run(f, "PasswMngKeyValEdit", "PasswMngKeyValEdit", parent, &["OKBtn", "CancelBtn"]).await;
        let mut g = grid.borrow_mut();
        self.col_widths.borrow_mut().insert("PasswMngKeyValEdit", (g.key_width, 300.0));
        let result = (r.as_deref() == Some("OKBtn")).then(|| g.values.iter().map(|v| Zeroizing::new(v.clone())).collect());
        // `Clear()`
        for v in g.values.iter_mut() {
            zeroize::Zeroize::zeroize(v);
        }
        result
    }

    // ------------------------------------------------------------------
    // password history

    /// Show the history; `None` if cancelled.
    pub async fn history(&self, hist: &PasswHistory, parent: ViewportId) -> Option<HistoryResult> {
        let f = udlg::load("PasswMngPwHistory");
        f.set_range("HistorySizeSpinBtn", 1, MAX_PASSW_HISTORY_SIZE as i64);
        f.set_checked("EnableHistoryCheck", hist.active());
        let enable = |f: &Form| {
            let on = f.checked("EnableHistoryCheck");
            f.set_enabled("HistorySizeBox", on);
            f.set_enabled("HistorySizeSpinBtn", on);
        };
        enable(&f);
        f.set_position("HistorySizeSpinBtn", hist.max_size() as i64);
        let (w0, w1) = self.col_widths.borrow().get("PasswHistoryDlg").copied().unwrap_or((f.sc(150), f.sc(250)));
        let table: Rc<ListView<usize>> = ListView::new("pm-history", true);
        table.set_columns(vec![(trl("Set Date/Time"), w0), (trl("Password"), w1)]);
        let rows: Vec<LvRow<usize>> = hist
            .iter()
            .enumerate()
            .map(|(i, (t, p))| {
                let ts = if *t == 0 { "-".to_string() } else { pwtool_core::util::time_stamp_to_string(*t) };
                LvRow::new(i, vec![ts, p.to_string()])
            })
            .collect();
        let has = !rows.is_empty();
        table.set_rows(rows);
        f.set_enabled("ClearBtn", has);
        f.set_enabled("CopyBtn", has);
        let t = table.clone();
        f.set_painter("HistoryView", move |ui, rect| {
            t.paint(ui, rect);
            if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) && t.has_focus() {
                copy_history(&t);
            }
        });
        let ff = Rc::downgrade(&f);
        let t = table.clone();
        f.on_any(move |e| {
            let (Some(f), crate::vcl::Event::Click(n)) = (ff.upgrade(), e) else { return };
            match n.as_str() {
                "EnableHistoryCheck" => enable(&f),
                "ClearBtn" => {
                    t.clear();
                    f.set_enabled("CopyBtn", false);
                }
                "CopyBtn" => copy_history(&t),
                _ => {}
            }
        });
        let r = self.run(f.clone(), "PasswMngPwHistory", "PasswHistoryDlg", parent, &["OKBtn", "CancelBtn"]).await;
        let w = table.column_widths();
        self.col_widths.borrow_mut().insert("PasswHistoryDlg", (w[0], w[1]));
        let cleared = has && table.is_empty();
        table.clear();
        if r.as_deref() != Some("OKBtn") {
            return None;
        }
        Some(HistoryResult {
            active: f.checked("EnableHistoryCheck"),
            max_size: f.position("HistorySizeSpinBtn").max(1) as u32,
            cleared,
        })
    }
}

/// `CopyBtnClick()`: selected rows (or all) as "time<TAB>password".
fn copy_history(t: &ListView<usize>) {
    if t.is_empty() {
        return;
    }
    let all = t.sel_count() == 0;
    let mut s = Zeroizing::new(String::new());
    for (i, r) in t.rows().iter().enumerate() {
        if all || t.is_selected(i) {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&r.cells[0]);
            s.push('\t');
            s.push_str(&r.cells[1]);
        }
    }
    crate::ui::clipboard::set_hidden(&s);
}

/// `GetSettings()`.
fn get_settings(f: &Form) -> DbSettings {
    let master_expiry = if f.checked("MasterPasswExpiryCheck") {
        let (y, m, d) = f.date("ExpiryDatePicker");
        encode_expiry_date(y, m, d)
    } else {
        0
    };
    let compressed = f.checked("EnableCompressionCheck");
    DbSettings {
        default_user_name: Zeroizing::new(f.text("DefUserNameBox")),
        passw_format_seq: Zeroizing::new(f.text("PasswFormatSeqBox")),
        description: Zeroizing::new(lf_to_crlf(&f.text("DescriptionBox"))),
        default_expiry_days: f.position("DefaultExpirySpinBtn").max(0) as u32,
        master_passw_expiry_date: master_expiry,
        default_max_passw_history_size: f.position("PasswHistorySpinBtn").max(0) as u32,
        cipher_type: f.item_index("EncryptionAlgoList").unwrap_or(0) as u32,
        num_kdf_rounds: f.text("NumKdfRoundsBox").trim().parse().unwrap_or(0),
        compressed,
        compression_level: if compressed { f.position("CompressionLevelBar") as i32 } else { 0 },
    }
}

/// `CalcRoundsBtnClick()`: number of PBKDF2 rounds for a delay of 1 s.
fn calc_rounds() -> u32 {
    let mut key = Zeroizing::new([0u8; 32]);
    let mut salt = [0u8; 32];
    pwtool_core::random::fast::fast_fill(&mut key[..]);
    pwtool_core::random::fast::fast_fill(&mut salt);
    const ROUGH_EST_ROUNDS: u32 = 10_000;
    const TEST_FACTOR: f64 = 2.0;
    let t = std::time::Instant::now();
    let _ = pwtool_core::crypto::pbkdf2_256bit(&key[..], &salt, ROUGH_EST_ROUNDS, None);
    let rough = (TEST_FACTOR * (ROUGH_EST_ROUNDS as f64 / t.elapsed().as_secs_f64().max(1e-6))).round() as u32;
    let t = std::time::Instant::now();
    let _ = pwtool_core::crypto::pbkdf2_256bit(&key[..], &salt, rough.max(1), None);
    ((rough as f64 / t.elapsed().as_secs_f64().max(1e-6)).round() as u32).max(1)
}

/// The key-value grid (`TStringGrid` with fixed key column).
struct KeyValGrid {
    names: Vec<String>,
    values: Vec<String>,
    key_width: f32,
    focus_first: bool,
}

impl KeyValGrid {
    fn paint(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let v = ui.visuals().clone();
        let stroke = v.widgets.noninteractive.bg_stroke;
        ui.painter().rect_filled(rect, 2.0, v.extreme_bg_color);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(3.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        let row_h = child.text_style_height(&egui::TextStyle::Body) + 8.0;
        let w = rect.width() - 6.0;
        // header
        let (hr, _) = child.allocate_exact_size(vec2(w, row_h), egui::Sense::hover());
        child.painter().rect_filled(hr, 0.0, v.faint_bg_color);
        let font = egui::TextStyle::Body.resolve(child.style());
        let bold = egui::FontId::new(font.size, egui::FontFamily::Name(crate::vcl::fonts::BOLD.into()));
        child.painter().text(egui::pos2(hr.min.x + 4.0, hr.center().y), egui::Align2::LEFT_CENTER, trl("Key"), bold.clone(), v.text_color());
        child.painter().text(egui::pos2(hr.min.x + self.key_width + 4.0, hr.center().y), egui::Align2::LEFT_CENTER, trl("Value"), bold, v.text_color());
        let edge = egui::Rect::from_center_size(egui::pos2(hr.min.x + self.key_width, hr.center().y), vec2(6.0, row_h));
        let er = child.interact(edge, child.id().with("kv-edge"), egui::Sense::drag());
        if er.hovered() || er.dragged() {
            child.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if er.dragged() {
            self.key_width = (self.key_width + er.drag_delta().x).clamp(40.0, w - 60.0);
        }
        for (i, name) in self.names.iter().enumerate() {
            let (rr, _) = child.allocate_exact_size(vec2(w, row_h), egui::Sense::hover());
            child.painter().hline(rr.x_range(), rr.min.y, stroke);
            child.painter().with_clip_rect(rr).text(egui::pos2(rr.min.x + 4.0, rr.center().y), egui::Align2::LEFT_CENTER, name, font.clone(), v.text_color());
            let er = egui::Rect::from_min_max(egui::pos2(rr.min.x + self.key_width + 2.0, rr.min.y + 2.0), egui::pos2(rr.max.x - 2.0, rr.max.y - 2.0));
            let resp = child.put(er, egui::TextEdit::singleline(&mut self.values[i]).id_salt(("kv", i)));
            if i == 0 && std::mem::take(&mut self.focus_first) {
                resp.request_focus();
            }
        }
        child.painter().vline(rect.min.x + 3.0 + self.key_width, rect.y_range(), stroke);
        ui.painter().rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
    }
}
