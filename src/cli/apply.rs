// src/cli/apply.rs — anchor apply command — batch plan executor with pre-flight (AN-017)
//
// Core invariant: no operation leaves a dangling reference.
// Pre-flight validates ALL Move ops before any op executes.
// Per-op lock — same as file mv. Already-committed moves are NOT rolled back on failure.

use crate::apply::post_apply_scan::{format_plain_text_warning, scan_partial_plain_text};
use crate::core::{
    acked::{parse_ref_line, AckedRefs},
    scanner, transaction,
};
use crate::infra::{lock, temp, workspace};
use crate::model::{
    manifest::Manifest,
    plan::{self, Op},
};
use std::io::Write;
use std::path::Path;

/// Execute `anchor apply <plan.toml>`.
///
/// Discovers workspace root, builds acked set from disk + flags, delegates to `run_impl`.
/// Returns exit code: 0 = success, 1 = plan/preflight/op error, 2 = workspace/infra error.
pub fn run(plan_path: &str, allow_broken: &[String], allow_broken_from: Option<&str>) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // Load acked refs from disk, then extend with explicitly specified refs.
    let mut acked = AckedRefs::load(&workspace_root);
    let mut newly_specified: Vec<(String, usize)> = Vec::new();

    for s in allow_broken {
        match parse_ref_line(s) {
            Some((f, l)) => {
                acked.add(&f, l);
                newly_specified.push((f, l));
            }
            None => {
                eprintln!("warning: invalid --allow-broken value: {s} (expected file:line)");
            }
        }
    }

    // --allow-broken-from resolves via std::fs::read_to_string — CWD-relative by default.
    if let Some(from_path) = allow_broken_from {
        match std::fs::read_to_string(from_path) {
            Ok(content) => {
                for line in content.lines() {
                    if let Some((f, l)) = parse_ref_line(line) {
                        acked.add(&f, l);
                        newly_specified.push((f, l));
                    }
                }
            }
            Err(e) => {
                eprintln!("error reading --allow-broken-from {from_path}: {e}");
                return 1;
            }
        }
    }

    let exit_code = run_impl(plan_path, &workspace_root, &mut std::io::stdout(), &acked);

    // Persist newly specified refs only on success.
    if exit_code == 0 && !newly_specified.is_empty() {
        AckedRefs::save(&workspace_root, &newly_specified);
    }

    exit_code
}

