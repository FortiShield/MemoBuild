//! Cache contracts and backend implementations. Avoid CLI and server dependencies.

#![forbid(unsafe_code)]

pub mod cache;
pub mod cache_cluster;
pub mod cache_redis;
pub mod cache_utils_exe;
pub mod metadata;
pub mod remote_cache;
pub mod scalable_db;
pub mod tls;

pub use memobuild_core::{error, events, graph};
pub use memobuild_observability::{cache_span, metrics, replicate_span};

/// Returns the crate name, useful for smoke-testing the scaffold.
pub const CRATE_NAME: &str = "memobuild-cache";
