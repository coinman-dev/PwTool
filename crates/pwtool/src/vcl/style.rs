//! One style for all windows: the sizes of the standard controls of the
//! original forms (edit, combo box, button, spacing), scaled like the forms
//! (`form_scale`). The forms take their sizes from their descriptions; the
//! windows built in code (message boxes, the font dialog…) use these, so
//! that every window looks the same.

use egui::{Context, FontId, Response, RichText, TextStyle, Ui, Vec2, WidgetText, vec2};

/// Text height of the original forms (`TextHeight` at 120 PPI).
const DESIGN_TEXT_HEIGHT: f32 = 17.0;

/// Sizes of the standard controls, in points.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Points per design pixel of the original forms.
    pub scale: f32,
    /// Height of edits and combo boxes (25 design pixels).
    pub control_h: f32,
    /// Size of a dialog button such as OK/Cancel (93 × 31).
    pub button: Vec2,
    /// Space between controls (8).
    pub gap: f32,
    /// Distance of the controls from the window edge (12).
    pub margin: f32,
}

/// Body font size of `ctx`.
pub fn body_size(ctx: &Context) -> f32 {
    ctx.global_style().text_styles.get(&TextStyle::Body).map_or(super::fonts::DEFAULT_SIZE, |f| f.size)
}

pub fn metrics(ctx: &Context) -> Metrics {
    let body = body_size(ctx);
    let row = ctx.fonts_mut(|f| f.row_height(&FontId::proportional(body)));
    let scale = (row / DESIGN_TEXT_HEIGHT).clamp(0.6, 2.0);
    Metrics {
        scale,
        control_h: (25.0 * scale).round(),
        button: vec2((93.0 * scale).round(), (31.0 * scale).round()),
        gap: (8.0 * scale).round(),
        margin: (12.0 * scale).round(),
    }
}

/// Set the spacing of `ui` for a window laid out with egui widgets:
/// edits, combo boxes, buttons and list rows get the heights of the forms.
pub fn apply(ui: &mut Ui) -> Metrics {
    let m = metrics(ui.ctx());
    let row = ui.text_style_height(&TextStyle::Body);
    let sp = ui.spacing_mut();
    sp.interact_size.y = m.control_h;
    sp.button_padding = vec2(8.0 * m.scale, ((m.control_h - row) / 2.0).max(2.0));
    sp.item_spacing = vec2(m.gap, (6.0 * m.scale).round());
    sp.combo_height = 320.0;
    m
}

/// A dialog button (OK, Cancel…) of the standard size.
pub fn button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let m = metrics(ui.ctx());
    ui.add(egui::Button::new(text).min_size(m.button))
}

/// [`button`], enabled or not.
pub fn button_enabled(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    let m = metrics(ui.ctx());
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text)).min_size(m.button))
}

/// Rows of a list as tall as in the forms' list boxes (text plus a little).
pub fn list_row_height(ui: &Ui) -> f32 {
    let m = metrics(ui.ctx());
    (ui.text_style_height(&TextStyle::Body) + 6.0 * m.scale).round()
}
