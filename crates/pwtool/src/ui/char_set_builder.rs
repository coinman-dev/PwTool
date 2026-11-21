//! Character Set Builder (`TCharSetBuilderForm`, `upstream/src/main/
//! CharSetBuilder.cpp`). Child module of `main_window`.

#![allow(deprecated)]

use super::*;

const CONFIG_ID: &str = "CharSetBuilder";
pub use pwtool_app::char_set_builder::*;

pub(super) struct CharSetBuilderWin {
    b: Built,
    win: gtk::Window,
    updating: Cell<bool>,
}

impl CharSetBuilderWin {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let b = build_form(load_form("CharSetBuilder"), "CharSetBuilder");
        let win = super::super::dialogs::form_window(&b, Some(&main.win()), false);
        win.set_hide_on_close(true);
        for i in 1..=NUM_PARAM {
            if let Some(u) = b.updowns.get(&format!("NumSpinBtn{i}")) {
                u.set_min(0);
                u.set_position(0);
            }
        }
        let d = Rc::new(CharSetBuilderWin { b, win, updating: Cell::new(false) });
        for (i, name) in CHARSET_USED.iter().enumerate() {
            let dd = Rc::downgrade(&d);
            d.b.get::<gtk::CheckButton>(name).connect_toggled(move |_| {
                if let Some(dd) = dd.upgrade() {
                    dd.used_changed(i);
                }
            });
            d.used_changed_enable(i);
        }
        let mut entries: Vec<String> = (1..=NUM_PARAM).map(|i| format!("NumBox{i}")).collect();
        entries.extend(["FromBox", "ToBox", "AdditionalCharsBox", "ExcludeCharsBox", "TagBox"].map(String::from));
        for e in entries {
            let dd = Rc::downgrade(&d);
            d.b.get::<gtk::Entry>(&e).connect_changed(move |_| {
                if let Some(dd) = dd.upgrade() {
                    dd.param_change();
                }
            });
        }
        for i in 1..=NUM_PARAM {
            let dd = Rc::downgrade(&d);
            d.b.get::<gtk::CheckButton>(&format!("AtLeastCheck{i}")).connect_toggled(move |_| {
                if let Some(dd) = dd.upgrade() {
                    dd.param_change();
                }
            });
        }
        let dd = Rc::downgrade(&d);
        d.b.get::<gtk::Button>("ResetBtn").connect_clicked(move |_| {
            if let Some(dd) = dd.upgrade() {
                dd.reset();
            }
        });
        let w = Rc::downgrade(main);
        let dd = Rc::downgrade(&d);
        d.b.get::<gtk::Button>("ApplyBtn").connect_clicked(move |_| {
            if let (Some(w), Some(dd)) = (w.upgrade(), dd.upgrade()) {
                let text = dd.b.get::<gtk::Entry>("ResultBox").text().to_string();
                set_combo_text(&w.combo("CharSetList"), &text);
                w.char_set_list_exit();
            }
        });
        main.attach_entropy_events(&d.win);
        d
    }

    fn used_changed_enable(&self, i: usize) {
        let on = self.b.get::<gtk::CheckButton>(CHARSET_USED[i]).is_active();
        let n = i + 1;
        self.b.w(&format!("NumBox{n}")).set_sensitive(on);
        self.b.updowns[&format!("NumSpinBtn{n}")].set_sensitive(on);
        self.b.w(&format!("AtLeastCheck{n}")).set_sensitive(on);
        match i {
            4 => {
                for w in ["FromLbl", "ToLbl", "FromBox", "ToBox"] {
                    self.b.w(w).set_sensitive(on);
                }
            }
            5 => self.b.w("AdditionalCharsBox").set_sensitive(on),
            _ => {}
        }
    }

    fn used_changed(&self, i: usize) {
        self.used_changed_enable(i);
        self.param_change();
        let on = self.b.get::<gtk::CheckButton>(CHARSET_USED[i]).is_active();
        if on && i == 4 {
            self.b.w("FromBox").grab_focus();
        } else if on && i == 5 {
            self.b.w("AdditionalCharsBox").grab_focus();
        }
    }

    fn params(&self) -> BuilderParams {
        let mut p = BuilderParams::default();
        for i in 0..NUM_PARAM {
            p.used[i] = self.b.get::<gtk::CheckButton>(CHARSET_USED[i]).is_active();
            p.num_chars[i] = self.b.updowns[&format!("NumSpinBtn{}", i + 1)].position();
            p.at_least[i] = self.b.get::<gtk::CheckButton>(&format!("AtLeastCheck{}", i + 1)).is_active();
        }
        let t = |n: &str| self.b.get::<gtk::Entry>(n).text().to_string();
        p.from = t("FromBox");
        p.to = t("ToBox");
        p.additional = t("AdditionalCharsBox");
        p.exclude = t("ExcludeCharsBox");
        p.tag = t("TagBox");
        p
    }

    fn param_change(&self) {
        if self.updating.get() {
            return;
        }
        match build_char_set(&self.params()) {
            Ok(s) => {
                self.b.get::<gtk::Entry>("ResultBox").set_text(&s);
                self.b.w("ApplyBtn").set_sensitive(!s.is_empty());
            }
            Err(BuildError::InvalidRange) => msg(Some(&self.win), &trl("Invalid character range."), MsgIcon::Error),
        }
    }

    fn reset(&self) {
        self.updating.set(true);
        for i in 0..NUM_PARAM {
            self.b.get::<gtk::CheckButton>(CHARSET_USED[i]).set_active(false);
            self.b.updowns[&format!("NumSpinBtn{}", i + 1)].set_position(0);
            self.b.get::<gtk::CheckButton>(&format!("AtLeastCheck{}", i + 1)).set_active(false);
        }
        for e in ["FromBox", "ToBox", "AdditionalCharsBox"] {
            self.b.get::<gtk::Entry>(e).set_text("");
        }
        self.updating.set(false);
        self.param_change();
    }

    fn save_config(&self, ini: &mut pwtool_core::config::IniFile) {
        ini.write_integer(CONFIG_ID, "WindowHeight", self.win.default_height() as i64);
        ini.write_integer(CONFIG_ID, "WindowWidth", self.win.default_width() as i64);
    }
}

impl MainWindow {
    /// `BuildBtnClick()`.
    pub(super) fn char_set_builder(self: &Rc<Self>) {
        let existing = self.char_set_builder.borrow().clone();
        let d = match existing {
            Some(d) => d,
            None => {
                let d = CharSetBuilderWin::new(self);
                *self.char_set_builder.borrow_mut() = Some(d.clone());
                d
            }
        };
        // `FormShow()`: only clear TagBox when opening from hidden state
        if !d.win.is_visible() {
            d.b.get::<gtk::Entry>("TagBox").set_text("");
        }
        d.win.present();
    }

    pub(super) fn save_char_set_builder_config(&self, ini: &mut pwtool_core::config::IniFile) {
        if let Some(d) = self.char_set_builder.borrow().as_ref() {
            d.save_config(ini);
        }
    }
}

