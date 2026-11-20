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

struct Face {
    data: Vec<u8>,
    index: u32,
}

fn database() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
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
    let (src, index) = db.face_source(id)?;
    let data = match src {
        fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => std::fs::read(p).ok()?,
        fontdb::Source::Binary(b) => b.as_ref().as_ref().to_vec(),
    };
    Some(Face { data, index })
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
        let mut fd = FontData::from_owned(face.data);
        fd.index = face.index;
        defs.font_data.insert(key.clone(), Arc::new(fd));
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
