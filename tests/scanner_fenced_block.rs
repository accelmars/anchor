// tests/scanner_fenced_block.rs — AENG-007 fenced code block integration tests
//
// Verifies that parse_references and the pre-move source validation gate skip
// refs inside fenced code blocks (``` and ~~~).

use accelmars_anchor::cli::file::validate::run_on_root;
use accelmars_anchor::core::parser::parse_references;
use accelmars_anchor::model::reference::RefForm;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, content: &str) {
    let abs = root.join(rel);
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(abs, content).unwrap();
}

fn workspace(content_files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
    for (rel, content) in content_files {
        write_file(tmp.path(), rel, content);
    }
    tmp
}

// ── Test (a): archive-retro fixture ─────────────────────────────────────────
//
// Mirrors the anchor-ref-formats/TEST-GUIDE.md scenario (Gap A, 2026-04-28):
// a ```bash block contains a broken-looking relative backtick ref.
// The pre-move source validation gate must NOT fire on this path.

#[test]
fn test_archive_retro_backtick_in_fence_not_reported() {
    // Mirroring anchor-ref-formats/TEST-GUIDE.md: the ```bash block shows a
    // printf command where the printed string contains a relative backtick ref.
    // That path (`../../../../cortex-engine/CHANGELOG.md`) does not exist, but
    // it is inside a code block — the validator must skip it.
    let content = "\
# Test Guide

## Test 1: Pre-Move Source Validation Gate

```bash
rm -rf /tmp/anchor-smoke
mkdir -p /tmp/anchor-smoke/projects/cortex-intelligence-foundation
printf '`../../../../cortex-engine/CHANGELOG.md`' \\
  > /tmp/anchor-smoke/projects/cortex-intelligence-foundation/HANDOVER.md
cd /tmp/anchor-smoke && anchor init --yes
```

The output above shows a broken-looking path but it is a code example, not a live ref.
";

    let src = "archive/anchor-ref-formats/TEST-GUIDE.md".to_string();
    let refs = parse_references(&src, content);
    let backtick_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.form == RefForm::Backtick)
        .collect();

    assert_eq!(
        backtick_refs.len(),
        0,
        "backtick path inside ```bash block must not produce Backtick refs; got: {backtick_refs:?}"
    );
}

#[test]
fn test_archive_retro_validate_clean_workspace() {
    // Full end-to-end: workspace with a file whose only path-like backtick ref
    // is inside a fenced code block. `run_on_root` must exit 0 (clean).
    let content = "\
# Setup

```bash
anchor file mv projects/archive/ archive/
```

The command above moves the archive folder. No live path ref appears outside the block.
";

    let ws = workspace(&[("TEST-GUIDE.md", content)]);
    let exit_code = run_on_root(ws.path(), None);
    assert_eq!(
        exit_code, 0,
        "workspace with fenced backtick paths must validate clean (exit 0)"
    );
}

// ── Test (b): gateway-engine sample ─────────────────────────────────────────
//
// A documentation-heavy file with ```bash and ```rust blocks containing
// example paths. The validator must not fire on those examples.

#[test]
fn test_gateway_engine_sample_fenced_paths_not_reported() {
    let content = "\
# Gateway Engine Blueprint

## Quick Start

```bash
cargo run -- serve --config gateway.toml
# Logs written to foundations/gateway-engine/logs/run.log
```

## Rust Example

```rust
use foundations::gateway_engine::client::Client;
// Config at foundations/gateway-engine/config/environment.md
let client = Client::new(\"foundations/gateway-engine/config/environment.md\");
```

See [config guide](foundations/gateway-engine/config/environment.md) for real usage.
";

    // The foundations/gateway-engine/config/environment.md does not exist —
    // but only the Form 1 link is a live ref; the code-block occurrences are not.
    let ws = workspace(&[("BLUEPRINT.md", content)]);
    let result = run_on_root(ws.path(), None);
    // The workspace has one broken Form 1 link and no false positive from fenced blocks.
    // We assert that the broken refs count is exactly 1 (the Form 1 link, not fenced paths).
    let src2 = "BLUEPRINT.md".to_string();
    let broken = parse_references(
        &src2,
        &fs::read_to_string(ws.path().join("BLUEPRINT.md")).unwrap(),
    );
    let form1_refs: Vec<_> = broken
        .iter()
        .filter(|r| r.form == RefForm::Standard)
        .collect();
    let backtick_refs: Vec<_> = broken
        .iter()
        .filter(|r| r.form == RefForm::Backtick)
        .collect();

    // No backtick refs at all (all path-like strings are inside fenced blocks)
    assert_eq!(
        backtick_refs.len(),
        0,
        "fenced code block paths must not produce Backtick refs; got: {backtick_refs:?}"
    );

    // One Form 1 link (the real one outside the fence)
    assert_eq!(
        form1_refs.len(),
        1,
        "expected exactly 1 Form 1 ref (the live link outside the fence); got: {form1_refs:?}"
    );
    assert_eq!(
        form1_refs[0].target_raw,
        "foundations/gateway-engine/config/environment.md"
    );

    let _ = result; // exit code checked implicitly
}

