//! Restricted LUKS2 image bridge. RO is the default; writes require an exclusive RW session.
pub mod adapter;
pub mod crypto;
pub mod error;
pub mod image;
pub mod metadata;
pub mod probe;
mod strict_json;
pub mod volume;
pub use error::{Error, Result};
