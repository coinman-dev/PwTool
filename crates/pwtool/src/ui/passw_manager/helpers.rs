//! Helpers of the password manager: fuzzy search, expiry dates, fonts,
//! a date picker (`TDateTimePicker`), file names.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use pwtool_core::passw::database::encode_expiry_date;

/// `levenshteinDist()` with the original costs (insert 2, delete 1,
/// replace 1), on UTF-16 code units like the original.
pub fn levenshtein_dist(src: &[u16], target: &[u16]) -> u32 {
    const INSERT_COST: u32 = 2;
    const DELETE_COST: u32 = 1;
    const REPLACE_COST: u32 = 1;
    if src.len() > target.len() {
        return levenshtein_dist(target, src);
    }
    let min_size = src.len();
    let mut lev = vec![0u32; min_size + 1];
    for i in 1..=min_size {
        lev[i] = lev[i - 1] + DELETE_COST;
    }
    for &t in target {
        let mut prev = lev[0];
        lev[0] += INSERT_COST;
        for i in 1..=min_size {
            let prev_save = lev[i];
            if src[i - 1] == t {
                lev[i] = prev;
            } else {
                lev[i] = (lev[i] + INSERT_COST).min(lev[i - 1] + DELETE_COST).min(prev + REPLACE_COST);
            }
            prev = prev_save;
        }
    }
    *lev.last().unwrap()
}

