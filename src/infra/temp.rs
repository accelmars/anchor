// src/infra/temp.rs — .mind/tmp/ creation and cleanup (MF-005)
#![allow(dead_code)]
//
// Temp directory structure per 04-TRANSACTIONS.md §Temp Directory Structure:
//   .mind/tmp/
//     op-{timestamp-ms}/
//       manifest.json
//       rewrites/
//       moved/

use crate::model::CanonicalPath;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A created operation temp directory.
pub struct TempOpDir {
    pub path: PathBuf,
}

/// Error returned by temp directory operations.
#[derive(Debug)]
pub enum TempError {
    Io(std::io::Error),
}

impl std::fmt::Display for TempError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TempError::Io(e) => write!(f, "temp directory error: {e}"),
        }
    }
}

impl From<std::io::Error> for TempError {
    fn from(e: std::io::Error) -> Self {
        TempError::Io(e)
    }
}

/// Create an operation temp directory at `.mind/tmp/op-{timestamp_ms}/`.
///
/// Creates:
/// - `.mind/tmp/op-{timestamp_ms}/`
/// - `.mind/tmp/op-{timestamp_ms}/rewrites/`
/// - `.mind/tmp/op-{timestamp_ms}/moved/`
///
/// Returns the created `TempOpDir`. The caller is responsible for cleanup via
/// `cleanup_op_dir` (during rollback or after commit).
///
/// Note: `.mind/tmp/` must reside on the same filesystem as the workspace so
/// that `rename(2)` is atomic during COMMIT. Do not relocate it.
pub fn create_op_dir(workspace_root: &Path) -> Result<TempOpDir, TempError> {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let op_dir_name = format!("op-{timestamp_ms}");
    let op_dir = workspace_root.join(".mind").join("tmp").join(&op_dir_name);

    std::fs::create_dir_all(op_dir.join("rewrites"))?;
    std::fs::create_dir_all(op_dir.join("moved"))?;

    Ok(TempOpDir { path: op_dir })
}

/// Encode a canonical path for use as a filename in `rewrites/`.
///
/// Replaces `/` with `__` per 04-TRANSACTIONS.md §Encoded path.
///
/// Example:
///   `"projects/os-council/STATUS.md"` → `"projects__os-council__STATUS.md"`
pub fn encode_path(canonical: &CanonicalPath) -> String {
    canonical.replace('/', "__")
}

/// Remove the operation temp directory and all its contents.
///
/// Called during ROLLBACK (any phase before COMMIT) and at the end of COMMIT.
/// On ROLLBACK, no original file was modified — the workspace remains byte-for-byte
/// identical to before the operation started.
pub fn cleanup_op_dir(op_dir: &TempOpDir) -> Result<(), TempError> {
    std::fs::remove_dir_all(&op_dir.path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Verify create_op_dir creates `.mind/tmp/op-{timestamp}/` with `rewrites/` and `moved/` subdirs.
    #[test]
    fn test_create_op_dir_structure() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".mind")).unwrap();

        let op = create_op_dir(root).unwrap();

        // op-{timestamp_ms}/ must be inside .mind/tmp/
        let tmp_dir = root.join(".mind").join("tmp");
        assert!(tmp_dir.exists(), ".mind/tmp/ must exist");
        assert!(op.path.exists(), "op dir must exist");

        // op dir name must start with "op-"
        let dir_name = op.path.file_name().unwrap().to_string_lossy();
        assert!(
            dir_name.starts_with("op-"),
            "op dir must be named op-{{timestamp}}, got: {dir_name}"
        );

        // rewrites/ and moved/ subdirs must exist
        assert!(op.path.join("rewrites").is_dir(), "rewrites/ must exist");
        assert!(op.path.join("moved").is_dir(), "moved/ must exist");
    }

    /// Verify encode_path replaces / with __.
    #[test]
    fn test_encode_path() {
        assert_eq!(
            encode_path(&"projects/os-council/STATUS.md".to_string()),
            "projects__os-council__STATUS.md"
        );
        assert_eq!(
            encode_path(&"accelmars-guild/CLAUDE.md".to_string()),
            "accelmars-guild__CLAUDE.md"
        );
        // Root-level file: no slashes, no change
        assert_eq!(encode_path(&"README.md".to_string()), "README.md");
    }

    /// Verify cleanup_op_dir removes the directory tree.
    #[test]
    fn test_cleanup_op_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".mind")).unwrap();

        let op = create_op_dir(root).unwrap();
        let op_path = op.path.clone();

        assert!(op_path.exists(), "op dir must exist before cleanup");
        cleanup_op_dir(&op).unwrap();
        assert!(!op_path.exists(), "op dir must not exist after cleanup");
    }
}
