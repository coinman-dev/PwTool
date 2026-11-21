//! Character Set Builder (`TCharSetBuilderForm`, `upstream/src/main/
//! CharSetBuilder.cpp`). Child module of `main_window`.

use pwtool_app::char_set_builder::{BuildError, BuilderParams, CHARSET_USED, NUM_PARAM, build_char_set};

use super::*;

const WINDOW_ID: &str = "CharSetBuilder";

pub(super) struct CharSetBuilderWin {
    form: Rc<Form>,
    updating: Cell<bool>,
}

impl CharSetBuilderWin {
    fn new(main: &Rc<MainWindow>) -> Rc<Self> {
        let form = dialogs::load("CharSetBuilder");
        for i in 1..=NUM_PARAM {
            form.set_range(&format!("NumSpinBtn{i}"), 0, form_max(&form, i));
            form.set_position(&format!("NumSpinBtn{i}"), 0);
        }
        let d = Rc::new(CharSetBuilderWin { form, updating: Cell::new(false) });
        for i in 0..NUM_PARAM {
            d.used_changed_enable(i);
        }
        let dd = Rc::downgrade(&d);
        let w = Rc::downgrade(main);
        d.form.on_any(move |e| {
            let (Some(d), Some(w)) = (dd.upgrade(), w.upgrade()) else { return };
            use crate::vcl::Event::*;
            match e {
                Click(n) if n == "ResetBtn" => d.reset(),
                Click(n) if n == "ApplyBtn" => {
                    let text = d.form.text("ResultBox");
                    w.f().set_text("CharSetList", &text);
                    w.char_set_list_exit();
                }
                Click(n) => {
                    if let Some(i) = CHARSET_USED.iter().position(|c| c == n) {
                        d.used_changed(i);
                    } else if n.starts_with("AtLeastCheck") || n.starts_with("NumSpinBtn") {
                        d.param_change();
                    }
                }
                Change(n) if n != "ResultBox" => d.param_change(),
                _ => {}
            }
        });
        d
    }

    fn used_changed_enable(&self, i: usize) {
        let f = &self.form;
        let on = f.checked(CHARSET_USED[i]);
        let n = i + 1;
        for c in [format!("NumBox{n}"), format!("NumSpinBtn{n}"), format!("AtLeastCheck{n}")] {
            f.set_enabled(&c, on);
        }
        match i {
            4 => {
                for c in ["FromLbl", "ToLbl", "FromBox", "ToBox"] {
                    f.set_enabled(c, on);
                }
            }
            5 => f.set_enabled("AdditionalCharsBox", on),
            _ => {}
        }
    }

    fn used_changed(&self, i: usize) {
        self.used_changed_enable(i);
        self.param_change();
        let on = self.form.checked(CHARSET_USED[i]);
        if on && i == 4 {
            self.form.focus("FromBox");
        } else if on && i == 5 {
            self.form.focus("AdditionalCharsBox");
        }
    }

    fn params(&self) -> BuilderParams {
        let f = &self.form;
        let mut p = BuilderParams::default();
        for i in 0..NUM_PARAM {
            p.used[i] = f.checked(CHARSET_USED[i]);
            p.num_chars[i] = f.position(&format!("NumSpinBtn{}", i + 1));
            p.at_least[i] = f.checked(&format!("AtLeastCheck{}", i + 1));
        }
        p.from = f.text("FromBox");
        p.to = f.text("ToBox");
        p.additional = f.text("AdditionalCharsBox");
        p.exclude = f.text("ExcludeCharsBox");
        p.tag = f.text("TagBox");
        p
    }

    fn param_change(&self) {
        if self.updating.get() {
            return;
        }
        match build_char_set(&self.params()) {
            Ok(s) => {
                self.form.set_text("ResultBox", &s);
                self.form.set_enabled("ApplyBtn", !s.is_empty());
            }
            Err(BuildError::InvalidRange) => msg(ViewportId::from_hash_of(WINDOW_ID), &trl("Invalid character range."), MsgIcon::Error),
        }
    }

    fn reset(&self) {
        self.updating.set(true);
        for i in 0..NUM_PARAM {
            self.form.set_checked(CHARSET_USED[i], false);
            self.form.set_position(&format!("NumSpinBtn{}", i + 1), 0);
            self.form.set_checked(&format!("AtLeastCheck{}", i + 1), false);
            self.used_changed_enable(i);
        }
        for e in ["FromBox", "ToBox", "AdditionalCharsBox"] {
            self.form.set_text(e, "");
        }
        self.updating.set(false);
        self.param_change();
    }
}

/// Maximum of a "number of characters" spin button (the form's `Max`).
fn form_max(form: &Form, i: usize) -> i64 {
    form.obj.find(&format!("NumSpinBtn{i}")).and_then(|o| o.int("Max")).filter(|&m| m > 0).unwrap_or(100)
}

impl MainWindow {
    /// `BuildBtnClick()`.
    pub(super) fn char_set_builder(self: &Rc<Self>) {
        let existing = self.tools.char_set_builder.borrow().clone();
        let d = match existing {
            Some(d) => d,
            None => {
                let d = CharSetBuilderWin::new(self);
                *self.tools.char_set_builder.borrow_mut() = Some(d.clone());
                d
            }
        };
        // `FormShow()`: only clear TagBox when opening from hidden state
        if !windows::is_open(ViewportId::from_hash_of(WINDOW_ID)) {
            d.form.set_text("TagBox", "");
            d.param_change();
        }
        self.show_tool(WINDOW_ID, "CharSetBuilder", true, &d.form, ROOT, false);
    }
}
