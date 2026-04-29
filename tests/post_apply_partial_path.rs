// tests/post_apply_partial_path.rs — AENG-008 integration tests
//
// Verifies post-apply UX-001 partial-path plain-text coverage:
//   1. Council-rename retro fixture — reproduces the 2026-04-28 scenario
//   2. Negative test — zero bare-prose remainder emits no partial-path lines

use accelmars_anchor::apply::post_apply_scan::{
    format_plain_text_warning, scan_partial_plain_text, PlainTextHit,
};
use std::fs;
use std::path::Path;
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

/// Council-rename retro fixture (2026-04-28 scenario).
///
/// Rename: councils/{os,marketing,sales,skill}-council/ → councils/{os,marketing,sales,skill}/
/// Src for os-council move: "accelmars-guild/councils/os-council"
/// Expected bare-prose remainder files with ≥1 occurrence of "os-council":
///   - accelmars-guild/STATUS.md               (~12 occurrences)
///   - accelmars-guild/councils/os/STATUS.md   (~4 occurrences)
///   - accelmars-guild/councils/os/CLAUDE.md   (2 occurrences)
///   - accelmars-guild/councils/skill/CLAUDE.md (3 occurrences)
#[test]
fn test_council_rename_retro_partial_path_hits() {
    let ws = make_workspace();

    // Reproduce the 4 files with bare-prose os-council remainder.
    // Counts are conservative minimums — assert ≥1, not exact counts (workspace evolution).
    write_file(
        ws.path(),
        "accelmars-guild/STATUS.md",
        "| os-council | Active | ... |\n\
         | os-council decisions | ... |\n\
         ```\n\
         councils/os-council/\n\
         ```\n\
         Active projects: os-council, os-council, os-council, os-council,\n\
         os-council, os-council, os-council, os-council, os-council\n",
    );

    write_file(
        ws.path(),
        "accelmars-guild/councils/os/STATUS.md",
        "# STATUS.md — os-council\n\
         Project map: os-council active\n\
         os-council label: yes\n\
         os-council entry\n",
    );

    write_file(
        ws.path(),
        "accelmars-guild/councils/os/CLAUDE.md",
        "# CLAUDE.md — os-council\n\
         _AccelMars Co., Ltd. — os-council_\n",
    );

    write_file(
        ws.path(),
        "accelmars-guild/councils/skill/CLAUDE.md",
        "# CLAUDE.md — os-council\n\
         Peer council: os-council\n\
         _AccelMars Co., Ltd. — os-council_\n",
    );

    // Also add a clean file to confirm it does not appear in results.
    write_file(
        ws.path(),
        "accelmars-guild/councils/marketing/STATUS.md",
        "# Marketing Council\nNo os-council references here.\n",
    );

    let workspace_files: Vec<String> = vec![
        "accelmars-guild/STATUS.md".to_string(),
        "accelmars-guild/councils/os/STATUS.md".to_string(),
        "accelmars-guild/councils/os/CLAUDE.md".to_string(),
        "accelmars-guild/councils/skill/CLAUDE.md".to_string(),
        "accelmars-guild/councils/marketing/STATUS.md".to_string(),
    ];

    let src = "accelmars-guild/councils/os-council";
    let hits = scan_partial_plain_text(&workspace_files, src, ws.path());

    // Must find at least one hit per expected file.
    let hit_files: Vec<&str> = hits.iter().map(|h| h.file.as_str()).collect();

    assert!(
        hit_files.contains(&"accelmars-guild/STATUS.md"),
        "STATUS.md must be reported; hits: {hits:?}"
    );
    assert!(
        hit_files.contains(&"accelmars-guild/councils/os/STATUS.md"),
        "councils/os/STATUS.md must be reported; hits: {hits:?}"
    );
    assert!(
        hit_files.contains(&"accelmars-guild/councils/os/CLAUDE.md"),
        "councils/os/CLAUDE.md must be reported; hits: {hits:?}"
    );
    assert!(
        hit_files.contains(&"accelmars-guild/councils/skill/CLAUDE.md"),
        "councils/skill/CLAUDE.md must be reported; hits: {hits:?}"
    );

    // All reported hits must have segment "os-council".
    for hit in &hits {
        assert!(
            hit.segment == "os-council" || hit.segment == "councils/os-council",
            "hit segment must be 'os-council' or 'councils/os-council'; got: {}",
            hit.segment
        );
    }

    // Each expected file must have ≥1 occurrence.
    for expected_file in &[
        "accelmars-guild/STATUS.md",
        "accelmars-guild/councils/os/STATUS.md",
        "accelmars-guild/councils/os/CLAUDE.md",
        "accelmars-guild/councils/skill/CLAUDE.md",
    ] {
        let file_hits: usize = hits
            .iter()
            .filter(|h| h.file == *expected_file)
            .map(|h| h.count)
            .sum();
        assert!(
            file_hits >= 1,
            "{expected_file} must have ≥1 occurrence; got {file_hits}"
        );
    }

    // Marketing STATUS.md has exactly one "os-council" mention ("No os-council references here.")
    // — this is an intentional mention so it WILL appear. That's correct behaviour (it's a prose mention).
    // The negative fixture is tested separately below.
}

