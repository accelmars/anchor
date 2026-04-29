// Integration tests for `anchor file mv`
//
// Exercises the full transaction pipeline: PLAN → APPLY → VALIDATE → COMMIT (or ROLLBACK).
// Tests use the library API directly via accelmars_anchor crate.

use accelmars_anchor::core::{scanner, transaction};
use accelmars_anchor::infra::{lock, temp};
use accelmars_anchor::model::manifest::Manifest;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn make_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
    tmp
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

fn read_file(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

fn file_exists(root: &Path, rel: &str) -> bool {
    root.join(rel).exists()
}

/// Outcome of running the mv pipeline.
enum MvOutcome {
    Success,
    ValidationFailed(Vec<transaction::BrokenRef>),
    Error(String),
}

/// Run the full anchor file mv pipeline (PLAN→APPLY→VALIDATE→COMMIT or ROLLBACK).
fn run_mv(root: &Path, src: &str, dst: &str) -> MvOutcome {
    let src_canonical = src.to_string();
    let dst_canonical = dst.to_string();

    if !root.join(src).exists() {
        return MvOutcome::Error(format!("src not found: {src}"));
    }
    if root.join(dst).exists() {
        return MvOutcome::Error(format!("dst already exists: {dst}"));
    }

    let lock_guard = match lock::acquire_lock(root, &format!("file mv {src} {dst}")) {
        Ok(g) => g,
        Err(e) => return MvOutcome::Error(format!("lock: {e}")),
    };

    let workspace_files = match scanner::scan_workspace(root) {
        Ok(f) => f,
        Err(e) => {
            drop(lock_guard);
            return MvOutcome::Error(format!("scan: {e}"));
        }
    };

    let plan = match transaction::plan(root, &src_canonical, &dst_canonical, &workspace_files) {
        Ok(p) => p,
        Err(e) => {
            drop(lock_guard);
            return MvOutcome::Error(format!("plan: {e}"));
        }
    };

    let op_dir = match temp::create_op_dir(root) {
        Ok(d) => d,
        Err(e) => {
            drop(lock_guard);
            return MvOutcome::Error(format!("temp: {e}"));
        }
    };

    let rewrite_files: Vec<String> = {
        let mut seen = HashSet::new();
        plan.entries
            .iter()
            .filter(|e| seen.insert(e.file.clone()))
            .map(|e| e.file.clone())
            .collect()
    };

    let mut manifest = Manifest {
        op: "file_mv".to_string(),
        src: src_canonical.clone(),
        dst: dst_canonical.clone(),
        rewrites: rewrite_files,
        phase: "PLAN".to_string(),
    };

    if let Err(e) = accelmars_anchor::model::manifest::write_manifest(&op_dir.path, &manifest) {
        drop(lock_guard);
        return MvOutcome::Error(format!("manifest: {e}"));
    }

    if let Err(e) = transaction::apply(root, &plan, &op_dir, &mut manifest) {
        transaction::rollback(&op_dir, lock_guard);
        return MvOutcome::Error(format!("apply: {e}"));
    }

    match transaction::validate(root, &plan, &op_dir) {
        Ok(()) => {}
        Err(transaction::ValidationError::BrokenRefs(broken)) => {
            transaction::rollback(&op_dir, lock_guard);
            return MvOutcome::ValidationFailed(broken);
        }
        Err(transaction::ValidationError::Io(e)) => {
            transaction::rollback(&op_dir, lock_guard);
            return MvOutcome::Error(format!("validate I/O: {e}"));
        }
    }

    if let Err(e) = transaction::commit(root, &plan, &op_dir, &mut manifest, lock_guard) {
        return MvOutcome::Error(format!("commit: {e}"));
    }

    MvOutcome::Success
}

// ─── Test 1: Single file move ─────────────────────────────────────────────────
// a.md references b.md. Move b.md → subdir/b.md.
// Verify: a.md updated, b.md absent, subdir/b.md exists.

#[test]
fn test_single_file_move_case_a_reference_updated() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "b.md", "# B\n");
    write_file(root, "a.md", "[link to b](b.md)\n");

    match run_mv(root, "b.md", "subdir/b.md") {
        MvOutcome::Success => {}
        MvOutcome::ValidationFailed(refs) => panic!("unexpected validation failure: {refs:?}"),
        MvOutcome::Error(e) => panic!("unexpected error: {e}"),
    }

    assert!(
        file_exists(root, "subdir/b.md"),
        "subdir/b.md must exist after move"
    );
    assert!(!file_exists(root, "b.md"), "b.md must be absent after move");

    let a_content = read_file(root, "a.md");
    assert!(
        a_content.contains("subdir/b.md"),
        "a.md must reference subdir/b.md, got:\n{a_content}"
    );
    assert!(
        !a_content.contains("](b.md)"),
        "a.md must not contain old reference ](b.md), got:\n{a_content}"
    );
}

// ─── Test 2: Directory move with Cases A, B, C ────────────────────────────────
// external.md → Case A (references project/internal.md)
// project/internal.md → Case B (references ../people/anna.md)
// project/a.md → project/b.md = Case C (both inside moved dir)

