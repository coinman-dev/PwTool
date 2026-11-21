//! Pictures of egui output without a window or GPU: the tessellated
//! meshes are rasterized in software. Used to compare the forms with the
//! original, in tests and on machines without a display.

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{Color32, ColorImage, Context, ImageData, Pos2, RawInput, Rect, TextureId, Ui, Vec2, ViewportId};

/// Renders frames of a context into images.
pub struct Snapshot {
    pub ctx: Context,
    textures: HashMap<TextureId, ColorImage>,
}

impl Snapshot {
    pub fn new(ctx: Context) -> Self {
        Snapshot { ctx, textures: HashMap::new() }
    }

    /// Run `frames` frames of a `size` (points) screen and rasterize the
    /// last one at `ppp` pixels per point.
    pub fn render(&mut self, size: Vec2, ppp: f32, frames: usize, mut ui_fn: impl FnMut(&mut Ui)) -> ColorImage {
        let mut prims = Vec::new();
        for frame in 0..frames.max(1) {
            let mut raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            raw.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
            let out = self.ctx.run_ui(raw, |ui| ui_fn(ui));
            let mut out = out;
            let delta_set = std::mem::take(&mut out.textures_delta.set);
            let delta_free = std::mem::take(&mut out.textures_delta.free);
            for (id, deltas) in delta_set {
                for delta in deltas {
                    let ImageData::Color(img) = delta.image;
                    match delta.pos {
                        None => {
                            self.textures.insert(id, (*img).clone());
                        }
                        Some([x, y]) => {
                            if let Some(t) = self.textures.get_mut(&id) {
                                for row in 0..img.size[1] {
                                    for col in 0..img.size[0] {
                                        let (tx, ty) = (x + col, y + row);
                                        if tx < t.size[0] && ty < t.size[1] {
                                            t.pixels[ty * t.size[0] + tx] = img.pixels[row * img.size[0] + col];
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if frame + 1 == frames.max(1) {
                prims = self.ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
            }
            for id in delta_free {
                self.textures.remove(&id);
            }
        }
        let w = (size.x * ppp).ceil() as usize;
        let h = (size.y * ppp).ceil() as usize;
        let mut img = ColorImage::filled([w, h], Color32::BLACK);
        rasterize(&mut img, &prims, ppp, &self.textures);
        img
    }
}

fn sample(t: &ColorImage, uv: Pos2) -> [f32; 4] {
    // bilinear, premultiplied
    let x = uv.x * t.size[0] as f32 - 0.5;
    let y = uv.y * t.size[1] as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let get = |xi: f32, yi: f32| {
        let xi = (xi as i64).clamp(0, t.size[0] as i64 - 1) as usize;
        let yi = (yi as i64).clamp(0, t.size[1] as i64 - 1) as usize;
        let c = t.pixels[yi * t.size[0] + xi];
        [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32]
    };
    let (a, b, c, d) = (get(x0, y0), get(x0 + 1.0, y0), get(x0, y0 + 1.0), get(x0 + 1.0, y0 + 1.0));
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bot = c[i] + (d[i] - c[i]) * fx;
        out[i] = top + (bot - top) * fy;
    }
    out
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Rasterize tessellated primitives (premultiplied colours, gamma-space
/// blending like egui's renderers).
pub fn rasterize(img: &mut ColorImage, prims: &[ClippedPrimitive], ppp: f32, textures: &HashMap<TextureId, ColorImage>) {
    let (w, h) = (img.size[0] as i64, img.size[1] as i64);
    for prim in prims {
        let Primitive::Mesh(mesh) = &prim.primitive else { continue };
        let Some(tex) = textures.get(&mesh.texture_id) else { continue };
        let clip = prim.clip_rect;
        let cx0 = ((clip.min.x * ppp).floor() as i64).max(0);
        let cy0 = ((clip.min.y * ppp).floor() as i64).max(0);
        let cx1 = ((clip.max.x * ppp).ceil() as i64).min(w);
        let cy1 = ((clip.max.y * ppp).ceil() as i64).min(h);
        for tri in mesh.indices.chunks_exact(3) {
            let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
            let p: Vec<Pos2> = v.iter().map(|v| Pos2::new(v.pos.x * ppp, v.pos.y * ppp)).collect();
            let area = edge(p[0], p[1], p[2]);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = (p.iter().map(|q| q.x).fold(f32::MAX, f32::min).floor() as i64).max(cx0);
            let x1 = (p.iter().map(|q| q.x).fold(f32::MIN, f32::max).ceil() as i64).min(cx1);
            let y0 = (p.iter().map(|q| q.y).fold(f32::MAX, f32::min).floor() as i64).max(cy0);
            let y1 = (p.iter().map(|q| q.y).fold(f32::MIN, f32::max).ceil() as i64).min(cy1);
            for y in y0..y1 {
                for x in x0..x1 {
                    let q = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = edge(p[1], p[2], q) / area;
                    let w1 = edge(p[2], p[0], q) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let uv = Pos2::new(
                        v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2,
                        v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2,
                    );
                    let t = sample(tex, uv);
                    let col = |i: usize| {
                        let c = |vv: &egui::epaint::Vertex| vv.color.to_array()[i] as f32;
                        c(v[0]) * w0 + c(v[1]) * w1 + c(v[2]) * w2
                    };
                    let src = [col(0) * t[0] / 255.0, col(1) * t[1] / 255.0, col(2) * t[2] / 255.0, col(3) * t[3] / 255.0];
                    let idx = (y * w + x) as usize;
                    let d = img.pixels[idx].to_array();
                    let ia = 1.0 - src[3] / 255.0;
                    let out = |i: usize| (src[i] + d[i] as f32 * ia).round().clamp(0.0, 255.0) as u8;
                    img.pixels[idx] = Color32::from_rgba_premultiplied(out(0), out(1), out(2), out(3).max(d[3]));
                }
            }
        }
    }
}

/// Save an image as PNG.
pub fn save_png(img: &ColorImage, path: &std::path::Path) -> std::io::Result<()> {
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    image::save_buffer(path, &bytes, img.size[0] as u32, img.size[1] as u32, image::ColorType::Rgba8).map_err(std::io::Error::other)
}
