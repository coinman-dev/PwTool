//! Smaller windows of the generator: Advanced Password Options, Password
//! List, Progress, Quick Help, Info Box; helpers for form windows.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::{Key, Ui, Vec2, ViewportId, vec2};
use pwtool_core::generate::{OPTION_INDEX_TO_BIT, PASSWOPTIONS_NUM, PASSWOPTIONS_STARRED, PasswOptions};
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::generator::WORDLIST_MAX_WORDLEN;
use zeroize::Zeroizing;

use crate::vcl::dialogs::{MsgButtons, MsgIcon, msg_box};
use crate::vcl::render::strip_amp;
use crate::vcl::{self, Event, Form};
use crate::windows::{self, Content, Window};

/// Load a form for the program's context.
pub fn load(name: &str) -> Rc<Form> {
    Rc::new(vcl::load_form(&windows::ctx(), name))
}

thread_local! {
    /// Forms of the windows by window id (test automation).
    static FORMS: RefCell<std::collections::HashMap<String, std::rc::Weak<Form>>> = RefCell::new(std::collections::HashMap::new());
}

/// The form last shown in window `id` (test automation).
pub fn form_of_window(id: &str) -> Option<Rc<Form>> {
    FORMS.with(|f| f.borrow().get(id).and_then(|w| w.upgrade()))
}

/// A window for a form: size of the form, caption as title.
pub fn form_window(id: &str, form: Rc<Form>, parent: ViewportId, modal: bool) -> Window {
    FORMS.with(|f| f.borrow_mut().insert(id.to_string(), Rc::downgrade(&form)));
    let ctx = windows::ctx();
    let size = vcl::window_size(&form, &ctx);
    let title = strip_amp(&form.caption());
    Window::new(id, &title, form, parent, size).modal(modal)
}

/// Show `form` as a modal dialog and wait until one of `buttons` is
/// clicked (its name is returned) or the window is closed (`None`).
pub async fn run_modal(form: Rc<Form>, id: &str, parent: ViewportId, buttons: &[&str]) -> Option<String> {
    let (tx, rx) = async_channel::bounded::<Option<String>>(1);
    let win = Rc::new(form_window(id, form.clone(), parent, true));
    let vid = win.id;
    for b in buttons {
        let tx = tx.clone();
        let name = b.to_string();
        form.on_click(b, move |_| {
            let _ = tx.try_send(Some(name.clone()));
            windows::close(vid);
        });
    }
    let tx2 = tx.clone();
    *win.on_close.borrow_mut() = Some(Box::new(move || {
        let _ = tx2.try_send(None);
        true
    }));
    windows::open(win);
    rx.recv().await.ok().flatten()
}

/// Message box without waiting for it (`MsgBox(...)` with one button).
pub fn msg(parent: ViewportId, text: &str, icon: MsgIcon) {
    let text = text.to_string();
    crate::runtime::spawn(async move {
        msg_box(&windows::ctx(), parent, &text, icon, MsgButtons::Ok, false).await;
    });
}

/// Modal message box.
pub async fn ask(parent: ViewportId, text: &str, icon: MsgIcon, buttons: MsgButtons, default_second: bool) -> crate::vcl::dialogs::MsgResult {
    msg_box(&windows::ctx(), parent, text, icon, buttons, default_second).await
}

// ---------------------------------------------------------------------------
// Info Box (`TInfoBoxForm::ShowInfo`): short-lived message.

struct InfoBox {
    text: String,
}

impl Content for InfoBox {
    fn ui(&self, ui: &mut Ui) {
        let r = ui.max_rect();
        ui.painter().rect_filled(r, 0.0, ui.visuals().panel_fill);
        ui.painter().rect_stroke(r, 0.0, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &self.text, font, ui.visuals().text_color());
    }
}

thread_local! {
    static INFO_SEQ: Cell<u64> = const { Cell::new(0) };
}

