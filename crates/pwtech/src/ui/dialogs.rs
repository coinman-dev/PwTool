//! Smaller windows of the generator: Advanced Password Options, Password
//! List, Progress, Quick Help, Info Box.

#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;
use pwtech_core::generate::{
    OPTION_INDEX_TO_BIT, PASSWOPTIONS_NUM, PASSWOPTIONS_STARRED, PasswOptions,
};
use pwtech_core::language::{trl, trl_format};
use pwtech_core::passw::generator::WORDLIST_MAX_WORDLEN;

use super::builder::{Built, attach_context_menu, build_form};
use super::form::load_form;

/// Create a window for a built form.
pub fn form_window(b: &Built, parent: Option<&gtk::Window>, modal: bool) -> gtk::Window {
    let win = gtk::Window::builder().title(&b.caption).modal(modal).build();
    if let Some(p) = parent {
        win.set_transient_for(Some(p));
    }
    win.set_child(Some(&b.root));
    if b.min.0 > 0 || b.min.1 > 0 {
        b.root.set_min_size(b.min.0.max(0), b.min.1.max(0));
    }
    win.set_default_size(b.client.0, b.client.1);
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
        info.set_text(&pwtech_core::unicode::format_w(
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
            let d2 = dd.clone();
            dlg.open(Some(&dd.win), gtk::gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r {
                    if let Some(p) = f.path() {
                        d2.b.get::<gtk::Entry>("TrigramFileBox").set_text(&p.to_string_lossy());
                    }
                }
            });
        });
        d
    }

    /// Text of an option by bit (`BitToString`).
    pub fn bit_to_string(&self, bit: usize) -> String {
        let idx = OPTION_INDEX_TO_BIT.iter().position(|&b| b == bit).unwrap_or(0);
        self.labels.get(idx).cloned().unwrap_or_default()
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
pub const PASSW_LIST_FONT_CLASS: &str = "pwtech-passwlist-font";

impl PasswListWindow {
    pub fn new(parent: &gtk::Window) -> Rc<Self> {
        let b = build_form(load_form("PasswList"), "PasswList");
        let win = form_window(&b, Some(parent), false);
        win.set_hide_on_close(true);
        let view: gtk::TextView = b.get("PasswList.view");
        view.set_monospace(true);
        view.set_editable(false);
        view.set_wrap_mode(gtk::WrapMode::None);
        if let Some(m) = b.menus.get("PasswListMenu") {
            attach_context_menu(&view, m);
        }
        let app_accels = b.accels.clone();
        view.add_css_class(PASSW_LIST_FONT_CLASS);
        let w = Rc::new(PasswListWindow { b, win, view, font: RefCell::new(None) });
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
            let (s, e) = buf.selection_bounds().unwrap_or((buf.start_iter(), buf.end_iter()));
            let t = zeroize::Zeroizing::new(buf.text(&s, &e, false).to_string());
            super::clipboard::set_data(&t);
        });
        let ww = w.clone();
        w.b.action("PasswListMenu_SelectAll").connect_activate(move |_, _| {
            let buf = ww.view.buffer();
            buf.select_range(&buf.start_iter(), &buf.end_iter());
        });
        let ww = w.clone();
        w.b.action("PasswListMenu_SaveAsFile").connect_activate(move |_, _| {
            let dlg = gtk::FileDialog::new();
            let w2 = ww.clone();
            dlg.save(Some(&ww.win), gtk::gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r {
                    if let Some(p) = f.path() {
                        let buf = w2.view.buffer();
                        let t = zeroize::Zeroizing::new(buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string());
                        if let Err(e) = std::fs::write(&p, t.as_bytes()) {
                            super::util::msg(Some(&w2.win), &e.to_string(), super::util::MsgIcon::Error);
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
    pub fn load_config(&self, ini: &pwtech_core::config::IniFile) {
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
    pub fn save_config(&self, ini: &mut pwtech_core::config::IniFile) {
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
        Rc::new(ProgressWindow { b, win, label, bar, cancel })
    }

    pub fn set(&self, done: u64, total: u64, fmt: &str) {
        if total > 0 {
            self.bar.set_fraction((done as f64 / total as f64).min(1.0));
        }
        self.label.set_text(&trl_format(fmt, &[&done.to_string(), &total.to_string()]));
    }
}

// ---------------------------------------------------------------------------
// Quick Help

pub fn quick_help(parent: &gtk::Window, text: &str) {
    thread_local! {
        static QH: RefCell<Option<(Built, gtk::Window)>> = const { RefCell::new(None) };
    }
    QH.with(|q| {
        let mut q = q.borrow_mut();
        if q.is_none() {
            let b = build_form(load_form("QuickHelp"), "QuickHelp");
            let win = form_window(&b, Some(parent), false);
            win.set_hide_on_close(true);
            *q = Some((b, win));
        }
        let (b, win) = q.as_ref().unwrap();
        let tv: gtk::TextView = b.get("QuickHelpBox.view");
        tv.set_monospace(true);
        tv.set_editable(false);
        tv.set_wrap_mode(gtk::WrapMode::None);
        tv.buffer().set_text(text);
        win.present();
    });
}

/// `TQuickHelpForm::FormatString()`: tabs separate columns; align them.
pub fn format_quick_help(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let width = lines
        .iter()
        .filter_map(|l| l.split_once('\t').map(|(a, _)| a.chars().count()))
        .max()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| match l.split_once('\t') {
            Some((a, b)) => format!("{a}{}  {b}", " ".repeat(width - a.chars().count())),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Info Box (`TInfoBoxForm::ShowInfo`): short-lived message.

pub fn info_box(parent: &gtk::Window, text: &str) {
    let win = gtk::Window::builder().decorated(false).resizable(false).transient_for(parent).build();
    let l = gtk::Label::new(Some(text));
    l.set_margin_top(14);
    l.set_margin_bottom(14);
    l.set_margin_start(24);
    l.set_margin_end(24);
    let fr = gtk::Frame::new(None);
    fr.set_child(Some(&l));
    win.set_child(Some(&fr));
    win.present();
    let w = win.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || w.close());
}
