// src/cli/apply.rs — anchor apply command — batch plan executor with pre-flight (AN-017)
//
// Core invariant: no operation leaves a dangling reference.
// Pre-flight validates ALL Move ops before any op executes.
// Per-op lock — same as file mv. Already-committed moves are NOT rolled back on failure.

use crate::core::{scanner, transaction};
use crate::infra::{lock, temp, workspace};
use crate::model::{
    manifest::Manifest,
    plan::{self, Op},
};
use std::io::Write;
use std::path::Path;

/// Execute `anchor apply <plan.toml>`.
///
/// Discovers workspace root, then delegates to `run_impl`.
/// Returns exit code: 0 = success, 1 = plan/preflight/op error, 2 = workspace/infra error.
pub fn run(plan_path: &str) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_impl(plan_path, &workspace_root, &mut std::io::stdout())
}

/// Core implementation — takes explicit workspace root and writer for testability.
///
/// Read: parse plan, scan workspace, pre-flight all Move ops, then execute sequentially.
/// On op failure: print stopped message and return 1 — do NOT roll back already-committed moves.
pub(crate) fn run_impl<W: Write>(plan_path: &str, workspace_root: &Path, out: &mut W) -> i32 {
    // Parse plan file
    let path = Path::new(plan_path);
    let plan = match plan::load_plan(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    // Scan workspace once — shared across all ops
    let workspace_files = match scanner::scan_workspace(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // Pre-flight: validate all Move ops before any execution begins.
    // A bad op at index N must not leave the first N-1 committed.
    if let Err(e) = preflight(&plan, workspace_root, &workspace_files) {
        eprintln!("{e}");
        return 1;
    }

    let total = plan.ops.len();
    let mut completed = 0usize;

    for op in &plan.ops {
        match op {
            Op::CreateDir { path: dir_path } => {
                // create_dir_all is idempotent — already-exists is not an error.
                let abs = workspace_root.join(dir_path);
                if let Err(e) = std::fs::create_dir_all(&abs) {
                    eprintln!("error creating {dir_path}/: {e}");
                    writeln!(
                        out,
                        "Stopped after {completed}/{total} operations completed."
                    )
                    .ok();
                    return 1;
                }
                completed += 1;
                writeln!(out, "[{completed}/{total}] created {dir_path}/").ok();
            }
            Op::Move { src, dst } => {
                match execute_move(workspace_root, src, dst, &workspace_files) {
                    Ok((refs_rewritten, files_touched)) => {
                        completed += 1;
                        writeln!(
                            out,
                            "[{completed}/{total}] moved {src} \u{2192} {dst}  ({refs_rewritten} refs in {files_touched} files)"
                        )
                        .ok();
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        writeln!(
                            out,
                            "Stopped after {completed}/{total} operations completed."
                        )
                        .ok();
                        return 1;
                    }
                }
            }
        }
    }

    writeln!(out, "Done. {total}/{total} operations completed.").ok();
    0
}

/// Pre-flight: validate all Move ops before any op executes.
///
/// For each Move op:
/// - src must exist on disk
/// - dst must not exist on disk
///
/// Returns Err with human-readable message on first failure.
/// Missing src error includes `similar: {suggestion}` when a close match exists.
fn preflight(
    plan: &plan::Plan,
    workspace_root: &Path,
    workspace_files: &[String],
) -> Result<(), String> {
    for op in &plan.ops {
        let Op::Move { src, dst } = op else {
            continue;
        };

        let src_abs = workspace_root.join(src);
        if !src_abs.exists() {
            let suggestions = suggest_similar(src, workspace_files);
            let mut msg = format!("preflight failed: src not found: {src}");
            if let Some(top) = suggestions.first() {
                msg.push_str(&format!("\n  similar: {top}"));
            }
            return Err(msg);
        }

        let dst_abs = workspace_root.join(dst);
        if dst_abs.exists() {
            return Err(format!("preflight failed: dst already exists: {dst}"));
        }
    }
    Ok(())
}

/// Execute a single Move operation via full PLAN → APPLY → VALIDATE → COMMIT transaction.
///
/// Returns (refs_rewritten, files_touched) on success.
///
/// IMPORTANT: Does NOT call `cli::file::mv::run` — that function uses `process::exit`
/// internally, which would terminate the entire apply loop. Transaction functions are
/// called directly here, following the same orchestration pattern as mv.rs.
fn execute_move(
    workspace_root: &Path,
    src: &str,
    dst: &str,
    workspace_files: &[String],
) -> Result<(usize, usize), String> {
    // Acquire per-op lock
    let lock_op = format!("apply: move {src} -> {dst}");
    let lock_guard =
        lock::acquire_lock(workspace_root, &lock_op).map_err(|e| format!("lock error: {e}"))?;

    // PLAN — CanonicalPath is String; convert &str to String
    let src_canonical = src.to_string();
    let dst_canonical = dst.to_string();
    let rewrite_plan = match transaction::plan(
        workspace_root,
        &src_canonical,
        &dst_canonical,
        workspace_files,
    ) {
        Ok(p) => p,
        Err(e) => {
            drop(lock_guard);
            return Err(format!("plan error: {e}"));
        }
    };

    let refs_rewritten = rewrite_plan.entries.len();
    let files_touched = {
        let files: std::collections::HashSet<&str> = rewrite_plan
            .entries
            .iter()
            .map(|e| e.file.as_str())
            .collect();
        files.len()
    };

    // Verify workspace is initialized
    let anchor_dir = workspace_root.join(".accelmars").join("anchor");
    if !anchor_dir.exists() {
        drop(lock_guard);
        return Err("workspace not initialized. Run 'anchor init' first.".to_string());
    }

    // Create temp op dir
    let op_dir = match temp::create_op_dir(workspace_root) {
        Ok(d) => d,
        Err(e) => {
            drop(lock_guard);
            return Err(format!("temp dir error: {e}"));
        }
    };

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
        src: src.to_string(),
        dst: dst.to_string(),
        rewrites: rewrite_file_list,
        phase: "PLAN".to_string(),
    };

    if let Err(e) = crate::model::manifest::write_manifest(&op_dir.path, &manifest) {
        transaction::rollback(&op_dir, lock_guard);
        return Err(format!("manifest error: {e}"));
    }

    // APPLY
    if let Err(e) = transaction::apply(workspace_root, &rewrite_plan, &op_dir, &mut manifest) {
        transaction::rollback(&op_dir, lock_guard);
        return Err(format!("apply error: {e}"));
    }

    // VALIDATE
    match transaction::validate(workspace_root, &rewrite_plan, &op_dir) {
        Ok(()) => {}
        Err(transaction::ValidationError::BrokenRefs(broken)) => {
            let msg = format!(
                "BROKEN REFERENCES AFTER REWRITE ({}): rolled back.",
                broken.len()
            );
            transaction::rollback(&op_dir, lock_guard);
            return Err(msg);
        }
        Err(transaction::ValidationError::Io(e)) => {
            transaction::rollback(&op_dir, lock_guard);
            return Err(format!("validate error: {e}"));
        }
    }

    // COMMIT — lock_guard consumed here (released via Drop)
    transaction::commit(
        workspace_root,
        &rewrite_plan,
        &op_dir,
        &mut manifest,
        lock_guard,
    )
    .map_err(|e| format!("commit error: {e}"))?;

    Ok((refs_rewritten, files_touched))
}

/// Return the top matching path from `candidates` for the given `missing` path.
///
/// Uses Levenshtein edit distance on basename (last `/`-separated component).
/// Returns at most 1 result; returns empty vec if no candidate is within 0.6
/// normalized distance.
///
/// [GAP]: AN-024 will implement `crate::core::suggest::suggest_similar` as a shared
/// utility. When AN-024 executes, replace this private function with the shared import
/// and remove `levenshtein` and `basename` below.
fn suggest_similar(missing: &str, candidates: &[String]) -> Vec<String> {
    let missing_base = basename(missing);
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let c_base = basename(c);
            let max_len = missing_base.len().max(c_base.len());
            if max_len == 0 {
                return None;
            }
            let dist = levenshtein(missing_base, c_base);
            let normalized = dist as f64 / max_len as f64;
            if normalized <= 0.6 {
                Some((dist, c))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by_key(|(dist, _)| *dist);
    scored.into_iter().take(1).map(|(_, c)| c.clone()).collect()
}

/// Extract the last `/`-separated component of a path string.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Compute the Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, val) in dp[0].iter_mut().enumerate() {
        *val = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        tmp
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    fn plan_file(ws: &TempDir, content: &str) -> String {
        let path = ws.path().join("test.toml");
        fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    // ── Exit criterion 1: Pre-flight detects missing src ─────────────────────

    /// Pre-flight stops before any op executes — workspace unchanged.
    #[test]
    fn test_preflight_missing_src_workspace_unchanged() {
        let ws = make_workspace();
        write_file(ws.path(), "foundations/guide.md", "# Guide\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "foundtion/guide.md"
dst = "foundations/moved.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_ne!(code, 0, "missing src must return non-zero exit code");

        // Workspace must be unchanged: dst not created, original file still exists
        assert!(
            !ws.path().join("foundations/moved.md").exists(),
            "dst must not be created when preflight fails"
        );
        assert!(
            ws.path().join("foundations/guide.md").exists(),
            "original file must still exist — workspace unchanged"
        );
    }

    /// Pre-flight error includes "similar: {path}" when a close match exists.
    #[test]
    fn test_preflight_missing_src_includes_similar() {
        let ws = make_workspace();
        write_file(ws.path(), "foundations/guide.md", "# Guide\n");

        let plan_loaded = plan::load_plan(std::path::Path::new(&{
            let p = ws.path().join("test.toml");
            fs::write(
                &p,
                r#"version = "1"
[[ops]]
type = "move"
src = "foundtion/guide.md"
dst = "foundations/moved.md"
"#,
            )
            .unwrap();
            p.to_string_lossy().into_owned()
        }))
        .unwrap();

        let workspace_files = scanner::scan_workspace(ws.path()).unwrap();
        let result = preflight(&plan_loaded, ws.path(), &workspace_files);

        assert!(result.is_err(), "preflight must fail for missing src");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("similar: foundations/guide.md"),
            "error must include similar suggestion; got:\n{msg}"
        );
    }

    // ── Exit criterion 2: Pre-flight detects dst already exists ──────────────

    /// Pre-flight stops when dst already exists before any op executes.
    #[test]
    fn test_preflight_dst_exists_stops_execution() {
        let ws = make_workspace();
        write_file(ws.path(), "src/a.md", "# A\n");
        write_file(ws.path(), "src/b.md", "# B — already exists\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "src/a.md"
dst = "src/b.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_ne!(code, 0, "dst-exists must return non-zero exit code");

        // src must still exist — no ops executed
        assert!(
            ws.path().join("src/a.md").exists(),
            "src must still exist when preflight fails"
        );
    }

    // ── Exit criterion 3: CreateDir is idempotent ────────────────────────────

    /// CreateDir with an already-existing path exits 0 — idempotent.
    #[test]
    fn test_create_dir_already_exists_exits_0() {
        let ws = make_workspace();
        fs::create_dir_all(ws.path().join("existing-dir")).unwrap();

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "existing-dir"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_eq!(
            code, 0,
            "CreateDir with existing path must exit 0 (idempotent)"
        );
    }

    // ── Exit criterion 4: Stopped after M/N on failure ───────────────────────

    /// Failed Move op after a successful op prints "Stopped after M/N operations completed."
    ///
    /// Setup: two ops with the same src. Pre-flight passes (src exists at pre-flight time).
    /// Op 1 moves src → dst1; src is now gone. Op 2 tries src → dst2; APPLY fails (src gone).
    /// Already-committed Op 1 is NOT rolled back.
    #[test]
    fn test_failed_move_prints_stopped_message() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# A\n");

        // Both ops reference the same src. Pre-flight sees a.md for both.
        // After op 1 executes, a.md is gone; op 2 fails in APPLY.
        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"

[[ops]]
type = "move"
src = "a.md"
dst = "c.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_ne!(code, 0, "second op must fail — non-zero exit code");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("Stopped after 1/2 operations completed."),
            "must print stopped message after 1 completed op; got:\n{output}"
        );

        // Op 1 committed: b.md exists, a.md gone
        assert!(
            ws.path().join("b.md").exists(),
            "first op must have committed — b.md must exist"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "src moved by first op — a.md must be gone"
        );
        // Op 2 did not complete: c.md must not exist
        assert!(
            !ws.path().join("c.md").exists(),
            "second op must not have committed — c.md must not exist"
        );
    }

    // ── Exit criterion 5: Successful plan prints "Done. N/N operations completed." ──

    /// Successful plan prints "Done. N/N operations completed." and exits 0.
    #[test]
    fn test_successful_plan_prints_done() {
        let ws = make_workspace();
        write_file(ws.path(), "docs/source.md", "# Source\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/source.md"
dst = "docs/destination.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_eq!(code, 0, "successful plan must exit 0");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("Done. 1/1 operations completed."),
            "success message must be printed; got:\n{output}"
        );
    }

    // ── Exit criterion 6: Move progress line includes src, dst, ref count, file count ──

    /// Each Move op progress line includes [N/total] prefix, src, dst, ref count, file count.
    #[test]
    fn test_move_progress_line_format() {
        let ws = make_workspace();
        write_file(ws.path(), "src/target.md", "# Target\n");
        // referrer.md links to target.md — produces 1 ref in 1 file
        write_file(ws.path(), "src/referrer.md", "See [target](target.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "src/target.md"
dst = "src/renamed.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_eq!(code, 0, "must succeed");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("[1/1]"),
            "progress line must contain [1/1]; got:\n{output}"
        );
        assert!(
            output.contains("src/target.md"),
            "progress line must contain src; got:\n{output}"
        );
        assert!(
            output.contains("src/renamed.md"),
            "progress line must contain dst; got:\n{output}"
        );
        assert!(
            output.contains("(1 refs in 1 files)"),
            "progress line must contain ref count and file count; got:\n{output}"
        );
    }

    // ── Exit criterion 7: Reference integrity maintained after apply ──────────

    /// After successful apply, moved file reachable from referrer — references rewritten.
    #[test]
    fn test_reference_integrity_after_apply() {
        let ws = make_workspace();
        write_file(ws.path(), "projects/source.md", "# Source\n");
        // referrer.md uses a relative link to source.md
        write_file(
            ws.path(),
            "projects/referrer.md",
            "See [source](source.md)\n",
        );

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "projects/source.md"
dst = "projects/renamed.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_eq!(code, 0, "must succeed");

        // src must be gone; dst must exist
        assert!(
            !ws.path().join("projects/source.md").exists(),
            "src must have been moved"
        );
        assert!(
            ws.path().join("projects/renamed.md").exists(),
            "dst must exist after apply"
        );

        // referrer.md must have been rewritten to point to renamed.md
        let referrer_content = fs::read_to_string(ws.path().join("projects/referrer.md")).unwrap();
        assert!(
            referrer_content.contains("renamed.md"),
            "referrer must point to new dst path; got:\n{referrer_content}"
        );
        assert!(
            !referrer_content.contains("source.md"),
            "old reference must be gone from referrer; got:\n{referrer_content}"
        );
    }
}
