//! Terminal image rendering.
//!
//! Full-resolution image support is delegated to the `ratatui-image` crate,
//! which queries the terminal at startup for the kitty / sixel / iTerm2
//! graphics protocols (and the font size), falling back to unicode half-blocks
//! when nothing else is available.
//!
//! This module keeps the small pieces the widgets still need: a truecolor
//! half-block renderer for list-row thumbnails, and the conversion from the
//! launcher's own decoded raster to the `image` crate's `DynamicImage` that the
//! widget protocols consume.

use mc_core::img::RgbaImage;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// `true` when the terminal advertises 24-bit colour support via `COLORTERM`.
///
/// Only used for the half-block thumbnails in list rows; the widget protocols
/// negotiated by `ratatui-image` do not depend on this.
pub fn terminal_supports_truecolor() -> bool {
    if let Ok(value) = std::env::var("COLORTERM") {
        let lower = value.to_ascii_lowercase();
        lower.contains("truecolor") || lower.contains("24bit")
    } else {
        false
    }
}

/// Decode raw image bytes into the launcher's RGBA raster. Tries the native
/// PNG decoder first (no `image` crate overhead), then the `image` crate for
/// raster formats (jpeg/gif/webp/bmp/tiff/ico/…), then SVG rasterization via
/// resvg (shields.io-style badges). Returns `None` for anything unsupported.
pub fn decode_rgba(data: &[u8]) -> Option<RgbaImage> {
    if let Ok(img) = mc_core::img::decode_image(data) {
        return Some(img);
    }
    if let Ok(dynamic) = image::load_from_memory(data) {
        let rgba = dynamic.to_rgba8();
        return Some(RgbaImage {
            width: rgba.width(),
            height: rgba.height(),
            pixels: rgba.into_raw(),
        });
    }
    decode_svg(data)
}

/// Rasterize an SVG document to an RGBA raster at its intrinsic size.
///
/// SVG bytes are sniffed (XML prolog / root tag) before parsing so non-SVG
/// input fails fast. The pixmap produced by resvg is premultiplied; pixels
/// are un-premultiplied here so downstream blitters expect straight alpha.
/// Sizes beyond [`SVG_MAX_DIM`] are rejected to bound allocations.
pub fn decode_svg(data: &[u8]) -> Option<RgbaImage> {
    const SVG_MAX_DIM: usize = 4096;
    let text = std::str::from_utf8(data).ok()?;
    let head = text.get(..4096).unwrap_or(text);
    if !head.contains("<svg") && !head.contains("<?xml") {
        return None;
    }
    let mut opt = resvg::usvg::Options::default();
    let mut fontdb = resvg::usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    opt.fontdb = std::sync::Arc::new(fontdb);
    let tree = resvg::usvg::Tree::from_str(text, &opt).ok()?;
    let size = tree.size();
    let w = size.width().ceil();
    let h = size.height().ceil();
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let w = w as usize;
    let h = h as usize;
    if w > SVG_MAX_DIM || h > SVG_MAX_DIM || w * h > SVG_MAX_DIM * SVG_MAX_DIM {
        return None;
    }
    let w = w as u32;
    let h = h as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    let mut pixels = pixmap.data().to_vec();
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            px[0] = ((px[0] as u32 * 255) / a).min(255) as u8;
            px[1] = ((px[1] as u32 * 255) / a).min(255) as u8;
            px[2] = ((px[2] as u32 * 255) / a).min(255) as u8;
        }
    }
    Some(RgbaImage {
        width: w,
        height: h,
        pixels,
    })
}

/// Convert a decoded RGBA raster into the `image` crate's `DynamicImage` used
/// by the `ratatui-image` widget protocols. Returns `None` for malformed
/// rasters so the caller can fall back to a placeholder.
pub fn to_dynamic_image(img: &RgbaImage) -> Option<image::DynamicImage> {
    image::RgbaImage::from_raw(img.width, img.height, img.pixels.clone()).map(
        image::DynamicImage::from,
    )
}