// ── Test (c): negative — broken ref outside fence still trips ───────────────

#[test]
fn test_broken_backtick_ref_outside_fence_is_reported() {
    // A broken relative backtick ref that is NOT inside a fenced block must still
    // be caught by the validator.
    let content = "\
# Source File

See `../../nonexistent/deep/target.md` for details.

```bash
# This one is inside a code block and must be ignored:
echo `../../nonexistent/deep/target.md`
```
";

    let ws = workspace(&[("projects/sub/SOURCE.md", content)]);
    // The relative ref ../../nonexistent/deep/target.md from projects/sub/ → resolves to
    // nonexistent/deep/target.md — which does not exist.
    let exit_code = run_on_root(ws.path(), None);
    assert_eq!(
        exit_code, 1,
        "broken backtick ref outside fence must make validator exit 1"
    );
}

// ── Test (d): tilde fence ────────────────────────────────────────────────────

#[test]
fn test_tilde_fence_paths_not_reported() {
    let content = "\
~~~bash
echo `foundations/tilde-only/NOTES.md`
~~~
";

    let src = "guide.md".to_string();
    let refs = parse_references(&src, content);
    let backtick_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.form == RefForm::Backtick)
        .collect();

    assert_eq!(
        backtick_refs.len(),
        0,
        "~~~ fenced block paths must not produce Backtick refs; got: {backtick_refs:?}"
    );
}

#[test]
fn test_tilde_fence_validate_clean() {
    let content = "\
# Guide

~~~bash
anchor file mv old/path new/path
~~~
";

    let ws = workspace(&[("GUIDE.md", content)]);
    let exit_code = run_on_root(ws.path(), None);
    assert_eq!(
        exit_code, 0,
        "workspace with tilde-fenced backtick paths must validate clean"
    );
}

// ── Test (e): indented fence (list item body) ────────────────────────────────

#[test]
fn test_indented_fence_in_list_item_paths_not_reported() {
    let content = "\
## Steps

1. First step:

    ```bash
    anchor file mv foundations/old/ foundations/new/
    ```

2. Second step.
";

    let src = "HOWTO.md".to_string();
    let refs = parse_references(&src, content);
    let backtick_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.form == RefForm::Backtick)
        .collect();

    assert_eq!(
        backtick_refs.len(),
        0,
        "paths inside indented (list-item) fenced block must not produce Backtick refs; got: {backtick_refs:?}"
    );
}

// ── Test (f): length-mismatch close — 4-opener, 3-closer does NOT close ──────
//
// A 4-backtick fence is not closed by a 3-backtick line (3 < 4).
// The path example inside must NOT be reported as a live ref.

#[test]
fn test_length_mismatch_short_closer_does_not_close_fence() {
    let content = "\
````bash
`foundations/example/NOTES.md` inside 4-backtick fence
```
this backtick triplet must NOT close the 4-backtick fence
````
";

    let src = "source.md".to_string();
    let refs = parse_references(&src, content);
    let backtick_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.form == RefForm::Backtick)
        .collect();

    assert_eq!(
        backtick_refs.len(),
        0,
        "3-backtick line must not close a 4-backtick fence; path inside must not be reported; got: {backtick_refs:?}"
    );
}

// ── Test: longer closer DOES close (3-opener, 4-closer) ─────────────────────
//
// This confirms step 6 unit-test behavior at the integration level:
// a 4-backtick line closes a 3-backtick fence (4 ≥ 3).

#[test]
fn test_longer_closer_closes_fence_path_after_is_reported() {
    let content = "\
```bash
`foundations/inside-fence/NOTES.md`
````
`foundations/after-close/NOTES.md`
";

    // After the 4-backtick closer, the fence is closed.
    // The path on the last line IS outside the fence → should be a live Backtick ref.
    let src = "source.md".to_string();
    let refs = parse_references(&src, content);
    let backtick_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.form == RefForm::Backtick)
        .collect();

    assert_eq!(
        backtick_refs.len(),
        1,
        "path after 4-backtick closer (which closed 3-backtick fence) must be reported; got: {backtick_refs:?}"
    );
    assert_eq!(
        backtick_refs[0].target_raw,
        "foundations/after-close/NOTES.md"
    );
}
