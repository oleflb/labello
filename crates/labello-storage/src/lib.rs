//! Filesystem-backed persistence for Labello datasets.

pub mod assignment;
mod completion_projection;
pub mod error;
pub mod fsjson;
pub mod fstoml;
pub mod import;
pub mod ingest;
pub mod keybindings;
pub mod paths;
pub mod repository;
pub mod stats;
pub mod sync;

pub use error::*;
pub use import::*;
pub use ingest::*;
pub use repository::*;
