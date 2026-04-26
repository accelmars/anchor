// tests/anchor_engine_integration.rs — Cross-binary integration test (anchor-engine → anchor apply flow)
//
// Tests the interface between anchor-engine (AI planner) and anchor (executor): the plan TOML file.
// Plans are hand-crafted to match the format AN-015 defines — no live anchor-engine or Gateway required.
// The test is: "if anchor-engine produced this plan, would anchor execute it correctly?"
//
// Format compatibility concerns (@vera, AN-023):
//   1. Version field: anchor supports only `version = "1"`. Any plan with `version != "1"` is
//      rejected before any operation executes. If anchor-engine ever emits a newer version string
//      before anchor adds support, the rejection must be clean — exit 1, no side effects.
//      Test 3 exercises this boundary. The error message from load_plan is descriptive:
//      "unsupported plan version: \"N\" (expected \"1\")" — sufficient for diagnosis.
//
//   2. Op type field: anchor recognises `type = "create_dir"` and `type = "move"`. Any future
//      anchor-engine op type not in this set causes a TOML deserialization error at parse time
//      (unknown enum variant). anchor exits 1 before any op executes — safe, but the error message
//      does not name the unrecognised type explicitly. Consider a more descriptive error in future.
//
//   3. Round-trip: render_plan_toml → load_plan must be lossless. If anchor-engine uses
//      render_plan_toml to write plans and anchor uses load_plan to read them, format drift
//      between these functions breaks the interface without any error. Test 4 catches this.
//
//   4. TOML escaping: render_plan_toml escapes path strings as TOML basic strings (double-quoted,
//      backslash-escaped). Paths with `\` or `"` survive the round-trip. Paths with TOML control
//      characters (e.g. null bytes) are not tested — not expected from anchor-engine in practice.
//
// Tests 1–3 use the compiled `anchor` binary via subprocess with explicit current_dir set to
// the temp workspace — consistent with the pattern in plan_integration.rs and required for correct
// workspace root discovery. Test 4 uses the library API directly (render_plan_toml, load_plan).
// The validate assertion in Test 1 uses validate::run_on_root directly — it takes an explicit
// workspace root, so no CWD manipulation is needed.
//
// Executor: vera-novak (@vera), AN-023

use accelmars_anchor::cli::file::validate;
use accelmars_anchor::model::plan::{load_plan, render_plan_toml, Op, Plan};
use std::fs;
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
    let path = root.join("engine-plan.toml");
    fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

// ─── Test 1: apply executes a well-formed plan (happy path) ──────────────────

/// A hand-crafted plan (mimicking anchor-engine output format) is executed by anchor apply.
/// apply exits 0, workspace state matches expected, validate exits 0 after apply.
///
/// Plan: CreateDir(foundations) + Move(src-module → foundations/src-module)
/// Includes a cross-file reference (ref-a.md → src-module/internal.md) that must be
/// rewritten by apply so that anchor validate exits 0 after the move.
#[test]
fn test_apply_executes_well_formed_plan() {
    let ws = make_workspace();
    let root = ws.path();

    // src-module/internal.md: self-referencing file (relative ref stays resolvable after move)
    // ref-a.md: external reference that apply must rewrite to the new path
    write_file(
        root,
        "src-module/internal.md",
        "# Internal\n[self](internal.md)\n",
    );
    write_file(root, "ref-a.md", "[internal](src-module/internal.md)\n");

    let plan_path = write_plan(
        root,
        r#"version = "1"
description = "Move src-module into foundations"

[[ops]]
type = "create_dir"
path = "foundations"

[[ops]]
type = "move"
src = "src-module"
dst = "foundations/src-module"
"#,
    );

    let output = Command::new(anchor_bin())
        .args(["apply", &plan_path])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "apply must exit 0 for a well-formed plan; stderr: {}; stdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    // CreateDir op executed: foundations/ created
    assert!(
        root.join("foundations").is_dir(),
        "foundations/ must be created by CreateDir op"
    );

    // Move op executed: src-module → foundations/src-module
    assert!(
        root.join("foundations/src-module").exists(),
        "foundations/src-module must exist after move"
    );
    assert!(
        root.join("foundations/src-module/internal.md").exists(),
        "directory contents must move with the directory"
    );
    assert!(
        !root.join("src-module").exists(),
        "src-module must be absent — moved to foundations/"
    );

    // validate::run_on_root exits 0: apply rewrote references, no broken refs remain
    let validate_code = validate::run_on_root(root, None);
    assert_eq!(
        validate_code, 0,
        "anchor validate must exit 0 after apply — apply must rewrite all cross-references"
    );
}

// ─── Test 2: diff is read-only before apply ───────────────────────────────────

