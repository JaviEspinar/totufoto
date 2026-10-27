//! imadive: fast local photo gallery with timeline, places and face grouping.
//! Used by the command-line app (`src/main.rs`) and the desktop app (`desktop/`).

mod app;
mod backup;
mod cluster;
mod db;
mod duplicates;
pub mod faces;
mod geo;
mod guard;
mod heif;
#[cfg(windows)]
mod heif_windows;
mod http;
mod imaging;
mod library;
mod metadata;
mod rotate;
pub mod scan;
#[cfg(test)]
mod testutil;
pub mod update;
mod video;

/// Whether HEIC/HEIF photos can be decoded with the system's libheif (see `heif`).
pub fn heic_libheif_available() -> bool {
    heif::available()
}

pub use app::{
    Config, DEFAULT_FACE_THRESHOLD, Gallery, Host, enable_faces, init_desktop_logging, init_logging, log_to_file,
};