#[test]
fn test_directory_move_cases_a_b_c() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "external.md", "[ref](project/internal.md)\n");
    write_file(root, "project/internal.md", "[anna](../people/anna.md)\n");
    write_file(root, "people/anna.md", "# Anna\n");
    write_file(root, "project/a.md", "[b](b.md)\n");
    write_file(root, "project/b.md", "# B\n");

    match run_mv(root, "project", "archive/project") {
        MvOutcome::Success => {}
        MvOutcome::ValidationFailed(refs) => {
            panic!("unexpected validation failure: {refs:?}")
        }
        MvOutcome::Error(e) => panic!("unexpected error: {e}"),
    }

    // Case A: external.md must reference new path
    let ext = read_file(root, "external.md");
    assert!(
        ext.contains("archive/project/internal.md"),
        "Case A: external.md must reference new path, got:\n{ext}"
    );

    // Case B: internal.md must have updated relative path to people/anna.md
    let internal = read_file(root, "archive/project/internal.md");
    assert!(
        internal.contains("people/anna.md"),
        "Case B: archive/project/internal.md must reference people/anna.md, got:\n{internal}"
    );
    // The old path "../people/anna.md" should be replaced with "../../people/anna.md"
    assert!(
        !internal.contains("](../people/anna.md)"),
        "Case B: old relative path must be rewritten, got:\n{internal}"
    );

    // Case C: a.md must have identical content (b.md ref unchanged)
    let a_content = read_file(root, "archive/project/a.md");
    assert_eq!(
        a_content.trim(),
        "[b](b.md)",
        "Case C: relative ref between moved files must be unchanged, got:\n{a_content}"
    );

    assert!(file_exists(root, "archive/project/a.md"));
    assert!(file_exists(root, "archive/project/b.md"));
    assert!(
        !file_exists(root, "project/a.md"),
        "old project/a.md must not exist"
    );
}

// ─── Test 3: Case C byte-identical after move ─────────────────────────────────

#[test]
fn test_case_c_references_byte_identical() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "project/a.md", "# A\nSee [B](b.md).\n");
    write_file(root, "project/b.md", "# B\n");

    let a_before = read_file(root, "project/a.md");

    match run_mv(root, "project", "archive/project") {
        MvOutcome::Success => {}
        _other => panic!("expected success"),
    }

    let a_after = read_file(root, "archive/project/a.md");
    assert_eq!(
        a_before, a_after,
        "Case C: file content must be byte-identical after directory move"
    );
}

// ─── Test 4: APPLY phase — originals not touched ──────────────────────────────
// Run PLAN+APPLY, then verify originals still have their original byte content.

#[test]
fn test_apply_originals_not_touched() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "b.md", "# B original\n");
    write_file(root, "a.md", "[link](b.md)\n");

    let a_original = read_file(root, "a.md");
    let b_original = read_file(root, "b.md");

    let src = "b.md".to_string();
    let dst = "subdir/b.md".to_string();

    let lock_guard = lock::acquire_lock(root, "file mv b.md subdir/b.md").unwrap();
    let workspace_files = scanner::scan_workspace(root).unwrap();
    let plan = transaction::plan(root, &src, &dst, &workspace_files).unwrap();

    let op_dir = temp::create_op_dir(root).unwrap();
    let mut manifest = Manifest {
        op: "file_mv".to_string(),
        src: src.clone(),
        dst: dst.clone(),
        rewrites: {
            let mut seen = HashSet::new();
            plan.entries
                .iter()
                .filter(|e| seen.insert(e.file.clone()))
                .map(|e| e.file.clone())
                .collect()
        },
        phase: "PLAN".to_string(),
    };
    accelmars_anchor::model::manifest::write_manifest(&op_dir.path, &manifest).unwrap();

    // Run APPLY only
    transaction::apply(root, &plan, &op_dir, &mut manifest).unwrap();

    // Originals must be untouched
    assert_eq!(
        read_file(root, "a.md"),
        a_original,
        "a.md must not be modified during APPLY"
    );
    assert_eq!(
        read_file(root, "b.md"),
        b_original,
        "b.md must not be modified during APPLY"
    );

    // Rewritten copy must be in tmp/rewrites/
    let rewrites_dir = op_dir.path.join("rewrites");
    let rewrite_count = fs::read_dir(&rewrites_dir).unwrap().count();
    assert!(
        rewrite_count > 0,
        "rewrites/ must contain at least one file after APPLY"
    );

    // Cleanup
    transaction::rollback(&op_dir, lock_guard);
}

// ─── Test 5: VALIDATE failure → rollback → workspace unchanged ────────────────
// a.md has a valid Case A ref (b.md) AND a broken ref (nonexistent.md).
// After rewrite, a.md still has the broken nonexistent.md ref → VALIDATE fails.

