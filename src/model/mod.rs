pub mod config;
pub mod manifest;
pub mod reference;
pub mod rewrite;

/// Canonical path form: workspace-root-relative, no `./` prefix, fully normalized,
/// forward slashes, case-sensitive. PHASE-2-BRIDGE Contract 4: this definition is frozen.
/// Phase 2 indexes files by this form. Do not change without design session + version bump.
pub type CanonicalPath = String;
