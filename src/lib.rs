//! imadive: fast local photo gallery with timeline, places and face grouping.
//! Used by the command-line app (`src/main.rs`) and the desktop app (`desktop/`).

mod app;
mod cluster;
mod db;
mod duplicates;
pub mod faces;
mod geo;
mod guard;
mod http;
mod imaging;
mod library;
mod metadata;
mod rotate;
pub mod scan;
#[cfg(test)]
mod testutil;

pub use app::{
    Config, DEFAULT_FACE_THRESHOLD, Gallery, Host, enable_faces, init_desktop_logging, init_logging, log_to_file,
};
