//! Shared library for the native GTK client.
//!
//! The backend does not depend on a running display. GTK widgets can use it
//! directly, while `cargo test --no-default-features --all-targets` exercises
//! project and compiler behavior without GTK development libraries.

pub mod backend;
