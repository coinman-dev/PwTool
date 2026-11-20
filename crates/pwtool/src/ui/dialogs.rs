//! Smaller windows of the generator: Advanced Password Options, Password
//! List, Progress, Quick Help, Info Box.

#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;
use pwtool_core::generate::{
    OPTION_INDEX_TO_BIT, PASSWOPTIONS_NUM, PASSWOPTIONS_STARRED, PasswOptions,
};
use pwtool_core::language::{trl, trl_format};
use pwtool_core::passw::generator::WORDLIST_MAX_WORDLEN;

use super::builder::{Built, attach_context_menu, attach_context_menu_capture, build_form};
use super::form::load_form;

/// Client size of the windows that differ from the size of the original
/// form, in design pixels of the form (before scaling): the main window is
/// a little smaller, the Configuration dialog wider — its tab captions do
/// not fit into the original width with the fonts and translations used
/// here.
const FIXED_CLIENT_SIZE: &[(&str, f64, f64)] = &[("MainForm", 542.5, 694.5), ("ConfigurationDlg", 607.0, 556.5)];

/// Client size of a form's window. The windows have a fixed size (see
/// [`form_window`]), so this is the only size they ever get.
pub fn fixed_client_size(b: &Built) -> (i32, i32) {
    match FIXED_CLIENT_SIZE.iter().find(|(name, _, _)| *name == b.form.name) {
        Some((_, w, h)) => ((w * b.scale).round() as i32, (h * b.scale).round() as i32),
        None => b.client,
    }
}

/// Create a window for a built form.
///
/// The windows are not resizable — on Wayland this is the only way to keep
/// them from being maximized (a window that can be resized can always be
/// maximized by the compositor). Their size is the size of the form, see
/// [`fixed_client_size`]. Windows whose content needs room (Password List,
/// PassCube) make themselves resizable again.
pub fn form_window(b: &Built, parent: Option<&gtk::Window>, modal: bool) -> gtk::Window {
    let win = gtk::Window::builder().title(&b.caption).modal(modal).resizable(false).build();
    if let Some(p) = parent {
        win.set_transient_for(Some(p));
    }
    win.set_child(Some(&b.root));
    let (w, h) = fixed_client_size(b);
    b.root.set_fixed_size(w, h);
    win.set_default_size(w, h);
    win.insert_action_group("m", Some(&b.actions));
    win
}

/// Wait until a window is closed.
pub async fn wait_closed(win: &gtk::Window) {
    let (tx, rx) = async_channel::bounded::<()>(1);
    let h = win.connect_close_request(move |_| {
        let _ = tx.try_send(());
        glib::Propagation::Proceed
    });
    let _ = rx.recv().await;
    win.disconnect(h);
}

// ---------------------------------------------------------------------------
// Advanced Password Options

pub struct PasswOptionsDlg {
    pub b: Built,
    pub win: gtk::Window,
    checks: Vec<gtk::CheckButton>,
    labels: Vec<String>,
}

