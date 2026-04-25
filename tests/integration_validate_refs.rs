// Integration tests for `mind file validate` and `mind file refs` — MF-007
//
// Tests invoke the compiled `mind` binary via subprocess and verify exit codes
// and stdout output. This validates the full CLI pipeline including TTY detection.

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn mind_bin() -> std::path::PathBuf {
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

// ─── validate tests ──────────────────────────────────────────────────────────

/// Test 1: Known-clean workspace — all references resolve.
/// Verify: exit 0; stdout contains "✓" and "No broken references".
#[test]
fn test_validate_clean_workspace() {
    let ws = make_workspace();
    let root = ws.path();
    write_file(root, "a.md", "[link](b.md)");
    write_file(root, "b.md", "# B");

    let output = Command::new(mind_bin())
        .args(["file", "validate"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "exit 0 for clean workspace. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains('✓'), "stdout must contain checkmark");
    assert!(
        stdout.contains("No broken references"),
        "stdout must contain 'No broken references'"
    );
}

/// Test 2: Known-broken workspace — one .md file references a non-existent path.
/// Verify: exit 1; stdout contains "BROKEN REFERENCES (1)" with correct file:line → target (not found) format.
#[test]
fn test_validate_broken_workspace_single() {
    let ws = make_workspace();
    let root = ws.path();
    write_file(root, "a.md", "[link](missing.md)");

    let output = Command::new(mind_bin())
        .args(["file", "validate"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "exit 1 for broken refs. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("BROKEN REFERENCES (1)"),
        "stdout must contain 'BROKEN REFERENCES (1)', got: {stdout}"
    );
    assert!(
        stdout.contains("a.md:1"),
        "stdout must contain file:line reference 'a.md:1', got: {stdout}"
    );
    assert!(
        stdout.contains("→ missing.md"),
        "stdout must contain '→ missing.md', got: {stdout}"
    );
    assert!(
        stdout.contains("(not found)"),
        "stdout must contain '(not found)', got: {stdout}"
    );
}

/// Test 3: Multiple broken references — workspace with 3 broken refs.
/// Verify: stdout contains "3 broken references in".
#[test]
fn test_validate_broken_workspace_multiple() {
    let ws = make_workspace();
    let root = ws.path();
    write_file(
        root,
        "a.md",
        "[one](missing1.md)\n[two](missing2.md)\n[three](missing3.md)",
    );

    let output = Command::new(mind_bin())
        .args(["file", "validate"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "exit 1 for broken refs. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("3 broken references in"),
        "stdout must contain '3 broken references in', got: {stdout}"
    );
}

// ─── refs tests ──────────────────────────────────────────────────────────────

/// Test 4: Known reference — a.md references b.md.
/// Verify: exit 0; stdout contains a.md:N line.
#[test]
fn test_refs_known_reference() {
    let ws = make_workspace();
    let root = ws.path();
    write_file(root, "a.md", "[link](b.md)");
    write_file(root, "b.md", "# B");

    let output = Command::new(mind_bin())
        .args(["file", "refs", "b.md"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "refs always exits 0. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("a.md:1"),
        "stdout must contain 'a.md:1', got: {stdout}"
    );
}

/// Test 5: Zero references — no file references b.md.
/// Verify: exit 0; stdout is exactly "No files reference b.md."
#[test]
fn test_refs_zero_references() {
    let ws = make_workspace();
    let root = ws.path();
    write_file(root, "a.md", "# No references here");
    write_file(root, "b.md", "# B");

    let output = Command::new(mind_bin())
        .args(["file", "refs", "b.md"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "refs exits 0 even with zero refs. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        stdout, "No references found.",
        "output must be exactly 'No references found.'"
    );
}

// ─── TTY tests ───────────────────────────────────────────────────────────────

/// Test 6: Piped output — stdout captured by Command is not a TTY.
/// Verify: no ANSI escape codes in output.
#[test]
fn test_validate_piped_no_ansi_codes() {
    let ws = make_workspace();
    let root = ws.path();
    // Create a broken-ref workspace so the scanning header is also emitted
    write_file(root, "a.md", "[broken](nonexistent.md)");

    let output = Command::new(mind_bin())
        .args(["file", "validate"])
        .current_dir(root)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('\x1b'),
        "piped output must not contain ANSI escape codes (\\x1b), got: {stdout}"
    );
}