#[test]
fn test_validate_failure_rollback_workspace_unchanged() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "b.md", "# B\n");
    // a.md has both a valid Case A ref and a pre-existing broken ref
    write_file(root, "a.md", "[valid](b.md)\n[broken](nonexistent.md)\n");

    let a_before = read_file(root, "a.md");
    let b_before = read_file(root, "b.md");

    match run_mv(root, "b.md", "subdir/b.md") {
        MvOutcome::ValidationFailed(_) => {
            // Expected: nonexistent.md causes validation failure
        }
        MvOutcome::Success => {
            panic!(
                "expected validation failure (nonexistent.md is a broken ref in rewritten a.md)"
            );
        }
        MvOutcome::Error(e) => panic!("unexpected error: {e}"),
    }

    // Workspace must be byte-for-byte identical to before
    assert!(file_exists(root, "a.md"), "a.md must exist after rollback");
    assert!(
        file_exists(root, "b.md"),
        "b.md must exist after rollback (not moved)"
    );
    assert!(
        !file_exists(root, "subdir/b.md"),
        "subdir/b.md must not exist after rollback"
    );
    assert_eq!(read_file(root, "a.md"), a_before, "a.md must be unchanged");
    assert_eq!(read_file(root, "b.md"), b_before, "b.md must be unchanged");
}

// ─── Test 6: Wiki-link rewrite: [[old-stem]] → [[new-stem]] ──────────────────

#[test]
fn test_wiki_link_rewrite_stem_change() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "old-decision.md", "# Old Decision\n");
    write_file(root, "notes.md", "See [[old-decision]] for details.\n");

    match run_mv(root, "old-decision.md", "new-decision.md") {
        MvOutcome::Success => {}
        MvOutcome::ValidationFailed(refs) => {
            panic!("unexpected validation failure: {refs:?}")
        }
        MvOutcome::Error(e) => panic!("unexpected error: {e}"),
    }

    assert!(
        file_exists(root, "new-decision.md"),
        "new-decision.md must exist"
    );
    assert!(
        !file_exists(root, "old-decision.md"),
        "old-decision.md must be absent"
    );

    let notes = read_file(root, "notes.md");
    assert!(
        notes.contains("[[new-decision]]"),
        "notes.md must contain [[new-decision]], got:\n{notes}"
    );
    assert!(
        !notes.contains("[[old-decision]]"),
        "notes.md must not contain [[old-decision]], got:\n{notes}"
    );
}

// ─── Test 7: dst already exists → error before PLAN ──────────────────────────

#[test]
fn test_dst_already_exists_error() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "a.md", "# A\n");
    write_file(root, "b.md", "# B\n");

    let a_before = read_file(root, "a.md");

    match run_mv(root, "a.md", "b.md") {
        MvOutcome::Error(msg) => {
            assert!(
                msg.contains("already exists"),
                "error must mention 'already exists', got: {msg}"
            );
        }
        MvOutcome::Success => panic!("expected error when dst exists"),
        MvOutcome::ValidationFailed(_) => panic!("expected error when dst exists"),
    }

    assert_eq!(read_file(root, "a.md"), a_before, "a.md must be unchanged");
}

// ─── Test 9: Consecutive moves both succeed (no stale .accelmars/anchor/tmp/) ─
// Regression test for the bug where cleanup_op_dir left behind an empty tmp/,
// causing the second acquire_lock to return StaleLock.

#[test]
fn test_consecutive_mv_both_succeed() {
    let tmp = make_workspace();
    let root = tmp.path();

    write_file(root, "a.md", "# A\n");
    write_file(root, "b.md", "See [a](a.md).\n");
    write_file(root, "c.md", "# C\n");

    match run_mv(root, "a.md", "moved-a.md") {
        MvOutcome::Success => {}
        MvOutcome::ValidationFailed(refs) => {
            panic!("first mv: unexpected validation failure: {refs:?}")
        }
        MvOutcome::Error(e) => panic!("first mv: unexpected error: {e}"),
    }

    match run_mv(root, "c.md", "moved-c.md") {
        MvOutcome::Success => {}
        MvOutcome::ValidationFailed(refs) => {
            panic!("second mv: unexpected validation failure: {refs:?}")
        }
        MvOutcome::Error(e) => panic!("second mv: unexpected error (stale lock?): {e}"),
    }

    assert!(file_exists(root, "moved-a.md"), "moved-a.md must exist");
    assert!(!file_exists(root, "a.md"), "a.md must be absent");
    assert!(file_exists(root, "moved-c.md"), "moved-c.md must exist");
    assert!(!file_exists(root, "c.md"), "c.md must be absent");
}

// ─── Test 8: src not found → error before PLAN ───────────────────────────────

#[test]
fn test_src_not_found_error() {
    let tmp = make_workspace();
    let root = tmp.path();

    match run_mv(root, "nonexistent.md", "dst.md") {
        MvOutcome::Error(msg) => {
            assert!(
                msg.contains("not found"),
                "error must mention 'not found', got: {msg}"
            );
        }
        MvOutcome::Success => panic!("expected error for nonexistent src"),
        MvOutcome::ValidationFailed(_) => panic!("expected error for nonexistent src"),
    }
}
