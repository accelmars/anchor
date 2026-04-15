use std::io;
use std::path::{Path, PathBuf};

use crate::model::config::WorkspaceConfig;

/// Errors returned by workspace discovery and config loading.
#[derive(Debug)]
pub enum WorkspaceError {
    /// No `.mind-root` marker was found anywhere up the directory tree.
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
                write!(f, "no workspace found. Run 'mind init' to configure.")
            }
            WorkspaceError::IoError(e) => write!(f, "I/O error: {}", e),
            WorkspaceError::UnsupportedSchemaVersion(v) => {
                write!(
                    f,
                    "mind workspace schema version \"{}\" is not supported by this version of mind.\nPlease upgrade: https://github.com/accelmars/mind-engine",
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

/// Walk up the directory tree from the current working directory, looking for a
/// `.mind-root` marker file. Returns the path of the directory containing the
/// marker, with no trailing slash.
///
/// Algorithm (verbatim from 02-WORKSPACE.md §Root Discovery Algorithm):
/// 1. Start at cwd
/// 2. Check if .mind-root exists in current directory
/// 3. If yes → workspace root found, return this path
/// 4. If no → move to parent directory
/// 5. If reached filesystem root (/) with no .mind-root found:
///    → hard error: "no workspace found. Run 'mind init' to configure."
/// 6. Repeat from step 2
pub fn find_workspace_root() -> Result<PathBuf, WorkspaceError> {
    let mut current = std::env::current_dir().map_err(WorkspaceError::IoError)?;

    loop {
        let marker = current.join(".mind-root");
        match marker.exists() {
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

/// Read `.mind/config.json` from the given workspace root, deserialize it,
/// and enforce schema version compatibility.
#[allow(dead_code)]
///
/// PHASE-2-BRIDGE Contract 2: schema_version is required and hard-enforced.
/// Any unknown version causes a hard stop — never degrade silently.
///
/// Error message format (exact, per 07-PHASE-BRIDGE.md §Contract 2):
/// `mind workspace schema version "{v}" is not supported by this version of mind.`
pub fn load_and_check_config(workspace_root: &Path) -> Result<WorkspaceConfig, WorkspaceError> {
    let config_path = workspace_root.join(".mind").join("config.json");
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

    /// Happy path: `.mind-root` exists in the current directory.
    /// Verifies: returns Ok(path) matching the temp dir, no trailing slash.
    #[test]
    fn test_found_happy_path() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let marker = dir.path().join(".mind-root");
        fs::write(&marker, "").expect("failed to write marker");

        // Change cwd to the temp dir, run discovery, restore cwd.
        let original = std::env::current_dir().expect("failed to get cwd");
        std::env::set_current_dir(dir.path()).expect("failed to set cwd");

        let result = find_workspace_root();

        std::env::set_current_dir(&original).expect("failed to restore cwd");

        let root = result.expect("should find workspace root");
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

    /// Not found: no `.mind-root` anywhere up the tree from a fresh temp directory.
    /// Verifies: returns Err(WorkspaceError::NotFound), does not loop infinitely.
    #[test]
    fn test_not_found_filesystem_root_stop() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        // No .mind-root anywhere up this tree (assuming /tmp or equivalent has none).

        let original = std::env::current_dir().expect("failed to get cwd");
        std::env::set_current_dir(dir.path()).expect("failed to set cwd");

        let result = find_workspace_root();

        std::env::set_current_dir(&original).expect("failed to restore cwd");

        match result {
            Err(WorkspaceError::NotFound) => {}
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    /// Nested ascent: `.mind-root` at root of temp dir, cwd is a deep subdirectory.
    /// Verifies: walk-up finds `.mind-root` at the correct root level.
    #[test]
    fn test_nested_subdirectory_ascent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let marker = dir.path().join(".mind-root");
        fs::write(&marker, "").expect("failed to write marker");

        let deep = dir.path().join("2").join("3").join("4");
        fs::create_dir_all(&deep).expect("failed to create nested dirs");

        let original = std::env::current_dir().expect("failed to get cwd");
        std::env::set_current_dir(&deep).expect("failed to set cwd to deep dir");

        let result = find_workspace_root();

        std::env::set_current_dir(&original).expect("failed to restore cwd");

        let root = result.expect("should find workspace root via ascent");
        assert_eq!(
            root.canonicalize().expect("canonicalize root"),
            dir.path().canonicalize().expect("canonicalize dir"),
            "walk-up should find .mind-root at the workspace root, not at the deep subdirectory"
        );
    }

    /// Unknown schema_version causes hard stop with exact error message.
    /// PHASE-2-BRIDGE Contract 2: hard-stop, never degrade silently.
    #[test]
    fn test_unsupported_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let mind_dir = dir.path().join(".mind");
        fs::create_dir_all(&mind_dir).unwrap();
        fs::write(mind_dir.join("config.json"), r#"{"schema_version":"99"}"#).unwrap();

        let result = load_and_check_config(dir.path());
        match result {
            Err(WorkspaceError::UnsupportedSchemaVersion(v)) => {
                assert_eq!(v, "99");
                let msg = format!("{}", WorkspaceError::UnsupportedSchemaVersion(v));
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
