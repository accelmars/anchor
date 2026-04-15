use serde::{Deserialize, Serialize};

/// Workspace configuration — stored in `.mind/config.json`.
///
/// Phase 1 schema: `{"schema_version": "1"}`
/// schema_version is a String (not integer) because future semver-style values
/// (e.g. "2.1") require string representation.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub schema_version: String,
}

impl WorkspaceConfig {
    /// Construct the Phase 1 default configuration.
    pub fn phase1() -> Self {
        Self {
            schema_version: "1".to_string(),
        }
    }
}
