// src/cli/file/mv.rs — mind file mv CLI entry point (MF-006)

use crate::core::{scanner, transaction};
use crate::infra::{lock, temp, workspace};
use crate::model::manifest::Manifest;
use std::process;

/// Errors specific to the `mind file mv` command.
#[derive(Debug)]
pub enum MvError {
    SrcNotFound,
    DstExists,
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

/// Execute `mind file mv <src> <dst>`.
///
/// Returns Ok(()) on success. On failure: prints to stderr and exits with the
/// appropriate exit code (1 = logical error, 2 = system error).
///
/// This function does NOT call process::exit directly — the caller (main.rs) handles the
/// exit code. Errors are printed to stderr here for user-facing messages.
pub fn run(src: &str, dst: &str) -> Result<(), MvError> {
    // ── Workspace discovery ──────────────────────────────────────────────────
    let workspace_root = workspace::find_workspace_root()?;

    // ── Resolve src and dst relative to workspace root ───────────────────────
    // Both paths are treated as workspace-root-relative (or absolute).
    let src_canonical = normalize_path(&workspace_root, src);
    let dst_canonical = normalize_path(&workspace_root, dst);

    // ── Pre-flight: hard errors before PLAN (03-COMMANDS.md §Rules) ──────────
    let src_abs = workspace_root.join(src_canonical.as_str());
    if !src_abs.exists() {
        eprintln!("src not found: {src}");
        process::exit(1);
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
    let file_count = workspace_files.len();

    print_progress(&format!("Scanning workspace... {file_count} files"));

    // ── PLAN ──────────────────────────────────────────────────────────────────
    let rewrite_plan = transaction::plan(
        &workspace_root,
        &src_canonical,
        &dst_canonical,
        &workspace_files,
    )?;
    let ref_count = rewrite_plan.entries.len();

    // Count distinct files being rewritten
    let files_updated: std::collections::HashSet<&str> = rewrite_plan
        .entries
        .iter()
        .map(|e| e.file.as_str())
        .collect();
    let files_count = files_updated.len();

    print_progress(&format!(
        "Planning rewrites... {ref_count} references to update"
    ));

    // Print REWRITE lines (up to 4, then "... and N more")
    let all_rewrite_lines: Vec<String> = rewrite_plan
        .entries
        .iter()
        .map(|e| {
            let line = byte_offset_to_line(
                &std::fs::read_to_string(workspace_root.join(e.file.as_str())).unwrap_or_default(),
                e.span.0,
            );
            format!("  REWRITE  {}:{}", e.file, line)
        })
        .collect::<std::collections::HashSet<_>>() // deduplicate by file:line
        .into_iter()
        .collect();

    let mut sorted_lines = all_rewrite_lines;
    sorted_lines.sort();

    if sorted_lines.len() <= 4 {
        for line in &sorted_lines {
            println!("{line}");
        }
    } else {
        for line in sorted_lines.iter().take(4) {
            println!("{line}");
        }
        let more = sorted_lines.len() - 4;
        println!("  ... and {more} more");
    }

    // ── Create temp op dir + manifest ─────────────────────────────────────────
    // Ensure .mind directory exists (created by `mind init`)
    let mind_dir = workspace_root.join(".mind");
    if !mind_dir.exists() {
        eprintln!("error: workspace not initialized. Run 'mind init' first.");
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
    print_progress("Validating...");

    match transaction::validate(&workspace_root, &rewrite_plan, &op_dir) {
        Ok(()) => {
            println!(" ✓");
        }
        Err(transaction::ValidationError::BrokenRefs(broken)) => {
            println!();
            println!();
            println!("BROKEN REFERENCES AFTER REWRITE ({}):", broken.len());
            for b in &broken {
                println!("  {}:{} → {}  (not found)", b.file, b.line, b.target);
            }
            println!();
            println!("Rolled back. No changes applied.");
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
    print_progress("Committing...");

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

    println!("  ✓");
    println!();
    println!("Done. Moved: {src_canonical} → {dst_canonical}");
    println!("{ref_count} references updated across {files_count} files.");

    Ok(())
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
        // Normalize forward slashes and strip leading ./
        let normalized = path.replace('\\', "/");
        normalized
            .strip_prefix("./")
            .unwrap_or(&normalized)
            .to_string()
    }
}

/// Print progress text without a trailing newline, flushing stdout immediately.
fn print_progress(msg: &str) {
    use std::io::Write;
    print!("{msg}");
    let _ = std::io::stdout().flush();
}

/// Convert a byte offset in `content` to a 1-based line number.
fn byte_offset_to_line(content: &str, byte_offset: usize) -> usize {
    content[..byte_offset.min(content.len())]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}
