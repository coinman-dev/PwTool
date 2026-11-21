//! VCL forms drawn with egui. The window logic works with controls by name
//! (`Form::text`, `Form::set_checked`, `Form::on_click`…) like the original
//! works with its VCL components; the forms come from the original form
//! descriptions (`assets/forms`), so the windows keep their layout.

pub mod dialogs;
pub mod fonts;
pub mod layout;
pub mod list_view;
pub mod model;
pub mod render;
pub mod snapshot;
pub mod text;

pub use model::{Event, Form, Icon};
pub use render::window_size;

use egui::{FontId, TextStyle};
use pwtool_app::form::FormObj;

/// Points per design pixel of a form: the form's text height
/// (`TextHeight` at its `PixelsPerInch`) is mapped to the height of a line
/// of the body font, like `TForm.Scaled` adapts forms to the system font.
pub fn form_scale(ctx: &egui::Context, form: &FormObj) -> f32 {
    let body = ctx.global_style().text_styles.get(&TextStyle::Body).map_or(fonts::DEFAULT_SIZE, |f| f.size);
    let row = ctx.fonts_mut(|f| f.row_height(&FontId::proportional(body)));
    let text_h = form.int_or("TextHeight", 17) as f32;
    (row / text_h).clamp(0.6, 2.0)
}

/// Load an embedded form scaled for `ctx`.
pub fn load_form(ctx: &egui::Context, name: &str) -> Form {
    Form::load(name, |o| form_scale(ctx, o))
}

/// Names of all embedded forms.
pub const FORMS: [&str; 21] = [
    "About",
    "CharSetBuilder",
    "Configuration",
    "CreateRandDataFile",
    "CreateTrigramFile",
    "InfoBox",
    "MPPasswGen",
    "Main",
    "PasswEnter",
    "PasswList",
    "PasswManager",
    "PasswMngColSelect",
    "PasswMngDbProp",
    "PasswMngDbSettings",
    "PasswMngKeyValEdit",
    "PasswMngPwHistory",
    "PasswOptions",
    "ProfileEditor",
    "Progress",
    "ProvideEntropy",
    "QuickHelp",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every form builds, lays out and draws; visible controls lie inside
    /// the window.
    #[test]
    fn all_forms_draw() {
        let ctx = egui::Context::default();
        fonts::install(&ctx, &fonts::UserFont::default());
        let mut snap = snapshot::Snapshot::new(ctx.clone());
        snap.render(egui::vec2(50.0, 50.0), 1.0, 1, |_| {});
        for name in FORMS {
            let form = load_form(&ctx, name);
            let size = window_size(&form, &ctx);
            assert!(size.x > 50.0 && size.y > 20.0, "{name}: size {size:?}");
            let img = snap.render(size, 1.0, 2, |ui| form.show(ui));
            assert_eq!(img.size[0], size.x.ceil() as usize);
            let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size).expand(2.0);
            for c in form.control_names() {
                let r = form.rect(&c);
                if r.is_positive() && form.visible(&c) {
                    // auto-sizing labels may be wider than the form, like in the VCL
                    let ok = if form.class(&c) == "TLabel" { window.contains(r.min) } else { window.contains_rect(r) };
                    assert!(ok, "{name}.{c}: {r:?} outside {size:?}");
                }
            }
        }
    }

    /// Fields are named by their labels for screen readers.
    #[test]
    fn field_labels() {
        let ctx = egui::Context::default();
        fonts::install(&ctx, &fonts::UserFont::default());
        snapshot::Snapshot::new(ctx.clone()).render(egui::vec2(50.0, 50.0), 1.0, 1, |_| {});
        let label_of = |form: &str, field: &str| {
            let f = load_form(&ctx, form);
            let d = f.data.borrow();
            d.by_name.get(field).and_then(|&i| d.controls[i].label).map(|l| d.controls[l].name.clone())
        };
        assert_eq!(label_of("Main", "CharsLengthBox").as_deref(), Some("CharsLengthLbl"));
        assert_eq!(label_of("Main", "SpecifyLengthBox").as_deref(), Some("SpecifyLengthCheck"));
        assert_eq!(label_of("PasswManager", "TitleBox").as_deref(), Some("TitleLbl"));
        assert_eq!(label_of("PasswEnter", "PasswBox").as_deref(), Some("PasswLbl"));
    }

    #[test]
    fn form_api() {
        let ctx = egui::Context::default();
        fonts::install(&ctx, &fonts::UserFont::default());
        let mut snap = snapshot::Snapshot::new(ctx.clone());
        snap.render(egui::vec2(50.0, 50.0), 1.0, 1, |_| {});
        let f = load_form(&ctx, "Main");
        f.set_text("PasswBox", "abc");
        assert_eq!(f.text("PasswBox"), "abc");
        f.set_checked("IncludeCharsCheck", true);
        assert!(f.checked("IncludeCharsCheck"));
        f.set_position("CharsLengthSpinBtn", 20);
        assert_eq!(f.text("CharsLengthBox"), "20");
        assert_eq!(f.position("CharsLengthSpinBtn"), 20);
        f.set_menu_checked("MainMenu_Options_AlwaysOnTop", true);
        assert!(f.menu_checked("MainMenu_Options_AlwaysOnTop"));
        f.push_event(model::Event::Click("GenerateBtn".into()));
        let hit = std::rc::Rc::new(std::cell::Cell::new(false));
        let h = hit.clone();
        f.on_click("GenerateBtn", move |_| h.set(true));
        assert!(f.dispatch().is_empty());
        assert!(hit.get());
    }
}
