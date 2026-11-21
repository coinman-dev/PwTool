//! Fonts from the operating system (nothing is embedded): the user
//! interface font with bold and italic faces, a monospaced font, and
//! fallbacks with Chinese, Japanese and Korean characters. egui's own
//! fonts come last.

use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions, FontFamily, FontId, TextStyle};

/// Family name of the bold face.
pub const BOLD: &str = "bold";
/// Family name of the italic face.
pub const ITALIC: &str = "italic";

#[cfg(windows)]
const UI_FONTS: &[&str] = &["Segoe UI", "Tahoma", "Arial"];
#[cfg(not(windows))]
const UI_FONTS: &[&str] = &["Noto Sans", "Cantarell", "Ubuntu", "DejaVu Sans", "Liberation Sans", "FreeSans"];

#[cfg(windows)]
const MONO_FONTS: &[&str] = &["Consolas", "Cascadia Mono", "Courier New"];
#[cfg(not(windows))]
const MONO_FONTS: &[&str] = &["Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono", "Ubuntu Mono", "FreeMono"];

/// Fallbacks for scripts the UI font lacks (CJK first, then wide-coverage fonts).
const FALLBACK_FONTS: &[&str] = &[
    "Microsoft YaHei",
    "Microsoft JhengHei",
    "Yu Gothic UI",
    "Meiryo",
    "Malgun Gothic",
    "Noto Sans CJK SC",
    "Noto Sans CJK TC",
    "Noto Sans CJK JP",
    "Noto Sans CJK KR",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
    "Droid Sans Fallback",
    "Segoe UI Symbol",
    "DejaVu Sans",
];

/// Body text size in points (Segoe UI 9 pt on Windows, 10 pt fonts on Linux).
#[cfg(windows)]
pub const DEFAULT_SIZE: f32 = 12.0;
#[cfg(not(windows))]
pub const DEFAULT_SIZE: f32 = 13.0;

type Face = Arc<FontData>;

fn database() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

thread_local! {
    /// Loaded faces by id (the fonts are installed again when the user
    /// changes a font).
    static CACHE: std::cell::RefCell<std::collections::HashMap<fontdb::ID, Face>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

fn face_data(id: fontdb::ID) -> Option<Face> {
    if let Some(f) = CACHE.with(|c| c.borrow().get(&id).cloned()) {
        return Some(f);
    }
    let (src, index) = database().face_source(id)?;
    let data = match src {
        fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => std::fs::read(p).ok()?,
        fontdb::Source::Binary(b) => b.as_ref().as_ref().to_vec(),
    };
    let mut fd = FontData::from_owned(data);
    fd.index = index;
    let f = Arc::new(fd);
    CACHE.with(|c| c.borrow_mut().insert(id, f.clone()));
    Some(f)
}

fn load(family: &str, weight: fontdb::Weight, style: fontdb::Style) -> Option<Face> {
    let db = database();
    let q = fontdb::Query { families: &[fontdb::Family::Name(family)], weight, style, ..Default::default() };
    let id = db.query(&q)?;
    let info = db.face(id)?;
    // the query falls back to other weights: accept only what was asked for
    if (weight == fontdb::Weight::BOLD) != (info.weight.0 >= 600) || (style != fontdb::Style::Normal) != (info.style != fontdb::Style::Normal) {
        return None;
    }
    face_data(id)
}

/// A face of `family` close to the wanted style (any face if not found).
fn load_near(family: &str, bold: bool, italic: bool) -> Option<Face> {
    let w = if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL };
    let s = if italic { fontdb::Style::Italic } else { fontdb::Style::Normal };
    if let Some(f) = load(family, w, s) {
        return Some(f);
    }
    let db = database();
    let id = db.query(&fontdb::Query { families: &[fontdb::Family::Name(family)], weight: w, style: s, ..Default::default() })?;
    if !db.face(id)?.families.iter().any(|(n, _)| n == family) {
        return None;
    }
    face_data(id)
}

/// Font chosen by the user (`GUIFontString`), if any: family and size in points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserFont {
    pub family: Option<String>,
    pub size: Option<f32>,
}

/// Installed font set: the effective body size.
#[derive(Debug, Clone, Copy)]
pub struct FontSet {
    pub body_size: f32,
}

