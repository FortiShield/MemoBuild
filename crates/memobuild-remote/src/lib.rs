//! Remote execution, worker lifecycle, scheduling, and protocol adapters.
//!
//! Migration shell only. Move implementation here only after ownership,
//! public API, dependency direction, and regression tests have been reviewed.

#![forbid(unsafe_code)]

/// Returns the crate name, useful for smoke-testing the scaffold.
pub const CRATE_NAME: &str = "memobuild-remote";
