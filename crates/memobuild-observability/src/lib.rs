//! Shared instrumentation abstractions and observability integration.

#![forbid(unsafe_code)]

pub mod logging;
pub mod metrics;
pub mod tracing;

/// Returns the crate name, useful for smoke-testing the scaffold.
pub const CRATE_NAME: &str = "memobuild-observability";