impl PasswOptionsDlg {
    pub fn new(parent: &gtk::Window) -> Rc<Self> {
        let b = build_form(load_form("PasswOptions"), "PasswOptions");
        let win = form_window(&b, Some(parent), true);
        win.set_hide_on_close(true);
        let obj = b.obj("PasswOptionsList").unwrap().clone();
        let items = obj.strings("Items.Strings");
        let suffix = [
            " (B8G6I1l|0OQDS5Z2) [1-3]", " [1-4]", " [2]", " [2]", " [1,2]", " [1,4]", " [1,4]", " [1,4]", " [1,4]",
            " [1]", " [1,3]", " [1-4]", " [2,3]", " [1]", " [2]", " [2]", " [1-4]", " [1]", " [1,3]",
        ];
        let mut labels: Vec<String> = items.iter().map(|s| trl(s)).collect();
        let bit_to_index = |bit: usize| OPTION_INDEX_TO_BIT.iter().position(|&b| b == bit).unwrap();
        let mut display = labels.clone();
        for (bit, suf) in suffix.iter().enumerate() {
            let idx = bit_to_index(bit);
            display[idx].push_str(suf);
            if PASSWOPTIONS_STARRED[bit] {
                display[idx].push_str(" *");
            }
        }
        let lb: gtk::ListBox = b.get("PasswOptionsList.list");
        while let Some(c) = lb.first_child() {
            lb.remove(&c);
        }
        let mut checks = Vec::new();
        for d in &display {
            let c = gtk::CheckButton::with_label(d);
            lb.append(&c);
            checks.push(c);
        }
        labels.truncate(PASSWOPTIONS_NUM);
        let info: gtk::Label = b.get("InfoLbl");
        info.set_wrap(true);
        info.set_text(&pwtool_core::unicode::format_w(
            "[1] %1 [2] %2 [3] %3 [4] %4\n* %5",
            &[
                &trl("Applies to pass_words_."),
                &trl("Applies to pass_phrases_."),
                &trl("Applies to formatted passwords."),
                &trl("Applies to phonetic passwords."),
                &trl("Selecting one of these options might reduce the security of generated passwords."),
            ],
        ));
        for name in ["MinWordLenSpinBtn", "MaxWordLenSpinBtn"] {
            if let Some(u) = b.updowns.get(name) {
                u.set_min(1);
                u.set_max(WORDLIST_MAX_WORDLEN as i64);
            }
        }
        let d = Rc::new(PasswOptionsDlg { b, win, checks, labels });
        // list menu
        if let Some(m) = d.b.menus.get("ListMenu") {
            attach_context_menu(&lb, m);
        }
        for (name, f) in [
            ("ListMenu_SelectAll", 0),
            ("ListMenu_DeselectAll", 1),
            ("ListMenu_InvertSelection", 2),
        ] {
            let dd = d.clone();
            d.b.action(name).connect_activate(move |_, _| {
                for c in &dd.checks {
                    c.set_active(match f {
                        0 => true,
                        1 => false,
                        _ => !c.is_active(),
                    });
                }
            });
        }
        let dd = d.clone();
        d.b.get::<gtk::Button>("BrowseBtn").connect_clicked(move |_| {
            let dlg = gtk::FileDialog::new();
            dlg.set_title(&trl("Select trigram file"));
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            let f = gtk::FileFilter::new();
            f.set_name(Some(&format!("{} (*.tgm)", trl("Trigram files"))));
            f.add_pattern("*.tgm");
            filters.append(&f);
            let f = gtk::FileFilter::new();
            f.set_name(Some(&format!("{} (*.*)", trl("All files"))));
            f.add_pattern("*");
            filters.append(&f);
            dlg.set_filters(Some(&filters));
            let d2 = dd.clone();
            dlg.open(Some(&dd.win), gtk::gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r {
                    if let Some(p) = f.path() {
                        let e = d2.b.get::<gtk::Entry>("TrigramFileBox");
                        e.set_text(&p.to_string_lossy());
                        e.grab_focus();
                    }
                }
            });
        });
        d
    }

    /// Translated label of an option by bit, usable before the dialog is constructed.
    pub fn bit_name(bit: usize) -> String {
        const LABELS: [&str; PASSWOPTIONS_NUM] = [
            "Exclude ambiguous characters",
            "First character must not be a lower-case letter",
            "Each character must occur only once",
            "Exclude repeating consecutive characters (e.g. aa, 11)",
            "Remove leading and trailing whitespace characters",
            "Don't separate words by a space",
            "Don't separate words and characters by a '-' character",
            "Reverse default order of character/word combinations",
            "Capitalize first letter of each word",
            "Convert all words in word lists to lower-case",
            "Each word must occur only once",
            "Include at least one upper-case letter (A-Z)",
            "Include at least one lower-case letter (a-z)",
            "Include at least one digit (0-9)",
            "Include at least one special symbol (!\"#$%...)",
            "Include at least one character from each subset",
            "Only include characters from custom character set",
            "Exclude duplicate entries in password lists",
            "Assign entropy value to each password in password lists",
        ];
        let idx = OPTION_INDEX_TO_BIT.iter().position(|&b| b == bit).unwrap_or(0);
        LABELS.get(idx).map(|s| trl(s)).unwrap_or_else(|| format!("#{bit}"))
    }

    /// Text of an option by bit (`BitToString`).
    pub fn bit_to_string(&self, bit: usize) -> String {
        let idx = OPTION_INDEX_TO_BIT.iter().position(|&b| b == bit).unwrap_or(0);
        self.labels.get(idx).cloned().unwrap_or_else(|| Self::bit_name(bit))
    }

    pub fn set_options(&self, o: &PasswOptions) {
        for bit in 0..PASSWOPTIONS_NUM {
            let idx = OPTION_INDEX_TO_BIT.iter().position(|&b| b == bit).unwrap();
            self.checks[idx].set_active(o.flags & (1 << bit) != 0);
        }
        self.b.get::<gtk::Entry>("AmbigCharsBox").set_text(&o.ambig_chars);
        self.b.get::<gtk::Entry>("SpecialSymBox").set_text(&o.special_symbols);
        self.b.updowns["MinWordLenSpinBtn"].set_position(o.min_word_len as i64);
        self.b.updowns["MaxWordLenSpinBtn"].set_position(o.max_word_len as i64);
        self.b.get::<gtk::Entry>("WordSepBox").set_text(&o.word_separator);
        self.b.get::<gtk::Entry>("WordCharSepBox").set_text(&o.word_char_separator);
        self.b.get::<gtk::Entry>("TrigramFileBox").set_text(&o.trigram_file_name);
    }

    pub fn get_options(&self) -> PasswOptions {
        let mut flags = 0u32;
        for (idx, c) in self.checks.iter().enumerate() {
            if c.is_active() {
                flags |= 1 << OPTION_INDEX_TO_BIT[idx];
            }
        }
        let min = self.b.updowns["MinWordLenSpinBtn"].position() as i32;
        let max = self.b.updowns["MaxWordLenSpinBtn"].position() as i32;
        PasswOptions {
            flags,
            ambig_chars: self.b.get::<gtk::Entry>("AmbigCharsBox").text().to_string(),
            special_symbols: self.b.get::<gtk::Entry>("SpecialSymBox").text().to_string(),
            min_word_len: min,
            max_word_len: max.max(min),
            word_separator: self.b.get::<gtk::Entry>("WordSepBox").text().to_string(),
            word_char_separator: self.b.get::<gtk::Entry>("WordCharSepBox").text().to_string(),
            trigram_file_name: self.b.get::<gtk::Entry>("TrigramFileBox").text().to_string(),
        }
    }

    /// `ShowModal()`: returns the options if OK was pressed.
    pub async fn run(self: &Rc<Self>, o: &PasswOptions) -> Option<PasswOptions> {
        self.set_options(o);
        let result = Rc::new(Cell::new(false));
        let (tx, rx) = async_channel::bounded::<()>(1);
        let ok: gtk::Button = self.b.get("OKBtn");
        let cancel: gtk::Button = self.b.get("CancelBtn");
        let r = result.clone();
        let w = self.win.clone();
        let h1 = ok.connect_clicked(move |_| {
            r.set(true);
            w.close();
        });
        let w = self.win.clone();
        let h2 = cancel.connect_clicked(move |_| w.close());
        let h3 = self.win.connect_close_request(move |_| {
            let _ = tx.try_send(());
            glib::Propagation::Proceed
        });
        self.win.set_default_widget(Some(&ok));
        self.win.present();
        let _ = rx.recv().await;
        ok.disconnect(h1);
        cancel.disconnect(h2);
        self.win.disconnect(h3);
        if result.get() { Some(self.get_options()) } else { None }
    }
}