/// Show a short message that closes by itself.
pub fn info_box(parent: ViewportId, text: &str) {
    let ctx = windows::ctx();
    let font = egui::TextStyle::Body.resolve(&ctx.global_style());
    let size = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, egui::Color32::WHITE).size());
    let seq = INFO_SEQ.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    windows::close(ViewportId::from_hash_of("infobox"));
    let mut w = Window::new("infobox", "", Rc::new(InfoBox { text: text.to_string() }), parent, size + vec2(48.0, 28.0));
    w.decorated = false;
    windows::open(Rc::new(w));
    let ms = (750 + text.chars().count() as u64 * 20).max(1500);
    crate::runtime::spawn(async move {
        pwtool_app::rt::sleep(std::time::Duration::from_millis(ms)).await;
        if INFO_SEQ.with(|s| s.get()) == seq {
            windows::close(ViewportId::from_hash_of("infobox"));
        }
    });
}

// ---------------------------------------------------------------------------
// Advanced Password Options

/// Translated label of an option by bit.
pub fn option_name(bit: usize) -> String {
    let form = pwtool_app::form::load_form("PasswOptions");
    let items = form.find("PasswOptionsList").map(|o| o.strings("Items.Strings")).unwrap_or_default();
    let idx = OPTION_INDEX_TO_BIT.iter().position(|&b| b == bit).unwrap_or(0);
    items.get(idx).map(|s| trl(s)).unwrap_or_else(|| format!("#{bit}"))
}

/// `ShowModal()` of the options dialog: the new options if OK was pressed.
pub async fn passw_options(parent: ViewportId, o: &PasswOptions) -> Option<PasswOptions> {
    let f = load("PasswOptions");
    let items = f.items("PasswOptionsList");
    let suffix = [
        " (B8G6I1l|0OQDS5Z2) [1-3]", " [1-4]", " [2]", " [2]", " [1,2]", " [1,4]", " [1,4]", " [1,4]", " [1,4]",
        " [1]", " [1,3]", " [1-4]", " [2,3]", " [1]", " [2]", " [2]", " [1-4]", " [1]", " [1,3]",
    ];
    let idx_of = |bit: usize| OPTION_INDEX_TO_BIT.iter().position(|&b| b == bit).unwrap();
    let mut display = items.clone();
    for (bit, suf) in suffix.iter().enumerate() {
        let i = idx_of(bit);
        if let Some(d) = display.get_mut(i) {
            d.push_str(suf);
            if PASSWOPTIONS_STARRED[bit] {
                d.push_str(" *");
            }
        }
    }
    f.set_items("PasswOptionsList", display);
    for bit in 0..PASSWOPTIONS_NUM {
        f.set_list_checked("PasswOptionsList", idx_of(bit), o.flags & (1 << bit) != 0);
    }
    f.set_text(
        "InfoLbl",
        &pwtool_core::unicode::format_w(
            "[1] %1 [2] %2 [3] %3 [4] %4\n* %5",
            &[
                &trl("Applies to pass_words_."),
                &trl("Applies to pass_phrases_."),
                &trl("Applies to formatted passwords."),
                &trl("Applies to phonetic passwords."),
                &trl("Selecting one of these options might reduce the security of generated passwords."),
            ],
        ),
    );
    for name in ["MinWordLenSpinBtn", "MaxWordLenSpinBtn"] {
        f.set_range(name, 1, WORDLIST_MAX_WORDLEN as i64);
    }
    f.set_text("AmbigCharsBox", &o.ambig_chars);
    f.set_text("SpecialSymBox", &o.special_symbols);
    f.set_position("MinWordLenSpinBtn", o.min_word_len as i64);
    f.set_position("MaxWordLenSpinBtn", o.max_word_len as i64);
    f.set_text("WordSepBox", &o.word_separator);
    f.set_text("WordCharSepBox", &o.word_char_separator);
    f.set_text("TrigramFileBox", &o.trigram_file_name);
    f.set_popup_menu("PasswOptionsList", "ListMenu");
    for (menu, op) in [("ListMenu_SelectAll", 0), ("ListMenu_DeselectAll", 1), ("ListMenu_InvertSelection", 2)] {
        let ff = Rc::downgrade(&f);
        f.on_menu(menu, move |_| {
            let Some(f) = ff.upgrade() else { return };
            for (i, c) in f.list_checked("PasswOptionsList").into_iter().enumerate() {
                f.set_list_checked("PasswOptionsList", i, match op {
                    0 => true,
                    1 => false,
                    _ => !c,
                });
            }
        });
    }
    let ff = Rc::downgrade(&f);
    f.on_click("BrowseBtn", move |_| {
        let ff = ff.clone();
        crate::runtime::spawn(async move {
            let filter_t = format!("{} (*.tgm)", trl("Trigram files"));
            let all = format!("{} (*.*)", trl("All files"));
            if let Some(p) = crate::vcl::dialogs::open_file(&trl("Select trigram file"), None, &[(&filter_t, &["tgm"]), (&all, &["*"])]).await {
                if let Some(f) = ff.upgrade() {
                    f.set_text("TrigramFileBox", &p.to_string_lossy());
                    f.focus("TrigramFileBox");
                }
            }
        });
    });
    let r = run_modal(f.clone(), "PasswOptions", parent, &["OKBtn", "CancelBtn"]).await;
    if r.as_deref() != Some("OKBtn") {
        return None;
    }
    let mut flags = 0u32;
    for (idx, c) in f.list_checked("PasswOptionsList").iter().enumerate() {
        if *c && idx < OPTION_INDEX_TO_BIT.len() {
            flags |= 1 << OPTION_INDEX_TO_BIT[idx];
        }
    }
    let min = f.position("MinWordLenSpinBtn") as i32;
    let max = f.position("MaxWordLenSpinBtn") as i32;
    Some(PasswOptions {
        flags,
        ambig_chars: f.text("AmbigCharsBox"),
        special_symbols: f.text("SpecialSymBox"),
        min_word_len: min,
        max_word_len: max.max(min),
        word_separator: f.text("WordSepBox"),
        word_char_separator: f.text("WordCharSepBox"),
        trigram_file_name: f.text("TrigramFileBox"),
    })
}