fn find_sub(hay: &[u16], needle: &[u16]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

/// `strFindFuzzy()`: score of `pattern` in `src` (2 = identical, 1 =
/// contained, otherwise the best Levenshtein score in 0..1).
pub fn str_find_fuzzy(src: &[u16], pattern: &[u16]) -> f32 {
    let src_len = src.len();
    let pat_len = pattern.len();
    if pat_len == 0 {
        return 0.0;
    }
    let lev_score = |p: &[u16]| {
        let d = levenshtein_dist(p, pattern) as usize;
        if d < pat_len { (pat_len - d) as f32 / pat_len as f32 } else { 0.0 }
    };
    let mut best = 0.0f32;
    if src_len <= pat_len {
        if src == pattern {
            best = 2.0;
        } else {
            best = lev_score(src);
        }
    } else {
        let mut i = 0;
        let mut pos = 0;
        while let Some(off) = src[pos..].iter().position(|&c| c == pattern[0]) {
            let next = pos + off;
            if i == 0 && find_sub(&src[next..], pattern) {
                best = 1.0;
                break;
            }
            let len = pat_len.min(src_len - next);
            best = best.max(lev_score(&src[next..next + len]));
            i += 1;
            pos = next + 1;
            if pos >= src_len {
                break;
            }
        }
    }
    best
}

/// Lower-case a string (`CharLower` / `AnsiLowerCase`).
pub fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `_wcsicmp`: case-insensitive comparison of UTF-16 code units (ASCII
/// letters folded, as in the C locale).
pub fn wcsicmp(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: u16| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c };
    a.encode_utf16().map(fold).cmp(b.encode_utf16().map(fold))
}

/// `wcscmp`.
pub fn wcscmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Today's local date as `(year, month, day)`.
pub fn today() -> (i32, u32, u32) {
    let d = glib::DateTime::now_local().expect("local time");
    (d.year(), d.month() as u32, d.day_of_month() as u32)
}

/// Local date `days` days from today.
pub fn today_plus_days(days: i32) -> (i32, u32, u32) {
    let d = glib::DateTime::now_local().expect("local time").add_days(days).expect("date");
    (d.year(), d.month() as u32, d.day_of_month() as u32)
}

/// `getExpiryCheckDates()`: encoded current date and the "expires soon"
/// limit (today + `warn_days`, clamped to 1..100).
pub fn expiry_check_dates(warn_days: i32) -> (u32, u32) {
    let (y, m, d) = today();
    let (y2, m2, d2) = today_plus_days(warn_days.clamp(1, 100));
    (encode_expiry_date(y, m, d), encode_expiry_date(y2, m2, d2))
}

/// `getTimeStampString()`: `YYYYMMDDhhmmss[_mmm]` in local time.
pub fn time_stamp_string(millisec: bool) -> String {
    let d = glib::DateTime::now_local().expect("local time");
    let mut s = format!(
        "{}{:02}{:02}{:02}{:02}{:02}",
        d.year(),
        d.month(),
        d.day_of_month(),
        d.hour(),
        d.minute(),
        d.second()
    );
    if millisec {
        s.push_str(&format!("_{:03}", d.microsecond() / 1000));
    }
    s
}

/// `ShortenFileName()`.
pub fn shorten_file_name(name: &str, max_len: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    let len = chars.len();
    if len <= max_len.max(10) {
        return name.to_string();
    }
    let start_chars = 10.min((max_len.saturating_sub(4)) / 2);
    let end_chars = max_len - start_chars - 3;
    let mut result: Vec<char> = chars[..start_chars].to_vec();
    result.extend("...".chars());
    result.extend_from_slice(&chars[len - end_chars..]);
    // remove the partial directory name after "..." (1-based indices in the
    // original: search starts behind "...")
    let rlen = result.len();
    let start = start_chars + 4; // 1-based position
    let mut del_pos = 0;
    let mut i = start;
    while i + 10 <= rlen {
        if result[i - 1] == '/' {
            del_pos = i;
            break;
        }
        i += 1;
    }
    if del_pos > start {
        result.drain(start - 1..del_pos - 1);
    }
    result.into_iter().collect()
}

/// `FontToString()`: `name;size;style;color` (style bits: bold 1,
/// italic 2, underline 4, strike-out 8).
pub fn font_to_string(desc: &pango::FontDescription) -> String {
    let name = desc.family().map(|f| f.to_string()).unwrap_or_default();
    let size = (desc.size() as f64 / pango::SCALE as f64).round() as i32;
    let mut style = 0;
    if desc.weight() >= pango::Weight::Bold {
        style |= 1;
    }
    if desc.style() != pango::Style::Normal {
        style |= 2;
    }
    format!("{name};{size};{style};clWindowText")
}

/// `StringToFont()`: `None` if the string is empty or invalid.
pub fn string_to_font(s: &str) -> Option<pango::FontDescription> {
    if s.is_empty() {
        return None;
    }
    let mut it = s.split(';');
    let name = it.next()?.trim();
    let size: i32 = it.next()?.trim().parse().ok()?;
    if name.is_empty() || size == 0 {
        return None;
    }
    let style: i32 = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let mut d = pango::FontDescription::new();
    d.set_family(name);
    d.set_size(size.abs() * pango::SCALE);
    if style & 1 != 0 {
        d.set_weight(pango::Weight::Bold);
    }
    if style & 2 != 0 {
        d.set_style(pango::Style::Italic);
    }
    Some(d)
}

/// CSS declarations for a font description.
pub fn font_css(desc: &pango::FontDescription) -> String {
    let mut css = String::new();
    if let Some(f) = desc.family() {
        css.push_str(&format!("font-family: \"{}\";", f.replace('"', "")));
    }
    if desc.size() > 0 {
        css.push_str(&format!("font-size: {}pt;", desc.size() / pango::SCALE));
    }
    if desc.weight() >= pango::Weight::Bold {
        css.push_str("font-weight: bold;");
    }
    if desc.style() != pango::Style::Normal {
        css.push_str("font-style: italic;");
    }
    css
}

/// A widget with a replaceable CSS provider (fonts/colours that change at
/// run time).
pub struct DynCss {
    provider: gtk::CssProvider,
    class: String,
}

thread_local! {
    static DYN_CSS_COUNTER: Cell<u32> = const { Cell::new(0) };
}

impl DynCss {
    pub fn new(w: &impl IsA<gtk::Widget>) -> Self {
        let n = DYN_CSS_COUNTER.with(|c| {
            c.set(c.get() + 1);
            c.get()
        });
        let class = format!("pwdyn{n}");
        let provider = gtk::CssProvider::new();
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        }
        w.add_css_class(&class);
        DynCss { provider, class }
    }

    /// Replace the declarations (applied to the widget and its descendants).
    pub fn set(&self, decls: &str) {
        let c = &self.class;
        self.provider.load_from_data(&format!(".{c}, .{c} * {{ {decls} }}"));
    }
}

/// `TDateTimePicker` (date only): a menu button showing the date in the
/// locale's short format with a calendar popover.
pub struct DatePicker {
    pub button: gtk::MenuButton,
    pub calendar: gtk::Calendar,
    date: Cell<(i32, u32, u32)>,
    on_change: RefCell<Option<Box<dyn Fn()>>>,
    updating: Cell<bool>,
}

