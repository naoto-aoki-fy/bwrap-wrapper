//! Phase 1 Rust port of bwrap-wrapper e614a089b80b6f7832edd66950bba8f06abe0040.
//! Argument rewriting is independent of discovery and process execution.

pub mod config;
pub mod mounts;
pub mod transform;

mod paths;