// ---------------------------------------------------------------------------
// Progress

pub struct ProgressWindow {
    pub form: Rc<Form>,
    pub cancel: Rc<Cell<bool>>,
    id: ViewportId,
    pulse: Cell<i64>,
}

struct ProgressContent {
    form: Rc<Form>,
    cancel: Rc<Cell<bool>>,
}

impl Content for ProgressContent {
    fn ui(&self, ui: &mut Ui) {
        Content::ui(&*self.form, ui);
        // Escape, Ctrl+C, Ctrl+Q, Ctrl+X cancel (`TProgressForm::FormKeyDown`)
        let cancel = ui.input(|i| {
            i.key_pressed(Key::Escape) || (i.modifiers.ctrl && (i.key_pressed(Key::C) || i.key_pressed(Key::Q) || i.key_pressed(Key::X)))
        });
        if cancel {
            self.cancel.set(true);
        }
    }
}

impl ProgressWindow {
    pub fn new(parent: ViewportId, title: &str, text: &str) -> Rc<Self> {
        let form = load("Progress");
        form.set_text("ProgressLbl", text);
        form.set_range("ProgressBar", 0, 1000);
        let cancel = Rc::new(Cell::new(false));
        let c = cancel.clone();
        form.on_click("CancelBtn", move |_| c.set(true));
        let ctx = windows::ctx();
        let size = vcl::window_size(&form, &ctx);
        let content = Rc::new(ProgressContent { form: form.clone(), cancel: cancel.clone() });
        let w = Window::new("progress", title, content, parent, size).modal(true);
        let c = cancel.clone();
        // closing the window cancels, it stays open until the work stops
        *w.on_close.borrow_mut() = Some(Box::new(move || {
            c.set(true);
            false
        }));
        let id = w.id;
        windows::open(Rc::new(w));
        Rc::new(ProgressWindow { form, cancel, id, pulse: Cell::new(0) })
    }

    pub fn set(&self, done: u64, total: u64, fmt: &str) {
        let pos = if total > 0 { (done as f64 / total as f64 * 1000.0).min(1000.0) as i64 } else { self.next_pulse() };
        self.form.set_position("ProgressBar", pos);
        self.form.set_text("ProgressLbl", &trl_format(fmt, &[&done.to_string(), &total.to_string()]));
    }

