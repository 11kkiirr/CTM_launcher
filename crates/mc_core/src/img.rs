//! Minimal in-memory image decoding and scaling for terminal previews.
//!
//! Only PNG is supported for now (Modrinth serves its icons/galleries as PNG
//! or WebP; PNG covers the vast majority of icons and is dependency-free by
//! using `flate2` for the zlib stream). WebP is rejected with a clear error
//! and the UI falls back to a text-only preview.

use crate::error::{CoreError, Result};

/// An RGBA8 raster, row-major, `width * height * 4` bytes per pixel.
#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Sanity cap: refuse to decode rasters larger than this in either dimension.
const MAX_DIM: u32 = 4096;

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Decode a PNG file into an RGBA raster. Supports non-interlaced images with
/// color types 0 (gray), 2 (RGB), 3 (palette), 4 (gray+alpha) and 6 (RGBA) at
/// any bit depth the format allows (sub-byte depths are expanded).
pub fn decode_png(data: &[u8]) -> Result<RgbaImage> {
    if data.len() < 8 || data[..8] != PNG_SIG[..] {
        return Err(CoreError::other("not a PNG file"));
    }

    let mut at = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut palette: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();

    while at + 8 <= data.len() {
        let len = be32(data, at) as usize;
        if at + 8 + len + 4 > data.len() {
            return Err(CoreError::other("truncated PNG chunk"));
        }
        let kind: &[u8] = &data[at + 4..at + 8];
        let body: &[u8] = &data[at + 8..at + 8 + len];
        if kind == &b"IHDR"[..] {
            if len < 13 {
                return Err(CoreError::other("bad PNG header"));
            }
            width = be32(body, 0);
            height = be32(body, 4);
            bit_depth = body[8];
            color_type = body[9];
            if body[10] != 0 || body[11] != 0 {
                return Err(CoreError::other("unsupported PNG compression/filter"));
            }
            interlace = body[12];
        } else if kind == &b"PLTE"[..] {
            palette = body.to_vec();
        } else if kind == &b"tRNS"[..] {
            trns = body.to_vec();
        } else if kind == &b"IDAT"[..] {
            idat.extend(body);
        } else if kind == &b"IEND"[..] {
            break;
        }
        at += 8 + len + 4;
    }

    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return Err(CoreError::other(format!("unsupported PNG dimensions {width}x{height}")));
    }
    if !valid_bit_depth(color_type, bit_depth) {
        return Err(CoreError::other(format!(
            "unsupported PNG color type {color_type} / bit depth {bit_depth}"
        )));
    }
    if interlace != 0 {
        return Err(CoreError::other("interlaced PNG is not supported"));
    }

    let raw = inflate(&idat)?;
    let channels: usize = match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let bits_per_pixel = channels * (bit_depth as usize);
    let bpp = if bit_depth < 8 {
        1
    } else {
        channels * (bit_depth / 8) as usize
    };
    let stride = (width as usize * bits_per_pixel + 7) / 8;
    if raw.len() < (stride + 1) * height as usize {
        return Err(CoreError::other("PNG pixel data is truncated"));
    }

    let total = width as usize * height as usize * 4;
    let mut pixels = Vec::with_capacity(total);
    let mut prev = vec![0u8; stride];
    for y in 0..height as usize {
        let row_start = y * (stride + 1);
        let filter = raw[row_start];
        if filter > 4 {
            return Err(CoreError::other("invalid PNG row filter"));
        }
        let mut cur = vec![0u8; stride];
        for i in 0..stride {
            let x = raw[row_start + 1 + i];
            let a = if i >= bpp { cur[i - bpp] } else { 0u8 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0u8 };
            // PNG unfiltering uses wrapping (mod 256) arithmetic, so every
            // predictor runs in u16 and folds back with `as u8`.
            let val = match filter {
                0 => x,
                1 => (x as u16 + a as u16) as u8,
                2 => (x as u16 + b as u16) as u8,
                3 => (x as u16 + ((a as u16 + b as u16) / 2)) as u8,
                _ => (x as u16 + paeth(a, b, c) as u16) as u8,
            };
            cur[i] = val;
        }
        decode_row(&cur, width, bit_depth, color_type, &palette, &trns, &mut pixels);
        prev = cur;
    }

    Ok(RgbaImage { width, height, pixels })
}

