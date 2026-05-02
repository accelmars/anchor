// src/cli/frontmatter/migrate.rs — anchor frontmatter migrate
//
// Applies schema_version transitions across a path.
//
// Exit codes: 0 = success, 1 = error
//
// FM-008 (rollback): uses collect-then-commit. All transforms computed in memory;
// only written to disk if ALL succeed. On any error: nothing is written.
//
// Rule 13: run_from_env() resolves paths from environment; run() accepts explicit roots.

use super::parser::{get_i64, has_key, parse_file, walk_md_files, write_atomic_str};
use crate::infra::workspace;
use std::path::{Path, PathBuf};

/// Computed transform for one file.
struct Transform {
    path: PathBuf,
    new_raw_fm: String,
    body: String,
    /// Some(title) for scaffold transforms; None for migrations.
    inferred_title: Option<String>,
}

/// Run `anchor frontmatter migrate --to N [PATH] [--apply]`. Returns exit code.
pub fn run(
    path: Option<&str>,
    to_version: u32,
    apply: bool,
    workspace_root: &Path,
    cwd: &Path,
) -> i32 {
    let target = resolve_path(path, cwd, workspace_root);

    let files = if target.is_file() {
        vec![target]
    } else {
        walk_md_files(&target)
    };

    // Collect transforms
    let mut transforms = Vec::new();
    let mut errors = 0usize;

    for file_path in &files {
        let parsed = match parse_file(file_path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error reading {}: {e}", file_path.display());
                errors += 1;
                continue;
            }
        };

        match parsed.raw_fm {
            Some(ref raw_fm) => {
                if let Some(new_fm) = compute_migration(raw_fm, &parsed.frontmatter, to_version) {
                    transforms.push(Transform {
                        path: file_path.clone(),
                        new_raw_fm: new_fm,
                        body: parsed.body.clone(),
                        inferred_title: None,
                    });
                }
            }
            None => {
                // FM-001: no frontmatter → scaffold inserted for --to 1; skip otherwise
                if to_version == 1 {
                    let title = infer_title(file_path, &parsed.body);
                    transforms.push(Transform {
                        path: file_path.clone(),
                        new_raw_fm: scaffold_frontmatter(file_path, &parsed.body),
                        body: parsed.body.clone(),
                        inferred_title: Some(title),
                    });
                }
            }
        }
    }

    if errors > 0 {
        eprintln!("error: {errors} file(s) could not be read — aborting");
        return 1;
    }

    if transforms.is_empty() {
        println!("No migrations needed (all files at target schema_version {to_version}).");
        return 0;
    }

    if !apply {
        // Dry-run: show what would change
        println!(
            "DRY RUN — {} file(s) would be migrated to schema_version: {to_version}",
            transforms.len()
        );
        println!("(run with --apply to write changes)");
        for t in &transforms {
            if let Some(ref title) = t.inferred_title {
                println!(
                    "  [scaffold] {}  →  schema_version: 1, title: \"{title}\"",
                    t.path.display()
                );
            } else {
                println!("  {}", t.path.display());
            }
        }
        return 0;
    }

    // FM-008: commit all or none — write only after all transforms are ready
    let mut write_errors = 0usize;
    for t in &transforms {
        if let Err(e) = write_atomic_str(&t.path, &t.new_raw_fm, &t.body) {
            eprintln!("error writing {}: {e}", t.path.display());
            write_errors += 1;
        }
    }

    if write_errors > 0 {
        eprintln!("error: {write_errors} file(s) failed to write");
        return 1;
    }

    println!(
        "✓ Migrated {} file(s) to schema_version: {to_version}.",
        transforms.len()
    );
    0
}

/// Compute the new raw_fm string for a migration, or None if no change is needed.
///
/// v0 → v1: insert `schema_version: 1` if absent.
fn compute_migration(
    raw_fm: &str,
    fm_value: &Option<serde_yaml::Value>,
    to_version: u32,
) -> Option<String> {
    if to_version == 1 {
        // Check if schema_version already present
        let already_has = fm_value
            .as_ref()
            .map(|fm| has_key(fm, "schema_version"))
            .unwrap_or(false);
        let existing_version = fm_value
            .as_ref()
            .and_then(|fm| get_i64(fm, "schema_version"))
            .unwrap_or(0);

        if already_has && existing_version >= 1 {
            return None; // already at v1+
        }

        // Insert schema_version: 1 after the `updated:` or `created:` line if present,
        // otherwise after the last line.
        Some(insert_schema_version(raw_fm, 1))
    } else {
        None // future versions: no-op until transforms are registered
    }
}

