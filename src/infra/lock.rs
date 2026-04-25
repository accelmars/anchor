// src/infra/lock.rs — .accelmars/anchor/lock create/release/check (MF-005)
#![allow(dead_code)]
//
// PHASE-2-BRIDGE Contract 3: silently ignore unrecognized .accelmars/anchor/ files.
// knowledge.db (Phase 2) must not be deleted or errored on.
// Any file in .accelmars/anchor/ not in {config.json, lock, tmp/} is silently skipped.

use crate::infra::atomic::{atomic_write, AtomicWriteError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Contents of `.accelmars/anchor/lock`. Serialized as JSON.
#[derive(Debug, Serialize, Deserialize)]
pub struct LockFile {
    pub pid: u32,
    pub started: String, // ISO 8601 UTC, e.g. "2026-04-15T10:23:00Z"
    pub op: String,
}

/// RAII guard that releases `.accelmars/anchor/lock` on drop.
///
/// Created by `acquire_lock`. Dropping this guard deletes the lock file,
/// which releases the workspace lock. This fires on both clean exit and panic unwind.
#[derive(Debug)]
pub struct LockGuard {
    lock_path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Best-effort: ignore errors — we are already cleaning up.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Error returned by `acquire_lock`.
#[derive(Debug)]
pub enum LockError {
    /// Another mind process is already running with the given PID.
    AlreadyRunning { pid: u32 },
    /// Stale `.accelmars/anchor/tmp/` found — a previous operation did not complete cleanly.
    StaleLock { message: String },
    /// I/O error during lock file operations.
    Io(std::io::Error),
    /// JSON deserialization error while reading an existing lock file.
    Json(serde_json::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::AlreadyRunning { pid } => {
                write!(
                    f,
                    "anchor is already running (pid {pid}). Wait for it to finish."
                )
            }
            LockError::StaleLock { message } => write!(f, "{message}"),
            LockError::Io(e) => write!(f, "I/O error: {e}"),
            LockError::Json(e) => write!(f, "JSON error: {e}"),
        }
    }
}

impl From<std::io::Error> for LockError {
    fn from(e: std::io::Error) -> Self {
        LockError::Io(e)
    }
}

impl From<serde_json::Error> for LockError {
    fn from(e: serde_json::Error) -> Self {
        LockError::Json(e)
    }
}

impl From<AtomicWriteError> for LockError {
    fn from(e: AtomicWriteError) -> Self {
        LockError::Io(e.0)
    }
}

/// Check if a process with the given PID is currently alive.
///
/// Uses `kill(pid, 0)` via the nix crate — sends signal 0 (which does NOT kill the process)
/// but returns whether the process exists and we have permission to signal it.
/// Returns `true` if alive, `false` if dead (ESRCH — no such process).
///
/// Anti-pattern from HANDOVER.md: signal None (0) ≠ SIGKILL. This check only tests existence.
fn is_pid_alive(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

/// Read `.accelmars/anchor/tmp/` and return the id suffix of the first op directory found.
/// Returns `"?"` if the directory is empty, doesn't exist, or has no `op-*` entries.
///
/// PHASE-2-BRIDGE Contract 3: silently skip entries not matching the `op-*` pattern.
fn read_op_id(workspace_root: &Path) -> String {
    let tmp_dir = workspace_root.join(".accelmars").join("anchor").join("tmp");
    let Ok(entries) = std::fs::read_dir(&tmp_dir) else {
        return "?".to_string();
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // PHASE-2-BRIDGE Contract 3: silently ignore entries not matching op-* pattern
        if let Some(id) = name.strip_prefix("op-") {
            return id.to_string();
        }
    }
    "?".to_string()
}

/// Build the verbatim stale error message from 260425-anchor-workspace-layout.md §Stale.
///
/// Exact format required:
/// ```text
/// Found incomplete operation in .accelmars/anchor/tmp/.
/// This usually means anchor was killed mid-commit.
///
/// Inspect: .accelmars/anchor/tmp/op-{id}/manifest.json
/// When safe, delete .accelmars/anchor/tmp/ and retry.
/// ```
fn stale_message(workspace_root: &Path) -> String {
    let id = read_op_id(workspace_root);
    format!(
        "Found incomplete operation in .accelmars/anchor/tmp/.\n\
         This usually means anchor was killed mid-commit.\n\
         \n\
         Inspect: .accelmars/anchor/tmp/op-{id}/manifest.json\n\
         When safe, delete .accelmars/anchor/tmp/ and retry."
    )
}

/// Return the current UTC time as an ISO 8601 string (e.g. `"2026-04-15T10:23:00Z"`).
fn current_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix_secs_to_iso8601(secs)
}

/// Convert Unix epoch seconds to ISO 8601 UTC string.
///
/// Uses Howard Hinnant's civil_from_days algorithm for Gregorian calendar conversion.
pub fn unix_secs_to_iso8601(secs: u64) -> String {
    let sec = (secs % 60) as u32;
    let min = ((secs / 60) % 60) as u32;
    let hour = ((secs / 3600) % 24) as u32;
    let days = (secs / 86400) as i64;

    // Shift epoch from 1970-03-01 to 0000-03-01 for the civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month period [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let mth = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let yr = if mth <= 2 { y + 1 } else { y };

    format!("{yr:04}-{mth:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Acquire the workspace lock.
///
/// Algorithm from 260425-anchor-workspace-layout.md §Lock check on operation start:
/// 1. If `.accelmars/anchor/lock` exists:
///    a. Deserialize and check PID liveness.
///    b. PID alive → `LockError::AlreadyRunning`
///    c. PID dead → `LockError::StaleLock`
/// 2. If `.accelmars/anchor/lock` does NOT exist but `.accelmars/anchor/tmp/` exists → `LockError::StaleLock`
/// 3. Neither exists → create `.accelmars/anchor/lock` atomically, return `LockGuard`.
///
/// The returned `LockGuard` releases the lock when dropped.
pub fn acquire_lock(workspace_root: &Path, op: &str) -> Result<LockGuard, LockError> {
    let anchor_dir = workspace_root.join(".accelmars").join("anchor");
    let lock_path = anchor_dir.join("lock");
    let tmp_dir = anchor_dir.join("tmp");

    if lock_path.exists() {
        // Lock file exists — determine liveness of the owning process.
        let content = std::fs::read_to_string(&lock_path)?;
        let lock_file: LockFile = serde_json::from_str(&content)?;

        if is_pid_alive(lock_file.pid) {
            return Err(LockError::AlreadyRunning { pid: lock_file.pid });
        } else {
            return Err(LockError::StaleLock {
                message: stale_message(workspace_root),
            });
        }
    }

    // No lock file — check for orphan .accelmars/anchor/tmp/ (stale state without lock).
    if tmp_dir.exists() {
        return Err(LockError::StaleLock {
            message: stale_message(workspace_root),
        });
    }

    // Safe to acquire: write lock file atomically.
    let lock_file = LockFile {
        pid: std::process::id(),
        started: current_iso8601(),
        op: op.to_string(),
    };
    let content = serde_json::to_string(&lock_file)?;
    atomic_write(&lock_path, &content)?;

    Ok(LockGuard { lock_path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_anchor_dir(root: &Path) -> PathBuf {
        let anchor = root.join(".accelmars").join("anchor");
        fs::create_dir_all(&anchor).unwrap();
        anchor
    }

    /// Test 1: Create lock — `.accelmars/anchor/lock` created with correct pid, started (ISO timestamp), op fields.
    #[test]
    fn test_create_lock_fields() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_anchor_dir(root);

        let guard = acquire_lock(root, "file mv src dst").unwrap();

        let lock_path = root.join(".accelmars").join("anchor").join("lock");
        assert!(
            lock_path.exists(),
            ".accelmars/anchor/lock must exist after acquire"
        );

        let content = fs::read_to_string(&lock_path).unwrap();
        let lock: LockFile = serde_json::from_str(&content).unwrap();

        assert_eq!(lock.pid, std::process::id(), "pid must be current process");
        assert_eq!(lock.op, "file mv src dst");
        assert!(
            lock.started.contains('T'),
            "started must be ISO 8601 with T separator, got: {}",
            lock.started
        );
        assert!(
            lock.started.ends_with('Z'),
            "started must end with Z (UTC), got: {}",
            lock.started
        );

        drop(guard);
    }

    /// Test 2: Acquire when lock exists + PID alive → AlreadyRunning with exact message.
    #[test]
    fn test_acquire_already_running() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let anchor_dir = make_anchor_dir(root);

        // Write a lock file with the current process's PID (always alive during the test)
        let pid = std::process::id();
        let lock = LockFile {
            pid,
            started: "2026-04-15T10:00:00Z".to_string(),
            op: "test".to_string(),
        };
        fs::write(
            anchor_dir.join("lock"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();

        let err = acquire_lock(root, "test").unwrap_err();
        match err {
            LockError::AlreadyRunning { pid: err_pid } => {
                assert_eq!(err_pid, pid);
                let msg = format!("{err}");
                assert_eq!(
                    msg,
                    format!("anchor is already running (pid {pid}). Wait for it to finish.")
                );
            }
            other => panic!("expected AlreadyRunning, got: {other:?}"),
        }
    }

    /// Test 3: Acquire when lock exists + PID dead → StaleLock with verbatim message.
    ///
    /// Uses PID 999_999_999 which exceeds any OS PID limit (macOS: 99999, Linux: 4194304)
    /// and is therefore guaranteed to return ESRCH (no such process).
    #[test]
    fn test_acquire_stale_dead_pid() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let anchor_dir = make_anchor_dir(root);

        // Create .accelmars/anchor/tmp/op-99999/ so the stale message has a known id
        fs::create_dir_all(anchor_dir.join("tmp").join("op-99999")).unwrap();

        // Write a lock file with a definitely-dead PID
        let dead_pid: u32 = 999_999_999;
        let lock = LockFile {
            pid: dead_pid,
            started: "2026-04-15T10:00:00Z".to_string(),
            op: "test".to_string(),
        };
        fs::write(
            anchor_dir.join("lock"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();

        let err = acquire_lock(root, "test").unwrap_err();
        match err {
            LockError::StaleLock { message } => {
                // Verify verbatim stale message format
                assert!(
                    message.contains("Found incomplete operation in .accelmars/anchor/tmp/."),
                    "missing first line, got:\n{message}"
                );
                assert!(
                    message.contains("This usually means anchor was killed mid-commit."),
                    "missing second line, got:\n{message}"
                );
                assert!(
                    message.contains("Inspect: .accelmars/anchor/tmp/op-99999/manifest.json"),
                    "missing inspect line with correct op id, got:\n{message}"
                );
                assert!(
                    message.contains("When safe, delete .accelmars/anchor/tmp/ and retry."),
                    "missing final line, got:\n{message}"
                );
            }
            other => panic!("expected StaleLock, got: {other:?}"),
        }
    }

    /// Test 4: `.accelmars/anchor/tmp/` exists without `.accelmars/anchor/lock` → StaleLock.
    #[test]
    fn test_acquire_tmp_without_lock() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let anchor_dir = make_anchor_dir(root);

        // Create .accelmars/anchor/tmp/op-12345/ but NO lock file
        fs::create_dir_all(anchor_dir.join("tmp").join("op-12345")).unwrap();

        let err = acquire_lock(root, "test").unwrap_err();
        match err {
            LockError::StaleLock { message } => {
                assert!(
                    message.contains("Found incomplete operation in .accelmars/anchor/tmp/."),
                    "stale message must contain first line, got:\n{message}"
                );
                assert!(
                    message.contains("Inspect: .accelmars/anchor/tmp/op-12345/manifest.json"),
                    "stale message must contain op id, got:\n{message}"
                );
            }
            other => panic!("expected StaleLock, got: {other:?}"),
        }
    }

    /// Test 5: LockGuard drop releases lock — `.accelmars/anchor/lock` must be absent after drop.
    #[test]
    fn test_lockguard_drop_releases_lock() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_anchor_dir(root);

        let lock_path = root.join(".accelmars").join("anchor").join("lock");

        {
            let _guard = acquire_lock(root, "test").unwrap();
            assert!(lock_path.exists(), "lock must exist while guard is held");
        } // guard dropped here

        assert!(
            !lock_path.exists(),
            ".accelmars/anchor/lock must be deleted after LockGuard is dropped"
        );
    }

    /// Verify unix_secs_to_iso8601 produces the correct result for a known epoch.
    ///
    /// 2026-04-15T00:00:00Z:
    ///   Days from epoch = 20558
    ///   = 1_776_211_200 seconds (verified by hand using civil_from_days)
    #[test]
    fn test_unix_secs_to_iso8601_known_date() {
        assert_eq!(unix_secs_to_iso8601(1_776_211_200), "2026-04-15T00:00:00Z");
    }

    /// Cross-check: 2025-04-15T00:00:00Z = 1_744_675_200 seconds.
    #[test]
    fn test_unix_secs_to_iso8601_2025() {
        assert_eq!(unix_secs_to_iso8601(1_744_675_200), "2025-04-15T00:00:00Z");
    }
}
