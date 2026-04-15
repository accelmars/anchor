use serde::{Deserialize, Serialize};

/// Workspace configuration — stored in `.mind/config.json`.
#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct WorkspaceConfig {
    pub schema_version: String,
}