    pub fn set_message(&self, msg: &str) {
        self.form.set_text("ProgressLbl", msg);
        let p = self.next_pulse();
        self.form.set_position("ProgressBar", p);
    }

    fn next_pulse(&self) -> i64 {
        let p = (self.pulse.get() + 50) % 1000;
        self.pulse.set(p);
        p
    }

    pub fn close(&self) {
        windows::close(self.id);
    }
}

// ---------------------------------------------------------------------------
// Quick Help

struct EscCloses {
    form: Rc<Form>,
    id: ViewportId,
}

impl Content for EscCloses {
    fn ui(&self, ui: &mut Ui) {
        Content::ui(&*self.form, ui);
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            windows::close(self.id);
        }
    }
}

thread_local! {
    static QH: RefCell<Option<Rc<Form>>> = const { RefCell::new(None) };
}

pub fn quick_help(parent: ViewportId, text: &str) {
    let form = QH.with(|q| {
        q.borrow_mut()
            .get_or_insert_with(|| {
                let f = load("QuickHelp");
                f.set_mono("QuickHelpBox", true);
                f.set_read_only("QuickHelpBox", true);
                f
            })
            .clone()
    });
    form.set_text("QuickHelpBox", text);
    let ctx = windows::ctx();
    let size = vcl::window_size(&form, &ctx);
    let id = ViewportId::from_hash_of("quickhelp");
    let content = Rc::new(EscCloses { form: form.clone(), id });
    let w = Window::new("quickhelp", &strip_amp(&form.caption()), content, parent, size).resizable(true, size * 0.5);
    windows::open(Rc::new(w));
}

