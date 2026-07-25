//! Shared Labello domain model, validation, schemas, and replay logic.
//!
//! This crate intentionally has no server, filesystem, or UI dependencies so
//! the backend, browser client, and future native offline client can share the
//! same rules.

pub mod agreement;
pub mod annotation;
pub mod assignment;
pub mod dataset;
pub mod error;
pub mod event;
pub mod geometry;
pub mod ids;
pub mod keybindings;
pub mod migration;
pub mod offline;
pub mod prelabel;
pub mod review;
pub mod schema;
pub mod state;
pub mod stats;
pub mod task;
pub mod user;

pub use agreement::*;
pub use annotation::*;
pub use assignment::*;
pub use dataset::*;
pub use error::*;
pub use event::*;
pub use geometry::*;
pub use ids::*;
pub use keybindings::*;
pub use migration::*;
pub use offline::*;
pub use prelabel::*;
pub use review::*;
pub use schema::*;
pub use state::*;
pub use stats::*;
pub use task::*;
pub use user::*;

pub const SCHEMA_VERSION: u32 = 2;

pub type Timestamp = chrono::DateTime<chrono::Utc>;

pub fn now() -> Timestamp {
    chrono::Utc::now()
}

#[cfg(test)]
mod v2_contract_tests;