impl DatePicker {
    pub fn new(button: gtk::MenuButton, calendar: gtk::Calendar) -> Rc<Self> {
        let p = Rc::new(DatePicker {
            button,
            calendar,
            date: Cell::new(today()),
            on_change: RefCell::new(None),
            updating: Cell::new(false),
        });
        let pp = Rc::downgrade(&p);
        p.calendar.connect_day_selected(move |cal| {
            let Some(p) = pp.upgrade() else { return };
            if p.updating.get() {
                return;
            }
            let d = cal.date();
            p.date.set((d.year(), d.month() as u32, d.day_of_month() as u32));
            p.update_label();
            if let Some(pop) = p.button.popover() {
                pop.popdown();
            }
            if let Some(f) = p.on_change.borrow().as_ref() {
                f();
            }
        });
        p.update_label();
        p
    }

    fn update_label(&self) {
        let (y, m, d) = self.date.get();
        self.button.set_label(&pwtool_core::util::format_short_date(y, m, d));
    }

    pub fn date(&self) -> (i32, u32, u32) {
        self.date.get()
    }

    /// Set the date without calling the change handler.
    pub fn set_date(&self, (y, m, d): (i32, u32, u32)) {
        self.date.set((y, m, d));
        if let Ok(dt) = glib::DateTime::from_local(y, m as i32, d as i32, 0, 0, 0.0) {
            self.updating.set(true);
            self.calendar.select_day(&dt);
            self.updating.set(false);
        }
        self.update_label();
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(f));
    }

    pub fn encoded(&self) -> u32 {
        let (y, m, d) = self.date.get();
        encode_expiry_date(y, m, d)
    }
}

/// Convert `\r\n` (file) to `\n` (text widgets).
pub fn crlf_to_lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Convert `\n` (text widgets) to `\r\n` (file, as the original stores the
/// text of memo controls).
pub fn lf_to_crlf(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut prev_cr = false;
    for c in s.chars() {
        if c == '\n' && !prev_cr {
            out.push('\r');
        }
        out.push(c);
        prev_cr = c == '\r';
    }
    out
}

/// Text of a text view.
pub fn text_view_text(tv: &gtk::TextView) -> zeroize::Zeroizing<String> {
    let buf = tv.buffer();
    zeroize::Zeroizing::new(buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string())
}

/// Local `FILETIME` (wall clock as UTC) of a file system time, for
/// `time_stamp_to_string()`.
pub fn system_time_to_local_filetime(t: std::time::SystemTime) -> u64 {
    use pwtool_core::random::hrtimer::{FILETIME_UNIX_EPOCH, utc_offset_seconds};
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let local = secs + utc_offset_seconds(secs as _);
    FILETIME_UNIX_EPOCH + local.max(0) as u64 * 10_000_000 + (d.subsec_nanos() / 100) as u64
}

/// Integer with thousands separators (`Format("%.0n")`).
pub fn format_thousands(n: u64) -> String {
    let sep = if pwtool_core::util::decimal_separator() == ',' { '.' } else { ',' };
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// Close a dialog window with Escape (`Cancel = true` buttons of VCL).
pub fn close_on_escape(win: &gtk::Window) {
    let key = gtk::EventControllerKey::new();
    let w = win.clone();
    key.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn fuzzy() {
        assert_eq!(str_find_fuzzy(&u("hello"), &u("hello")), 2.0);
        assert_eq!(str_find_fuzzy(&u("say hello world"), &u("hello")), 1.0);
        assert!(str_find_fuzzy(&u("say hallo world"), &u("hello")) >= 0.5);
        assert_eq!(str_find_fuzzy(&u("xyz"), &u("hello")), 0.0);
        assert_eq!(levenshtein_dist(&u("abc"), &u("abc")), 0);
        assert_eq!(levenshtein_dist(&u("ab"), &u("abc")), 2);
    }

    #[test]
    fn shorten() {
        assert_eq!(shorten_file_name("/short/name.pwdb", 60), "/short/name.pwdb");
        let long = "/home/user/some/very/long/directory/structure/that/goes/on/and/on/file.pwdb";
        let s = shorten_file_name(long, 60);
        assert!(s.chars().count() <= 60, "{s}");
        assert!(s.starts_with("/home/user"));
        assert!(s.ends_with("file.pwdb"));
    }

    #[test]
    fn crlf() {
        assert_eq!(lf_to_crlf("a\nb\r\nc"), "a\r\nb\r\nc");
        assert_eq!(crlf_to_lf("a\r\nb"), "a\nb");
    }

    #[test]
    fn fonts() {
        let d = string_to_font("Consolas;10;1;clWindowText").unwrap();
        assert_eq!(font_to_string(&d), "Consolas;10;1;clWindowText");
        assert!(string_to_font("").is_none());
    }
}
