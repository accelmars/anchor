// src/cli/file/mv.rs — anchor file mv
//
// Atomicity is filesystem-level — the final rename() step is atomic on
// same-filesystem moves. Cross-filesystem moves (different mount points) are
// not atomic. Cross-filesystem atomicity is a Phase 2 concern.

use crate::cli::file::refs::OutputFormat;
use crate::core::{scanner, transaction};
use crate::infra::{lock, temp, workspace};
use crate::model::manifest::Manifest;
use std::io::{self, Write};
use std::process;

/// Errors specific to the `anchor file mv` command.
#[derive(Debug)]
pub enum MvError {
    SrcNotFound,
    DstExists,
    ConflictingFlags(String),
    Lock(lock::LockError),
    Workspace(workspace::WorkspaceError),
    Scanner(scanner::ScannerError),
    Transaction(transaction::TransactionError),
    Validation(transaction::ValidationError),
    Commit(transaction::CommitError),
    Temp(temp::TempError),
}

impl std::fmt::Display for MvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MvError::SrcNotFound => write!(f, "src not found"),
            MvError::DstExists => write!(f, "dst already exists"),
            MvError::ConflictingFlags(msg) => write!(f, "{msg}"),
            MvError::Lock(e) => write!(f, "lock error: {e}"),
            MvError::Workspace(e) => write!(f, "workspace error: {e}"),
            MvError::Scanner(e) => write!(f, "scanner error: {e}"),
            MvError::Transaction(e) => write!(f, "transaction error: {e}"),
            MvError::Validation(e) => write!(f, "validation error: {e}"),
            MvError::Commit(e) => write!(f, "commit error: {e}"),
            MvError::Temp(e) => write!(f, "temp error: {e}"),
        }
    }
}

impl From<lock::LockError> for MvError {
    fn from(e: lock::LockError) -> Self {
        MvError::Lock(e)
    }
}
impl From<workspace::WorkspaceError> for MvError {
    fn from(e: workspace::WorkspaceError) -> Self {
        MvError::Workspace(e)
    }
}
impl From<scanner::ScannerError> for MvError {
    fn from(e: scanner::ScannerError) -> Self {
        MvError::Scanner(e)
    }
}
impl From<transaction::TransactionError> for MvError {
    fn from(e: transaction::TransactionError) -> Self {
        MvError::Transaction(e)
    }
}
impl From<transaction::CommitError> for MvError {
    fn from(e: transaction::CommitError) -> Self {
        MvError::Commit(e)
    }
}
impl From<temp::TempError> for MvError {
    fn from(e: temp::TempError) -> Self {
        MvError::Temp(e)
    }
}

