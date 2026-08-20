//! Web frontend adapter for tidev.
//!
//! The crate owns the HTTP boundary only. Product state and agent semantics
//! remain in tidev_core::Runtime. In debug builds the web shell is served
//! by Vite; release builds embed the Vite output into the binary.

mod api;
mod frontend;
mod server;

pub use server::{WebOptions, run};
