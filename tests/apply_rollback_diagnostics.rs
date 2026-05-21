// tests/apply_rollback_diagnostics.rs — AENG-002 + Intake A Gap 2 integration coverage
//
// AENG-002 originally asserted per-ref diagnostics (file:line, target, similar:) on
// rollback caused by pre-existing broken refs. After Intake A Gap 2 (v0.9.0) pre-existing
// broken refs are auto-acked, so the OLD trigger path is no longer the common case —
// the diagnostic format is still unit-tested in `core/diagnostics.rs`.
//
// This file now provides integration coverage for the auto-ack flow:
//   - Pre-existing broken refs survive the apply (no rollback, warning emitted).
//   - Files moved correctly, content preserved.
//
// Rollback still triggers when a ref is NEWLY broken (rewriter bug). That path is
// hard to exercise naturally without injecting a fault into the rewriter, so it is
// covered by unit tests in `core/diagnostics.rs` rather than integration tests.

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
    fs::write(anchor_dir.join("config.json"), r#"{"schema_version":"1"}"#).unwrap();
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

/// Intake A Gap 2 — pre-existing broken refs in a moved file are auto-acked.
/// Apply succeeds with exit 0 and emits a "Pre-existing broken ref preserved"
/// warning per broken ref.
#[test]
fn test_preexisting_broken_refs_auto_acked_and_warned() {
    let ws = make_workspace();

    // a.md has 2 broken refs that the move can't fix (the targets don't exist).
    write_file(
        ws.path(),
        "a.md",
        "[broken1](nonexistent/one.md)\n[broken2](nonexistent/two.md)\n",
    );

    let plan = plan_file(
        &ws,
        r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
    );

    let output = Command::new(anchor_bin())
        .args(["apply", plan.to_str().unwrap()])
        .current_dir(ws.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap_or(-1),
        0,
        "apply must succeed despite pre-existing broken refs (auto-acked)"
    );

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        combined.contains("Pre-existing broken ref preserved"),
        "output must include the auto-ack warning; got:\n{combined}"
    );

    assert!(
        ws.path().join("b.md").exists(),
        "b.md must exist after move"
    );
    assert!(!ws.path().join("a.md").exists(), "a.md must be gone");
}

/// Intake A Gap 2 — Case B rewriter re-anchoring (relative ref recomputed)
/// must NOT be classified as newly broken: identity uses resolved target,
/// not target_raw, so a typo'd ref points to the same non-existent file
/// pre- and post-move.
#[test]
fn test_case_b_re_anchored_preexisting_broken_still_acked() {
    let ws = make_workspace();

    // actual-target.md EXISTS at root, but b.md points to a typo'd sibling
    // that doesn't exist. The typo'd ref is pre-existing broken.
    write_file(ws.path(), "actual-target.md", "# Actual target\n");
    write_file(ws.path(), "b.md", "[typo](./actaul-target.md)\n");

    // Move b.md into a subdirectory. The rewriter re-anchors `./actaul-target.md`
    // → `../actaul-target.md` (Case B). The resolved target stays the same
    // non-existent root file. Identity-by-resolved-path classifies this as
    // pre-existing broken → auto-acked.
    let plan = plan_file(
        &ws,
        r#"version = "1"
[[ops]]
type = "create_dir"
path = "subdir"
[[ops]]
type = "move"
src = "b.md"
dst = "subdir/b.md"
"#,
    );

    let output = Command::new(anchor_bin())
        .args(["apply", plan.to_str().unwrap()])
        .current_dir(ws.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap_or(-1),
        0,
        "Case-B re-anchored pre-existing broken ref must be auto-acked"
    );

    assert!(
        ws.path().join("subdir/b.md").exists(),
        "b.md must be at subdir/b.md after move"
    );
}