/// Execute `anchor file mv <src> <dst>`.
///
/// Default (no flags): silent on success (exit 0, no output).
/// `--verbose`: prints "Moved. Rewrote N references in M files." on success.
/// `--format json`: prints JSON result on success.
/// `--verbose` and `--format` are mutually exclusive.
pub fn run(
    src: &str,
    dst: &str,
    verbose: bool,
    format: Option<OutputFormat>,
) -> Result<(), MvError> {
    // Flag mutual exclusion check at entry point — before any mutations
    if verbose && format.is_some() {
        return Err(MvError::ConflictingFlags(
            "--verbose and --format are mutually exclusive".to_string(),
        ));
    }

    // ── Workspace discovery ──────────────────────────────────────────────────
    let workspace_root = workspace::find_workspace_root()?;

    // ── Resolve src and dst relative to workspace root ───────────────────────
    let src_canonical = normalize_path(&workspace_root, src);
    let dst_canonical = normalize_path(&workspace_root, dst);

    // ── Pre-flight: hard errors before PLAN (03-COMMANDS.md §Rules) ──────────
    let src_abs = workspace_root.join(src_canonical.as_str());
    if !src_abs.exists() {
        return Err(MvError::SrcNotFound);
    }

    let dst_abs = workspace_root.join(dst_canonical.as_str());
    if dst_abs.exists() {
        eprintln!("dst already exists: {dst}");
        process::exit(1);
    }

    // ── Acquire lock ─────────────────────────────────────────────────────────
    let lock_op = format!("file mv {src_canonical} {dst_canonical}");
    let lock_guard = lock::acquire_lock(&workspace_root, &lock_op)?;

    // ── Scan workspace ────────────────────────────────────────────────────────
    let workspace_files = scanner::scan_workspace(&workspace_root)?;

    // ── PLAN ──────────────────────────────────────────────────────────────────
    let rewrite_plan = transaction::plan(
        &workspace_root,
        &src_canonical,
        &dst_canonical,
        &workspace_files,
    )?;

    // Counts needed for verbose/JSON output — available from PLAN phase
    let ref_count = rewrite_plan.entries.len();
    let files_count = {
        let files_updated: std::collections::HashSet<&str> = rewrite_plan
            .entries
            .iter()
            .map(|e| e.file.as_str())
            .collect();
        files_updated.len()
    };

    // ── Create temp op dir + manifest ─────────────────────────────────────────
    let anchor_dir = workspace_root.join(".accelmars").join("anchor");
    if !anchor_dir.exists() {
        eprintln!("error: workspace not initialized. Run 'anchor init' first.");
        drop(lock_guard);
        process::exit(2);
    }

    let op_dir = temp::create_op_dir(&workspace_root)?;

    let rewrite_file_list: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        rewrite_plan
            .entries
            .iter()
            .filter(|e| seen.insert(e.file.clone()))
            .map(|e| e.file.clone())
            .collect()
    };

    let mut manifest = Manifest {
        op: "file_mv".to_string(),
        src: src_canonical.clone(),
        dst: dst_canonical.clone(),
        rewrites: rewrite_file_list,
        phase: "PLAN".to_string(),
    };

    crate::model::manifest::write_manifest(&op_dir.path, &manifest)
        .map_err(|e| MvError::Transaction(transaction::TransactionError::Manifest(e)))?;

    // ── APPLY ─────────────────────────────────────────────────────────────────
    if let Err(e) = transaction::apply(&workspace_root, &rewrite_plan, &op_dir, &mut manifest) {
        transaction::rollback(&op_dir, lock_guard);
        eprintln!("error during apply: {e}");
        process::exit(2);
    }

    // ── VALIDATE ──────────────────────────────────────────────────────────────
    match transaction::validate(&workspace_root, &rewrite_plan, &op_dir) {
        Ok(()) => {}
        Err(transaction::ValidationError::BrokenRefs(broken)) => {
            eprintln!();
            eprintln!("BROKEN REFERENCES AFTER REWRITE ({}):", broken.len());
            for b in &broken {
                eprintln!("  {}:{} → {}  (not found)", b.file, b.line, b.target);
            }
            eprintln!();
            eprintln!("Rolled back. No changes applied.");
            transaction::rollback(&op_dir, lock_guard);
            process::exit(1);
        }
        Err(transaction::ValidationError::Io(e)) => {
            transaction::rollback(&op_dir, lock_guard);
            eprintln!("error during validate: {e}");
            process::exit(2);
        }
    }

    // ── COMMIT ────────────────────────────────────────────────────────────────
    if let Err(e) = transaction::commit(
        &workspace_root,
        &rewrite_plan,
        &op_dir,
        &mut manifest,
        lock_guard,
    ) {
        eprintln!("error during commit: {e}");
        process::exit(2);
    }

    // ── Output ────────────────────────────────────────────────────────────────
    if verbose {
        write_verbose_output(
            &mut io::stdout(),
            &src_canonical,
            &dst_canonical,
            ref_count,
            files_count,
        )
        .ok();
    } else if format == Some(OutputFormat::Json) {
        write_json_output(
            &mut io::stdout(),
            &src_canonical,
            &dst_canonical,
            ref_count,
            files_count,
        )
        .ok();
    }
    // Default (no flags): silent on success

    Ok(())
}

