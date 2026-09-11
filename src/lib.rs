//! Restricted, read-only LUKS2 image reader. See docs/design-v0.2.0.ko.md.
pub mod adapter;
pub mod crypto;
pub mod error;
pub mod image;
pub mod metadata;
pub mod probe;
mod strict_json;
pub mod volume;
pub use error::{Error, Result};
