// lib.rs — re-exports for integration testing (MF-006)
// This makes the internal modules accessible to tests/ without duplicating code.
pub mod apply;
pub mod cli;
pub mod core;
pub mod infra;
pub mod model;
pub mod refs;
pub mod server;
pub use server::{build_state, routes, AnchorState};
