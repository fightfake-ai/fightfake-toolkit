//! Still-image ingest: PNG/JPEG/WebP → planar YUV 4:2:0 for Eva macroblocks.
//!
//! Decode uses the `image` crate (no ffmpeg). Dimensions are cropped to the
//! largest top-left multiple of 16 so Eva's 16×16 macroblock grid fits.

use std::path::Path;

use anyhow::{bail, Context, Result};
use image::DynamicImage;

/// Extensions we treat as a single-frame still (case-insensitive).
const STILL_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];

pub fn is_still_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| STILL_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        .unwrap_or(false)
}

/// Decode a still image to planar YUV 4:2:0.
///
/// Returns `(yuv_bytes, width, height)` after cropping to multiples of 16.
/// `num_frames` is always 1; callers should use fps `1/1`.
pub fn decode_still_to_yuv420(path: &Path) -> Result<(Vec<u8>, usize, usize)> {
    let img = image::open(path)
        .with_context(|| format!("failed to decode still image {}", path.display()))?;
    let rgb = img.to_rgb8();
    let (orig_w, orig_h) = rgb.dimensions();
    let orig_w = orig_w as usize;
    let orig_h = orig_h as usize;

    if orig_w < 16 || orig_h < 16 {
        bail!(
            "image {} is {orig_w}×{orig_h}; need at least 16×16 for Eva macroblocks",
            path.display()
        );
    }

    let width = (orig_w / 16) * 16;
    let height = (orig_h / 16) * 16;
    if width != orig_w || height != orig_h {
        eprintln!(
            "[image] cropping {orig_w}×{orig_h} → {width}×{height} (top-left, multiples of 16)"
        );
    }

    let cropped = if width == orig_w && height == orig_h {
        DynamicImage::ImageRgb8(rgb)
    } else {
        DynamicImage::ImageRgb8(rgb).crop_imm(0, 0, width as u32, height as u32)
    };
    let rgb = cropped.to_rgb8();

    let yuv = rgb8_to_yuv420p(rgb.as_raw(), width, height);
    Ok((yuv, width, height))
}

/// BT.601 full-range RGB→YUV 4:2:0 (same formulas ffmpeg uses for `yuv420p` from RGB).
fn rgb8_to_yuv420p(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    debug_assert_eq!(rgb.len(), width * height * 3);
    debug_assert_eq!(width % 2, 0);
    debug_assert_eq!(height % 2, 0);

    let y_size = width * height;
    let uv_size = (width / 2) * (height / 2);
    let mut out = vec![0u8; y_size + 2 * uv_size];
    let (y_plane, rest) = out.split_at_mut(y_size);
    let (u_plane, v_plane) = rest.split_at_mut(uv_size);

    for row in 0..height {
        for col in 0..width {
            let i = (row * width + col) * 3;
            let r = rgb[i] as i32;
            let g = rgb[i + 1] as i32;
            let b = rgb[i + 2] as i32;
            // Full-range BT.601 (JPEG-style), matching common ffmpeg rgb→yuv420p.
            let y = (77 * r + 150 * g + 29 * b + 128) >> 8;
            y_plane[row * width + col] = y.clamp(0, 255) as u8;
        }
    }

    for row in 0..(height / 2) {
        for col in 0..(width / 2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let px = ((row * 2 + dy) * width + (col * 2 + dx)) * 3;
                    r_sum += rgb[px] as i32;
                    g_sum += rgb[px + 1] as i32;
                    b_sum += rgb[px + 2] as i32;
                }
            }
            let r = r_sum / 4;
            let g = g_sum / 4;
            let b = b_sum / 4;
            let u = ((-43 * r - 85 * g + 128 * b + 128) >> 8) + 128;
            let v = ((128 * r - 107 * g - 21 * b + 128) >> 8) + 128;
            let idx = row * (width / 2) + col;
            u_plane[idx] = u.clamp(0, 255) as u8;
            v_plane[idx] = v.clamp(0, 255) as u8;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use std::io::Cursor;

    #[test]
    fn detects_still_extensions() {
        assert!(is_still_image(Path::new("a.PNG")));
        assert!(is_still_image(Path::new("b.jpeg")));
        assert!(!is_still_image(Path::new("c.mp4")));
    }

    #[test]
    fn rgb_to_yuv_size_and_crop() {
        let mut img = RgbImage::new(20, 18);
        for p in img.pixels_mut() {
            *p = Rgb([200, 100, 50]);
        }
        let mut bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        // Write temp via decode path using image::load_from_memory indirectly:
        let dynimg = image::load_from_memory(&bytes).unwrap();
        let rgb = dynimg.to_rgb8();
        let yuv = rgb8_to_yuv420p(rgb.as_raw(), 20, 18);
        // 20×18 is not multiple of 16 for chroma height... height 18 % 2 == 0 OK
        assert_eq!(yuv.len(), 20 * 18 + 2 * (10 * 9));

        // Cropped 16×16
        let cropped = dynimg.crop_imm(0, 0, 16, 16).to_rgb8();
        let yuv16 = rgb8_to_yuv420p(cropped.as_raw(), 16, 16);
        assert_eq!(yuv16.len(), 16 * 16 + 2 * (8 * 8));
    }
}
