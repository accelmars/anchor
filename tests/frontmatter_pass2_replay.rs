// tests/frontmatter_pass2_replay.rs — Pass 2 replay test
//
// Reproduces the 2026-04-29 Pass 2 Python script output via anchor frontmatter commands.
//
// Pass 2 context: 147 files normalized in foundations/anchor-engine/. The bespoke Python
// scripts added schema_version: 1 to files missing it, normalized status synonyms, and
// added type-conditional defaults (pass_status: NOT_RUN for evals, depends_on: [] for analysis).
//
// This test creates fixture files simulating pre-Pass-2 state (as documented in the design
// intake) and verifies anchor frontmatter produces equivalent normalized output.
//
// NOTE: The Python scripts were deleted after Pass 2. Equivalence is verified against
// the documented expected outputs from the design intake, not a byte-for-byte diff.

use accelmars_anchor::cli::frontmatter::add_required;
use accelmars_anchor::cli::frontmatter::migrate;
use accelmars_anchor::cli::frontmatter::normalize;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn make_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
    tmp
}

fn write_md(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

fn read_md(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

fn write_schema(root: &Path) {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sibling = manifest.join("../accelmars-workspace/FRONTMATTER.schema.json");
    let nested = manifest.join("accelmars-workspace/FRONTMATTER.schema.json");
    let content = if sibling.exists() {
        std::fs::read_to_string(&sibling).expect("schema read failed (sibling path)")
    } else {
        std::fs::read_to_string(&nested)
            .expect("FRONTMATTER.schema.json not found (tried sibling and nested paths)")
    };
    fs::write(root.join("FRONTMATTER.schema.json"), content).unwrap();
}

fn schema_path(root: &Path) -> PathBuf {
    root.join("FRONTMATTER.schema.json")
}

// ─── Pass 2 Wave 1: Add schema_version: 1 ────────────────────────────────────

/// Replay Wave 1: 81 files needed only schema_version: 1 added.
/// Test with a representative subset: gap file, analysis file, capability file.
#[test]
fn replay_wave1_add_schema_version() {
    let ws = make_workspace();
    write_schema(ws.path());

    // Pre-Pass-2 state: files with frontmatter but missing schema_version
    let gap_pre = "---\ntitle: GAP-AENG-001 Reference rewrite not context-scoped\ntype: gap\nengine: anchor\nstatus: active\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: GAP-AENG-001\npriority: P1\nmaturity: PLANNED\nboundary: open\n---\n# Body\n";
    let analysis_pre = "---\ntitle: Analysis Doc\ntype: analysis\nengine: anchor\nstatus: active\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: a-an01\npriority: high\ndepends_on: []\n---\n# Body\n";
    let cap_pre = "---\ntitle: Capability Doc\ntype: capability\nengine: anchor\nstatus: active\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: a-cap01\nmaturity: LIVE\n---\n# Body\n";

    write_md(ws.path(), "gap.md", gap_pre);
    write_md(ws.path(), "analysis.md", analysis_pre);
    write_md(ws.path(), "cap.md", cap_pre);

    // Wave 1 command: anchor frontmatter migrate --to 1 . --apply
    let exit = migrate::run(None, 1, true, ws.path(), ws.path());
    assert_eq!(exit, 0, "replay Wave 1: migrate must exit 0");

    // All three files must now have schema_version: 1
    for rel in &["gap.md", "analysis.md", "cap.md"] {
        let content = read_md(ws.path(), rel);
        assert!(
            content.contains("schema_version: 1"),
            "replay Wave 1: {rel} must have schema_version: 1 after migrate; content: {content}"
        );
        // Original content must be preserved
        assert!(
            content.contains("# Body"),
            "replay Wave 1: body must be preserved in {rel}"
        );
    }
}

// ─── Pass 2 Wave 2: Normalize status synonyms ─────────────────────────────────

/// Replay Wave 2: files with non-canonical status values (consumed, complete, open).
/// Python script mapped: consumed → archived, complete → archived, open → active.
#[test]
fn replay_wave2_normalize_status_synonyms() {
    let ws = make_workspace();
    write_schema(ws.path());

    let consumed_doc = "---\ntitle: Consumed Doc\ntype: meta\nengine: anchor\nstatus: consumed\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\n---\n# Body\n";
    let complete_doc = "---\ntitle: Complete Doc\ntype: meta\nengine: anchor\nstatus: complete\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\n---\n# Body\n";
    let open_doc = "---\ntitle: Open Doc\ntype: meta\nengine: anchor\nstatus: open\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\n---\n# Body\n";
    let partial_doc = "---\ntitle: Partial Doc\ntype: meta\nengine: anchor\nstatus: partially-resolved\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\n---\n# Body\n";

    write_md(ws.path(), "consumed.md", consumed_doc);
    write_md(ws.path(), "complete.md", complete_doc);
    write_md(ws.path(), "open.md", open_doc);
    write_md(ws.path(), "partial.md", partial_doc);

    let schema_str = schema_path(ws.path());
    let exit = normalize::run(
        None,
        true,
        false,
        Some(schema_str.to_str().unwrap()),
        ws.path(),
        ws.path(),
    );
    assert_eq!(exit, 0, "replay Wave 2: normalize must exit 0");

    // Python script output equivalence:
    let after_consumed = read_md(ws.path(), "consumed.md");
    assert!(
        after_consumed.contains("status: archived"),
        "consumed → archived; content: {after_consumed}"
    );
    assert!(
        !after_consumed.contains("status: consumed"),
        "original must be replaced"
    );

    let after_complete = read_md(ws.path(), "complete.md");
    assert!(
        after_complete.contains("status: archived"),
        "complete → archived; content: {after_complete}"
    );

    let after_open = read_md(ws.path(), "open.md");
    assert!(
        after_open.contains("status: active"),
        "open → active; content: {after_open}"
    );

    let after_partial = read_md(ws.path(), "partial.md");
    assert!(
        after_partial.contains("status: active"),
        "partially-resolved → active; content: {after_partial}"
    );
}

// ─── Pass 2 Wave 3: Add type-conditional defaults ─────────────────────────────

/// Replay Wave 3: 17 files needing type-conditional fields.
/// Python script added: pass_status: NOT_RUN for evals, depends_on: [] for analysis.
#[test]
fn replay_wave3_add_type_conditional_defaults() {
    let ws = make_workspace();
    write_schema(ws.path());

    // Pre-Pass-2: eval file missing pass_status
    let eval_pre = "---\ntitle: Eval Scenario\ntype: eval\nengine: anchor\nstatus: active\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: a-ev01\n---\n# Eval body\n";
    // Pre-Pass-2: analysis file missing depends_on
    let analysis_pre = "---\ntitle: Analysis\ntype: analysis\nengine: anchor\nstatus: active\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: a-an02\npriority: high\n---\n# Analysis body\n";

    write_md(ws.path(), "eval.md", eval_pre);
    write_md(ws.path(), "analysis.md", analysis_pre);

    let schema_str = schema_path(ws.path());
    let exit_eval = add_required::run(
        "eval.md",
        true,
        false,
        Some(schema_str.to_str().unwrap()),
        ws.path(),
        ws.path(),
    );
    assert_eq!(exit_eval, 0, "replay Wave 3 eval: add-required must exit 0");

    let exit_analysis = add_required::run(
        "analysis.md",
        true,
        false,
        Some(schema_str.to_str().unwrap()),
        ws.path(),
        ws.path(),
    );
    assert_eq!(
        exit_analysis, 0,
        "replay Wave 3 analysis: add-required must exit 0"
    );

    // Python script output equivalence:
    let after_eval = read_md(ws.path(), "eval.md");
    assert!(
        after_eval.contains("pass_status: NOT_RUN"),
        "replay Wave 3: eval must get pass_status: NOT_RUN; content: {after_eval}"
    );

    let after_analysis = read_md(ws.path(), "analysis.md");
    assert!(
        after_analysis.contains("depends_on:"),
        "replay Wave 3: analysis must get depends_on field; content: {after_analysis}"
    );
}

// ─── Full pipeline replay ──────────────────────────────────────────────────────

/// Replay the full 5-command pipeline on a mixed fixture set.
/// Verifies the anchor frontmatter commands collectively reproduce the Pass 2 outcome.
#[test]
fn replay_full_pipeline() {
    let ws = make_workspace();
    write_schema(ws.path());

    // Mixed pre-Pass-2 fixture: gap file missing schema_version, non-canonical status,
    // eval missing pass_status
    let gap_pre = "---\ntitle: GAP Test\ntype: gap\nengine: anchor\nstatus: partially-resolved\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: GAP-T1\npriority: P2\nmaturity: PLANNED\nboundary: open\n---\n# GAP body\n";
    let eval_pre = "---\ntitle: Eval Test\ntype: eval\nengine: anchor\nstatus: active\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: a-ev99\n---\n# Eval body\n";

    write_md(ws.path(), "gap.md", gap_pre);
    write_md(ws.path(), "eval.md", eval_pre);

    let schema_str = schema_path(ws.path()).to_str().unwrap().to_string();

    // Step 1: migrate --to 1 --apply
    assert_eq!(migrate::run(None, 1, true, ws.path(), ws.path()), 0);
    // Step 2: normalize --apply
    assert_eq!(
        normalize::run(None, true, false, Some(&schema_str), ws.path(), ws.path()),
        0
    );
    // Step 3: add-required --batch --auto (eval only — gap has all required fields)
    assert_eq!(
        add_required::run(
            "eval.md",
            true,
            false,
            Some(&schema_str),
            ws.path(),
            ws.path()
        ),
        0
    );

    // Verify post-pipeline state
    let gap_after = read_md(ws.path(), "gap.md");
    assert!(
        gap_after.contains("schema_version: 1"),
        "pipeline: gap must have schema_version"
    );
    assert!(
        gap_after.contains("status: active"),
        "pipeline: partially-resolved → active; content: {gap_after}"
    );

    let eval_after = read_md(ws.path(), "eval.md");
    assert!(
        eval_after.contains("schema_version: 1"),
        "pipeline: eval must have schema_version"
    );
    assert!(
        eval_after.contains("pass_status: NOT_RUN"),
        "pipeline: eval must get pass_status"
    );
}

// ─── Field preservation through normalize ─────────────────────────────────────

/// Replay field preservation: unknown YAML keys must round-trip through normalize unmodified.
#[test]
fn replay_field_preservation_unknown_keys() {
    let ws = make_workspace();
    write_schema(ws.path());

    // File with an unknown field (custom engine extension not in schema)
    let content = "---\ntitle: Custom Doc\ntype: meta\nengine: anchor\nstatus: active\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\ncustom_engine_field: important-value\nanother_field: 42\n---\n# Body\n";
    write_md(ws.path(), "custom.md", content);

    let schema_str = schema_path(ws.path());
    normalize::run(
        Some("custom.md"),
        true,
        false,
        Some(schema_str.to_str().unwrap()),
        ws.path(),
        ws.path(),
    );

    let after = read_md(ws.path(), "custom.md");
    assert!(
        after.contains("custom_engine_field") && after.contains("important-value"),
        "field preservation: unknown field must survive normalize; content: {after}"
    );
    assert!(
        after.contains("another_field"),
        "field preservation: another_field must survive normalize; content: {after}"
    );
}

/// Field preservation through migrate: unknown fields must survive migrate.
#[test]
fn replay_field_preservation_through_migrate() {
    let ws = make_workspace();
    write_schema(ws.path());

    let content = "---\ntitle: Doc\ntype: meta\nengine: anchor\nstatus: active\ncreated: 2026-04-29\nupdated: 2026-04-29\nmy_custom_field: preserved-value\n---\n# Body\n";
    write_md(ws.path(), "doc.md", content);

    migrate::run(Some("doc.md"), 1, true, ws.path(), ws.path());

    let after = read_md(ws.path(), "doc.md");
    assert!(
        after.contains("my_custom_field") && after.contains("preserved-value"),
        "field preservation: custom field must survive migrate; content: {after}"
    );
    assert!(
        after.contains("schema_version: 1"),
        "schema_version must be added; content: {after}"
    );
}

// ─── Idempotence ──────────────────────────────────────────────────────────────

/// normalize is idempotent: running it twice produces the same output.
#[test]
fn normalize_idempotent() {
    let ws = make_workspace();
    write_schema(ws.path());

    let content = "---\ntitle: Test\ntype: gap\nengine: anchor\nstatus: consumed\nschema_version: 1\ncreated: 2026-04-29\nupdated: 2026-04-29\nid: GAP-X1\npriority: P1\nmaturity: PLANNED\nboundary: open\n---\n# Body\n";
    write_md(ws.path(), "idempotent.md", content);

    let schema_str = schema_path(ws.path()).to_str().unwrap().to_string();

    normalize::run(
        Some("idempotent.md"),
        true,
        false,
        Some(&schema_str),
        ws.path(),
        ws.path(),
    );
    let after_first = read_md(ws.path(), "idempotent.md");

    normalize::run(
        Some("idempotent.md"),
        true,
        false,
        Some(&schema_str),
        ws.path(),
        ws.path(),
    );
    let after_second = read_md(ws.path(), "idempotent.md");

    assert_eq!(
        after_first, after_second,
        "normalize must be idempotent: second run must produce zero diff"
    );
}
