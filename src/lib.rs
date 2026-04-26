// lib.rs — re-exports for integration testing (MF-006)
// This makes the internal modules accessible to tests/ without duplicating code.
pub mod cli;
pub mod core;
pub mod infra;
pub mod model;
pub mod server;
pub use server::{routes, build_state, AnchorState};
