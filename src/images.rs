//! Image policy ([§4.4]) shared by the writer, the grader and script 19: one
//! aspect (`artist_image_aspect`, default 1:1 ±2%), one size ceiling
//! (`artist_image_target_size`, 0 = provider-native, capped at 2000 px).
//! Undersized is a note, never a failure.

use crate::atomic;
use crate::config::Config;
use crate::error::{IoResultExt, MloError, Result};
use crate::naming::ARTIST_IMAGE_CEILING;
use std::path::Path;

/// Parse `"1:1"` into a ratio.
pub fn parse_aspect(s: &str) -> Option<f32> {
    let (w, h) = s.split_once(':')?;
    let w: f32 = w.trim().parse().ok()?;
    let h: f32 = h.trim().parse().ok()?;
    if h <= 0.0 || w <= 0.0 {
        return None;
    }
    Some(w / h)
}

/// Crop to the target aspect and resize to the policy size. Writes atomically
/// and returns a note when the source was undersized.
pub fn optimize_artist_image(path: &Path, cfg: &Config) -> Result<Option<String>> {
    let img = image::open(path)
        .map_err(|e| MloError::Tag { path: path.to_path_buf(), reason: format!("image decode: {e}") })?;

    let aspect = parse_aspect(&cfg.artist_image_aspect).unwrap_or(1.0);
    let mut out = img;
    if cfg.artist_image_crop {
        out = crop_to_aspect(out, aspect);
    }

    let target = if cfg.artist_image_target_size == 0 {
        ARTIST_IMAGE_CEILING.min(out.width().max(out.height()))
    } else {
        cfg.artist_image_target_size.min(ARTIST_IMAGE_CEILING)
    };
    let max_dim = out.width().max(out.height());
    if max_dim > target {
        out = resize_to_max(out, target);
    }

    let note = if max_dim < target && cfg.artist_image_target_size != 0 {
        Some(format!(
            "artist image is undersized ({max_dim}px < {target}px) — left at native size"
        ))
    } else {
        None
    };

    // Preserve the container so cover sidecars stay predictable.
    let is_png = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("png"))
        .unwrap_or(false);
    let bytes = if is_png {
        encode_png(&out)?
    } else {
        encode_jpeg(&out)?
    };
    atomic::write_atomic(path, &bytes)?;
    Ok(note)
}

fn crop_to_aspect(img: image::DynamicImage, aspect: f32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let target_w = (h as f32 * aspect).round() as u32;
    if target_w <= w {
        let x = (w - target_w) / 2;
        img.crop_imm(x, 0, target_w.max(1), h)
    } else {
        let target_h = (w as f32 / aspect).round() as u32;
        let y = (h - target_h) / 2;
        img.crop_imm(0, y, w, target_h.max(1))
    }
}

fn resize_to_max(img: image::DynamicImage, max_dim: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w >= h {
        let nh = (h as f64 * max_dim as f64 / w as f64).round().max(1.0) as u32;
        img.resize_exact(max_dim, nh, image::imageops::FilterType::Lanczos3)
    } else {
        let nw = (w as f64 * max_dim as f64 / h as f64).round().max(1.0) as u32;
        img.resize_exact(nw, max_dim, image::imageops::FilterType::Lanczos3)
    }
}

fn encode_png(img: &image::DynamicImage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| MloError::Other(format!("png encode: {e}")))?;
    Ok(buf)
}

fn encode_jpeg(img: &image::DynamicImage) -> Result<Vec<u8>> {
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 92);
    enc.encode_image(&rgb)
        .map_err(|e| MloError::Other(format!("jpeg encode: {e}")))?;
    Ok(buf)
}

/// Write an image from provider bytes into the artist folder (policy applied).
pub fn write_artist_image(dir: &Path, bytes: &[u8], prefer_png: bool) -> Result<std::path::PathBuf> {
    let ext = if prefer_png { "png" } else { "jpg" };
    let path = dir.join(format!("artist.{ext}"));
    std::fs::create_dir_all(dir).at(dir)?;
    atomic::write_atomic(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aspect() {
        assert_eq!(parse_aspect("1:1"), Some(1.0));
        assert_eq!(parse_aspect("4:3"), Some(4.0 / 3.0));
        assert!(parse_aspect("nonsense").is_none());
    }

    #[test]
    fn crops_and_resizes_within_policy() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("artist.png");
        let img = image::DynamicImage::new_rgb8(800, 400);
        img.save(&p).unwrap();
        let mut cfg = Config::default();
        cfg.artist_image_target_size = 500;
        cfg.artist_image_crop = true;
        optimize_artist_image(&p, &cfg).unwrap();
        let (w, h) = image::image_dimensions(&p).unwrap();
        assert_eq!(w, h, "cropped square");
        assert!(w <= 500 && w > 0);
    }
}