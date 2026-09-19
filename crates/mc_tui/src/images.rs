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
    let fitted = mc_core::img::fit_image(img, max_w, max_h * 2);
    let w = fitted.width;
    let h = fitted.height;
    let rows = (h + 1) / 2;
    let mut out: Vec<Line> = Vec::new();
    for r in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        for x in 0..w {
            let top = pixel(&fitted, r * 2, x, bg);
            let bottom = if r * 2 + 1 < h {
                pixel(&fitted, r * 2 + 1, x, bg)
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
}