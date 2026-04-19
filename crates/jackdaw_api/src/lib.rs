//! Public API for Jackdaw editor extensions.
//!
//! This crate is a thin facade over [`jackdaw_api_internal`]. Users
//! import it exactly the same way regardless of whether the editor
//! is linking jackdaw statically or dynamically, and the
//! `dynamic_linking` cargo feature handles the plumbing to make
//! dylib-loaded extensions see the same `TypeId` as the host.
//!
//! # For static consumers
//!
//! ```toml
//! jackdaw_api = "0.4"
//! ```
//!
//! # For dylib extensions
//!
//! ```toml
//! jackdaw_api = { version = "0.4", features = ["dynamic_linking"] }
//! bevy = "0.18"  # `dynamic_linking` is pulled in transitively
//! ```
//!
//! Matching the `dynamic_linking` feature on the host binary
//! (`jackdaw`'s `dylib` feature) is mandatory for runtime dylib
//! loading to be sound.

// Force a link dependency on `jackdaw_dylib` so the compiled jackdaw
// types live in a single shared `.so` that both the editor binary
// and every extension dylib see. Mirrors the `bevy_dylib` trick that
// `bevy/dynamic_linking` uses.
#[cfg(feature = "dynamic_linking")]
#[allow(unused_imports)]
use jackdaw_dylib as _;

// Re-export everything from the internal crate. Extension authors
// use these exactly as they did before the facade split.
pub use jackdaw_api_internal::*;