/// Decode a raster from raw bytes, sniffing the format. Only PNG is supported;
/// anything else yields an error so callers can fall back gracefully.
pub fn decode_image(data: &[u8]) -> Result<RgbaImage> {
    if data.len() >= 8 && data[..8] == PNG_SIG[..] {
        decode_png(data)
    } else {
        Err(CoreError::other("unsupported image format (only PNG is supported)"))
    }
}

/// Downscale `img` to at most `max_w`×`max_h` pixels (nearest neighbour),
/// preserving aspect ratio. Returns the image unchanged when already small.
pub fn fit_image(img: &RgbaImage, max_w: u32, max_h: u32) -> RgbaImage {
    if img.width <= max_w && img.height <= max_h {
        return img.clone();
    }
    let scale = (img.width as f64 / max_w as f64)
        .max(img.height as f64 / max_h as f64);
    let w = (img.width as f64 / scale).floor() as u32;
    let h = (img.height as f64 / scale).floor() as u32;
    resize_nearest(img, w.max(1), h.max(1))
}

/// Resize a raster to exactly `w`×`h` pixels using nearest-neighbour sampling.
pub fn resize_nearest(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if w == 0 || h == 0 {
        return RgbaImage { width: 0, height: 0, pixels: Vec::new() };
    }
    if img.width == w && img.height == h {
        return img.clone();
    }
    // Guard against malformed rasters so a bad decode can never panic the UI.
    if img.pixels.len() < img.width as usize * img.height as usize * 4 {
        return img.clone();
    }
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for ty in 0..h {
        let sy = (ty * img.height / h) as usize;
        for tx in 0..w {
            let sx = (tx * img.width / w) as usize;
            let src = (sy as u32 * img.width + sx as u32) as usize * 4;
            pixels.extend(&img.pixels[src..src + 4]);
        }
    }
    RgbaImage { width: w, height: h, pixels }
}

// ---------------------------------------------------------------------------
// PNG internals
// ---------------------------------------------------------------------------

fn be32(data: &[u8], at: usize) -> u32 {
    (data[at] as u32) << 24
        | (data[at + 1] as u32) << 16
        | (data[at + 2] as u32) << 8
        | data[at + 3] as u32
}

fn inflate(idat: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(&idat[..]);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = decoder.read(&mut buf[..])?;
        if n == 0 {
            break;
        }
        out.extend(&buf[..n]);
    }
    Ok(out)
}

fn valid_bit_depth(color_type: u8, bit_depth: u8) -> bool {
    match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        4 => matches!(bit_depth, 8 | 16),
        6 => matches!(bit_depth, 8 | 16),
        _ => false,
    }
}

/// The PNG Paeth predictor.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i32 + b as i32 - c as i32;
    let mut pa = p - a as i32;
    let mut pb = p - b as i32;
    let mut pc = p - c as i32;
    pa = if pa < 0 { -pa } else { pa };
    pb = if pb < 0 { -pb } else { pb };
    pc = if pc < 0 { -pc } else { pc };
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Value of the `idx`-th sample in a scanline row (handles sub-byte depths).
fn sample_at(row: &[u8], idx: usize, bit_depth: u8) -> u8 {
    if bit_depth >= 8 {
        row[idx * (bit_depth / 8) as usize]
    } else {
        let bit = idx * (bit_depth as usize);
        let shift = 8u8 - bit_depth - ((bit % 8) as u8);
        (row[bit / 8] >> shift) & ((1u8 << bit_depth) - 1)
    }
}