/// Write the human-readable verbose success summary.
fn write_verbose_output<W: Write>(
    w: &mut W,
    _src: &str,
    _dst: &str,
    refs_rewritten: usize,
    files_touched: usize,
) -> io::Result<()> {
    writeln!(
        w,
        "Moved. Rewrote {refs_rewritten} references in {files_touched} files."
    )
}

/// Write the JSON success output.
///
/// PHASE 2 STABLE CONTRACT: This JSON schema is a stable interface for AI agents and
/// machine consumers. Do not change field names without a design session.
/// Schema: {"moved":true,"refs_rewritten":N,"files_touched":M,"src":"...","dst":"..."}
fn write_json_output<W: Write>(
    w: &mut W,
    src: &str,
    dst: &str,
    refs_rewritten: usize,
    files_touched: usize,
) -> io::Result<()> {
    let output = serde_json::json!({
        "moved": true,
        "refs_rewritten": refs_rewritten,
        "files_touched": files_touched,
        "src": src,
        "dst": dst,
    });
    writeln!(w, "{output}")
}

/// Normalize a user-provided path to a workspace-root-relative canonical path.
///
/// If `path` is absolute, strips the workspace_root prefix.
/// If relative, returns as-is (already workspace-root-relative by convention).
fn normalize_path(workspace_root: &std::path::Path, path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.strip_prefix(workspace_root)
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string())
    } else {
        let normalized = path.replace('\\', "/");
        normalized
            .strip_prefix("./")
            .unwrap_or(&normalized)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--verbose` output contains the confirmation summary with correct counts.
    #[test]
    fn test_mv_verbose_emits_confirmation() {
        let mut out = Vec::new();
        write_verbose_output(&mut out, "projects/foo", "projects/archive/foo", 12, 5).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("Moved. Rewrote 12 references in 5 files."),
            "verbose output must contain summary, got: {s}"
        );
    }

    /// `--format json` output is valid JSON with all required fields and correct values.
    #[test]
    fn test_mv_format_json_success() {
        let mut out = Vec::new();
        write_json_output(&mut out, "projects/foo", "projects/archive/foo", 12, 5).unwrap();
        let s = String::from_utf8(out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(parsed["moved"], true);
        assert_eq!(parsed["refs_rewritten"], 12);
        assert_eq!(parsed["files_touched"], 5);
        assert_eq!(parsed["src"], "projects/foo");
        assert_eq!(parsed["dst"], "projects/archive/foo");
    }

    /// `--verbose` and `--format json` together return an error before any filesystem operations.
    #[test]
    fn test_mv_verbose_and_json_errors() {
        let result = run("anything", "anywhere", true, Some(OutputFormat::Json));
        assert!(
            matches!(result, Err(MvError::ConflictingFlags(_))),
            "both flags must return ConflictingFlags error"
        );
    }

    /// When src does not exist, run() returns Err(SrcNotFound) — not process::exit.
    ///
    /// This is the key contract change from AN-025: the inline exit was replaced with
    /// a typed Err return so the caller (main.rs) can show "Did you mean?" suggestions.
    /// If run() called process::exit, the test process would terminate and this assertion
    /// would never be reached.
    #[test]
    fn test_src_not_found_returns_err_not_exit() {
        use tempfile::tempdir;
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".accelmars").join("anchor")).unwrap();
        std::fs::write(
            root.path().join(".accelmars").join("anchor").join("config.json"),
            r#"{"schema_version":"1"}"#,
        )
        .unwrap();
        // Set cwd to the tempdir workspace so find_workspace_root() resolves here.
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(root.path()).unwrap();

        let result = run(
            "this-file-does-not-exist-9f3k2j.md",
            "some-dst.md",
            false,
            None,
        );

        std::env::set_current_dir(&original_dir).unwrap();

        assert!(
            matches!(result, Err(MvError::SrcNotFound)),
            "expected SrcNotFound for nonexistent src, got: {:?}",
            result
        );
    }
}
