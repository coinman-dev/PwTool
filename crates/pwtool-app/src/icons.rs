//! Images embedded at build time: the icons of the original forms
//! (`assets/icons`, PNG) and the program icons (`assets/appicons`).

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// PNG of image `name` of image collection `collection` in form `form`
/// (e.g. `("Main", "ImageCollection32", "password")`).
pub fn form_icon(form: &str, collection: &str, name: &str) -> Option<&'static [u8]> {
    icon_bytes(&format!("{form}/{collection}/{name}"))
}

/// PNG images (16, 32, 48 pixels) of the program icon for
/// `Configuration::AppIconName` ("Blue", "Red", … or the default icon).
pub fn app_icon_pngs(name: &str) -> [&'static [u8]; 3] {
    macro_rules! icon {
        ($n:literal) => {
            [
                include_bytes!(concat!("../../../assets/appicons/", $n, "_16.png")),
                include_bytes!(concat!("../../../assets/appicons/", $n, "_32.png")),
                include_bytes!(concat!("../../../assets/appicons/", $n, "_48.png")),
            ]
        };
    }
    match name {
        "Blue" => icon!("pwtool_icon_blue"),
        "Red" => icon!("pwtool_icon_red"),
        "Orange" => icon!("pwtool_icon_orange"),
        "Green" => icon!("pwtool_icon_green"),
        "Colors" => icon!("pwtool_icon_colored"),
        _ => icon!("pwtool_default"),
    }
}

/// Decoded image: straight (not premultiplied) RGBA pixels, top-down.
#[derive(Debug, Clone)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Decode a PNG to RGBA (8 bits per channel).
pub fn decode_png(bytes: &[u8]) -> Option<Rgba> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width, info.height);
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some(Rgba { width: w, height: h, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_icons() {
        for name in ["", "Blue", "Colors"] {
            for (png, size) in app_icon_pngs(name).iter().zip([16, 32, 48]) {
                let img = decode_png(png).expect("app icon decodes");
                assert_eq!((img.width, img.height), (size, size));
                assert_eq!(img.pixels.len(), (size * size * 4) as usize);
            }
        }
        assert!(form_icon("Main", "ImageCollection32", "password").and_then(decode_png).is_some());
    }
}
