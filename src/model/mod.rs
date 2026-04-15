pub mod config;
pub mod manifest;
pub mod reference;
pub mod rewrite;

/// Workspace-root-relative path. Normalized, no `./` prefix, forward slashes, case-sensitive.
#[allow(dead_code)]
pub type CanonicalPath = String;