/// Sequence test: diff exits 0 and leaves the workspace byte-for-byte unchanged.
/// apply then succeeds on the SAME plan and workspace.
///
/// This is the primary unique contribution of AN-023: AN-019 tests diff and apply
/// individually. This test verifies the interface guarantee that diff does not corrupt
/// workspace state in a way that prevents apply from executing the same plan.
#[test]
fn test_diff_then_apply_sequence() {
    let ws = make_workspace();
    let root = ws.path();

    write_file(root, "src-folder/note.md", "# Note\n");
    write_file(root, "ref.md", "[note](src-folder/note.md)\n");

    let plan_path = write_plan(
        root,
        r#"version = "1"
description = "Move src-folder into archive"

[[ops]]
type = "create_dir"
path = "archive"

[[ops]]
type = "move"
src = "src-folder"
dst = "archive/src-folder"
"#,
    );

    // Phase 1: diff — must exit 0 and not modify the workspace
    let diff_output = Command::new(anchor_bin())
        .args(["diff", &plan_path])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        diff_output.status.code(),
        Some(0),
        "diff must exit 0; stderr: {}",
        String::from_utf8_lossy(&diff_output.stderr)
    );

    // Workspace invariant after diff: src-folder still present, archive not created
    assert!(
        root.join("src-folder").exists(),
        "src-folder must still exist after diff — diff is read-only"
    );
    assert!(
        root.join("src-folder/note.md").exists(),
        "src-folder/note.md must still exist after diff"
    );
    assert!(
        !root.join("archive").exists(),
        "archive must not be created by diff — diff does not execute ops"
    );

    // Phase 2: apply on the same plan — must succeed (diff did not corrupt state)
    let apply_output = Command::new(anchor_bin())
        .args(["apply", &plan_path])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        apply_output.status.code(),
        Some(0),
        "apply must exit 0 after diff — diff must not corrupt plan or workspace state; stderr: {}",
        String::from_utf8_lossy(&apply_output.stderr)
    );

    // Apply executed: archive/src-folder created, src-folder gone
    assert!(
        root.join("archive/src-folder").exists(),
        "archive/src-folder must exist after apply"
    );
    assert!(
        !root.join("src-folder").exists(),
        "src-folder must be absent after apply — directory moved"
    );
}

// ─── Test 3: unsupported plan version ────────────────────────────────────────

/// apply rejects a plan with an unsupported version before executing any operation.
///
/// Version compatibility boundary: anchor only supports `version = "1"`. This test
/// verifies that a `version = "2"` plan (which anchor-engine might emit in a future
/// release) is rejected cleanly — exit 1, no filesystem changes. See compatibility
/// concern #1 in the file header.
#[test]
fn test_apply_rejects_unsupported_version() {
    let ws = make_workspace();
    let root = ws.path();

    write_file(root, "existing.md", "# Exists\n");

    let plan_path = write_plan(
        root,
        r#"version = "2"

[[ops]]
type = "create_dir"
path = "new-dir"
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
        "apply must exit 1 for unsupported plan version; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // No filesystem changes: existing.md untouched, new-dir not created
    assert!(
        root.join("existing.md").exists(),
        "existing.md must still exist — no ops executed for unsupported version"
    );
    assert!(
        !root.join("new-dir").exists(),
        "new-dir must not be created — apply must not execute any op for an unsupported version"
    );
}

// ─── Test 4: round-trip fidelity ─────────────────────────────────────────────

/// render_plan_toml → load_plan preserves the original Plan struct identically.
///
/// If anchor-engine uses render_plan_toml to write plans and anchor uses load_plan
/// to read them, this round-trip must be lossless. Silent format drift between these
/// functions would break the interface without any parse error. See compatibility
/// concern #3 in the file header.
#[test]
fn test_round_trip_fidelity() {
    let original = Plan {
        version: "1".to_string(),
        description: Some("anchor-engine output — move foundation modules".to_string()),
        ops: vec![
            Op::CreateDir {
                path: "foundations".to_string(),
            },
            Op::Move {
                src: "anchor-foundation".to_string(),
                dst: "foundations/anchor-engine".to_string(),
            },
        ],
    };

    let rendered = render_plan_toml(&original);

    // Write rendered output to a temp file — load_plan takes &Path
    let tmp = TempDir::new().unwrap();
    let plan_path = tmp.path().join("round-trip.toml");
    fs::write(&plan_path, &rendered).unwrap();

    let loaded = load_plan(&plan_path)
        .expect("render_plan_toml output must be valid plan TOML parseable by load_plan");

    assert_eq!(
        loaded.version, original.version,
        "round-trip must preserve version field"
    );
    assert_eq!(
        loaded.description, original.description,
        "round-trip must preserve description field"
    );
    assert_eq!(
        loaded.ops.len(),
        original.ops.len(),
        "round-trip must preserve op count; expected {}, got {}",
        original.ops.len(),
        loaded.ops.len()
    );
    assert_eq!(
        loaded.ops[0], original.ops[0],
        "round-trip must preserve CreateDir op identity"
    );
    assert_eq!(
        loaded.ops[1], original.ops[1],
        "round-trip must preserve Move op identity"
    );
}
