//! Rust rewrite of bwrap-wrapper e614a089b80b6f7832edd66950bba8f06abe0040,
//! with Linux integration and compatibility corrections; original Apache-2.0 license retained.
//! Argument rewriting is independent of discovery and process execution.

pub mod config;
pub mod mounts;
pub mod transform;

mod paths;

pub mod logging;
pub mod runtime;
