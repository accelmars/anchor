// src/cli/recover.rs — anchor recover: inspect stale tmp dirs, roll back or warn.
//
// Manifest phases (from model/manifest.rs §Phase transitions):
//   PLAN     — initial state, apply not started; originals untouched → safe rollback
//   VALIDATE — apply complete, rewritten files in tmp/rewrites/; originals untouched → safe rollback
//   COMMIT   — rename phase in progress; some originals may be moved → manual required
//
// If manifest.json is absent or unreadable (crash before write), treats as PLAN → safe rollback.

use crate::infra::workspace;
use crate::model::manifest;
use std::path::Path;

pub fn run() -> i32 {
    match workspace::find_workspace_root() {
        Ok(root) => recover_workspace(&root),
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn recover_workspace(workspace_root: &Path) -> i32 {
    let tmp_dir = workspace_root.join(".accelmars").join("anchor").join("tmp");

    if !tmp_dir.exists() {
        println!("No stale operations found.");
        return 0;
    }

    let entries = match std::fs::read_dir(&tmp_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error reading {}: {e}", tmp_dir.display());
            return 1;
        }
    };

    // Collect op-* directories. Skip non-matching entries per PHASE-2-BRIDGE Contract 3.
    let mut op_dirs: Vec<std::fs::DirEntry> = entries
        .flatten()
        .filter(|e| {
            // Rule 12: use file_type().is_dir() — do NOT use path.is_dir() (follows symlinks).
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && e.file_name().to_string_lossy().starts_with("op-")
        })
        .collect();

    if op_dirs.is_empty() {
        println!("No stale operations found.");
        return 0;
    }

    op_dirs.sort_by_key(|e| e.file_name());

    let mut needs_manual = false;

    for entry in &op_dirs {
        let op_dir = entry.path();
        let op_id = entry.file_name().to_string_lossy().into_owned();

        match resolve_op(&op_dir) {
            OpOutcome::RolledBack => println!("Rolled back: {op_id}"),
            OpOutcome::NeedsManual { phase } => {
                eprintln!(
                    "Warning: {op_id} is in phase {phase} — some renames may have been applied."
                );
                eprintln!("  Manual resolution required:");
                eprintln!("  1. Inspect: .accelmars/anchor/tmp/{op_id}/manifest.json");
                eprintln!("  2. Determine which renames completed by comparing workspace files.");
                eprintln!("  3. Once resolved, delete: .accelmars/anchor/tmp/{op_id}");
                needs_manual = true;
            }
            OpOutcome::Failed { reason } => {
                eprintln!("error processing {op_id}: {reason}");
                needs_manual = true;
            }
        }
    }

    // Release lock file if it belongs to a dead process.
    let lock_path = workspace_root
        .join(".accelmars")
        .join("anchor")
        .join("lock");
    if lock_path.exists() {
        maybe_release_stale_lock(&lock_path);
    }

    if needs_manual {
        1
    } else {
        0
    }
}

enum OpOutcome {
    RolledBack,
    NeedsManual { phase: String },
    Failed { reason: String },
}

fn resolve_op(op_dir: &Path) -> OpOutcome {
    let phase = match manifest::read_manifest(op_dir) {
        Ok(m) => m.phase,
        Err(_) => "PLAN".to_string(),
    };

    match phase.as_str() {
        "PLAN" | "VALIDATE" => match std::fs::remove_dir_all(op_dir) {
            Ok(()) => OpOutcome::RolledBack,
            Err(e) => OpOutcome::Failed {
                reason: format!("could not delete {}: {e}", op_dir.display()),
            },
        },
        _ => OpOutcome::NeedsManual { phase },
    }
}

fn maybe_release_stale_lock(lock_path: &Path) {
    let content = match std::fs::read_to_string(lock_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let pid: u32 = match serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|v| v.get("pid")?.as_u64())
    {
        Some(p) => p as u32,
        None => return,
    };
    if !is_pid_alive(pid) {
        let _ = std::fs::remove_file(lock_path);
        println!("Released stale lock (pid {pid} is no longer running).");
    }
}

// Uses kill(pid, 0) — signal 0 does not kill the process; returns whether it exists.
fn is_pid_alive(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace(tmp: &TempDir) -> std::path::PathBuf {
        let root = tmp.path().to_path_buf();
        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        root
    }

    fn make_op_dir(root: &Path, op_id: &str) -> std::path::PathBuf {
        let op_dir = root
            .join(".accelmars")
            .join("anchor")
            .join("tmp")
            .join(op_id);
        fs::create_dir_all(op_dir.join("rewrites")).unwrap();
        fs::create_dir_all(op_dir.join("moved")).unwrap();
        op_dir
    }

    fn write_phase(op_dir: &Path, phase: &str) {
        let m = manifest::Manifest {
            op: "file_mv".to_string(),
            src: "a.md".to_string(),
            dst: "b.md".to_string(),
            rewrites: vec![],
            phase: phase.to_string(),
        };
        manifest::write_manifest(op_dir, &m).unwrap();
    }

    /// Empty tmp dir → exit 0 ("No stale operations found.").
    #[test]
    fn test_recover_no_stale_ops() {
        let tmp = TempDir::new().unwrap();
        let root = make_workspace(&tmp);
        fs::create_dir_all(root.join(".accelmars").join("anchor").join("tmp")).unwrap();

        let exit_code = recover_workspace(&root);
        assert_eq!(exit_code, 0, "empty tmp must return exit 0");
    }

    /// Op in VALIDATE phase (pre-commit) → rolled back, op dir deleted, exit 0.
    #[test]
    fn test_recover_rolls_back_pre_commit_op() {
        let tmp = TempDir::new().unwrap();
        let root = make_workspace(&tmp);
        let op_dir = make_op_dir(&root, "op-111");
        write_phase(&op_dir, "VALIDATE");

        let exit_code = recover_workspace(&root);
        assert_eq!(exit_code, 0, "pre-commit op must return exit 0");
        assert!(!op_dir.exists(), "op dir must be deleted after rollback");
    }

    /// Op in COMMIT phase → warning emitted, op dir NOT deleted, exit 1.
    #[test]
    fn test_recover_warns_partial_commit() {
        let tmp = TempDir::new().unwrap();
        let root = make_workspace(&tmp);
        let op_dir = make_op_dir(&root, "op-222");
        write_phase(&op_dir, "COMMIT");

        let exit_code = recover_workspace(&root);
        assert_eq!(exit_code, 1, "partial commit must return exit 1");
        assert!(
            op_dir.exists(),
            "op dir must NOT be deleted for COMMIT phase"
        );
    }
}
