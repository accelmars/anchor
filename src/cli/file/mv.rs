// src/cli/file/mv.rs — anchor file mv
//
// Atomicity is filesystem-level — the final rename() step is atomic on
// same-filesystem moves. Cross-filesystem moves (different mount points) are
// not atomic. Cross-filesystem atomicity is a Phase 2 concern.

use crate::cli::file::refs::OutputFormat;
use crate::core::{parser, resolver, scanner, transaction};
use crate::infra::{lock, temp, workspace};
use crate::model::manifest::Manifest;
use crate::model::reference::RefForm;
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

    let workspace_root = workspace::find_workspace_root()?;
    let cwd = std::env::current_dir().ok();
    run_impl(src, dst, verbose, format, &workspace_root, cwd.as_deref())
}

pub(crate) fn run_impl(
    src: &str,
    dst: &str,
    verbose: bool,
    format: Option<OutputFormat>,
    workspace_root: &std::path::Path,
    cwd: Option<&std::path::Path>,
) -> Result<(), MvError> {
    // ── Resolve src and dst relative to workspace root ───────────────────────
    // dst: CWD-relative when called from a subdirectory (dst doesn't exist yet, so no
    // existence fallback — always resolve relative to CWD when inside the workspace)
    let dst_canonical = {
        let p = std::path::Path::new(dst);
        if p.is_absolute() {
            normalize_path(workspace_root, dst)
        } else {
            cwd.map(|c| c.join(dst))
                .filter(|abs| abs.starts_with(workspace_root))
                .and_then(|abs| {
                    abs.strip_prefix(workspace_root)
                        .ok()
                        .map(|rel| rel.to_path_buf())
                })
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| normalize_path(workspace_root, dst))
        }
    };

    // src: workspace-root-relative first; CWD-relative fallback if not found
    let src_canonical = {
        let ws_relative = normalize_path(workspace_root, src);
        if workspace_root.join(&ws_relative).exists() {
            ws_relative
        } else {
            let fallback = cwd
                .map(|c| c.join(src))
                .filter(|p| p.exists() && p.starts_with(workspace_root))
                .and_then(|p| {
                    p.strip_prefix(workspace_root)
                        .ok()
                        .map(|rel| rel.to_path_buf())
                })
                .map(|rel| rel.to_string_lossy().replace('\\', "/"));
            match fallback {
                Some(rel) => rel,
                None => return Err(MvError::SrcNotFound),
            }
        }
    };

    // ── Pre-flight: hard errors before PLAN (03-COMMANDS.md §Rules) ──────────
    let dst_abs = workspace_root.join(dst_canonical.as_str());
    if dst_abs.exists() {
        eprintln!("dst already exists: {dst}");
        process::exit(1);
    }

    // ── Acquire lock ─────────────────────────────────────────────────────────
    let lock_op = format!("file mv {src_canonical} {dst_canonical}");
    let lock_guard = lock::acquire_lock(workspace_root, &lock_op)?;

    // ── Scan workspace ────────────────────────────────────────────────────────
    let workspace_files = scanner::scan_workspace(workspace_root)?;

    // ── Pre-move source validation ────────────────────────────────────────────
    let broken_source = validate_source_refs(workspace_root, &src_canonical)?;
    if !broken_source.is_empty() {
        eprintln!("BROKEN REFERENCES IN SOURCE ({}):", broken_source.len());
        for (file, line, target) in &broken_source {
            eprintln!("  {file}:{line}");
            eprintln!("    → {target}  (not found)");
        }
        eprintln!();
        eprintln!("Fix the broken reference before moving. No changes applied.");
        drop(lock_guard);
        process::exit(1);
    }

    // ── PLAN ──────────────────────────────────────────────────────────────────
    let rewrite_plan = transaction::plan(
        workspace_root,
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

    let op_dir = temp::create_op_dir(workspace_root)?;

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
    if let Err(e) = transaction::apply(workspace_root, &rewrite_plan, &op_dir, &mut manifest) {
        transaction::rollback(&op_dir, lock_guard);
        eprintln!("error during apply: {e}");
        process::exit(2);
    }

    // ── VALIDATE ──────────────────────────────────────────────────────────────
    match transaction::validate(workspace_root, &rewrite_plan, &op_dir) {
        Ok(()) => {}
        Err(transaction::ValidationError::BrokenRefs(broken)) => {
            let capped = &workspace_files[..200.min(workspace_files.len())];
            eprintln!("BROKEN REFERENCES AFTER REWRITE ({}):", broken.len());
            eprintln!();
            for b in &broken {
                eprint!(
                    "{}",
                    crate::core::diagnostics::format_broken_ref(
                        &b.file, b.line, &b.target, capped,
                    )
                );
            }
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
        workspace_root,
        &rewrite_plan,
        &op_dir,
        &mut manifest,
        lock_guard,
    ) {
        eprintln!("error during commit: {e}");
        process::exit(2);
    }

    // ── Post-commit: non-.md file rewriting + plain-text prose warning ────────
    let non_md_updated = crate::cli::apply::rewrite_non_md_occurrences(
        workspace_root,
        &src_canonical,
        &dst_canonical,
        None, // no plan file context for `anchor file mv`
    );
    if non_md_updated > 0 {
        eprintln!("{non_md_updated} non-markdown file(s) updated.");
    }
    if ref_count == 0 {
        let plaintext_count =
            crate::cli::apply::count_plaintext_md_occurrences(workspace_root, &src_canonical);
        if plaintext_count > 0 {
            eprintln!(
                "note: 0 markdown refs rewritten. {plaintext_count} plain-text occurrence(s) of '{src_canonical}' in .md files were not rewritten."
            );
        }
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

/// Build the workspace-root hint appended to SrcNotFound error messages.
pub fn format_src_not_found_hint(src: &str, workspace_root: &std::path::Path) -> String {
    let cwd = std::env::current_dir().ok();
    let mut hint = format!(
        "  Hint: paths are resolved from workspace root ({})",
        workspace_root.display()
    );
    if let Some(cwd_path) = cwd {
        let cwd_abs = cwd_path.join(src);
        if cwd_abs.starts_with(workspace_root) {
            if let Ok(rel) = cwd_abs.strip_prefix(workspace_root) {
                hint.push_str(&format!(
                    "\n  If you meant '{}', use the path '{}'",
                    cwd_abs.display(),
                    rel.to_string_lossy()
                ));
            }
        }
    }
    hint
}

/// Pre-move gate: scan all .md files inside `src_canonical` for relative references
/// that cannot be resolved. Returns `(canonical_file_path, 1-based_line, target_raw)` for
/// each broken ref. Empty return means the source is clean and the move may proceed.
fn validate_source_refs(
    workspace_root: &std::path::Path,
    src_canonical: &str,
) -> Result<Vec<(String, usize, String)>, MvError> {
    let src_abs = workspace_root.join(src_canonical);
    let mut broken: Vec<(String, usize, String)> = Vec::new();

    let mut md_files: Vec<std::path::PathBuf> = Vec::new();
    if src_abs.is_file() {
        if src_abs.extension().and_then(|e| e.to_str()) == Some("md") {
            md_files.push(src_abs);
        }
    } else if src_abs.is_dir() {
        collect_md_files_recursive(&src_abs, &mut md_files)?;
    }

    for abs_path in &md_files {
        let canonical = abs_path
            .strip_prefix(workspace_root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        if canonical.is_empty() {
            continue;
        }

        let content = std::fs::read_to_string(abs_path)
            .map_err(|e| MvError::Transaction(transaction::TransactionError::Io(e)))?;

        let refs = parser::parse_references(&canonical, &content);

        for reference in &refs {
            let should_check = match reference.form {
                RefForm::Standard => true,
                RefForm::Backtick => {
                    reference.target_raw.starts_with("./")
                        || reference.target_raw.starts_with("../")
                }
                RefForm::HtmlHref => {
                    !reference.target_raw.is_empty()
                        && !reference.target_raw.starts_with("http://")
                        && !reference.target_raw.starts_with("https://")
                        && !reference.target_raw.starts_with("//")
                        && !reference.target_raw.starts_with('/')
                }
                _ => false,
            };

            if !should_check {
                continue;
            }

            let resolved = resolver::resolve_form1(&canonical, &reference.target_raw);
            if !workspace_root.join(&resolved).exists() {
                let line = content[..reference.span.0]
                    .chars()
                    .filter(|&c| c == '\n')
                    .count()
                    + 1;
                broken.push((canonical.clone(), line, reference.target_raw.clone()));
            }
        }
    }

    Ok(broken)
}

/// Recursively collect .md files under `dir`. Skips symlinks per Rule 12.
fn collect_md_files_recursive(
    dir: &std::path::Path,
    files: &mut Vec<std::path::PathBuf>,
) -> Result<(), MvError> {
    let read_dir = std::fs::read_dir(dir)
        .map_err(|e| MvError::Transaction(transaction::TransactionError::Io(e)))?;
    for entry in read_dir {
        let entry =
            entry.map_err(|e| MvError::Transaction(transaction::TransactionError::Io(e)))?;
        let file_type = entry
            .file_type()
            .map_err(|e| MvError::Transaction(transaction::TransactionError::Io(e)))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_md_files_recursive(&path, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            files.push(path);
        }
    }
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

    static CWD_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// CWD-relative src path resolves transparently when called from a subdirectory.
    #[test]
    fn test_file_mv_cwd_relative_path_from_subdir() {
        let _cwd_lock = CWD_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        use tempfile::tempdir;
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".accelmars").join("anchor")).unwrap();
        std::fs::write(
            root.path()
                .join(".accelmars")
                .join("anchor")
                .join("config.json"),
            r#"{"schema_version":"1"}"#,
        )
        .unwrap();
        let subdir = root.path().join("src");
        let old_dir = subdir.join("old-dir");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::write(old_dir.join("f.md"), "# F\n").unwrap();

        let result = run_impl(
            "old-dir",
            "new-dir",
            false,
            None,
            root.path(),
            Some(&subdir),
        );

        assert!(
            result.is_ok(),
            "CWD-relative src should resolve via fallback, got: {:?}",
            result
        );
        assert!(!old_dir.exists(), "src/old-dir should have been moved");
        assert!(
            subdir.join("new-dir").exists(),
            "new-dir should resolve CWD-relative to src/new-dir"
        );
    }

    /// SrcNotFound error message contains workspace root hint.
    #[test]
    fn test_file_mv_error_message_hints_workspace_root() {
        let workspace_root = std::path::Path::new("/tmp/fake-workspace");
        let hint = format_src_not_found_hint("old-dir", workspace_root);
        assert!(
            hint.contains("workspace root"),
            "error hint must mention workspace root, got: {hint}"
        );
    }

    /// When src does not exist, run_impl() returns Err(SrcNotFound) — not process::exit.
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
            root.path()
                .join(".accelmars")
                .join("anchor")
                .join("config.json"),
            r#"{"schema_version":"1"}"#,
        )
        .unwrap();

        let result = run_impl(
            "this-file-does-not-exist-9f3k2j.md",
            "some-dst.md",
            false,
            None,
            root.path(),
            Some(root.path()),
        );

        assert!(
            matches!(result, Err(MvError::SrcNotFound)),
            "expected SrcNotFound for nonexistent src, got: {:?}",
            result
        );
    }

    /// validate_source_refs returns a non-empty vec with correct (file, line, target_raw)
    /// when the source directory contains a file with a broken relative Form 1 ref.
    #[test]
    fn test_validate_source_refs_detects_broken_form1_ref() {
        use tempfile::tempdir;
        let root = tempdir().unwrap();
        let src_dir = root.path().join("src-proj");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("README.md"),
            "[broken](../../../nonexistent/target.md)\n",
        )
        .unwrap();

        let broken = validate_source_refs(root.path(), "src-proj").unwrap();

        assert_eq!(broken.len(), 1, "expected 1 broken ref; got: {broken:?}");
        let (file, line, target) = &broken[0];
        assert_eq!(file, "src-proj/README.md");
        assert_eq!(*line, 1usize);
        assert_eq!(target, "../../../nonexistent/target.md");
    }

    /// validate_source_refs returns an empty vec when all relative refs in the source
    /// directory resolve to existing files on disk.
    #[test]
    fn test_validate_source_refs_empty_when_all_refs_resolve() {
        use tempfile::tempdir;
        let root = tempdir().unwrap();
        let src_dir = root.path().join("src-proj");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(root.path().join("sibling")).unwrap();
        std::fs::write(root.path().join("sibling").join("target.md"), "# T\n").unwrap();
        std::fs::write(src_dir.join("README.md"), "[valid](../sibling/target.md)\n").unwrap();

        let broken = validate_source_refs(root.path(), "src-proj").unwrap();

        assert!(
            broken.is_empty(),
            "expected no broken refs for valid ref; got: {broken:?}"
        );
    }

    /// Gate condition test: validate_source_refs returns non-empty for a source directory
    /// matching the cortex-move scenario from the intake doc (already-broken relative ref
    /// that anchor would misreport post-rewrite). This non-empty result is what run_impl()
    /// checks before transaction::plan() — when non-empty, process::exit(1) is called.
    #[test]
    fn test_validate_source_refs_gate_triggers_for_cortex_scenario() {
        use tempfile::tempdir;
        let root = tempdir().unwrap();
        let src_dir = root
            .path()
            .join("projects")
            .join("cortex-intelligence-foundation");
        std::fs::create_dir_all(&src_dir).unwrap();
        // This is the broken ref from the intake doc: 4 levels up from projects/CIF/ only
        // reaches the workspace parent — cortex-engine is not there.
        std::fs::write(
            src_dir.join("HANDOVER.md"),
            "# Handover\n[changelog](../../../../cortex-engine/CHANGELOG.md)\n",
        )
        .unwrap();

        let broken =
            validate_source_refs(root.path(), "projects/cortex-intelligence-foundation").unwrap();

        assert!(
            !broken.is_empty(),
            "gate must trigger: validate_source_refs must detect the broken cortex-engine ref"
        );
        assert_eq!(
            broken[0].0,
            "projects/cortex-intelligence-foundation/HANDOVER.md"
        );
        assert_eq!(broken[0].1, 2usize);
        assert_eq!(broken[0].2, "../../../../cortex-engine/CHANGELOG.md");
    }

    /// AR-010 parity: non-.md rewriting is wired into the mv post-commit path.
    ///
    /// Tests rewrite_non_md_occurrences directly (pub(crate)) to avoid set_current_dir
    /// contamination between parallel tests.
    #[test]
    fn test_file_mv_rewrites_non_md_files_post_commit() {
        use tempfile::tempdir;
        let root = tempdir().unwrap();

        std::fs::write(
            root.path().join("config.json"),
            r#"{"path": "old-engine/config.yaml"}"#,
        )
        .unwrap();

        let updated = crate::cli::apply::rewrite_non_md_occurrences(
            root.path(),
            "old-engine",
            "new-engine",
            None,
        );

        assert_eq!(updated, 1, "expected 1 file updated");
        let content = std::fs::read_to_string(root.path().join("config.json")).unwrap();
        assert!(
            content.contains("new-engine"),
            "config.json must contain new-engine; got: {content}"
        );
        assert!(
            !content.contains("old-engine"),
            "old-engine must be gone from config.json; got: {content}"
        );
    }
}
