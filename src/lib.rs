//! totufoto: fast local photo gallery with timeline, places and face grouping.
//! Used by the command-line app (`src/main.rs`) and the desktop app (`desktop/`).

mod app;
mod cluster;
mod db;
pub mod faces;
mod geo;
mod imaging;
pub mod scan;
mod server;

pub use app::{Config, Gallery, Host, enable_faces, init_logging};
