use std::io;
use std::path::{Path, PathBuf};

use crate::model::config::WorkspaceConfig;

/// Errors returned by workspace discovery and config loading.
#[derive(Debug)]
pub enum WorkspaceError {
    /// No `.accelmars/` directory was found anywhere up the directory tree.
    NotFound,
    /// An I/O error occurred while traversing the filesystem.
    IoError(io::Error),
    /// The workspace config contains an unsupported schema_version.
    #[allow(dead_code)]
    UnsupportedSchemaVersion(String),
    /// The workspace config.json could not be parsed.
    #[allow(dead_code)]
    InvalidConfig(serde_json::Error),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceError::NotFound => {
                write!(f, "no workspace found. Run 'anchor init' to configure.")
            }
            WorkspaceError::IoError(e) => write!(f, "I/O error: {}", e),
            WorkspaceError::UnsupportedSchemaVersion(v) => {
                write!(
                    f,
                    "anchor workspace schema version \"{}\" is not supported by this version of anchor.\nPlease upgrade: https://github.com/accelmars/anchor",
                    v
                )
            }
            WorkspaceError::InvalidConfig(e) => write!(f, "invalid config.json: {}", e),
        }
    }
}

impl From<io::Error> for WorkspaceError {
    fn from(e: io::Error) -> Self {
        WorkspaceError::IoError(e)
    }
}

/// Walk up the directory tree from `start`, looking for a `.accelmars/` directory.
/// Returns the path of the directory containing `.accelmars/`, with no trailing slash.
///
/// Algorithm (verbatim from 260425-anchor-workspace-layout.md §4 Root Discovery):
/// 1. Start at `start`
/// 2. Check if .accelmars/ directory exists in current directory
/// 3. If yes → workspace root found, return this path
/// 4. If no → move to parent directory
/// 5. If reached filesystem root (/) with no .accelmars/ found:
///    → hard error: "no workspace found. Run 'anchor init' to configure."
/// 6. Repeat from step 2
///
/// Extracted from `find_workspace_root` so callers that already know their start
/// directory (e.g., tests) can call this directly without touching the global cwd.
pub(crate) fn find_workspace_root_from(start: &Path) -> Result<PathBuf, WorkspaceError> {
    let mut current = start.to_path_buf();

    loop {
        let marker = current.join(".accelmars");
        match marker.is_dir() {
            true => return Ok(current),
            false => {
                let parent = current.parent().map(|p| p.to_path_buf());
                match parent {
                    Some(p) if p != current => {
                        current = p;
                    }
                    _ => return Err(WorkspaceError::NotFound),
                }
            }
        }
    }
}

/// Walk up the directory tree from the current working directory, looking for a
/// `.accelmars/` directory. Returns the path of the directory containing it,
/// with no trailing slash.
pub fn find_workspace_root() -> Result<PathBuf, WorkspaceError> {
    let start = std::env::current_dir().map_err(WorkspaceError::IoError)?;
    find_workspace_root_from(&start)
}

/// Read `.accelmars/anchor/config.json` from the given workspace root, deserialize it,
/// and enforce schema version compatibility.
#[allow(dead_code)]
///
/// PHASE-2-BRIDGE Contract 2: schema_version is required and hard-enforced.
/// Any unknown version causes a hard stop — never degrade silently.
///
/// Error message format (exact, per 260425-anchor-workspace-layout.md §4):
/// `anchor workspace schema version "{v}" is not supported by this version of anchor.`
pub fn load_and_check_config(workspace_root: &Path) -> Result<WorkspaceConfig, WorkspaceError> {
    let config_path = workspace_root.join(".accelmars").join("anchor").join("config.json");
    let content = std::fs::read_to_string(&config_path).map_err(WorkspaceError::IoError)?;
    let config: WorkspaceConfig =
        serde_json::from_str(&content).map_err(WorkspaceError::InvalidConfig)?;
    if config.schema_version != "1" {
        return Err(WorkspaceError::UnsupportedSchemaVersion(
            config.schema_version,
        ));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Happy path: `.accelmars/` directory exists in the start directory.
    /// Verifies: returns Ok(path) matching the temp dir, no trailing slash.
    #[test]
    fn test_found_happy_path() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        fs::create_dir_all(dir.path().join(".accelmars")).unwrap();

        let root = find_workspace_root_from(dir.path()).expect("should find workspace root");
        // No trailing slash
        let as_str = root.to_string_lossy();
        assert!(
            !as_str.ends_with('/'),
            "root path must not have a trailing slash, got: {}",
            as_str
        );
        // Path must match the temp dir (canonicalized)
        assert_eq!(
            root.canonicalize().expect("canonicalize root"),
            dir.path().canonicalize().expect("canonicalize dir")
        );
    }

    /// Not found: no `.accelmars/` anywhere up the tree from a fresh temp directory.
    /// Verifies: returns Err(WorkspaceError::NotFound), does not loop infinitely.
    #[test]
    fn test_not_found_filesystem_root_stop() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        // No .accelmars/ in dir or its ancestors (tempdir is not inside the workspace).
        let result = find_workspace_root_from(dir.path());
        match result {
            Err(WorkspaceError::NotFound) => {}
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    /// Nested ascent: `.accelmars/` at root of temp dir, start is a deep subdirectory.
    /// Verifies: walk-up finds `.accelmars/` at the correct root level.
    #[test]
    fn test_nested_subdirectory_ascent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        fs::create_dir_all(dir.path().join(".accelmars")).unwrap();

        let deep = dir.path().join("2").join("3").join("4");
        fs::create_dir_all(&deep).expect("failed to create nested dirs");

        let root = find_workspace_root_from(&deep).expect("should find workspace root via ascent");
        assert_eq!(
            root.canonicalize().expect("canonicalize root"),
            dir.path().canonicalize().expect("canonicalize dir"),
            "walk-up should find .accelmars/ at the workspace root, not at the deep subdirectory"
        );
    }

    /// Unknown schema_version causes hard stop with exact error message.
    /// PHASE-2-BRIDGE Contract 2: hard-stop, never degrade silently.
    #[test]
    fn test_unsupported_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let anchor_dir = dir.path().join(".accelmars").join("anchor");
        fs::create_dir_all(&anchor_dir).unwrap();
        fs::write(anchor_dir.join("config.json"), r#"{"schema_version":"99"}"#).unwrap();

        let result = load_and_check_config(dir.path());
        match result {
            Err(WorkspaceError::UnsupportedSchemaVersion(v)) => {
                assert_eq!(v, "99");
                let msg = format!("{}", WorkspaceError::UnsupportedSchemaVersion(v));
                assert!(
                    msg.contains("anchor workspace schema version"),
                    "error message must reference 'anchor workspace schema version', got: {}",
                    msg
                );
                assert!(
                    msg.contains("not supported"),
                    "error message must contain 'not supported', got: {}",
                    msg
                );
            }
            other => panic!("expected UnsupportedSchemaVersion, got: {:?}", other),
        }
    }
}