/// Insert `schema_version: N` into raw YAML frontmatter string.
///
/// Inserts after the `updated:` line if present, else after `created:`, else at end.
fn insert_schema_version(raw_fm: &str, version: u32) -> String {
    let new_line = format!("schema_version: {version}");

    // If already present (e.g. version=0 or malformed), replace in-place
    if raw_fm.contains("schema_version:") {
        return raw_fm
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("schema_version:") {
                    new_line.clone()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut result: Vec<String> = raw_fm.lines().map(str::to_string).collect();
    // Find insert position: after `updated:`, or after `created:`, or at end
    let insert_after = result
        .iter()
        .rposition(|l| l.starts_with("updated:"))
        .or_else(|| result.iter().rposition(|l| l.starts_with("created:")))
        .map(|i| i + 1)
        .unwrap_or(result.len());

    result.insert(insert_after, new_line);
    result.join("\n")
}

/// Infer a display title from the file body or filename stem.
///
/// Extracts the first `# Heading`; falls back to filename stem (dashes → spaces, title-cased).
fn infer_title(file_path: &Path, body: &str) -> String {
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("# ") {
            let title = heading.trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    to_title_case(&stem.replace('-', " "))
}

/// Capitalize the first letter of each whitespace-delimited word.
fn to_title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the raw YAML scaffold block (no --- delimiters) for a file with no frontmatter.
fn scaffold_frontmatter(file_path: &Path, body: &str) -> String {
    let title = infer_title(file_path, body);
    let escaped = title.replace('"', "\\\"");
    format!("schema_version: 1\ntitle: \"{escaped}\"\n")
}

fn resolve_path(path: Option<&str>, cwd: &Path, workspace_root: &Path) -> PathBuf {
    match path {
        None => cwd.to_path_buf(),
        Some(p) => {
            let cwd_rel = cwd.join(p);
            if cwd_rel.exists() {
                cwd_rel
            } else {
                let ws_rel = workspace_root.join(p);
                if ws_rel.exists() {
                    ws_rel
                } else {
                    cwd_rel
                }
            }
        }
    }
}

pub fn run_from_env(path: Option<&str>, to_version: u32, apply: bool) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| workspace_root.clone());
    run(path, to_version, apply, &workspace_root, &cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_schema_version_at_end_when_no_anchor() {
        let raw = "title: Test\ntype: gap\n";
        let result = insert_schema_version(raw, 1);
        assert!(result.contains("schema_version: 1"), "result: {result}");
    }

    #[test]
    fn insert_schema_version_after_updated() {
        let raw = "title: Test\nupdated: 2026-04-29\ntype: gap\n";
        let result = insert_schema_version(raw, 1);
        let lines: Vec<&str> = result.lines().collect();
        let updated_pos = lines
            .iter()
            .position(|l| l.starts_with("updated:"))
            .unwrap();
        let sv_pos = lines
            .iter()
            .position(|l| l.starts_with("schema_version:"))
            .unwrap();
        assert_eq!(
            sv_pos,
            updated_pos + 1,
            "schema_version should follow updated; lines: {lines:?}"
        );
    }

    #[test]
    fn replace_existing_schema_version_zero() {
        let raw = "title: Test\nschema_version: 0\ntype: gap\n";
        let result = insert_schema_version(raw, 1);
        assert!(result.contains("schema_version: 1"), "result: {result}");
        assert!(
            !result.contains("schema_version: 0"),
            "old version must be replaced; result: {result}"
        );
    }

    #[test]
    fn scaffold_frontmatter_heading_present() {
        let body = "# My Gap Document\n\nSome content here.\n";
        let result = scaffold_frontmatter(Path::new("some-file.md"), body);
        assert!(result.contains("schema_version: 1"), "result: {result}");
        assert!(
            result.contains("title: \"My Gap Document\""),
            "result: {result}"
        );
        assert!(
            !result.contains("---"),
            "raw_fm must not contain delimiters; result: {result}"
        );
    }

    #[test]
    fn scaffold_frontmatter_no_heading_filename_fallback() {
        let body = "Some content without a heading.\n";
        let result = scaffold_frontmatter(Path::new("my-gap-file.md"), body);
        assert!(result.contains("schema_version: 1"), "result: {result}");
        assert!(
            result.contains("title: \"My Gap File\""),
            "result: {result}"
        );
    }

    #[test]
    fn existing_v0_frontmatter_migrates_not_scaffolded() {
        let raw_fm = "title: Test\ntype: gap\n";
        let fm: serde_yaml::Value = serde_yaml::from_str(raw_fm).unwrap();
        let result = compute_migration(raw_fm, &Some(fm), 1);
        assert!(
            result.is_some(),
            "v0 frontmatter should produce a migration"
        );
        let new_fm = result.unwrap();
        assert!(new_fm.contains("schema_version: 1"), "new_fm: {new_fm}");
        assert!(
            !new_fm.contains("---"),
            "raw_fm should not contain delimiters; new_fm: {new_fm}"
        );
    }

    #[test]
    fn dry_run_does_not_write_unfrontmattered_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.md");
        let original = "# Test\n\nBody content.\n";
        std::fs::write(&file_path, original).unwrap();

        let exit_code = run(Some("test.md"), 1, false, dir.path(), dir.path());

        assert_eq!(exit_code, 0);
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, original, "dry-run must not modify files");
    }
}