fn decode_row(
    row: &[u8],
    width: u32,
    bit_depth: u8,
    color_type: u8,
    palette: &[u8],
    trns: &[u8],
    out: &mut Vec<u8>,
) {
    for x in 0..width as usize {
        match color_type {
            0 => {
                let mut v = sample_at(row, x, bit_depth);
                if bit_depth < 8 {
                    v = ((v as u16 * 255) / ((1u8 << bit_depth) as u16 - 1)) as u8;
                }
                out.extend(&[v, v, v, 0xFF]);
            }
            2 => {
                let r = sample_at(row, x * 3, bit_depth);
                let g = sample_at(row, x * 3 + 1, bit_depth);
                let b = sample_at(row, x * 3 + 2, bit_depth);
                out.extend(&[r, g, b, 0xFF]);
            }
            3 => {
                let idx = sample_at(row, x, bit_depth) as usize;
                let r = if idx * 3 + 2 < palette.len() {
                    palette[idx * 3]
                } else {
                    0
                };
                let g = if idx * 3 + 2 < palette.len() {
                    palette[idx * 3 + 1]
                } else {
                    0
                };
                let b = if idx * 3 + 2 < palette.len() {
                    palette[idx * 3 + 2]
                } else {
                    0
                };
                let a = if idx < trns.len() { trns[idx] } else { 0xFF };
                out.extend(&[r, g, b, a]);
            }
            4 => {
                let g = sample_at(row, x * 2, bit_depth);
                let a = sample_at(row, x * 2 + 1, bit_depth);
                out.extend(&[g, g, g, a]);
            }
            _ => {
                let r = sample_at(row, x * 4, bit_depth);
                let g = sample_at(row, x * 4 + 1, bit_depth);
                let b = sample_at(row, x * 4 + 2, bit_depth);
                let a = sample_at(row, x * 4 + 3, bit_depth);
                out.extend(&[r, g, b, a]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny 2×2 RGBA PNG built by hand (color type 6, no filter).
    fn png_fixture() -> Vec<u8> {
        let mut data: Vec<u8> = PNG_SIG.to_vec();
        // IHDR: 2x2, 8-bit RGBA.
        let ihdr: Vec<u8> = vec![
            0, 0, 0, 13, b'I', b'H', b'D', b'R',
            0, 0, 0, 2, // width
            0, 0, 0, 2, // height
            8, 6, 0, 0, 0, // bit depth, color type, compression, filter, interlace
        ];
        data.extend(&ihdr);
        // CRC fields are ignored by the decoder; pad with zeroes.
        data.extend(&[0, 0, 0, 0]);

        // Two scanlines, each: filter byte 0 + 4 pixels (RGBA).
        let scanlines: Vec<u8> = vec![
            0, 255, 0, 0, 255, 0, 255, 0, 255, // red, green
            0, 0, 0, 255, 255, 255, 255, 255, 255, // blue, white
        ];
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&scanlines).unwrap();
        let z = encoder.finish().unwrap();

        let mut idat: Vec<u8> = vec![0, 0, 0, z.len() as u8, b'I', b'D', b'A', b'T'];
        idat.extend(&z);
        idat.extend(&[0, 0, 0, 0]);
        data.extend(&idat);

        data.extend(&[0, 0, 0, 0, b'I', b'E', b'N', b'D']);
        data.extend(&[0, 0, 0, 0]);
        data
    }

    #[test]
    fn decodes_basic_png() {
        let img = decode_png(&png_fixture()).unwrap();
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.pixels.len(), 16);
        // Row 0: red, green.
        assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.pixels[4..8], &[0, 255, 0, 255]);
        // Row 1: blue, white.
        assert_eq!(&img.pixels[8..12], &[0, 0, 255, 255]);
        assert_eq!(&img.pixels[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn rejects_non_png() {
        assert!(decode_image(b"hello world".to_vec().as_ref()).is_err());
    }

    #[test]
    fn truncated_png_is_an_error_not_a_panic() {
        // Cut the fixture in the middle of a chunk body: the decoder must
        // report an error instead of slicing out of bounds.
        let data = png_fixture();
        let cut = data.len() / 2;
        assert!(decode_png(&data[..cut]).is_err());
    }

    #[test]
    fn decodes_palette_png() {
        // 2x2, 8-bit palette (color type 3), PLTE with two entries.
        let mut data: Vec<u8> = PNG_SIG.to_vec();
        let ihdr: Vec<u8> = vec![
            0, 0, 0, 13, b'I', b'H', b'D', b'R',
            0, 0, 0, 2, 0, 0, 0, 2, 8, 3, 0, 0, 0,
        ];
        data.extend(&ihdr);
        data.extend(&[0, 0, 0, 0]);
        let plte: Vec<u8> = vec![
            0, 0, 0, 6, b'P', b'L', b'T', b'E',
            255, 0, 0, 0, 255, 0,
        ];
        data.extend(&plte);
        data.extend(&[0, 0, 0, 0]);
        let scanlines: Vec<u8> = vec![0, 0, 1, 0, 0, 1];
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&scanlines).unwrap();
        let z = encoder.finish().unwrap();
        let mut idat: Vec<u8> = vec![0, 0, 0, z.len() as u8, b'I', b'D', b'A', b'T'];
        idat.extend(&z);
        idat.extend(&[0, 0, 0, 0]);
        data.extend(&idat);
        data.extend(&[0, 0, 0, 0, b'I', b'E', b'N', b'D']);
        data.extend(&[0, 0, 0, 0]);

        let img = decode_png(&data).unwrap();
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        // Row 0: indices 0,1 -> red, green. Row 1: indices 0,1 -> red, green.
        assert_eq!(&img.pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.pixels[4..8], &[0, 255, 0, 255]);
        assert_eq!(&img.pixels[8..12], &[255, 0, 0, 255]);
        assert_eq!(&img.pixels[12..16], &[0, 255, 0, 255]);
    }

    #[test]
    fn interlaced_png_is_rejected() {
        // Flip the interlace byte in the IHDR of the fixture.
        let data = png_fixture();
        let mut interlaced = data.clone();
        // IHDR starts at offset 8; interlace is the 13th byte of IHDR payload.
        let interlace_byte = 8 + 8 + 12;
        interlaced[interlace_byte] = 1;
        assert!(decode_png(&interlaced).is_err());
    }

    #[test]
    fn sub_filter_wraps_mod_256() {
        // A 2x1 RGBA image with filter type 1 (Sub). The second pixel's bytes
        // are stored as `pixel - previous_pixel (mod 256)`, and one of them
        // would overflow a plain u8 add: 200 + 100 = 300 -> 44 (mod 256).
        let mut data: Vec<u8> = PNG_SIG.to_vec();
        let ihdr: Vec<u8> = vec![
            0, 0, 0, 13, b'I', b'H', b'D', b'R',
            0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0, 0, 0,
        ];
        data.extend(&ihdr);
        data.extend(&[0, 0, 0, 0]);
        // Row: filter byte + first pixel (200,200,200,255) + second pixel
        // stored as deltas. Byte 4: 100 + 200 = 300 -> 44 (mod 256);
        // byte 5: 100 + 200 = 44; byte 6: 50 + 200 = 250; byte 7: 50 + 255 = 49.
        let scanlines: Vec<u8> = vec![1, 200, 200, 200, 255, 100, 100, 50, 50];
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&scanlines).unwrap();
        let z = encoder.finish().unwrap();
        let mut idat: Vec<u8> = vec![0, 0, 0, z.len() as u8, b'I', b'D', b'A', b'T'];
        idat.extend(&z);
        idat.extend(&[0, 0, 0, 0]);
        data.extend(&idat);
        data.extend(&[0, 0, 0, 0, b'I', b'E', b'N', b'D']);
        data.extend(&[0, 0, 0, 0]);

        let img = decode_png(&data).unwrap();
        assert_eq!(&img.pixels[0..4], &[200, 200, 200, 255]);
        // 200 + 100 = 300 wraps to 44; 50 + 200 = 250; 50 + 255 = 49.
        assert_eq!(&img.pixels[4..8], &[44, 44, 250, 49]);
    }

    #[test]
    fn fit_preserves_aspect() {
        let img = RgbaImage {
            width: 400,
            height: 200,
            pixels: vec![0u8; 400 * 200 * 4],
        };
        let fitted = fit_image(&img, 40, 40);
        assert_eq!(fitted.width, 40);
        assert_eq!(fitted.height, 20);
    }
}