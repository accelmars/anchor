// tests/apply_rollback_diagnostics.rs — AENG-002: rollback names failing refs
//
// Verifies that `anchor apply` emits per-ref diagnostics (file:line, target, similar:)
// on a VALIDATE-phase rollback, matching `anchor file validate` format.
//
// Fixture strategy: a.md references a path being moved (so it lands in op_dir/rewrites/)
// AND pre-existing broken refs that fail VALIDATE. This reliably triggers the rollback
// path regardless of anchor's correct ref-rewriting logic.
//
// Executor: idris-mensah (@idris), AENG-002

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn anchor_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_anchor"))
}

fn make_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let anchor_dir = tmp.path().join(".accelmars").join("anchor");
    fs::create_dir_all(&anchor_dir).unwrap();
    fs::write(
        anchor_dir.join("config.json"),
        r#"{"schema_version":"1"}"#,
    )
    .unwrap();
    tmp
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

fn plan_file(ws: &TempDir, content: &str) -> std::path::PathBuf {
    let p = ws.path().join("plan.toml");
    fs::write(&p, content).unwrap();
    p
}

// ── Test 1: ≥2 broken refs named in rollback output ──────────────────────────

/// Apply rollback names each failing ref with file:line and (target not found).
///
/// Fixture: a.md references moved/note.md (gets into rewrites/) AND two pre-existing
/// broken refs (nonexistent/one.md, nonexistent/two.md) that fail VALIDATE.
#[test]
fn test_rollback_names_two_broken_refs() {
    let ws = make_workspace();

    // moved/note.md — the file being relocated
    write_file(ws.path(), "moved/note.md", "# Note\n");

    // a.md: ref to moved/note.md (ensures a.md enters op_dir/rewrites/)
    //       plus two pre-existing broken refs that will fail VALIDATE
    write_file(
        ws.path(),
        "a.md",
        "[note](moved/note.md)\n[broken1](nonexistent/one.md)\n[broken2](nonexistent/two.md)\n",
    );

    let plan = plan_file(
        &ws,
        r#"version = "1"
[[ops]]
type = "move"
src = "moved"
dst = "archive"
"#,
    );

    let output = Command::new(anchor_bin())
        .args(["apply", plan.to_str().unwrap()])
        .current_dir(ws.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert_ne!(
        output.status.code().unwrap_or(0),
        0,
        "rollback must return non-zero exit code"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("BROKEN REFERENCES AFTER REWRITE (2):"),
        "stderr must contain header with count 2; got:\n{stderr}"
    );
    assert!(
        stderr.contains("a.md:2"),
        "stderr must contain file:line for first broken ref; got:\n{stderr}"
    );
    assert!(
        stderr.contains("nonexistent/one.md"),
        "stderr must name first broken target; got:\n{stderr}"
    );
    assert!(
        stderr.contains("a.md:3"),
        "stderr must contain file:line for second broken ref; got:\n{stderr}"
    );
    assert!(
        stderr.contains("nonexistent/two.md"),
        "stderr must name second broken target; got:\n{stderr}"
    );
    assert!(
        stderr.contains("(target not found)"),
        "stderr must contain '(target not found)'; got:\n{stderr}"
    );
    assert!(
        stderr.contains("rolled back."),
        "stderr must confirm rollback; got:\n{stderr}"
    );
}

// ── Test 2: similar: suggestion emitted when close match exists ───────────────

/// Apply rollback emits `similar:` line when a close filename match exists in workspace.
///
/// Fixture: b.md references moved/doc.md (enters rewrites/) AND "actaul-target.md"
/// (single-char typo of "actual-target.md" which exists). VALIDATE fails on the typo
/// ref; similar: should suggest the correct file.
#[test]
fn test_rollback_similar_suggestion() {
    let ws = make_workspace();

    // moved/doc.md — being relocated
    write_file(ws.path(), "moved/doc.md", "# Doc\n");

    // actual-target.md EXISTS — close match for the typo below
    write_file(ws.path(), "actual-target.md", "# Actual target\n");

    // b.md: ref to moved/doc.md (enters rewrites/) + single-char-typo broken ref
    write_file(
        ws.path(),
        "b.md",
        "[doc](moved/doc.md)\n[typo](actaul-target.md)\n",
    );

    let plan = plan_file(
        &ws,
        r#"version = "1"
[[ops]]
type = "move"
src = "moved"
dst = "archive2"
"#,
    );

    let output = Command::new(anchor_bin())
        .args(["apply", plan.to_str().unwrap()])
        .current_dir(ws.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert_ne!(
        output.status.code().unwrap_or(0),
        0,
        "rollback must return non-zero exit code"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("actaul-target.md"),
        "stderr must name the broken ref target; got:\n{stderr}"
    );
    assert!(
        stderr.contains("similar: actual-target.md"),
        "stderr must contain similar suggestion for the typo; got:\n{stderr}"
    );
}
