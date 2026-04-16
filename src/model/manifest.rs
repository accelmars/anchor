// src/model/manifest.rs — Manifest struct serialized to .mind/tmp/.../manifest.json (MF-005)
#![allow(dead_code)]
//
// The manifest records the current operation and its phase. The phase field is updated
// at each phase transition (PLAN → APPLY → VALIDATE → COMMIT), providing the diagnostic
// signal needed to understand stale state if a process dies mid-operation.

use crate::infra::atomic::atomic_write;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Operation manifest. Written to `.mind/tmp/op-{id}/manifest.json`.
///
/// Phase transitions (from 04-TRANSACTIONS.md §manifest.json):
///   PLAN → APPLY → VALIDATE → COMMIT
///
/// If the process dies, `phase` shows exactly where it stopped. This is the
/// first thing to read when diagnosing stale state in `.mind/tmp/`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    /// Operation type. Always `"file_mv"` for Phase 1.
    pub op: String,
    /// Canonical source path (workspace-root-relative).
    pub src: String,
    /// Canonical destination path (workspace-root-relative).
    pub dst: String,
    /// Canonical paths of all files whose references will be rewritten.
    pub rewrites: Vec<String>,
    /// Current phase: `"PLAN"` | `"APPLY"` | `"VALIDATE"` | `"COMMIT"`.
    pub phase: String,
}

/// Error returned by manifest read/write operations.
#[derive(Debug)]
pub enum ManifestError {
    Io(std::io::Error),
    Json(serde_json::Error),
    AtomicWrite(std::io::Error),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(e) => write!(f, "manifest I/O error: {e}"),
            ManifestError::Json(e) => write!(f, "manifest JSON error: {e}"),
            ManifestError::AtomicWrite(e) => write!(f, "manifest atomic write error: {e}"),
        }
    }
}

impl From<std::io::Error> for ManifestError {
    fn from(e: std::io::Error) -> Self {
        ManifestError::Io(e)
    }
}

impl From<serde_json::Error> for ManifestError {
    fn from(e: serde_json::Error) -> Self {
        ManifestError::Json(e)
    }
}

/// Write `manifest` to `{op_dir}/manifest.json` atomically.
///
/// Uses `infra::atomic::atomic_write` — write-to-tmp then rename — so a partial write
/// never leaves a corrupted manifest.json. The manifest is always either fully written
/// or absent.
pub fn write_manifest(op_dir: &Path, manifest: &Manifest) -> Result<(), ManifestError> {
    let path = op_dir.join("manifest.json");
    let content = serde_json::to_string_pretty(manifest)?;
    atomic_write(&path, &content).map_err(|e| ManifestError::AtomicWrite(e.0))?;
    Ok(())
}

/// Read and deserialize `{op_dir}/manifest.json`.
pub fn read_manifest(op_dir: &Path) -> Result<Manifest, ManifestError> {
    let path = op_dir.join("manifest.json");
    let content = std::fs::read_to_string(&path)?;
    let manifest: Manifest = serde_json::from_str(&content)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Test 9 (contract): Write manifest with phase="PLAN", read back, verify phase equals "PLAN".
    #[test]
    fn test_manifest_roundtrip_phase() {
        let tmp = TempDir::new().unwrap();
        let op_dir = tmp.path();

        let manifest = Manifest {
            op: "file_mv".to_string(),
            src: "projects/foo".to_string(),
            dst: "projects/archive/foo".to_string(),
            rewrites: vec![
                "my-workspace/CLAUDE.md".to_string(),
                "projects/team/STATUS.md".to_string(),
            ],
            phase: "PLAN".to_string(),
        };

        write_manifest(op_dir, &manifest).unwrap();

        let read_back = read_manifest(op_dir).unwrap();
        assert_eq!(
            read_back.phase, "PLAN",
            "phase must survive serialize/deserialize roundtrip"
        );
        assert_eq!(read_back.op, "file_mv");
        assert_eq!(read_back.src, "projects/foo");
        assert_eq!(read_back.dst, "projects/archive/foo");
        assert_eq!(read_back.rewrites.len(), 2);
    }

    /// Write with phase="APPLY", read back, verify phase equals "APPLY".
    #[test]
    fn test_manifest_phase_apply() {
        let tmp = TempDir::new().unwrap();
        let op_dir = tmp.path();

        let mut manifest = Manifest {
            op: "file_mv".to_string(),
            src: "src".to_string(),
            dst: "dst".to_string(),
            rewrites: vec![],
            phase: "PLAN".to_string(),
        };

        write_manifest(op_dir, &manifest).unwrap();

        // Simulate phase transition: PLAN → APPLY
        manifest.phase = "APPLY".to_string();
        write_manifest(op_dir, &manifest).unwrap();

        let read_back = read_manifest(op_dir).unwrap();
        assert_eq!(read_back.phase, "APPLY");
    }

    /// Write atomically: no .tmp file left behind after successful write.
    #[test]
    fn test_manifest_no_tmp_leftover() {
        let tmp = TempDir::new().unwrap();
        let op_dir = tmp.path();

        let manifest = Manifest {
            op: "file_mv".to_string(),
            src: "a".to_string(),
            dst: "b".to_string(),
            rewrites: vec![],
            phase: "PLAN".to_string(),
        };

        write_manifest(op_dir, &manifest).unwrap();

        let tmp_path = op_dir.join("manifest.json.tmp");
        assert!(
            !tmp_path.exists(),
            ".tmp file must not be left behind after atomic write"
        );
    }
}