/// Render an image as truecolor half-block lines fitting within a `max_w`×
/// `max_h` cell box. Transparent pixels fall back to `bg`.
pub fn image_lines(img: &RgbaImage, max_w: u32, max_h: u32, bg: Color) -> Vec<Line<'_>> {
    if max_w == 0 || max_h == 0 || img.width == 0 || img.height == 0 {
        return Vec::new();
    }
    // Every cell stacks two pixel rows (half-block), so fit to `2*max_h` px.
    // Borrow in place when the image already fits — this runs every frame.
    let owned;
    let fitted = if img.width <= max_w && img.height <= max_h * 2 {
        img
    } else {
        owned = mc_core::img::fit_image(img, max_w, max_h * 2);
        &owned
    };
    let w = fitted.width;
    let h = fitted.height;
    let rows = (h + 1) / 2;
    let mut out: Vec<Line> = Vec::new();
    for r in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        for x in 0..w {
            let top = pixel(fitted, r * 2, x, bg);
            let bottom = if r * 2 + 1 < h {
                pixel(fitted, r * 2 + 1, x, bg)
            } else {
                bg
            };
            spans.push(Span::styled("▀", Style::default().fg(top).bg(bottom)));
        }
        out.push(Line::from(spans));
    }
    out
}

/// RGBA colour of a pixel, resolving transparency to `bg`.
fn pixel(img: &RgbaImage, y: u32, x: u32, bg: Color) -> Color {
    let idx = (y as usize * img.width as usize + x as usize) * 4;
    if idx + 3 >= img.pixels.len() {
        return bg;
    }
    let alpha = img.pixels[idx + 3];
    if alpha < 128 {
        bg
    } else {
        Color::Rgb(img.pixels[idx], img.pixels[idx + 1], img.pixels[idx + 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_half_blocks() {
        let img = RgbaImage {
            width: 2,
            height: 4,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 0, 255, 255, 255, 255, 255, 255, //
                128, 128, 0, 255, 0, 128, 128, 255, //
                128, 0, 128, 255, 255, 128, 0, 255,
            ],
        };
        let lines = image_lines(&img, 2, 2, Color::Rgb(0, 0, 0));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans.len(), 2);
    }

    #[test]
    fn converts_raster_to_dynamic_image() {
        let img = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
            ],
        };
        let Some(dynamic) = to_dynamic_image(&img) else {
            panic!("expected a dynamic image");
        };
        assert_eq!(dynamic.width(), 2);
        assert_eq!(dynamic.height(), 1);

        // A raster with a too-short pixel buffer must yield `None`.
        let broken = RgbaImage {
            width: 100,
            height: 100,
            pixels: vec![0u8; 4],
        };
        assert!(to_dynamic_image(&broken).is_none());
    }

    #[test]
    fn detects_truecolor_env() {
        // Should not panic on missing env and should return a bool.
        let _ = terminal_supports_truecolor();
    }

    #[test]
    fn decodes_svg_rasters() {
        let svg = br##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4">
  <rect width="8" height="4" fill="#50FA7B"/>
</svg>"##;
        let img = decode_svg(svg).expect("svg should rasterize");
        assert_eq!(img.width, 8);
        assert_eq!(img.height, 4);
        assert_eq!(img.pixels.len(), 8 * 4 * 4);
        assert_eq!(decode_rgba(svg).map(|i| i.width), Some(8));
    }

    #[test]
    fn svg_sniff_rejects_plain_text() {
        assert!(decode_svg(b"hello world, not an image").is_none());
        assert!(decode_svg(b"tag soup <svg").is_none());
        assert!(decode_rgba(b"definitely not an image").is_none());
    }

    #[test]
    fn svg_size_cap_rejects_huge_documents() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000"><rect width="100000" height="100000" fill="red"/></svg>"#;
        assert!(decode_svg(svg).is_none(), "huge svg must be rejected");
    }
}

