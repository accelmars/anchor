// tests/plan_integration.rs — Integration tests for plan adapter commands (AN-019)
//
// Exercises the end-to-end plan pipeline: plan file → diff (read-only preview) →
// apply (create dirs + move files + rewrite refs) → validate (no broken refs).
// Also tests pre-flight rejection and wizard scaffold output.
//
// Tests use the compiled `anchor` binary via subprocess for CLI operations (consistent
// with integration_validate_refs.rs). The wizard test uses the library API directly
// since run_wizard<R,W> is parameterized on I/O.
//
// Coverage gaps:
//   - Concurrent apply runs (true OS-level race between pre-flight and execution)
//     are not directly exercised; Test 4 simulates via same-src double-move, which
//     is the deterministic equivalent pattern from apply.rs unit tests.
//   - anchor validate internal logic is covered by integration_validate_refs.rs;
//     Test 2 uses the binary to verify the full validate path after apply.
//   - Plan files with no ops, malformed TOML, or unsupported versions are covered
//     by plan.rs unit tests and apply.rs/diff.rs unit tests; not duplicated here.

use accelmars_anchor::cli::plan::new::run_wizard;
use accelmars_anchor::model::plan::{load_plan, Op};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn anchor_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_anchor"))
}

fn make_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
    fs::write(
        root.join(".accelmars").join("anchor").join("config.json"),
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

fn write_plan(root: &Path, content: &str) -> String {
    let path = root.join("test-plan.toml");
    fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

/// Set up the shared workspace for Tests 1–2:
///   ref-a.md  — external referrer to src-folder/internal.md (Case A)
///   ref-b.md  — references ref-a.md (unrelated to move)
///   src-folder/internal.md — the directory that will be moved
///   plan: CreateDir(dst-folder) + Move(src-folder → dst-folder/src-folder)
fn setup_move_workspace(tmp: &TempDir) -> String {
    let root = tmp.path();
    write_file(root, "ref-a.md", "[internal](src-folder/internal.md)\n");
    write_file(root, "ref-b.md", "[ref-a](ref-a.md)\n");
    write_file(root, "src-folder/internal.md", "# Internal\n");
    write_plan(
        root,
        r#"version = "1"
description = "Move src-folder into dst-folder"

[[ops]]
type = "create_dir"
path = "dst-folder"

[[ops]]
type = "move"
src = "src-folder"
dst = "dst-folder/src-folder"
"#,
    )
}

// ─── Test 1: diff is read-only ────────────────────────────────────────────────

/// diff exits 0 and leaves workspace byte-for-byte identical.
/// src-folder must still exist; no new dirs created; ops not executed.
#[test]
fn test_diff_is_read_only() {
    let ws = make_workspace();
    let plan_path = setup_move_workspace(&ws);
    let root = ws.path();

    let output = Command::new(anchor_bin())
        .args(["diff", &plan_path])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "diff must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Core invariant: workspace is read-only after diff
    assert!(
        root.join("src-folder").exists(),
        "src-folder must still exist — diff is read-only"
    );
    assert!(
        root.join("src-folder/internal.md").exists(),
        "src-folder/internal.md must still exist — diff is read-only"
    );
    assert!(
        !root.join("dst-folder").exists(),
        "dst-folder must not be created — diff does not execute CreateDir"
    );
}

// ─── Test 2: apply executes ops correctly ────────────────────────────────────

/// apply exits 0, creates dst-folder, moves src-folder, rewrites cross-references,
/// and anchor validate exits 0 after completion.
#[test]
fn test_apply_executes_correctly() {
    let ws = make_workspace();
    let plan_path = setup_move_workspace(&ws);
    let root = ws.path();

    let output = Command::new(anchor_bin())
        .args(["apply", &plan_path])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "apply must exit 0; stderr: {}; stdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    // CreateDir executed: dst-folder created
    assert!(
        root.join("dst-folder").is_dir(),
        "dst-folder must be created by CreateDir op"
    );

    // Move executed: src-folder → dst-folder/src-folder
    assert!(
        root.join("dst-folder/src-folder").exists(),
        "dst-folder/src-folder must exist after move"
    );
    assert!(
        root.join("dst-folder/src-folder/internal.md").exists(),
        "dst-folder/src-folder/internal.md must exist after directory move"
    );
    assert!(
        !root.join("src-folder").exists(),
        "src-folder must be absent — directory was moved"
    );

    // Cross-reference rewritten: ref-a.md Case A — external ref updated to new path
    let ref_a = fs::read_to_string(root.join("ref-a.md")).unwrap();
    assert!(
        ref_a.contains("dst-folder/src-folder/internal.md"),
        "ref-a.md must reference new path after directory move; got:\n{ref_a}"
    );
    // Old standalone path (in parens) must be gone. Note: new path contains "src-folder/internal.md"
    // as a suffix so we check the paren-enclosed form of the old reference specifically.
    assert!(
        !ref_a.contains("](src-folder/internal.md)"),
        "ref-a.md must not contain old reference in paren form; got:\n{ref_a}"
    );

    // anchor validate exits 0 — no broken references after apply
    let validate_output = Command::new(anchor_bin())
        .args(["file", "validate"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        validate_output.status.code(),
        Some(0),
        "anchor validate must exit 0 after apply; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&validate_output.stdout),
        String::from_utf8_lossy(&validate_output.stderr)
    );
}

// ─── Test 3: apply pre-flight catches missing src ─────────────────────────────

/// apply exits 1 and leaves workspace unchanged when a plan Move op src does not exist.
/// No file must be moved; no directory must be created; workspace identical to before.
#[test]
fn test_apply_preflight_missing_src() {
    let ws = make_workspace();
    let root = ws.path();

    write_file(root, "existing.md", "# Exists\n");

    let plan_path = write_plan(
        root,
        r#"version = "1"

[[ops]]
type = "move"
src = "nonexistent-file.md"
dst = "moved.md"
"#,
    );

    let output = Command::new(anchor_bin())
        .args(["apply", &plan_path])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "apply must exit 1 when src missing at pre-flight; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Workspace unchanged: dst never created, existing file untouched
    assert!(
        root.join("existing.md").exists(),
        "existing.md must still exist — workspace unchanged after pre-flight failure"
    );
    assert!(
        !root.join("moved.md").exists(),
        "moved.md must not be created — pre-flight failed before any op ran"
    );
}

// ─── Test 4: apply stops and reports on transaction failure ──────────────────

/// Two ops with the same src simulate a race condition (src deleted between
/// pre-flight and op 2 execution). Pre-flight passes for both (src exists).
/// Op 1 commits (a.md → b.md, leaving a.md gone). Op 2 tries a.md → c.md
/// but src is gone at execution time — transaction fails.
///
/// Expected state after apply:
///   - Op 1 remains committed: b.md exists, a.md absent
///   - Op 2 never committed: c.md absent
///   - Exit code 1
///   - stdout contains "Stopped after 1/2 operations completed."
///
/// This is NOT a full plan rollback — AN-017 does not undo committed moves.
#[test]
fn test_apply_stops_and_reports_on_failure() {
    let ws = make_workspace();
    let root = ws.path();

    write_file(root, "a.md", "# A\n");

    let plan_path = write_plan(
        root,
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

    let output = Command::new(anchor_bin())
        .args(["apply", &plan_path])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "must exit 1 when second op fails; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Stopped after 1/2 operations completed."),
        "must print stopped-after message; got stdout:\n{stdout}"
    );

    // Op 1 committed and remains in place — not rolled back
    assert!(
        root.join("b.md").exists(),
        "first move must be committed — b.md must exist"
    );
    assert!(
        !root.join("a.md").exists(),
        "src consumed by first op — a.md must be absent"
    );

    // Op 2 transaction rolled back — never committed
    assert!(
        !root.join("c.md").exists(),
        "second move must not have committed — c.md must not exist"
    );
}

// ─── Test 5: wizard scaffold output is valid plan TOML ───────────────────────

/// Scaffold template (5) with 2 directory inputs produces valid plan TOML
/// with exactly 2 CreateDir ops at the correct paths.
///
/// Mocked stdin: "5\nmy-dir\nother-dir\n\n\n"
///   5       — select scaffold template
///   my-dir  — first directory
///   other-dir — second directory
///   (blank) — blank to finish collecting dirs
///   (blank) — blank description (optional, Enter to skip)
#[test]
fn test_wizard_scaffold_produces_valid_plan() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("wizard-output.toml");
    let out_str = out.to_str().unwrap().to_string();

    let input = "5\nmy-dir\nother-dir\n\n\n";
    let mut reader = Cursor::new(input.as_bytes().to_vec());
    let mut writer = Vec::<u8>::new();

    let code = run_wizard(&mut reader, &mut writer, Some(&out_str), None);
    assert_eq!(code, 0, "run_wizard must exit 0");

    // Output file must be valid plan TOML
    let plan = load_plan(&out).expect("wizard output must be valid plan TOML");

    // 2 CreateDir ops with correct paths
    assert_eq!(
        plan.ops.len(),
        2,
        "scaffold with 2 dirs must produce 2 ops; got: {:?}",
        plan.ops
    );
    assert_eq!(
        plan.ops[0],
        Op::CreateDir {
            path: "my-dir".to_string()
        },
        "first op must be CreateDir(my-dir)"
    );
    assert_eq!(
        plan.ops[1],
        Op::CreateDir {
            path: "other-dir".to_string()
        },
        "second op must be CreateDir(other-dir)"
    );
}
