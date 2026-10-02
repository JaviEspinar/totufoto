//! imadive: fast local photo gallery with timeline, places and face grouping.
//! Used by the command-line app (`src/main.rs`) and the desktop app (`desktop/`).

mod app;
mod cluster;
mod db;
mod duplicates;
pub mod faces;
mod geo;
mod guard;
mod imaging;
mod library;
mod rotate;
pub mod scan;
mod server;
#[cfg(test)]
mod testutil;

pub use app::{Config, Gallery, Host, enable_faces, init_logging};
