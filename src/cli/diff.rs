// src/cli/diff.rs — anchor diff command — read-only plan preview (AN-016)
//
// Core invariant: running `anchor diff` leaves the workspace byte-for-byte identical.
// No lock acquisition, no apply/validate/commit — PLAN phase only per Move op.

use crate::core::{scanner, transaction};
use crate::infra::workspace;
use crate::model::plan::{self, Op};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

/// Execute `anchor diff <plan.toml>`.
///
/// Discovers workspace root from the current working directory, then delegates
/// to `run_impl`. Returns exit code: 0 = success, 1 = plan error, 2 = workspace error.
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

/// Core implementation — takes an explicit workspace root and writer for testability.
///
/// Read-only: scans workspace once, runs PLAN phase per Move op, prints preview.
/// Does not acquire a lock; does not call apply, validate, or commit.
pub(crate) fn run_impl<W: Write>(
    plan_path: &str,
    workspace_root: &Path,
    out: &mut W,
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

    // Scan workspace once — shared across all Move ops
    let workspace_files = match scanner::scan_workspace(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // Print optional plan description
    if let Some(desc) = &plan.description {
        writeln!(out, "{desc}").ok();
        writeln!(out).ok();
    }

    let mut total_refs = 0usize;
    let mut total_files = 0usize;

    for op in &plan.ops {
        match op {
            Op::CreateDir { path } => {
                let already_exists = workspace_root.join(path).exists();
                if already_exists {
                    writeln!(out, "  create   {path}/ (already exists)").ok();
                } else {
                    writeln!(out, "  create   {path}/").ok();
                }
            }
            Op::Move { src, dst } => {
                let src_abs = workspace_root.join(src);
                if !src_abs.exists() {
                    writeln!(out, "  move     {src} \u{2192} {dst}  [ERROR: src not found]").ok();
                    let suggestions = suggest_similar(src, &workspace_files);
                    if let Some(top) = suggestions.first() {
                        writeln!(out, "           similar: {top}").ok();
                    }
                    continue;
                }

                // PLAN phase only — no apply/validate/commit
                let rewrite_plan =
                    match transaction::plan(workspace_root, src, dst, &workspace_files) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("error: {e}");
                            continue;
                        }
                    };

                let ref_count = rewrite_plan.entries.len();
                let file_count = rewrite_plan
                    .entries
                    .iter()
                    .map(|e| e.file.as_str())
                    .collect::<HashSet<_>>()
                    .len();

                total_refs += ref_count;
                total_files += file_count;

                writeln!(
                    out,
                    "  move     {src} \u{2192} {dst}  ({ref_count} refs in {file_count} files)"
                )
                .ok();
            }
        }
    }

    // Summary
    let op_count = plan.ops.len();
    writeln!(out, "{op_count} operations \u{00b7} {total_refs} refs \u{00b7} {total_files} files")
        .ok();
    writeln!(out, "Run `anchor apply {plan_path}` to execute.").ok();

    0
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
    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
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

    // ── Exit criterion 4: Missing plan file → stderr error, exit 1 ──────────

    /// Missing plan file returns exit code 1.
    #[test]
    fn test_missing_plan_file_exits_1() {
        let ws = make_workspace();
        let mut out = Vec::new();
        let code = run_impl("/nonexistent/path/plan.toml", ws.path(), &mut out);
        assert_eq!(code, 1, "missing plan file must return exit code 1");
    }

    // ── Exit criterion 1: Valid plan → exits 0, no files modified ───────────

    /// Valid plan with one Move op → exits 0 and workspace is unchanged.
    #[test]
    fn test_valid_plan_exits_0_no_modifications() {
        let ws = make_workspace();
        write_file(ws.path(), "docs/guide.md", "# Guide\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/guide.md"
dst = "docs/moved.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_eq!(code, 0, "valid plan must return exit code 0");

        // Core invariant: workspace is read-only
        assert!(
            ws.path().join("docs/guide.md").exists(),
            "original file must still exist — diff is read-only"
        );
        assert!(
            !ws.path().join("docs/moved.md").exists(),
            "dst must not be created — diff is read-only"
        );
    }

    // ── Exit criterion 2: Move op line includes src, dst, ref count, file count ──

    /// Move op line includes src, dst, ref count, and file count from PLAN phase.
    #[test]
    fn test_move_line_includes_ref_and_file_count() {
        let ws = make_workspace();
        write_file(ws.path(), "src/target.md", "# Target\n");
        // referrer.md links to target.md — will produce 1 ref in 1 file
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
        assert_eq!(code, 0);
        let output = String::from_utf8(out).unwrap();

        // Line must contain src, dst, ref count, file count
        assert!(
            output.contains("src/target.md"),
            "output must contain src; got:\n{output}"
        );
        assert!(
            output.contains("src/renamed.md"),
            "output must contain dst; got:\n{output}"
        );
        assert!(
            output.contains("refs in"),
            "output must contain ref count; got:\n{output}"
        );
        assert!(
            output.contains("files"),
            "output must contain file count; got:\n{output}"
        );
        // 1 referrer → "(1 refs in 1 files)"
        assert!(
            output.contains("(1 refs in 1 files)"),
            "expected '(1 refs in 1 files)'; got:\n{output}"
        );
    }

    // ── Exit criterion 3: CreateDir with pre-existing path prints (already exists) ──

    /// CreateDir op where path already exists prints "(already exists)".
    #[test]
    fn test_create_dir_already_exists() {
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
        assert_eq!(code, 0);
        let output = String::from_utf8(out).unwrap();

        assert!(
            output.contains("(already exists)"),
            "pre-existing dir must print '(already exists)'; got:\n{output}"
        );
    }

    /// CreateDir op where path does NOT exist omits "(already exists)".
    #[test]
    fn test_create_dir_not_exists_no_suffix() {
        let ws = make_workspace();

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "new-dir"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_eq!(code, 0);
        let output = String::from_utf8(out).unwrap();

        assert!(
            !output.contains("already exists"),
            "non-existent dir must not print 'already exists'; got:\n{output}"
        );
        assert!(
            output.contains("new-dir/"),
            "output must contain the dir path; got:\n{output}"
        );
    }

    // ── Exit criterion 5: Missing src → [ERROR: src not found] + similar ────

    /// Move with non-existent src prints ERROR inline and similar path on next line.
    #[test]
    fn test_move_missing_src_prints_error_and_similar() {
        let ws = make_workspace();
        // Add a similarly-named file to trigger suggestion (basename "guide.md" == "guide.md")
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
        // Diff continues to next op after error — still exits 0
        assert_eq!(code, 0, "diff must continue past missing-src error");
        let output = String::from_utf8(out).unwrap();

        assert!(
            output.contains("[ERROR: src not found]"),
            "missing src must print [ERROR: src not found]; got:\n{output}"
        );
        assert!(
            output.contains("similar: foundations/guide.md"),
            "similar path must be shown; got:\n{output}"
        );
    }

    /// Move with non-existent src and no similar candidates — no similar line printed.
    #[test]
    fn test_move_missing_src_no_similar_when_none() {
        let ws = make_workspace();
        // No files with a similar name

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "xyz123qwerty.md"
dst = "other.md"
"#,
        );

        let mut out = Vec::new();
        run_impl(&plan_path, ws.path(), &mut out);
        let output = String::from_utf8(out).unwrap();

        assert!(
            !output.contains("similar:"),
            "no similar line expected when no close match exists; got:\n{output}"
        );
    }

    // ── Exit criterion 6: Summary line present with correct totals ───────────

    /// Summary line is present and contains correct operation, ref, and file counts.
    #[test]
    fn test_summary_line_correct_totals() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# A\n");
        write_file(ws.path(), "b.md", "# B\n");
        // One referrer for a.md to produce a non-zero ref count
        write_file(ws.path(), "referrer.md", "See [a](a.md).\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "moved-a.md"

[[ops]]
type = "move"
src = "b.md"
dst = "moved-b.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(&plan_path, ws.path(), &mut out);
        assert_eq!(code, 0);
        let output = String::from_utf8(out).unwrap();

        // "2 operations · 1 refs · 1 files" (referrer.md for a.md; b.md has none)
        assert!(
            output.contains("2 operations"),
            "summary must show 2 operations; got:\n{output}"
        );
        assert!(
            output.contains("1 refs"),
            "summary must show 1 refs; got:\n{output}"
        );
        assert!(
            output.contains("1 files"),
            "summary must show 1 files; got:\n{output}"
        );
    }

    /// Hint line is present with the plan path.
    #[test]
    fn test_hint_line_present() {
        let ws = make_workspace();
        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "new-dir"
"#,
        );

        let mut out = Vec::new();
        run_impl(&plan_path, ws.path(), &mut out);
        let output = String::from_utf8(out).unwrap();

        assert!(
            output.contains("anchor apply"),
            "hint line must contain 'anchor apply'; got:\n{output}"
        );
    }

    // ── suggest_similar helper tests ─────────────────────────────────────────

    /// Close basename match → suggestion returned.
    #[test]
    fn test_suggest_similar_close_match() {
        let candidates = vec![
            "foundations/guide.md".to_string(),
            "unrelated/other.md".to_string(),
        ];
        // "guide.md" == "guide.md" → distance 0 → included
        let result = suggest_similar("foundtion/guide.md", &candidates);
        assert!(!result.is_empty(), "should find a close match");
        assert_eq!(result[0], "foundations/guide.md");
    }

    /// Completely unrelated name → no suggestions.
    #[test]
    fn test_suggest_similar_no_match() {
        let candidates = vec!["anchor-foundation/identity.md".to_string()];
        let result = suggest_similar("xyz123qwerty", &candidates);
        assert!(result.is_empty(), "unrelated name must return no suggestions");
    }

    /// Single-character typo in basename → suggestion returned.
    #[test]
    fn test_suggest_similar_single_typo() {
        let candidates = vec!["docs/status.md".to_string()];
        // "status.md" vs "statis.md" → distance 1 / 9 = 0.11 → included
        let result = suggest_similar("docs/statis.md", &candidates);
        assert!(!result.is_empty(), "single-char typo must be a match");
        assert_eq!(result[0], "docs/status.md");
    }

    /// Returns at most 1 result even with multiple close candidates.
    #[test]
    fn test_suggest_similar_capped_at_one() {
        let candidates = vec![
            "a/guide.md".to_string(),
            "b/guide.md".to_string(),
            "c/guide.md".to_string(),
        ];
        let result = suggest_similar("x/guide.md", &candidates);
        assert!(
            result.len() <= 1,
            "suggest_similar must return at most 1 result; got: {result:?}"
        );
    }

    /// Empty candidates list → empty result, no panic.
    #[test]
    fn test_suggest_similar_empty_candidates() {
        let result = suggest_similar("anything", &[]);
        assert!(result.is_empty());
    }
}