/// Install the fonts into `ctx`.
pub fn install(ctx: &egui::Context, user: &UserFont) -> FontSet {
    let mut defs = FontDefinitions::default();
    let egui_prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let egui_mono = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let add = |defs: &mut FontDefinitions, key: String, face: Face| {
        defs.font_data.insert(key.clone(), face);
        key
    };
    let normal = fontdb::Style::Normal;
    let candidates: Vec<&str> = user.family.as_deref().into_iter().chain(UI_FONTS.iter().copied()).collect();
    // the first family with a bold face (bold labels are common in the
    // forms), else the first family found
    let has = |fam: &str, w| {
        let db = database();
        db.query(&fontdb::Query { families: &[fontdb::Family::Name(fam)], weight: w, style: normal, ..Default::default() })
            .and_then(|id| db.face(id))
            .is_some_and(|f| f.families.iter().any(|(n, _)| n == fam) && (w != fontdb::Weight::BOLD || f.weight.0 >= 600))
    };
    let family = candidates
        .iter()
        .find(|f| has(f, fontdb::Weight::NORMAL) && has(f, fontdb::Weight::BOLD))
        .or_else(|| candidates.iter().find(|f| has(f, fontdb::Weight::NORMAL)));
    let mut regular = None;
    let mut bold = None;
    let mut italic = None;
    if let Some(fam) = family {
        if let Some(f) = load(fam, fontdb::Weight::NORMAL, normal) {
            regular = Some(add(&mut defs, format!("ui:{fam}"), f));
            bold = load(fam, fontdb::Weight::BOLD, normal).map(|f| add(&mut defs, format!("ui-bold:{fam}"), f));
            italic = load(fam, fontdb::Weight::NORMAL, fontdb::Style::Italic).map(|f| add(&mut defs, format!("ui-italic:{fam}"), f));
        }
    }
    let mono = MONO_FONTS.iter().find_map(|fam| load(fam, fontdb::Weight::NORMAL, normal).map(|f| add(&mut defs, format!("mono:{fam}"), f)));
    let fallbacks: Vec<String> = FALLBACK_FONTS
        .iter()
        .filter(|f| regular.as_deref() != Some(&format!("ui:{f}")))
        .filter_map(|fam| load(fam, fontdb::Weight::NORMAL, normal).map(|f| add(&mut defs, format!("fallback:{fam}"), f)))
        .collect();

    if std::env::var_os("PWTOOL_FONT_DEBUG").is_some() {
        eprintln!("fonts: regular={regular:?} bold={bold:?} italic={italic:?} mono={mono:?} fallbacks={fallbacks:?}");
    }
    let chain = |first: Option<&String>, egui: &[String]| -> Vec<String> {
        first.into_iter().cloned().chain(fallbacks.iter().cloned()).chain(egui.iter().cloned()).collect()
    };
    defs.families.insert(FontFamily::Proportional, chain(regular.as_ref(), &egui_prop));
    defs.families.insert(FontFamily::Monospace, chain(mono.as_ref().or(regular.as_ref()), &egui_mono));
    defs.families.insert(FontFamily::Name(BOLD.into()), chain(bold.as_ref().or(regular.as_ref()), &egui_prop));
    defs.families.insert(FontFamily::Name(ITALIC.into()), chain(italic.as_ref().or(regular.as_ref()), &egui_prop));
    // fonts chosen by the user for the password boxes and the font dialog
    let (named, preview) = EXTRA.with(|e| {
        let mut e = e.borrow_mut();
        e.user = user.clone();
        (e.named.clone(), e.preview.clone())
    });
    for fam in NAMED.iter().copied() {
        let spec = named.iter().find(|(n, _)| *n == fam).map(|(_, s)| s);
        let key = spec.and_then(|p| load_near(&p.name, p.style & 1 != 0, p.style & 2 != 0).map(|f| add(&mut defs, format!("{fam}:{}", p.name), f)));
        // the password boxes fall back to the monospaced font
        let fallback = if fam.contains("passw") { mono.as_ref().or(regular.as_ref()) } else { regular.as_ref() };
        defs.families.insert(FontFamily::Name(fam.into()), chain(key.as_ref().or(fallback), if fam.contains("passw") { &egui_mono } else { &egui_prop }));
    }
    let preview = preview.and_then(|(p, b, i)| load_near(&p, b, i).map(|f| add(&mut defs, format!("preview:{p}:{b}:{i}"), f)));
    defs.families.insert(FontFamily::Name(PREVIEW.into()), chain(preview.as_ref().or(regular.as_ref()), &egui_prop));
    ctx.set_fonts(defs);

    let body = user.size.filter(|s| (6.0..=40.0).contains(s)).unwrap_or(DEFAULT_SIZE);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(body * 0.85)),
            (TextStyle::Body, FontId::proportional(body)),
            (TextStyle::Button, FontId::proportional(body)),
            (TextStyle::Heading, FontId::proportional(body * 1.4)),
            (TextStyle::Monospace, FontId::monospace(body)),
        ]
        .into();
    });
    FontSet { body_size: body }
}