/// `TQuickHelpForm::FormatString()`: tabs separate columns; align them (+4 spaces).
pub fn format_quick_help(text: &str) -> String {
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    loop {
        let mut tab_lines = 0usize;
        let mut max_col = 0usize;
        for l in &lines {
            if let Some(pos) = l.find('\t') {
                tab_lines += 1;
                max_col = max_col.max(l[..pos].chars().count());
            }
        }
        if tab_lines < 2 || max_col == 0 {
            break;
        }
        for l in &mut lines {
            if let Some(pos) = l.find('\t') {
                let col = l[..pos].chars().count();
                let pad = " ".repeat((max_col - col) + 4);
                l.replace_range(pos..pos + 1, &pad);
            }
        }
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Password List

pub struct PasswListWindow {
    pub form: Rc<Form>,
    pub id: ViewportId,
    window: RefCell<Option<Rc<Window>>>,
    size: Cell<Vec2>,
}

impl PasswListWindow {
    pub fn new(parent: ViewportId, file_opts: Rc<dyn Fn() -> (pwtool_core::text_file::CharEncoding, String)>) -> Rc<Self> {
        let form = load("PasswList");
        form.set_mono("PasswList", true);
        form.set_read_only("PasswList", true);
        form.set_secret("PasswList");
        form.set_popup_menu("PasswList", "PasswListMenu");
        let ctx = windows::ctx();
        let size = vcl::window_size(&form, &ctx);
        let id = ViewportId::from_hash_of("passwlist");
        let w = Rc::new(PasswListWindow { form: form.clone(), id, window: RefCell::new(None), size: Cell::new(size) });
        let ff = Rc::downgrade(&form);
        form.on_menu("PasswListMenu_Copy", move |_| {
            if let Some(f) = ff.upgrade() {
                let t = f.selected_text("PasswList");
                if !t.is_empty() {
                    super::clipboard::set_data(&t);
                }
            }
        });
        let ff = Rc::downgrade(&form);
        form.on_menu("PasswListMenu_SelectAll", move |_| {
            if let Some(f) = ff.upgrade() {
                f.select_all("PasswList");
            }
        });
        let ff = Rc::downgrade(&form);
        form.on_menu("PasswListMenu_SaveAsFile", move |_| {
            let (ff, opts) = (ff.clone(), file_opts.clone());
            crate::runtime::spawn(async move {
                let Some(p) = crate::vcl::dialogs::save_file(&trl("Save password list"), None, None, &[]).await else { return };
                let Some(f) = ff.upgrade() else { return };
                let raw = f.secret_text("PasswList");
                let (enc, newline) = opts();
                let text = if newline == "\n" { Zeroizing::new(raw.replace("\r\n", "\n")) } else { Zeroizing::new(raw.replace('\n', "\r\n")) };
                let res = pwtool_core::text_file::TextFileWriter::create(&p, enc, true).and_then(|mut w| w.write_str(&text));
                let parent = ViewportId::from_hash_of("passwlist");
                match res {
                    Ok(()) => msg(parent, &trl_format("File \"%1\" successfully created.", &[&p.file_name().unwrap_or_default().to_string_lossy()]), MsgIcon::Info),
                    Err(e) => msg(parent, &trl_format("Error while creating file\n\"%1\":\n%2.", &[&p.to_string_lossy(), &e.to_string()]), MsgIcon::Error),
                }
            });
        });
        let _ = parent;
        w
    }

    /// Update the enabled state of the list menu (each frame).
    pub fn update_menu(&self, db_open: bool) {
        let f = &self.form;
        let has_sel = f.selection("PasswList").is_some();
        let has_text = !f.secret_text("PasswList").is_empty();
        f.set_menu_enabled("PasswListMenu_Copy", has_sel);
        if f.has_menu_item("PasswListMenu_EncryptCopy") {
            f.set_menu_enabled("PasswListMenu_EncryptCopy", has_sel);
        }
        if f.has_menu_item("PasswListMenu_AddToDb") {
            f.set_menu_enabled("PasswListMenu_AddToDb", has_sel && db_open);
        }
        f.set_menu_enabled("PasswListMenu_SelectAll", has_text);
        f.set_menu_enabled("PasswListMenu_SaveAsFile", has_text);
    }

    pub fn load_config(&self, ini: &pwtool_core::config::IniFile) {
        let s = self.size.get();
        self.size.set(vec2(
            (ini.read_int("PasswList", "WindowWidth", s.x as i32).max(100)) as f32,
            (ini.read_int("PasswList", "WindowHeight", s.y as i32).max(100)) as f32,
        ));
    }

    pub fn save_config(&self, ini: &mut pwtool_core::config::IniFile) {
        let s = self.window.borrow().as_ref().map_or(self.size.get(), |w| w.size.get());
        ini.write_integer("PasswList", "WindowHeight", s.y as i64);
        ini.write_integer("PasswList", "WindowWidth", s.x as i64);
    }

    pub fn selected_text(&self) -> Zeroizing<String> {
        self.form.selected_text("PasswList")
    }

    pub fn is_open(&self) -> bool {
        windows::is_open(self.id)
    }

    pub fn close(&self) {
        self.form.set_text("PasswList", "");
        windows::close(self.id);
    }

    /// `Execute()`: show the list.
    pub fn show_list(&self, parent: ViewportId, text: &str) {
        self.form.set_text("PasswList", text);
        if self.is_open() {
            return;
        }
        let content = Rc::new(EscCloses { form: self.form.clone(), id: self.id });
        let min = self.form.min_size();
        let w = Rc::new(
            Window::new("passwlist", &strip_amp(&self.form.caption()), content, parent, self.size.get()).resizable(true, min.max(vec2(150.0, 100.0))),
        );
        let f = self.form.clone();
        *w.on_close.borrow_mut() = Some(Box::new(move || {
            // `FormClose()`: clear the list
            f.set_text("PasswList", "");
            true
        }));
        *self.window.borrow_mut() = Some(w.clone());
        windows::open(w);
    }
}

/// Events of a form not handled by handlers (helper for windows that poll).
pub fn clicked(events: &[Event], name: &str) -> bool {
    events.iter().any(|e| matches!(e, Event::Click(n) if n == name))
}

/// Make the form of window `id` known to the test automation (windows not
/// made with [`form_window`]).
pub fn register_form(id: &str, form: &Rc<Form>) {
    FORMS.with(|f| f.borrow_mut().insert(id.to_string(), Rc::downgrade(form)));
}