/// Core implementation — takes explicit workspace root and writer for testability.
///
/// Read: parse plan, scan workspace, pre-flight all Move ops, then execute sequentially.
/// On op failure: print stopped message and return 1 — do NOT roll back already-committed moves.
pub(crate) fn run_impl<W: Write>(
    plan_path: &str,
    workspace_root: &Path,
    out: &mut W,
    acked: &AckedRefs,
) -> i32 {
    // Parse plan file
    let path = Path::new(plan_path);
    let plan = match plan::load_plan(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    // Canonical plan file path — used to exclude the plan file itself from non-.md rewriting.
    let plan_file_abs = std::fs::canonicalize(path).ok();

    // Scan workspace for pre-flight — validate all Move ops before any execution begins.
    let preflight_files = match scanner::scan_workspace(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // Pre-flight: validate all Move ops before any execution begins.
    // A bad op at index N must not leave the first N-1 committed.
    if let Err(e) = preflight(&plan, workspace_root, &preflight_files) {
        if is_already_applied(&plan, workspace_root) {
            eprintln!("note: all sources are missing and destinations already exist — this plan may have already been applied. Nothing was changed.");
        } else {
            eprintln!("{e}");
        }
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
                match execute_move(workspace_root, src, dst, plan_file_abs.as_deref(), acked) {
                    Ok((refs_rewritten, files_touched, acked_warnings)) => {
                        completed += 1;
                        for w in &acked_warnings {
                            writeln!(out, "{w}").ok();
                        }
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

/// Returns true iff all Move ops in the plan have src absent and dst present on disk.
///
/// Used to detect re-apply: when the plan was already successfully applied, every src
/// will have been moved to dst. A non-Move op (CreateDir) is ignored.
fn is_already_applied(plan: &plan::Plan, workspace_root: &Path) -> bool {
    let move_ops: Vec<(&str, &str)> = plan
        .ops
        .iter()
        .filter_map(|op| {
            if let Op::Move { src, dst } = op {
                Some((src.as_str(), dst.as_str()))
            } else {
                None
            }
        })
        .collect();
    !move_ops.is_empty()
        && move_ops.iter().all(|(src, dst)| {
            !workspace_root.join(src).exists() && workspace_root.join(dst).exists()
        })
}

/// Execute a single Move operation via full PLAN → APPLY → VALIDATE → COMMIT transaction.
///
/// Returns `(refs_rewritten, files_touched, acked_warnings)` on success.
/// `acked_warnings` contains `⚠  Allowing…` lines for broken refs suppressed by the acked set.
///
/// IMPORTANT: Does NOT call `cli::file::mv::run` — that function uses `process::exit`
/// internally, which would terminate the entire apply loop. Transaction functions are
/// called directly here, following the same orchestration pattern as mv.rs.
fn execute_move(
    workspace_root: &Path,
    src: &str,
    dst: &str,
    plan_file_abs: Option<&std::path::Path>,
    acked: &AckedRefs,
) -> Result<(usize, usize, Vec<String>), String> {
    // Acquire per-op lock
    let lock_op = format!("apply: move {src} -> {dst}");
    let lock_guard =
        lock::acquire_lock(workspace_root, &lock_op).map_err(|e| format!("lock error: {e}"))?;

    // Scan workspace fresh for this op — captures files moved by prior ops.
    let workspace_files =
        scanner::scan_workspace(workspace_root).map_err(|e| format!("scan error: {e}"))?;

    // PLAN — CanonicalPath is String; convert &str to String
    let src_canonical = src.to_string();
    let dst_canonical = dst.to_string();
    let rewrite_plan = match transaction::plan(
        workspace_root,
        &src_canonical,
        &dst_canonical,
        &workspace_files,
        false, // apply always uses prose heuristic (AENG-010)
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

    // VALIDATE — filter broken refs against acked set.
    let acked_warnings: Vec<String> =
        match transaction::validate(workspace_root, &rewrite_plan, &op_dir) {
            Ok(()) => vec![],
            Err(transaction::ValidationError::BrokenRefs(broken)) => {
                let (acked_refs, unacked_refs): (Vec<_>, Vec<_>) = broken
                    .into_iter()
                    .partition(|b| acked.contains(&b.file, b.line));

                if !unacked_refs.is_empty() {
                    let capped = &workspace_files[..200.min(workspace_files.len())];
                    eprintln!("BROKEN REFERENCES AFTER REWRITE ({}):", unacked_refs.len());
                    eprintln!();
                    for b in &unacked_refs {
                        eprint!(
                            "{}",
                            crate::core::diagnostics::format_broken_ref(
                                &b.file, b.line, &b.target, capped,
                            )
                        );
                    }
                    transaction::rollback(&op_dir, lock_guard);
                    return Err("rolled back.".to_string());
                }

                // All broken refs are acked — collect warnings and proceed to COMMIT.
                acked_refs
                    .iter()
                    .map(|b| {
                        format!(
                            "⚠  Allowing known broken ref: {}:{}  (acked)",
                            b.file, b.line
                        )
                    })
                    .collect()
            }
            Err(transaction::ValidationError::Io(e)) => {
                transaction::rollback(&op_dir, lock_guard);
                return Err(format!("validate error: {e}"));
            }
        };

    // COMMIT — lock_guard consumed here (released via Drop)
    transaction::commit(
        workspace_root,
        &rewrite_plan,
        &op_dir,
        &mut manifest,
        lock_guard,
    )
    .map_err(|e| format!("commit error: {e}"))?;

    // Post-commit: rewrite non-.md files containing text occurrences of old path.
    let non_md_updated = rewrite_non_md_occurrences(workspace_root, src, dst, plan_file_abs);
    if non_md_updated > 0 {
        eprintln!("{non_md_updated} non-markdown file(s) updated.");
    }

    // Post-commit: UX-001 — emit full-path and partial-path plain-text occurrence warning.
    // Runs after every move (not just zero-ref moves) so the operator always sees the residual.
    let workspace_md: Vec<String> = workspace_files
        .iter()
        .filter(|f| f.ends_with(".md"))
        .cloned()
        .collect();

    // Full-path: files containing the full src string as plain text, sorted by file.
    let mut full_path_lines: Vec<(String, usize)> = workspace_md
        .iter()
        .filter_map(|f| {
            let content = std::fs::read_to_string(workspace_root.join(f.as_str())).ok()?;
            let count = content.matches(src).count();
            if count > 0 {
                Some((f.clone(), count))
            } else {
                None
            }
        })
        .collect();
    full_path_lines.sort_by(|a, b| a.0.cmp(&b.0));

    // Partial-path: trailing segment occurrences via post_apply_scan.
    let partial_hits = scan_partial_plain_text(&workspace_md, src, workspace_root);

    // Trailing segment for the closing hint line.
    let trailing = src.rsplit('/').next().unwrap_or(src);

    if let Some(warning) = format_plain_text_warning(&full_path_lines, &partial_hits, trailing) {
        eprintln!("{warning}");
    }

    Ok((refs_rewritten, files_touched, acked_warnings))
}

/// Walk `workspace_root` and count text occurrences of `needle` in non-.md files.
///
/// Scans files with extensions: json, yaml, yml, toml (excluding Cargo.toml), ts, js, py.
/// Returns the total count of substring matches across all matching files.
/// Kept for test use — production path now calls rewrite_non_md_occurrences.
#[cfg(test)]
fn count_text_occurrences(workspace_root: &Path, needle: &str) -> usize {
    let extensions = ["json", "yaml", "yml", "toml", "ts", "js", "py"];
    let mut total = 0usize;
    count_in_dir(workspace_root, needle, &extensions, &mut total);
    total
}

/// Walk `workspace_root` and count plain-text occurrences of `needle` in .md files.
///
/// Uses scanner::scan_workspace to enumerate files, then filters for .md.
/// Returns the total count of substring matches across all .md files.
pub(crate) fn count_plaintext_md_occurrences(workspace_root: &Path, needle: &str) -> usize {
    let files = match scanner::scan_workspace(workspace_root) {
        Ok(f) => f,
        Err(_) => return 0,
    };
    files
        .iter()
        .filter(|f| f.ends_with(".md"))
        .filter_map(|f| std::fs::read_to_string(workspace_root.join(f)).ok())
        .map(|content| content.matches(needle).count())
        .sum()
}

#[cfg(test)]
fn count_in_dir(dir: &Path, needle: &str, extensions: &[&str], total: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();

        // Skip .accelmars/ system directory
        if path.components().any(|c| c.as_os_str() == ".accelmars") {
            continue;
        }

        if path.is_dir() {
            count_in_dir(&path, needle, extensions, total);
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !extensions.contains(&ext) {
                continue;
            }
            // Exclude Cargo.toml — Rust build manifest
            if ext == "toml" && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                let mut start = 0;
                while let Some(pos) = content[start..].find(needle) {
                    *total += 1;
                    start += pos + needle.len();
                    if start >= content.len() {
                        break;
                    }
                }
            }
        }
    }
}

/// Walk `workspace_root` and replace text occurrences of `src` with `dst`
/// in non-.md files (json, yaml, yml, toml, ts, js, py; excluding Cargo.toml).
/// `plan_file_abs` is the canonical path of the active plan file; it is skipped to
/// prevent self-modification during apply.
/// Returns the number of files updated.
pub(crate) fn rewrite_non_md_occurrences(
    workspace_root: &Path,
    src: &str,
    dst: &str,
    plan_file_abs: Option<&std::path::Path>,
) -> usize {
    let extensions = ["json", "yaml", "yml", "toml", "ts", "js", "py"];
    let mut updated = 0usize;
    rewrite_in_dir(
        workspace_root,
        src,
        dst,
        &extensions,
        &mut updated,
        plan_file_abs,
    );
    updated
}

fn rewrite_in_dir(
    dir: &Path,
    src: &str,
    dst: &str,
    extensions: &[&str],
    updated: &mut usize,
    plan_file_abs: Option<&std::path::Path>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == ".accelmars") {
            continue;
        }
        // Use entry.file_type() to avoid following symlinks (Rule 12)
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            rewrite_in_dir(&path, src, dst, extensions, updated, plan_file_abs);
            continue;
        }
        // Skip the active plan file — prevent self-modification during apply.
        if let Some(plan_path) = plan_file_abs {
            if let Ok(canonical) = std::fs::canonicalize(&path) {
                if canonical == plan_path {
                    continue;
                }
            }
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !extensions.contains(&ext) {
            continue;
        }
        if ext == "toml" && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !content.contains(src) {
            continue;
        }
        let new_content = content.replace(src, dst);
        if let Err(e) = std::fs::write(&path, new_content.as_bytes()) {
            eprintln!("warning: could not rewrite {}: {e}", path.display());
        } else {
            *updated += 1;
        }
    }
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
    use crate::core::acked::AckedRefs;
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
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

    // ── Zero-ref plain-text .md warning (UX-001) ─────────────────────────────

    /// count_plaintext_md_occurrences finds matches in .md files; returns correct count.
    #[test]
    fn test_zero_ref_plaintext_warning_emitted() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "docs/notes.md",
            "See also gateway-foundation for more details.\n",
        );
        let count = count_plaintext_md_occurrences(ws.path(), "gateway-foundation");
        assert!(
            count > 0,
            "expected >0 plain-text occurrences in notes.md, got: {count}"
        );
    }

    /// When refs_rewritten > 0, the plain-text warning condition is false — move succeeds normally.
    #[test]
    fn test_zero_ref_no_warning_when_refs_found() {
        let ws = make_workspace();
        write_file(ws.path(), "src/target.md", "# Target\n");
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
        assert_eq!(code, 0, "move with refs must succeed");
        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("(1 refs in 1 files)"),
            "refs were rewritten — plaintext warning condition does not apply; got:\n{output}"
        );
    }

    /// count_plaintext_md_occurrences returns 0 when no .md files contain the needle.
    #[test]
    fn test_zero_ref_no_warning_when_no_plaintext() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "docs/clean.md",
            "# Clean document with no mentions\n",
        );
        let count = count_plaintext_md_occurrences(ws.path(), "gateway-foundation");
        assert_eq!(
            count, 0,
            "expected 0 plain-text occurrences when needle absent from all .md files"
        );
    }

    // ── Non-.md occurrence warning ─────────────────────────────────────────────

    /// count_text_occurrences finds matches in .json files; returns correct count.
    #[test]
    fn test_nonmd_warning_emitted_when_occurrences_exist() {
        let ws = make_workspace();
        // Write a .json file that mentions the old path
        write_file(
            ws.path(),
            "config.json",
            r#"{"path": "gateway-foundation/config.yaml"}"#,
        );

        let count = count_text_occurrences(ws.path(), "gateway-foundation");
        assert!(
            count > 0,
            "expected >0 occurrences in config.json, got: {count}"
        );
    }

    /// count_text_occurrences returns 0 when no non-.md files contain the needle.
    #[test]
    fn test_nonmd_no_warning_when_clean() {
        let ws = make_workspace();
        // Only .md files — no non-.md files at all
        write_file(ws.path(), "a.md", "# Hello\n");

        let count = count_text_occurrences(ws.path(), "gateway-foundation");
        assert_eq!(count, 0, "expected 0 occurrences when only .md files exist");
    }

    // ── rewrite_non_md_occurrences (AR-010 / REF-005) ────────────────────────

    /// rewrite_non_md_occurrences rewrites a .json file containing the old path and returns 1.
    #[test]
    fn test_rewrite_non_md_occurrences_updates_json() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "config.json",
            r#"{"path": "old-engine/config.yaml", "ref": "old-engine/index.md"}"#,
        );

        let updated = rewrite_non_md_occurrences(ws.path(), "old-engine", "new-engine", None);
        assert_eq!(updated, 1, "expected 1 file updated");

        let content = fs::read_to_string(ws.path().join("config.json")).unwrap();
        assert!(
            content.contains("new-engine"),
            "file must contain new path; got:\n{content}"
        );
        assert!(
            !content.contains("old-engine"),
            "old path must be gone from file; got:\n{content}"
        );
    }

    /// rewrite_non_md_occurrences returns 0 and leaves the file unchanged when no match.
    #[test]
    fn test_rewrite_non_md_occurrences_no_match_returns_zero() {
        let ws = make_workspace();
        let original = r#"{"path": "unrelated/path"}"#;
        write_file(ws.path(), "config.json", original);

        let updated = rewrite_non_md_occurrences(ws.path(), "old-engine", "new-engine", None);
        assert_eq!(updated, 0, "expected 0 files updated when no match");

        let content = fs::read_to_string(ws.path().join("config.json")).unwrap();
        assert_eq!(content, original, "file must be unchanged when no match");
    }

    // ── Intra-plan chain: per-op re-scan (REF-003 / SIM-E) ───────────────────

    /// Intra-plan chain: op N moves alpha (adjusting its refs to beta), op N+1 moves beta.
    /// With per-op re-scan, op N+1 sees the post-op-N filesystem and updates the adjusted refs.
    #[test]
    fn test_intra_plan_chain_refs_updated() {
        let ws = make_workspace();
        write_file(ws.path(), "alpha/index.md", "[beta](../beta/index.md)\n");
        write_file(ws.path(), "beta/index.md", "# Beta\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "foundations"

[[ops]]
type = "move"
src = "alpha"
dst = "foundations/alpha-engine"

[[ops]]
type = "move"
src = "beta"
dst = "foundations/beta-engine"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
        assert_eq!(
            code,
            0,
            "plan must succeed; output:\n{}",
            String::from_utf8_lossy(&out)
        );

        let content =
            fs::read_to_string(ws.path().join("foundations/alpha-engine/index.md")).unwrap();
        assert!(
            content.contains("../beta-engine/index.md"),
            "ref must point to beta-engine after intra-plan chain; got:\n{content}"
        );
        assert!(
            !content.contains("../../beta/index.md"),
            "stale intermediate ref must be gone; got:\n{content}"
        );
    }

    /// Multi-level relative ref: file moved to a deeper location, its ref to another move target
    /// is updated to the correct final path through two sequential ops.
    #[test]
    fn test_multilevel_relative_ref_updated() {
        let ws = make_workspace();
        write_file(ws.path(), "a/README.md", "[b](../b/README.md)\n");
        write_file(ws.path(), "b/README.md", "# B\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a"
dst = "deep/nested/a"

[[ops]]
type = "move"
src = "b"
dst = "other/b"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
        assert_eq!(
            code,
            0,
            "plan must succeed; output:\n{}",
            String::from_utf8_lossy(&out)
        );

        let content = fs::read_to_string(ws.path().join("deep/nested/a/README.md")).unwrap();
        assert!(
            content.contains("../../../other/b/README.md"),
            "ref must point to other/b after multi-level chain; got:\n{content}"
        );
        assert!(
            !content.contains("../b/README.md"),
            "original ref must be gone; got:\n{content}"
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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
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

    // ── Re-apply detection tests (AR-007) ─────────────────────────────────────

    /// Re-apply: all srcs absent and all dsts present → hint message on stderr, exit 1.
    #[test]
    fn test_apply_reapply_hint_emitted() {
        let ws = make_workspace();
        // dst already exists (simulates a completed apply); src is absent
        write_file(ws.path(), "docs/destination.md", "# Already moved\n");

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
        let code = run_impl(&plan_path, ws.path(), &mut out, &AckedRefs::empty());
        assert_eq!(code, 1, "re-apply must return exit 1");

        // stderr capture: use a buffer-based approach via is_already_applied helper directly
        // (stderr goes to real stderr in run_impl; test the helper logic here)
        let plan = plan::load_plan(std::path::Path::new(&plan_path)).unwrap();
        assert!(
            is_already_applied(&plan, ws.path()),
            "is_already_applied must return true when all srcs absent and dsts present"
        );
    }

    /// No re-apply hint when src is absent but dst is also absent — genuine missing src.
    #[test]
    fn test_apply_no_reapply_hint_when_src_missing_but_dst_also_absent() {
        let ws = make_workspace();
        // Neither src nor dst exist — genuine missing src scenario

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/source.md"
dst = "docs/destination.md"
"#,
        );

        let plan = plan::load_plan(std::path::Path::new(&plan_path)).unwrap();
        assert!(
            !is_already_applied(&plan, ws.path()),
            "is_already_applied must return false when dst is also absent"
        );
    }

    // ── Plan file self-modification exclusion (AR-015) ────────────────────────

    /// apply does not rewrite the plan file itself — src value must remain unchanged.
    #[test]
    fn test_apply_does_not_rewrite_plan_file() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# A\n");

        let plan_content =
            "version = \"1\"\n[[ops]]\ntype = \"move\"\nsrc = \"a.md\"\ndst = \"b.md\"\n";
        let plan_path = ws.path().join("plan.toml");
        fs::write(&plan_path, plan_content).unwrap();

        let mut out = Vec::new();
        let code = run_impl(
            plan_path.to_str().unwrap(),
            ws.path(),
            &mut out,
            &AckedRefs::empty(),
        );
        assert_eq!(code, 0, "plan must succeed");

        let plan_after = fs::read_to_string(&plan_path).unwrap();
        assert_eq!(
            plan_after, plan_content,
            "plan file must not be rewritten during apply; got:\n{plan_after}"
        );
    }

    /// apply rewrites adjacent non-.md files but not the plan file — exclusion is targeted.
    #[test]
    fn test_apply_rewrites_adjacent_toml_but_not_plan_file() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# A\n");
        write_file(ws.path(), "config.toml", "ref = \"a.md\"\n");

        let plan_content =
            "version = \"1\"\n[[ops]]\ntype = \"move\"\nsrc = \"a.md\"\ndst = \"b.md\"\n";
        let plan_path = ws.path().join("plan.toml");
        fs::write(&plan_path, plan_content).unwrap();

        let mut out = Vec::new();
        let code = run_impl(
            plan_path.to_str().unwrap(),
            ws.path(),
            &mut out,
            &AckedRefs::empty(),
        );
        assert_eq!(code, 0, "plan must succeed");

        // plan file is unchanged
        let plan_after = fs::read_to_string(&plan_path).unwrap();
        assert_eq!(plan_after, plan_content, "plan file must not be rewritten");

        // config.toml IS rewritten — exclusion targets only the plan file
        let config_after = fs::read_to_string(ws.path().join("config.toml")).unwrap();
        assert!(
            config_after.contains("b.md"),
            "config.toml must be rewritten; got:\n{config_after}"
        );
        assert!(
            !config_after.contains("a.md"),
            "old path must be gone from config.toml; got:\n{config_after}"
        );
    }

    // ── AENG-003 — --allow-broken acked-refs tests ────────────────────────────

    /// Apply with 1 broken ref + matching acked entry → apply succeeds, warning in output.
    ///
    /// a.md references nonexistent.md (broken). Moving a.md → b.md triggers Case B rewrite;
    /// validate finds b.md:1 → nonexistent.md broken. Acking b.md:1 suppresses the rollback.
    #[test]
    fn test_allow_broken_acked_suppresses_rollback() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "[broken](nonexistent.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        let mut acked = AckedRefs::empty();
        acked.add("b.md", 1);

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out, &acked);
        assert_eq!(code, 0, "acked broken ref must not cause rollback");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("⚠  Allowing known broken ref: b.md:1  (acked)"),
            "acked warning must appear in output; got:\n{output}"
        );
        assert!(
            ws.path().join("b.md").exists(),
            "b.md must exist after apply"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "a.md must be gone after apply"
        );
    }

    /// Apply with 1 broken ref but wrong file:line → rollback not suppressed.
    #[test]
    fn test_allow_broken_wrong_ref_still_rolls_back() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "[broken](nonexistent.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        let mut acked = AckedRefs::empty();
        acked.add("b.md", 999); // wrong line number — does not match b.md:1

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out, &acked);
        assert_ne!(code, 0, "wrong file:line must not suppress rollback");
        assert!(
            !ws.path().join("b.md").exists(),
            "rollback must have occurred — b.md must not exist"
        );
        assert!(
            ws.path().join("a.md").exists(),
            "rollback must restore a.md"
        );
    }

    /// Persistence: acked ref saved to disk then loaded on second run → apply succeeds without flag.
    #[test]
    fn test_allow_broken_persisted_applies_on_reload() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "[broken](nonexistent.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        // Simulate a prior --allow-broken run that persisted the acked ref to disk.
        AckedRefs::save(ws.path(), &[("b.md".to_string(), 1)]);

        // Second run: no flags — load acked refs from disk only.
        let acked = AckedRefs::load(ws.path());

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out, &acked);
        assert_eq!(code, 0, "acked ref loaded from disk must suppress rollback");
        assert!(
            ws.path().join("b.md").exists(),
            "b.md must exist after apply"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "a.md must be gone after apply"
        );
    }

    /// Partial ack: 2 broken refs, only 1 acked → rollback (partial ack does not suppress).
    #[test]
    fn test_allow_broken_partial_ack_still_rolls_back() {
        let ws = make_workspace();
        // Two broken refs on lines 1 and 2
        write_file(
            ws.path(),
            "a.md",
            "[broken1](nonexistent1.md)\n[broken2](nonexistent2.md)\n",
        );

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        // Only ack line 1 — line 2 remains unacked.
        let mut acked = AckedRefs::empty();
        acked.add("b.md", 1);

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out, &acked);
        assert_ne!(code, 0, "partial ack must not suppress rollback");
        assert!(
            !ws.path().join("b.md").exists(),
            "rollback must have occurred — b.md must not exist"
        );
        assert!(
            ws.path().join("a.md").exists(),
            "rollback must restore a.md"
        );
    }
}