// ---------------------------------------------------------------------------
// Fonts chosen by the user

/// Family of the password boxes (`PasswBoxFont`).
pub const PASSW: &str = "passw";
/// Families of PassCube: entry list and password box.
pub const PM_LIST: &str = "pm-list";
pub const PM_PASSW: &str = "pm-passw";
/// Families the user can choose a font for.
pub const NAMED: [&str; 3] = [PASSW, PM_LIST, PM_PASSW];
/// Family of the sample in the font dialog.
pub const PREVIEW: &str = "preview";

/// Font of a `FontToString()` string ("Name;Size;Style;Color"; size in
/// points; style bits: 1 bold, 2 italic, 4 underline, 8 strikeout).
#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub name: String,
    pub size: i32,
    pub style: i32,
    pub color: String,
}

impl FontSpec {
    /// `StringToFont()`; `None` if name or size are missing.
    pub fn parse(s: &str) -> Option<FontSpec> {
        let mut it = s.split(';');
        let name = it.next()?.trim().to_string();
        let size: i32 = it.next()?.trim().parse().ok().filter(|&v| v != 0)?;
        let style = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let color = it.next().unwrap_or("clWindowText").trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some(FontSpec { name, size, style, color })
    }

    /// `FontToString()`.
    pub fn to_font_string(&self) -> String {
        format!("{};{};{};{}", self.name, self.size, self.style, self.color)
    }

    /// Size in egui points (1 pt = 4/3 px at 96 dpi).
    pub fn points(&self) -> f32 {
        self.size.unsigned_abs() as f32 * 4.0 / 3.0
    }

    /// The user interface font of this specification.
    pub fn user_font(&self) -> UserFont {
        UserFont { family: Some(self.name.clone()), size: Some(self.points()) }
    }
}

struct Extra {
    user: UserFont,
    named: Vec<(&'static str, FontSpec)>,
    preview: Option<(String, bool, bool)>,
}

thread_local! {
    static EXTRA: std::cell::RefCell<Extra> = const { std::cell::RefCell::new(Extra { user: UserFont { family: None, size: None }, named: Vec::new(), preview: None }) };
}

/// Font chosen by the user for family `name` (one of [`NAMED`]).
pub fn named_font(name: &str) -> Option<FontSpec> {
    EXTRA.with(|e| e.borrow().named.iter().find(|(n, _)| *n == name).map(|(_, s)| s.clone()))
}

/// Size of a named family in points (`None` = size of the control).
pub fn named_size(name: &str) -> Option<f32> {
    named_font(name).map(|f| f.points())
}

/// Set (or reset) the font of a named family and install the fonts again.
pub fn set_named_font(ctx: &egui::Context, name: &'static str, f: Option<FontSpec>) {
    EXTRA.with(|e| {
        let mut e = e.borrow_mut();
        e.named.retain(|(n, _)| *n != name);
        if let Some(f) = f {
            e.named.push((name, f));
        }
    });
    reinstall(ctx);
}

/// Size of the password box font in points (`None` = size of the control).
pub fn passw_size() -> Option<f32> {
    named_size(PASSW)
}

/// The password box font (`None` = monospaced default).
pub fn passw_font() -> Option<FontSpec> {
    named_font(PASSW)
}

fn reinstall(ctx: &egui::Context) {
    let user = EXTRA.with(|e| e.borrow().user.clone());
    install(ctx, &user);
}

/// Set the password box font and install the fonts again.
pub fn set_passw_font(ctx: &egui::Context, f: Option<FontSpec>) {
    set_named_font(ctx, PASSW, f);
}

/// Family and style (bold, italic) of the [`PREVIEW`] font (`None` = user
/// interface font).
pub fn set_preview_font(ctx: &egui::Context, font: Option<(&str, bool, bool)>) {
    let changed = EXTRA.with(|e| {
        let mut e = e.borrow_mut();
        let new = font.map(|(f, b, i)| (f.to_string(), b, i));
        let ch = e.preview != new;
        e.preview = new;
        ch
    });
    if changed {
        reinstall(ctx);
    }
}

/// Names of all font families of the system, sorted, and whether a
/// family is monospaced.
pub fn families() -> Vec<(String, bool)> {
    let mut v: Vec<(String, bool)> = database()
        .faces()
        .filter_map(|f| f.families.first().map(|(n, _)| (n.clone(), f.monospaced)))
        .collect();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then(a.0.cmp(&b.0)));
    v.dedup_by(|a, b| a.0 == b.0);
    v
}

/// The user interface font installed last.
pub fn user_font() -> UserFont {
    EXTRA.with(|e| e.borrow().user.clone())
}
