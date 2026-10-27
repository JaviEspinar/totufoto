//! HEIC/HEIF photos (iPhones take them) decoded with libheif, loaded from the system when the
//! program first needs it, as ONNX Runtime is: nothing is linked at build time, and the HEVC
//! decoder libheif uses (libde265) comes with the system's package.
//!
//! Only the few functions needed are looked up. libheif applies the image's rotation and
//! mirroring (`irot`, `imir`) itself, so what comes back is upright.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Result, anyhow, bail};
use image::RgbImage;

/// What to install when libheif isn't there, said with the error.
pub const MISSING: &str = "HEIC photos need libheif: sudo apt install libheif1 libheif-plugin-libde265";

#[repr(C)]
#[derive(Clone, Copy)]
struct HeifError {
    code: u32,
    subcode: u32,
    message: *const c_char,
}

// From libheif's heif.h: heif_colorspace_RGB, heif_chroma_interleaved_RGB,
// heif_channel_interleaved.
const COLORSPACE_RGB: u32 = 1;
const CHROMA_INTERLEAVED_RGB: u32 = 10;
const CHANNEL_INTERLEAVED: u32 = 10;

type Init = unsafe extern "C" fn(*mut c_void) -> HeifError;
type ContextAlloc = unsafe extern "C" fn() -> *mut c_void;
type ContextFree = unsafe extern "C" fn(*mut c_void);
type ReadFromFile = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_void) -> HeifError;
type PrimaryHandle = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> HeifError;
type HandleRelease = unsafe extern "C" fn(*const c_void);
type DecodeImage = unsafe extern "C" fn(*const c_void, *mut *mut c_void, u32, u32, *const c_void) -> HeifError;
type ImageRelease = unsafe extern "C" fn(*const c_void);
type PlaneReadonly = unsafe extern "C" fn(*const c_void, u32, *mut c_int) -> *const u8;
type ImageSize = unsafe extern "C" fn(*const c_void, u32) -> c_int;

struct Lib {
    // Kept loaded for the program's lifetime: the functions below point into it.
    _lib: libloading::Library,
    context_alloc: ContextAlloc,
    context_free: ContextFree,
    read_from_file: ReadFromFile,
    primary_handle: PrimaryHandle,
    handle_release: HandleRelease,
    decode_image: DecodeImage,
    image_release: ImageRelease,
    plane_readonly: PlaneReadonly,
    image_width: ImageSize,
    image_height: ImageSize,
}

// SAFETY: libheif's functions may be called from any thread, each decode with its own context.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

fn names() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["libheif.1.dylib", "/opt/homebrew/lib/libheif.1.dylib", "/usr/local/lib/libheif.1.dylib"]
    } else if cfg!(windows) {
        &["heif.dll", "libheif.dll"]
    } else {
        &["libheif.so.1", "libheif.so"]
    }
}

fn load() -> Result<Lib, String> {
    let mut last = String::new();
    for name in names() {
        // SAFETY: loading libheif runs its initialisers, which only set up its own state.
        match unsafe { libloading::Library::new(*name) } {
            Ok(lib) => return bind(lib).map_err(|e| format!("{name}: {e}")),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn bind(lib: libloading::Library) -> Result<Lib, libloading::Error> {
    // SAFETY: the types above are those of libheif's C declarations.
    unsafe {
        // Newer libheif loads its decoder plugins (libde265) here; older ones don't have it.
        if let Ok(init) = lib.get::<Init>(b"heif_init\0") {
            init(std::ptr::null_mut());
        }
        Ok(Lib {
            context_alloc: *lib.get(b"heif_context_alloc\0")?,
            context_free: *lib.get(b"heif_context_free\0")?,
            read_from_file: *lib.get(b"heif_context_read_from_file\0")?,
            primary_handle: *lib.get(b"heif_context_get_primary_image_handle\0")?,
            handle_release: *lib.get(b"heif_image_handle_release\0")?,
            decode_image: *lib.get(b"heif_decode_image\0")?,
            image_release: *lib.get(b"heif_image_release\0")?,
            plane_readonly: *lib.get(b"heif_image_get_plane_readonly\0")?,
            image_width: *lib.get(b"heif_image_get_width\0")?,
            image_height: *lib.get(b"heif_image_get_height\0")?,
            _lib: lib,
        })
    }
}

/// libheif, loaded on first use; the reason when it can't be.
fn lib() -> Result<&'static Lib> {
    static LIB: OnceLock<Result<Lib, String>> = OnceLock::new();
    match LIB.get_or_init(load) {
        Ok(lib) => Ok(lib),
        Err(why) => {
            tracing::debug!("libheif not loaded: {why}");
            bail!("{MISSING}")
        }
    }
}

/// Whether libheif can be loaded on this computer.
pub fn available() -> bool {
    lib().is_ok()
}

fn check(e: HeifError, what: &str) -> Result<()> {
    if e.code == 0 {
        return Ok(());
    }
    let message = if e.message.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(e.message) }.to_string_lossy().into_owned()
    };
    Err(anyhow!("{what}: {message} ({}.{})", e.code, e.subcode))
}

/// Frees what libheif allocated, however decoding ends.
struct Owned<'a> {
    lib: &'a Lib,
    context: *mut c_void,
    handle: *mut c_void,
    image: *mut c_void,
}

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        // SAFETY: each pointer is libheif's, released once, image before handle before context.
        unsafe {
            if !self.image.is_null() {
                (self.lib.image_release)(self.image);
            }
            if !self.handle.is_null() {
                (self.lib.handle_release)(self.handle);
            }
            if !self.context.is_null() {
                (self.lib.context_free)(self.context);
            }
        }
    }
}

/// The primary image of the HEIF file at `path`, upright, in RGB.
pub fn decode(path: &Path) -> Result<RgbImage> {
    let lib = lib()?;
    let name = CString::new(path.to_string_lossy().as_bytes())?;
    // SAFETY: every call gets pointers libheif gave, or null where its API allows it; `Owned`
    // frees them.
    unsafe {
        let mut owned =
            Owned { lib, context: (lib.context_alloc)(), handle: std::ptr::null_mut(), image: std::ptr::null_mut() };
        if owned.context.is_null() {
            bail!("libheif couldn't start");
        }
        check((lib.read_from_file)(owned.context, name.as_ptr(), std::ptr::null()), "reading the file")?;
        check((lib.primary_handle)(owned.context, &mut owned.handle), "finding its image")?;
        check(
            (lib.decode_image)(
                owned.handle,
                &mut owned.image,
                COLORSPACE_RGB,
                CHROMA_INTERLEAVED_RGB,
                std::ptr::null(),
            ),
            "decoding it",
        )?;
        let width = (lib.image_width)(owned.image, CHANNEL_INTERLEAVED);
        let height = (lib.image_height)(owned.image, CHANNEL_INTERLEAVED);
        let mut stride: c_int = 0;
        let plane = (lib.plane_readonly)(owned.image, CHANNEL_INTERLEAVED, &mut stride);
        if plane.is_null() || width <= 0 || height <= 0 || stride < width * 3 {
            bail!("libheif returned no picture");
        }
        let (width, height, stride) = (width as usize, height as usize, stride as usize);
        let mut pixels = Vec::with_capacity(width * height * 3);
        for row in 0..height {
            pixels.extend_from_slice(std::slice::from_raw_parts(plane.add(row * stride), width * 3));
        }
        RgbImage::from_raw(width as u32, height as u32, pixels)
            .ok_or_else(|| anyhow!("libheif returned a broken picture"))
    }
}
