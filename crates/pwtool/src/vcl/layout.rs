//! Positions of the controls like the VCL computes them: design
//! coordinates, `Align` (top/bottom/left/right/client) and `Anchors`
//! (left/top/right/bottom) relative to the design size of the parent.

use egui::{Rect, pos2, vec2};

use super::model::{AK_BOTTOM, AK_LEFT, AK_RIGHT, AK_TOP, Align, Control, FormData, State};

/// Height of the tab row of a page control (`TextHeight + 10`, scaled).
pub fn tab_height(text_height: f32, scale: f32) -> f32 {
    text_height + 10.0 * scale
}

/// Client rectangle of a container control for its children.
pub fn client_rect(c: &Control, text_height: f32, scale: f32) -> Rect {
    match c.class.as_str() {
        "TPageControl" => {
            let th = tab_height(text_height, scale);
            Rect::from_min_max(pos2(c.rect.min.x + 4.0 * scale, c.rect.min.y + th + 2.0 * scale), pos2(c.rect.max.x - 4.0 * scale, c.rect.max.y - 4.0 * scale))
        }
        _ => c.rect,
    }
}

/// Lay out the children `kids` of a container with client rectangle `area`
/// and design client size `design`. `measure` gives the natural width of an
/// auto-sizing label.
fn place(controls: &mut [Control], kids: &[usize], area: Rect, design: egui::Vec2, measure: &mut dyn FnMut(&Control) -> f32) {
    let dx = area.width() - design.x;
    let dy = area.height() - design.y;
    // client rectangle left after the aligned controls
    let mut client = area;
    for kind in [Align::Top, Align::Bottom, Align::Left, Align::Right, Align::Client] {
        // like the VCL, controls of one alignment in the order of their
        // positions (a splitter right of the list it resizes)
        let mut same: Vec<usize> = kids.iter().copied().filter(|&k| controls[k].align == kind && controls[k].visible).collect();
        let key = |k: usize| {
            let d = controls[k].design;
            match kind {
                Align::Top => d.min.y,
                Align::Bottom => -d.max.y,
                Align::Left => d.min.x,
                Align::Right => -d.max.x,
                _ => 0.0,
            }
        };
        same.sort_by(|&a, &b| key(a).total_cmp(&key(b)));
        for k in same {
            let c = &mut controls[k];
            let (w, h) = (c.design.width(), c.design.height());
            c.rect = match kind {
                Align::Top => {
                    let r = Rect::from_min_size(client.min, vec2(client.width(), h));
                    client.min.y += h;
                    r
                }
                Align::Bottom => {
                    let r = Rect::from_min_size(pos2(client.min.x, client.max.y - h), vec2(client.width(), h));
                    client.max.y -= h;
                    r
                }
                Align::Left => {
                    let r = Rect::from_min_size(client.min, vec2(w, client.height()));
                    client.min.x += w;
                    r
                }
                Align::Right => {
                    let r = Rect::from_min_size(pos2(client.max.x - w, client.min.y), vec2(w, client.height()));
                    client.max.x -= w;
                    r
                }
                Align::Client | Align::None => client,
            };
        }
    }
    for &k in kids {
        if controls[k].align != Align::None || !controls[k].visible {
            continue;
        }
        let natural = controls[k].autosize.map(|_| measure(&controls[k]));
        let c = &mut controls[k];
        let d = c.design;
        let (mut x, mut w) = (d.min.x, d.width());
        match (c.anchors & AK_LEFT != 0, c.anchors & AK_RIGHT != 0) {
            (true, true) => w += dx,
            (false, true) => x += dx,
            (false, false) => x += dx / 2.0,
            _ => {}
        }
        let (mut y, mut h) = (d.min.y, d.height());
        match (c.anchors & AK_TOP != 0, c.anchors & AK_BOTTOM != 0) {
            (true, true) => h += dy,
            (false, true) => y += dy,
            (false, false) => y += dy / 2.0,
            _ => {}
        }
        if let (Some(xa), Some(nat)) = (c.autosize, natural) {
            if nat > w || xa != 0.0 {
                x += (w - nat) * xa;
                w = nat;
            } else {
                w = w.max(nat);
            }
        }
        c.rect = Rect::from_min_size(area.min + vec2(x, y), vec2(w.max(0.0), h.max(0.0)));
    }
}

/// Compute `rect` of every control for a form whose client area is `area`.
pub(super) fn layout(d: &mut FormData, area: Rect, measure: &mut dyn FnMut(&Control) -> f32) {
    let roots = d.roots.clone();
    let design = d.client;
    place(&mut d.controls, &roots, area, design, measure);
    // containers, parents before children
    let mut stack: Vec<usize> = roots;
    while let Some(i) = stack.pop() {
        let kids = d.controls[i].children.clone();
        if kids.is_empty() || !d.controls[i].visible {
            continue;
        }
        match &d.controls[i].state {
            State::Pages { .. } => {
                let client = client_rect(&d.controls[i], d.text_height, d.scale);
                for &k in &kids {
                    d.controls[k].rect = client;
                    let design = d.controls[k].design_client;
                    let sub = d.controls[k].children.clone();
                    place(&mut d.controls, &sub, client, design, measure);
                    stack.extend(sub);
                }
            }
            State::ToolBar => {
                // tool bar children flow from the left; their positions are
                // set when drawing
            }
            _ => {
                let area = d.controls[i].rect;
                let design = d.controls[i].design_client;
                place(&mut d.controls, &kids, area, design, measure);
                stack.extend(kids);
            }
        }
    }
}
