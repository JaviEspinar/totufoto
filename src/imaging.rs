//! Decoding, orientation, resizing and encoding helpers.

use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use fast_image_resize::Resizer;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "tif", "tiff", "gif", "bmp"];

pub fn is_supported(path: &Path) -> bool {
    let ext = extension(path);
    // HEIC/HEIF is decoded with macOS `sips`.
    IMAGE_EXTENSIONS.contains(&ext.as_str()) || (cfg!(target_os = "macos") && matches!(ext.as_str(), "heic" | "heif"))
}

/// Formats a browser can display directly, so originals can be served untouched.
pub const BROWSER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

pub fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

pub fn is_heif(path: &Path) -> bool {
    matches!(extension(path).as_str(), "heic" | "heif")
}

/// Decodes an image and applies its EXIF orientation.
pub fn decode(path: &Path, bytes: &[u8]) -> Result<DynamicImage> {
    if is_heif(path) {
        return decode_heif(path);
    }
    let mut decoder = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// HEIC/HEIF is decoded with macOS `sips`, which also applies the orientation.
fn decode_heif(path: &Path) -> Result<DynamicImage> {
    let out =
        std::env::temp_dir().join(format!("imadive-{}-{:?}.jpg", std::process::id(), std::thread::current().id()));
    let status = Command::new("sips")
        .args(["-s", "format", "jpeg", "-s", "formatOptions", "90", "-Z", "2560"])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("HEIC needs macOS `sips`")?;
    if !status.success() {
        return Err(anyhow!("sips failed to convert {}", path.display()));
    }
    let img = image::open(&out);
    let _ = std::fs::remove_file(&out);
    Ok(img?)
}

pub fn resize_rgb(img: &RgbImage, w: u32, h: u32) -> Result<RgbImage> {
    if img.dimensions() == (w, h) {
        return Ok(img.clone());
    }
    let mut dst = RgbImage::new(w, h);
    Resizer::new().resize(img, &mut dst, None)?;
    Ok(dst)
}

/// Resizes so that the longest side is at most `max_side` (never upscales).
pub fn fit(img: &RgbImage, max_side: u32) -> Result<RgbImage> {
    fit_box(img, max_side, max_side)
}

/// Resizes to fit inside `max_w` x `max_h` (never upscales).
pub fn fit_box(img: &RgbImage, max_w: u32, max_h: u32) -> Result<RgbImage> {
    let (w, h) = img.dimensions();
    let scale = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    if scale >= 1.0 {
        return Ok(img.clone());
    }
    resize_rgb(img, ((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1))
}

pub fn encode_jpeg(img: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(64 * 1024);
    JpegEncoder::new_with_quality(&mut out, quality).encode_image(img)?;
    Ok(out)
}

/// Square crop around a face box (with some margin), resized for the people view.
pub fn face_thumbnail(img: &RgbImage, bbox: [f32; 4], size: u32) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions();
    let cx = (bbox[0] + bbox[2]) / 2.0;
    let cy = (bbox[1] + bbox[3]) / 2.0;
    let side = ((bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) * 1.5).min(w.min(h) as f32).max(1.0);
    let x = (cx - side / 2.0).clamp(0.0, w as f32 - side) as u32;
    let y = (cy - side / 2.0).clamp(0.0, h as f32 - side) as u32;
    let s = side as u32;
    let crop = image::imageops::crop_imm(img, x, y, s.min(w - x), s.min(h - y)).to_image();
    encode_jpeg(&resize_rgb(&crop, size, size)?, 85)
}
