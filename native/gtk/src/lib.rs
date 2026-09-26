//! Shared library for the native GTK client.
//!
//! The backend does not depend on a running display. GTK widgets can use it
//! directly, while `cargo test --lib` exercises project and compiler behavior
//! headlessly.

pub mod backend;
