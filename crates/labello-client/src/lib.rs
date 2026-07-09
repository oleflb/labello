//! Shared client contract for Labello frontends.
//!
//! UI crates should depend on these traits instead of hard-coding HTTP,
//! browser storage, or direct filesystem access.

pub mod demo;
pub mod dto;
pub mod error;
pub mod http;
pub mod traits;

pub use demo::DemoLabelloApi;
pub use dto::*;
pub use error::*;
pub use http::{AuthHeaders, HttpLabelloApi};
pub use traits::*;