// ---------------------------------------------------------------------------
// Password List

pub struct PasswListWindow {
    pub b: Built,
    pub win: gtk::Window,
    pub view: gtk::TextView,
    /// Font set by the user (`m_pDefaultPasswFont != nullptr`).
    pub font: RefCell<Option<super::util::FontSpec>>,
}

/// CSS class of the password list font.
pub const PASSW_LIST_FONT_CLASS: &str = "pwtool-passwlist-font";

impl PasswListWindow {
    pub fn new(
        parent: &gtk::Window,
        get_file_opts: Box<dyn Fn() -> (pwtool_core::text_file::CharEncoding, String)>,
        is_db_open: Box<dyn Fn() -> bool>,
    ) -> Rc<Self> {
        let b = build_form(load_form("PasswList"), "PasswList");
        let win = form_window(&b, Some(parent), false);
        // a list of passwords needs room: this window can be resized
        win.set_resizable(true);
        b.root.set_fixed_size(0, 0);
        b.root.set_min_size(b.min.0.max(0), b.min.1.max(0));
        win.set_hide_on_close(true);
        let view: gtk::TextView = b.get("PasswList.view");
        view.set_monospace(true);
        view.set_editable(false);
        view.set_wrap_mode(gtk::WrapMode::None);
        let pop = b.menus.get("PasswListMenu").map(|m| attach_context_menu_capture(&view, m));
        let app_accels = b.accels.clone();
        view.add_css_class(PASSW_LIST_FONT_CLASS);
        let w = Rc::new(PasswListWindow { b, win, view, font: RefCell::new(None) });
        if let Some(pop) = pop {
            let ww = Rc::downgrade(&w);
            pop.connect_show(move |_| {
                if let Some(w) = ww.upgrade() {
                    let buf = w.view.buffer();
                    let has_sel = buf.selection_bounds().is_some();
                    let has_text = buf.char_count() > 0;
                    let db_open = is_db_open();
                    w.b.action("PasswListMenu_Copy").set_enabled(has_sel);
                    w.b.action("PasswListMenu_EncryptCopy").set_enabled(has_sel);
                    w.b.action("PasswListMenu_AddToDb").set_enabled(has_sel && db_open);
                    w.b.action("PasswListMenu_SelectAll").set_enabled(has_text);
                    w.b.action("PasswListMenu_SaveAsFile").set_enabled(has_text);
                }
            });
        }
        let ww = w.clone();
        w.b.action("PasswListMenu_ChangeFont").connect_activate(move |_, _| {
            let w2 = ww.clone();
            let cur = ww.font.borrow().clone();
            super::util::choose_font(&ww.win, cur.as_ref(), move |f| {
                super::util::set_class_font(PASSW_LIST_FONT_CLASS, Some(&f));
                *w2.font.borrow_mut() = Some(f);
            });
        });
        let ww = w.clone();
        w.b.action("PasswListMenu_ResetFont").connect_activate(move |_, _| {
            if ww.font.borrow_mut().take().is_some() {
                super::util::set_class_font(PASSW_LIST_FONT_CLASS, None);
            }
        });
        let esc = gtk::EventControllerKey::new();
        let ww = w.clone();
        esc.connect_key_pressed(move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape {
                ww.win.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        w.win.add_controller(esc);
        let ww = w.clone();
        w.win.connect_close_request(move |_| {
            // `FormClose()`: clear the list
            ww.view.buffer().set_text("");
            glib::Propagation::Proceed
        });
        let ww = w.clone();
        w.b.action("PasswListMenu_Copy").connect_activate(move |_, _| {
            let buf = ww.view.buffer();
            if let Some((s, e)) = buf.selection_bounds() {
                let t = zeroize::Zeroizing::new(buf.text(&s, &e, false).to_string());
                super::clipboard::set_data(&t);
            }
        });
        let ww = w.clone();
        w.b.action("PasswListMenu_SelectAll").connect_activate(move |_, _| {
            let buf = ww.view.buffer();
            buf.select_range(&buf.start_iter(), &buf.end_iter());
        });
        let ww = w.clone();
        let get_opts = Rc::new(get_file_opts);
        w.b.action("PasswListMenu_SaveAsFile").connect_activate(move |_, _| {
            let dlg = gtk::FileDialog::new();
            dlg.set_title(&trl("Save password list"));
            let w2 = ww.clone();
            let get_opts2 = get_opts.clone();
            dlg.save(Some(&ww.win), gtk::gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r {
                    if let Some(p) = f.path() {
                        let buf = w2.view.buffer();
                        let raw = zeroize::Zeroizing::new(buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string());
                        let (enc, newline) = get_opts2();
                        let normalized = if newline == "\n" {
                            zeroize::Zeroizing::new(raw.replace("\r\n", "\n"))
                        } else if newline == "\r\n" && !raw.contains("\r\n") {
                            zeroize::Zeroizing::new(raw.replace('\n', "\r\n"))
                        } else {
                            raw
                        };
                        let res = pwtool_core::text_file::TextFileWriter::create(&p, enc, true)
                            .and_then(|mut fw| fw.write_str(&normalized));
                        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                        match res {
                            Ok(()) => super::util::msg(
                                Some(&w2.win),
                                &trl_format("File \"%1\" successfully created.", &[&name]),
                                super::util::MsgIcon::Info,
                            ),
                            Err(e) => super::util::msg(
                                Some(&w2.win),
                                &trl_format("Error while creating file\n\"%1\":\n%2.", &[&p.to_string_lossy(), &e.to_string()]),
                                super::util::MsgIcon::Error,
                            ),
                        }
                    }
                }
            });
        });
        let sc = gtk::ShortcutController::new();
        for (action, accel) in app_accels {
            if let Some(t) = gtk::ShortcutTrigger::parse_string(&accel) {
                sc.add_shortcut(gtk::Shortcut::new(Some(t), Some(gtk::NamedAction::new(&action))));
            }
        }
        w.win.add_controller(sc);
        w
    }

    /// `LoadConfig()`.
    pub fn load_config(&self, ini: &pwtool_core::config::IniFile) {
        self.win.set_default_size(
            ini.read_int("PasswList", "WindowWidth", self.b.client.0).max(100),
            ini.read_int("PasswList", "WindowHeight", self.b.client.1).max(100),
        );
        if let Some(f) = super::util::FontSpec::parse(&ini.read_string("PasswList", "Font", "")) {
            super::util::set_class_font(PASSW_LIST_FONT_CLASS, Some(&f));
            *self.font.borrow_mut() = Some(f);
        }
    }

    /// `SaveConfig()`.
    pub fn save_config(&self, ini: &mut pwtool_core::config::IniFile) {
        ini.write_integer("PasswList", "WindowHeight", self.win.default_height() as i64);
        ini.write_integer("PasswList", "WindowWidth", self.win.default_width() as i64);
        ini.write_string(
            "PasswList",
            "Font",
            &self.font.borrow().as_ref().map(|f| f.to_font_string()).unwrap_or_default(),
        );
    }

    /// Selected text (`GetRichEditSelTextBuf()`).
    pub fn selected_text(&self) -> zeroize::Zeroizing<String> {
        let buf = self.view.buffer();
        match buf.selection_bounds() {
            Some((s, e)) => zeroize::Zeroizing::new(buf.text(&s, &e, false).to_string()),
            None => zeroize::Zeroizing::new(String::new()),
        }
    }

    /// `Execute()`: show the list.
    pub fn show_list(&self, text: &str) {
        self.view.buffer().set_text(text);
        self.win.present();
    }
}

// ---------------------------------------------------------------------------
// Progress

pub struct ProgressWindow {
    pub b: Built,
    pub win: gtk::Window,
    pub label: gtk::Label,
    pub bar: gtk::ProgressBar,
    pub cancel: Rc<Cell<bool>>,
}

impl ProgressWindow {
    pub fn new(parent: &gtk::Window, title: &str, text: &str) -> Rc<Self> {
        let b = build_form(load_form("Progress"), "Progress");
        let win = form_window(&b, Some(parent), true);
        win.set_title(Some(title));
        win.set_deletable(false);
        let label: gtk::Label = b.get("ProgressLbl");
        label.set_text(text);
        // replace the drawing area by a progress bar
        let area = b.w("ProgressBar");
        let bar = gtk::ProgressBar::new();
        if let Some((x, y, w, h)) = b.root.child_rect(&area) {
            area.set_visible(false);
            b.root.put(&bar, x, y, w, h, super::layout::AK_LEFT | super::layout::AK_TOP | super::layout::AK_RIGHT, super::layout::Align::None);
        }
        let cancel = Rc::new(Cell::new(false));
        let c = cancel.clone();
        b.get::<gtk::Button>("CancelBtn").connect_clicked(move |_| c.set(true));
        // Keyboard cancel: Escape, Ctrl+C, Ctrl+Q, Ctrl+X (`TProgressForm::FormKeyDown`)
        let key = gtk::EventControllerKey::new();
        let c2 = cancel.clone();
        key.connect_key_pressed(move |_, k, _, mods| {
            let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            if k == gtk::gdk::Key::Escape
                || (ctrl && matches!(k.to_lower(), gtk::gdk::Key::c | gtk::gdk::Key::q | gtk::gdk::Key::x))
            {
                c2.set(true);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.add_controller(key);
        Rc::new(ProgressWindow { b, win, label, bar, cancel })
    }

    pub fn set(&self, done: u64, total: u64, fmt: &str) {
        if total > 0 {
            self.bar.set_fraction((done as f64 / total as f64).min(1.0));
        } else {
            self.bar.pulse();
        }
        self.label.set_text(&trl_format(fmt, &[&done.to_string(), &total.to_string()]));
    }

    /// Show a custom status message (e.g. `"Waiting for script ..."`).
    pub fn set_message(&self, msg: &str) {
        self.label.set_text(msg);
        self.bar.pulse();
    }
}

// ---------------------------------------------------------------------------
// Quick Help

/// CSS class of the Quick Help text view font.
pub const QUICK_HELP_FONT_CLASS: &str = "pwtool-quickhelp-font";

thread_local! {
    static QH: RefCell<Option<(Built, gtk::Window)>> = const { RefCell::new(None) };
    static QH_FONT: RefCell<Option<super::util::FontSpec>> = const { RefCell::new(None) };
}

pub fn load_quick_help_config(ini: &pwtool_core::config::IniFile) {
    if let Some(f) = super::util::FontSpec::parse(&ini.read_string("QuickHelp", "Font", "")) {
        super::util::set_class_font(QUICK_HELP_FONT_CLASS, Some(&f));
        QH_FONT.with(|slot| *slot.borrow_mut() = Some(f));
    }
}

pub fn save_quick_help_config(ini: &mut pwtool_core::config::IniFile) {
    QH_FONT.with(|slot| {
        if let Some(f) = slot.borrow().as_ref() {
            ini.write_string("QuickHelp", "Font", &f.to_font_string());
        }
    });
}

pub fn quick_help(parent: &gtk::Window, text: &str) {
    QH.with(|q| {
        let mut q = q.borrow_mut();
        if q.is_none() {
            let b = build_form(load_form("QuickHelp"), "QuickHelp");
            let win = form_window(&b, Some(parent), false);
            win.set_hide_on_close(true);
            super::util::close_on_escape(&win);
            let tv: gtk::TextView = b.get("QuickHelpBox.view");
            tv.set_monospace(true);
            tv.set_editable(false);
            tv.set_wrap_mode(gtk::WrapMode::None);
            tv.add_css_class(QUICK_HELP_FONT_CLASS);
            if let Some(m) = b.menus.get("QuickHelpBoxMenu") {
                attach_context_menu_capture(&tv, m);
            }
            if b.has_action("QuickHelpBoxMenu_ChangeFont") {
                let w2 = win.clone();
                b.action("QuickHelpBoxMenu_ChangeFont").connect_activate(move |_, _| {
                    let cur = QH_FONT.with(|s| s.borrow().clone());
                    super::util::choose_font(&w2, cur.as_ref(), move |f| {
                        super::util::set_class_font(QUICK_HELP_FONT_CLASS, Some(&f));
                        QH_FONT.with(|s| *s.borrow_mut() = Some(f));
                    });
                });
            }
            *q = Some((b, win));
        }
        let (b, win) = q.as_ref().unwrap();
        let tv: gtk::TextView = b.get("QuickHelpBox.view");
        tv.buffer().set_text(text);
        win.present();
    });
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
// Info Box (`TInfoBoxForm::ShowInfo`): short-lived message.

pub fn info_box(parent: &gtk::Window, text: &str) {
    thread_local! {
        static ACTIVE_INFO: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
    }
    ACTIVE_INFO.with(|slot| {
        if let Some(prev) = slot.borrow_mut().take() {
            prev.close();
        }
    });
    let win = gtk::Window::builder().decorated(false).resizable(false).transient_for(parent).build();
    let l = gtk::Label::new(Some(text));
    l.set_margin_top(14);
    l.set_margin_bottom(14);
    l.set_margin_start(24);
    l.set_margin_end(24);
    let fr = gtk::Frame::new(None);
    fr.set_child(Some(&l));
    win.set_child(Some(&fr));
    ACTIVE_INFO.with(|slot| *slot.borrow_mut() = Some(win.clone()));
    win.present();
    let w = win.clone();
    let ms = (750 + text.chars().count() as u64 * 20).max(1500);
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
        ACTIVE_INFO.with(|slot| {
            if slot.borrow().as_ref().is_some_and(|cur| cur == &w) {
                slot.borrow_mut().take();
            }
        });
        w.close();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_help_tab_alignment() {
        let input = "a\tb\tc\nlong\tx\ty";
        let out = format_quick_help(input);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        // Both columns aligned with +4 spaces minimum gap
        assert_eq!(lines[0], "a       b    c");
        assert_eq!(lines[1], "long    x    y");
    }
}
