//! Filesystem-backed persistence for Labello datasets.

pub mod assignment;
pub mod error;
pub mod fsjson;
pub mod fstoml;
pub mod ingest;
pub mod keybindings;
pub mod paths;
pub mod repository;
pub mod stats;
pub mod sync;

pub use error::*;
pub use ingest::*;
pub use repository::*;
