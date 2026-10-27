//! Decoding, orientation, resizing and encoding helpers.

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use fast_image_resize::Resizer;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "tif", "tiff", "gif", "bmp"];

/// A photo the gallery shows. HEIC/HEIF too, on every system: where it can't be decoded (no
/// libheif, no Windows HEIF extension), the file is listed as failed, saying what to install.
pub fn is_supported(path: &Path) -> bool {
    IMAGE_EXTENSIONS.contains(&extension(path).as_str()) || is_heif(path)
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

/// HEIC/HEIF, upright: with libheif when the system has it, else with Windows' decoders (WIC)
/// or macOS `sips`.
fn decode_heif(path: &Path) -> Result<DynamicImage> {
    if crate::heif::available() || !(cfg!(windows) || cfg!(target_os = "macos")) {
        return Ok(DynamicImage::ImageRgb8(crate::heif::decode(path)?));
    }
    decode_heif_system(path)
}

/// HEIC/HEIF with Windows' decoders (WIC), which give the picture as stored: it is turned
/// here by its EXIF orientation, which iPhones set to match the HEIF rotation.
#[cfg(windows)]
fn decode_heif_system(path: &Path) -> Result<DynamicImage> {
    let mut img = DynamicImage::ImageRgb8(crate::heif_windows::decode(path)?);
    if let Some(orientation) = heif_orientation(path) {
        img.apply_orientation(orientation);
    }
    Ok(img)
}

#[cfg(not(windows))]
fn decode_heif_system(path: &Path) -> Result<DynamicImage> {
    decode_sips(path)
}

/// The EXIF orientation of a HEIF file, if it says one.
#[cfg(windows)]
fn heif_orientation(path: &Path) -> Option<image::metadata::Orientation> {
    let file = std::fs::File::open(path).ok()?;
    let exif = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).ok()?;
    let value = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?.value.get_uint(0)?;
    image::metadata::Orientation::from_exif(u8::try_from(value).ok()?)
}

/// HEIC/HEIF converted with macOS `sips`, which also applies the orientation.
#[cfg(not(windows))]
fn decode_sips(path: &Path) -> Result<DynamicImage> {
    let out =
        std::env::temp_dir().join(format!("imadive-{}-{:?}.jpg", std::process::id(), std::thread::current().id()));
    let status = std::process::Command::new("sips")
        // At its own size: `-Z` would also enlarge smaller photos (the gallery resizes later).
        .args(["-s", "format", "jpeg", "-s", "formatOptions", "90"])
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The HEIC test photo: 256x320, portrait, with a GPS position.
    pub(crate) fn heic_fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/heic/portrait.heic")
    }

    /// Whether HEIC must decode here (CI's Linux runner installs libheif and says so).
    pub(crate) fn heic_expected() -> bool {
        cfg!(target_os = "macos") || std::env::var_os("IMADIVE_EXPECT_HEIC").is_some()
    }

    #[test]
    fn heic_photos_are_decoded_upright_or_say_what_to_install() {
        let path = heic_fixture();
        assert!(is_supported(&path), "HEIC is listed on every system");
        match decode(&path, &std::fs::read(&path).unwrap()) {
            Ok(img) => assert_eq!((img.width(), img.height()), (256, 320)),
            Err(e) => {
                assert!(!heic_expected(), "HEIC should decode here: {e:#}");
                let message = format!("{e:#}");
                assert!(
                    message.contains("libheif") || message.contains("HEIF Image Extensions"),
                    "the error says what to install: {message}"
                );
            }
        }
    }
}
