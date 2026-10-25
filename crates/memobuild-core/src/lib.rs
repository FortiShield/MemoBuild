//! Foundational domain types, graph contracts, hashing abstractions, and reproducibility primitives.

#![forbid(unsafe_code)]

pub mod constants;
pub mod core;
pub mod env;
pub mod error;
pub mod events;
pub mod git;
pub mod graph;
pub mod hasher;
pub mod reproducible;

/// Returns the crate name, useful for smoke-testing the scaffold.
pub const CRATE_NAME: &str = "memobuild-core";