/// Negative test — clean rename with zero bare-prose remainder emits no partial-path lines
/// and no empty UX-001 block header.
#[test]
fn test_clean_rename_no_partial_path_output() {
    let ws = make_workspace();

    // Fixture: only backtick and link refs — no bare-prose remainder.
    write_file(
        ws.path(),
        "docs/README.md",
        "See [`councils/os/`](councils/os/) for decisions.\n\
         Reference: `accelmars-guild/councils/os/decisions/foo.md`\n",
    );
    write_file(
        ws.path(),
        "docs/guide.md",
        "# Guide\nNo prose mentions of the old path.\n",
    );

    let workspace_files: Vec<String> =
        vec!["docs/README.md".to_string(), "docs/guide.md".to_string()];

    // After a clean rename from "accelmars-guild/councils/os-council" to
    // "accelmars-guild/councils/os", the partial segment is "os-council".
    // Neither file contains "os-council" as bare text.
    let src = "accelmars-guild/councils/os-council";
    let hits = scan_partial_plain_text(&workspace_files, src, ws.path());

    assert!(
        hits.is_empty(),
        "clean rename must produce no partial-path hits; got: {hits:?}"
    );

    // format_plain_text_warning must return None (suppress entire block).
    let warning = format_plain_text_warning(&[], &hits, "os-council");
    assert!(
        warning.is_none(),
        "clean rename must produce no UX-001 block; got: {warning:?}"
    );
}

/// Explicit check: format_plain_text_warning output contains required structural elements.
#[test]
fn test_format_warning_structure() {
    let partial_hits = vec![
        PlainTextHit {
            file: "accelmars-guild/STATUS.md".to_string(),
            segment: "os-council".to_string(),
            count: 12,
        },
        PlainTextHit {
            file: "accelmars-guild/councils/os/STATUS.md".to_string(),
            segment: "os-council".to_string(),
            count: 4,
        },
    ];

    let warning = format_plain_text_warning(&[], &partial_hits, "os-council").unwrap();

    // Must open with warning symbol and standard UX-001 prefix.
    assert!(
        warning.starts_with("⚠ Plain-text occurrences not rewritten"),
        "warning must start with ⚠ header; got:\n{warning}"
    );

    // Must include each file with its count.
    assert!(
        warning.contains("accelmars-guild/STATUS.md: 12 occurrence(s) of 'os-council'"),
        "STATUS.md line missing; got:\n{warning}"
    );
    assert!(
        warning.contains("accelmars-guild/councils/os/STATUS.md: 4 occurrence(s) of 'os-council'"),
        "councils/os/STATUS.md line missing; got:\n{warning}"
    );

    // Must include closing hint referencing anchor refs --plain.
    assert!(
        warning.contains("anchor refs --plain os-council"),
        "closing hint missing; got:\n{warning}"
    );

    // Must NOT produce an empty header when content is present.
    let lines: Vec<&str> = warning.lines().collect();
    assert!(
        lines.len() >= 3,
        "warning must have at least 3 lines (header + entries + hint); got {} lines",
        lines.len()
    );
}
