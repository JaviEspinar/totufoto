//! HEIC/HEIF photos on Windows, decoded with Windows' own image decoders (WIC). They decode
//! HEIC once Microsoft's "HEIF Image Extensions" and "HEVC Video Extensions" are installed
//! (from the Microsoft Store; often already there).
//!
//! WIC hands the picture as stored; the orientation is applied by the caller, from the
//! photo's EXIF data.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use anyhow::{Result, anyhow};
use image::RgbImage;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat24bppRGB, IWICImagingFactory, WICConvertBitmapSource,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::core::PCWSTR;

/// What to install when Windows can't decode HEIC, said with the error.
pub const MISSING: &str =
    "HEIC photos need Microsoft's HEIF Image Extensions and HEVC Video Extensions (Microsoft Store)";

/// The first picture of the HEIF file at `path`, as stored (not turned), in RGB.
pub fn decode(path: &Path) -> Result<RgbImage> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: COM is set up for this thread (a second call is harmless), and every call gets
    // valid arguments; the interfaces release themselves when dropped.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| anyhow!("Windows Imaging Component: {e}"))?;
        let decoded = (|| -> windows::core::Result<RgbImage> {
            let decoder = factory.CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )?;
            let frame = decoder.GetFrame(0)?;
            let rgb = WICConvertBitmapSource(&GUID_WICPixelFormat24bppRGB, &frame)?;
            let (mut width, mut height) = (0u32, 0u32);
            rgb.GetSize(&mut width, &mut height)?;
            let stride = width * 3;
            let mut pixels = vec![0u8; stride as usize * height as usize];
            rgb.CopyPixels(std::ptr::null(), stride, &mut pixels)?;
            Ok(RgbImage::from_raw(width, height, pixels).unwrap_or_default())
        })();
        // No HEIF decoder, or one without HEVC: the extensions are missing.
        decoded.map_err(|e| anyhow!("{MISSING} ({e})"))
    }
}
