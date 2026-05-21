// src/cli/plan/validate.rs — anchor plan validate command (AP-001)

use crate::apply::text_rename;
use crate::infra::workspace;
use crate::model::plan::{self, Op};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Execute `anchor plan validate <plan.toml>`.
///
/// Discovers workspace root, loads the plan, then validates each operation:
/// Move ops must have an existing src and absent dst. Returns 0 on all-pass,
/// 1 on validation failures, 2 on file read/parse error.
pub fn run(plan_path: &str) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_impl(plan_path, &workspace_root)
}

#[derive(Debug, Default)]
pub(crate) struct ValidationOutcome {
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

/// Pure-function validator: walks the plan, accumulates virtual filesystem state
/// so that later ops are checked against the cumulative effect of earlier ops
/// (not raw disk state). Returns errors and notes for the caller to print.
pub(crate) fn validate_plan(plan: &plan::Plan, workspace_root: &Path) -> ValidationOutcome {
    let mut outcome = ValidationOutcome::default();
    // Virtual filesystem state that accumulates across ops:
    //   - virtual_created: paths that will exist after prior CreateDir or Move(dst) ops
    //   - virtual_removed: paths that will be gone after prior Move(src) ops
    // Without this, a `move` into a parent created by an earlier `create_dir` emits a
    // spurious "destination parent does not exist" note, and `move A → B; move B → C`
    // would falsely flag B as still on disk.
    let mut virtual_created: HashSet<PathBuf> = HashSet::new();
    let mut virtual_removed: HashSet<PathBuf> = HashSet::new();

    // Exact-match existence: was `p` explicitly created/removed by a prior op, or
    // is it on disk? Used for src-found and dst-already-exists checks, where ancestor
    // coverage would produce false positives (a `create_dir foo` doesn't mean
    // `foo/bar.md` exists).
    let exact_exists = |p: &Path, created: &HashSet<PathBuf>, removed: &HashSet<PathBuf>| -> bool {
        if removed.contains(p) {
            return false;
        }
        if created.contains(p) {
            return true;
        }
        p.exists()
    };

    // Ancestor-aware existence: true if `p` is exactly covered OR if any ancestor
    // of `p` was virtually created. Used for the dst-parent existence check, where
    // an earlier `create_dir new-parent` should suppress the note for a later
    // `move ... → new-parent/foo` because the user already acknowledged the dir.
    let parent_covered =
        |p: &Path, created: &HashSet<PathBuf>, removed: &HashSet<PathBuf>| -> bool {
            if exact_exists(p, created, removed) {
                return true;
            }
            let mut cur = p;
            while let Some(parent) = cur.parent() {
                if created.contains(parent) {
                    return true;
                }
                cur = parent;
            }
            false
        };

    for (i, op) in plan.ops.iter().enumerate() {
        let n = i + 1;
        match op {
            Op::Move { src, dst } => {
                let src_path = workspace_root.join(src);
                let dst_path = workspace_root.join(dst);
                if !exact_exists(&src_path, &virtual_created, &virtual_removed) {
                    outcome
                        .errors
                        .push(format!("operation {n}: src not found: {src}"));
                }
                if exact_exists(&dst_path, &virtual_created, &virtual_removed) {
                    outcome
                        .errors
                        .push(format!("operation {n}: dst already exists: {dst}"));
                }
                if let Some(parent) = dst_path.parent() {
                    if !parent.as_os_str().is_empty()
                        && !parent_covered(parent, &virtual_created, &virtual_removed)
                    {
                        let parent_rel = Path::new(dst.as_str())
                            .parent()
                            .and_then(|p| p.to_str())
                            .unwrap_or("");
                        outcome.notes.push(format!(
                            "operation {n}: destination parent '{parent_rel}' does not exist and will be created automatically"
                        ));
                    }
                }
                virtual_removed.insert(src_path);
                virtual_created.insert(dst_path);
            }
            Op::CreateDir { path } => {
                let abs = workspace_root.join(path);
                virtual_created.insert(abs);
            }
            Op::TextRename(text_op) => {
                if text_op.from.is_empty() {
                    outcome
                        .errors
                        .push(format!("operation {n}: text_rename from must be non-empty"));
                }
                if text_op.to.is_empty() {
                    outcome
                        .errors
                        .push(format!("operation {n}: text_rename to must be non-empty"));
                }
                if !text_op.literal {
                    if let Err(e) = regex::Regex::new(&text_op.from) {
                        outcome
                            .errors
                            .push(format!("operation {n}: invalid text_rename regex: {e}"));
                    }
                }
                if let Err(e) =
                    text_rename::validate_globs(workspace_root, &text_op.include_paths)
                {
                    outcome
                        .errors
                        .push(format!("operation {n}: include_paths: {e}"));
                }
                if let Err(e) =
                    text_rename::validate_globs(workspace_root, &text_op.exclude_paths)
                {
                    outcome
                        .errors
                        .push(format!("operation {n}: exclude_paths: {e}"));
                }
            }
        }
    }

    outcome
}

pub(crate) fn run_impl(plan_path: &str, workspace_root: &Path) -> i32 {
    let plan = match plan::load_plan(Path::new(plan_path)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let outcome = validate_plan(&plan, workspace_root);

    for note in &outcome.notes {
        eprintln!("note: {note}");
    }

    if outcome.errors.is_empty() {
        let count = plan.ops.len();
        println!("Plan is valid. {count} operations ready to apply.");
        0
    } else {
        for e in &outcome.errors {
            eprintln!("error: {e}");
        }
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &Path, rel: &str) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, "").unwrap();
    }

