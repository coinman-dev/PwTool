//! Character Set Builder (`TCharSetBuilderForm`, `upstream/src/main/
//! CharSetBuilder.cpp`). Child module of `main_window`.

#![allow(deprecated)]

use std::collections::BTreeSet;

use pwtech_core::passw::generator::CHARSET_SYMBOLS;
use pwtech_core::unicode::{W32, str_to_w32, w32_to_string};

use super::*;

const CONFIG_ID: &str = "CharSetBuilder";
const NUM_PARAM: usize = 6;
const CHARSET_USED: [&str; NUM_PARAM] = [
    "LowerCaseCheck",
    "UpperCaseCheck",
    "DigitsCheck",
    "SpecialSymbolsCheck",
    "CharactersRangeCheck",
    "AdditionalCharsCheck",
];
const CODES: [&str; 4] = ["<az>", "<AZ>", "<09>", "<symbols>"];

/// Input of the builder (`CharSetParamChange()`).
#[derive(Debug, Default, Clone)]
pub struct BuilderParams {
    pub used: [bool; NUM_PARAM],
    pub num_chars: [i64; NUM_PARAM],
    pub at_least: [bool; NUM_PARAM],
    pub from: String,
    pub to: String,
    pub additional: String,
    pub exclude: String,
    pub tag: String,
}

/// Error of [`build_char_set`].
#[derive(Debug, PartialEq, Eq)]
pub enum BuildError {
    InvalidRange,
}

fn default_set(i: usize) -> BTreeSet<W32> {
    match i {
        0 => (b'a'..=b'z').map(W32::from).collect(),
        1 => (b'A'..=b'Z').map(W32::from).collect(),
        2 => (b'0'..=b'9').map(W32::from).collect(),
        _ => str_to_w32(CHARSET_SYMBOLS).into_iter().collect(),
    }
}

fn set_to_string(s: &BTreeSet<W32>) -> String {
    w32_to_string(&s.iter().copied().collect::<Vec<_>>())
}

/// `CharSetParamChange()`: character set definition for the parameters
/// (empty if it would contain fewer than 2 characters).
pub fn build_char_set(p: &BuilderParams) -> Result<String, BuildError> {
    let mut result = String::new();
    let mut size: u64 = 0;
    let exclude: BTreeSet<W32> = str_to_w32(&p.exclude).into_iter().collect();
    let has_exclude = !exclude.is_empty();
    for i in 0..NUM_PARAM {
        if !p.used[i] {
            continue;
        }
        match i {
            4 => {
                let from = str_to_w32(&p.from);
                let to = str_to_w32(&p.to);
                if from.len() != 1 || to.len() != 1 {
                    continue;
                }
                let (first, last) = (from[0], to[0]);
                if last <= first {
                    return Err(BuildError::InvalidRange);
                }
                if has_exclude && exclude.range(first..=last).next().is_some() {
                    continue;
                }
                result.push('<');
                result.push_str(&w32_to_string(&[first, last]));
                result.push('>');
                size += (last - first) as u64 + 1;
            }
            5 => {
                let mut set: BTreeSet<W32> = str_to_w32(&p.additional).into_iter().collect();
                if set.is_empty() {
                    continue;
                }
                if has_exclude {
                    for c in &exclude {
                        set.remove(c);
                    }
                    if set.is_empty() {
                        continue;
                    }
                }
                size += set.len() as u64;
                result.push_str("<<");
                result.push_str(&set_to_string(&set));
                result.push_str(">>");
            }
            _ => {
                let full = default_set(i);
                let mut use_default = true;
                if has_exclude {
                    let set: BTreeSet<W32> = full.difference(&exclude).copied().collect();
                    if set.is_empty() {
                        continue;
                    }
                    if set.len() < full.len() {
                        result.push_str("<<");
                        result.push_str(&set_to_string(&set));
                        result.push_str(">>");
                        size += set.len() as u64;
                        use_default = false;
                    }
                }
                if use_default {
                    result.push_str(CODES[i]);
                    size += full.len() as u64;
                }
            }
        }
        let n = p.num_chars[i];
        if n > 0 {
            result.push_str(&format!(":{n}"));
            if p.at_least[i] {
                result.push('+');
            }
        }
    }
    if size >= 2 {
        if !p.tag.is_empty() {
            result = format!("[{}]{result}", p.tag);
        }
        Ok(result)
    } else {
        Ok(String::new())
    }
}

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
        {
            let core = main.core.borrow();
            win.set_default_size(
                core.ini.read_int(CONFIG_ID, "WindowWidth", b.client.0).max(b.client.0),
                core.ini.read_int(CONFIG_ID, "WindowHeight", b.client.1).max(b.client.1),
            );
        }
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
        d.b.get::<gtk::Button>("ApplyBtn").set_sensitive(false);
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

    fn save_config(&self, ini: &mut pwtech_core::config::IniFile) {
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
        // `FormShow()`
        d.b.get::<gtk::Entry>("TagBox").set_text("");
        d.win.present();
    }

    pub(super) fn save_char_set_builder_config(&self, ini: &mut pwtech_core::config::IniFile) {
        if let Some(d) = self.char_set_builder.borrow().as_ref() {
            d.save_config(ini);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> BuilderParams {
        BuilderParams::default()
    }

    #[test]
    fn basic_sets() {
        let mut x = p();
        x.used[0] = true;
        x.used[2] = true;
        x.num_chars[2] = 2;
        x.at_least[2] = true;
        assert_eq!(build_char_set(&x).unwrap(), "<az><09>:2+");
        x.exclude = "aeiou0".into();
        assert_eq!(build_char_set(&x).unwrap(), "<<bcdfghjklmnpqrstvwxyz>><<123456789>>:2+");
        x.tag = "t".into();
        assert_eq!(build_char_set(&x).unwrap(), "[t]<<bcdfghjklmnpqrstvwxyz>><<123456789>>:2+");
    }

    #[test]
    fn range_and_additional() {
        let mut x = p();
        x.used[4] = true;
        x.from = "a".into();
        x.to = "a".into();
        assert_eq!(build_char_set(&x), Err(BuildError::InvalidRange));
        x.to = "f".into();
        assert_eq!(build_char_set(&x).unwrap(), "<af>");
        x.exclude = "c".into();
        assert_eq!(build_char_set(&x).unwrap(), "");
        x.used[5] = true;
        x.additional = "zyxc".into();
        assert_eq!(build_char_set(&x).unwrap(), "<<xyz>>");
        // a single character is not a set
        x.additional = "zc".into();
        assert_eq!(build_char_set(&x).unwrap(), "");
    }
}