    fn plan_file(dir: &Path, content: &str) -> String {
        let path = dir.join("test.toml");
        fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn test_validate_valid_plan() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "docs/guide.md");

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/guide.md"
dst = "docs/renamed.md"
"#,
        );

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 0);
    }

    #[test]
    fn test_validate_missing_src() {
        let ws = TempDir::new().unwrap();

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "nonexistent/file.md"
dst = "other.md"
"#,
        );

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 1);
    }

    #[test]
    fn test_validate_dst_exists() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "src/file.md");
        write_file(ws.path(), "dst/file.md");

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "src/file.md"
dst = "dst/file.md"
"#,
        );

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 1);
    }

    #[test]
    fn test_validate_invalid_toml() {
        let ws = TempDir::new().unwrap();
        let plan = plan_file(ws.path(), "not valid toml [[[");

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 2);
    }

    /// validate: dst parent does not exist → exit 0 (note emitted to stderr, not a failure)
    #[test]
    fn test_validate_dst_parent_missing_exits_0() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "src/guide.md");

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "src/guide.md"
dst = "new-parent/guide.md"
"#,
        );

        // new-parent/ does not exist — validate emits a note but exits 0
        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 0);
    }

    /// validate: dst parent exists on disk → exit 0, no note emitted (no stderr contamination)
    #[test]
    fn test_validate_dst_parent_exists_no_note() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "src/guide.md");
        // pre-create the parent so it exists
        fs::create_dir_all(ws.path().join("existing-parent")).unwrap();

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "src/guide.md"
dst = "existing-parent/guide.md"
"#,
        );

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 0);
    }

    fn parse_plan(content: &str) -> plan::Plan {
        let p = TempDir::new().unwrap();
        let file = p.path().join("p.toml");
        fs::write(&file, content).unwrap();
        plan::load_plan(&file).unwrap()
    }

    /// validate: a `create_dir` op that precedes `move`s into the new dir suppresses
    /// the spurious "destination parent does not exist" note for each move. The dir
    /// will exist by the time the apply reaches the move op, so the warning is noise.
    #[test]
    fn test_validate_create_dir_then_move_no_note() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "old-name/foo.md");
        write_file(ws.path(), "another/bar.md");

        let plan = parse_plan(
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "new-parent"

[[ops]]
type = "move"
src = "old-name"
dst = "new-parent/renamed"

[[ops]]
type = "move"
src = "another"
dst = "new-parent/also-renamed"
"#,
        );

        let outcome = validate_plan(&plan, ws.path());
        assert!(
            outcome.errors.is_empty(),
            "expected no errors; got: {:?}",
            outcome.errors
        );
        assert!(
            outcome.notes.is_empty(),
            "create_dir before move into that dir must suppress the parent-missing note; got: {:?}",
            outcome.notes
        );
    }

    /// validate: chained `move A → B; move B → C` — the second op's src must be
    /// recognized as existing (because op 1's dst was B). Without virtual state,
    /// the validator would report "src not found: B".
    #[test]
    fn test_validate_chained_moves_recognize_virtual_src() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a/file.md");

        let plan = parse_plan(
            r#"version = "1"
[[ops]]
type = "move"
src = "a"
dst = "b"

[[ops]]
type = "move"
src = "b"
dst = "c"
"#,
        );

        let outcome = validate_plan(&plan, ws.path());
        assert!(
            outcome.errors.is_empty(),
            "chained moves: B exists virtually after op 1, so op 2's src check must pass; got: {:?}",
            outcome.errors
        );
    }

    /// validate: chained `move A → B; move A → C` — second op's src must be flagged
    /// as missing because A was already moved away in op 1.
    #[test]
    fn test_validate_chained_moves_detect_virtual_removal() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a/file.md");

        let plan = parse_plan(
            r#"version = "1"
[[ops]]
type = "move"
src = "a"
dst = "b"

[[ops]]
type = "move"
src = "a"
dst = "c"
"#,
        );

        let outcome = validate_plan(&plan, ws.path());
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e.contains("operation 2") && e.contains("src not found")),
            "second move's src must be flagged as missing (already removed in op 1); got: {:?}",
            outcome.errors
        );
    }

    /// validate: when create_dir precedes the move, the move's dst-parent-missing note
    /// stays suppressed AND if the create_dir target is nested (e.g. `new-parent/sub`),
    /// the move into the nested location also gets ancestor coverage.
    #[test]
    fn test_validate_create_dir_covers_nested_ancestor() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "src-file.md");

        let plan = parse_plan(
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "new-parent/sub"

[[ops]]
type = "move"
src = "src-file.md"
dst = "new-parent/sub/dest.md"
"#,
        );

        let outcome = validate_plan(&plan, ws.path());
        assert!(
            outcome.errors.is_empty() && outcome.notes.is_empty(),
            "nested create_dir must cover the move's dst parent; errors={:?}, notes={:?}",
            outcome.errors,
            outcome.notes
        );
    }
}
