// src/cli/frontmatter/add_required.rs — anchor frontmatter add-required
//
// Fills in type-conditional required fields that are absent but deterministically fillable.
// Non-deterministic fields (id:, entry_point:) are gap-reported, never auto-generated.
//
// Exit codes: 0 = success, 1 = error
//
// Rule 13: run_from_env() resolves paths; run() accepts explicit roots for tests.

use super::parser::{
    get_str, has_key, insert_empty_array, insert_i64, insert_str, parse_file, walk_md_files,
    write_atomic,
};
use super::schema::SchemaRules;
use crate::infra::workspace;
use serde_yaml::Value;
use std::path::{Path, PathBuf};

/// Fields that can be filled deterministically in --auto mode with a safe default.
/// Key = field name, Value = default YAML value type to insert.
enum SafeDefault {
    Str(&'static str),
    EmptyArray,
}

fn safe_defaults_for_type(type_val: &str) -> Vec<(&'static str, SafeDefault)> {
    match type_val {
        "analysis" => vec![("depends_on", SafeDefault::EmptyArray)],
        "eval" => vec![("pass_status", SafeDefault::Str("NOT_RUN"))],
        "capability" => vec![],
        "gap" => vec![],
        "index" => vec![],
        "identity" => vec![],
        "workflow" => vec![], // entry_point is non-deterministic
        _ => vec![],
    }
}

/// Fields that are non-deterministic and must be gap-reported rather than auto-filled.
fn non_deterministic_for_type(type_val: &str, schema: &SchemaRules) -> Vec<String> {
    let all_required = schema
        .type_conditionals
        .get(type_val)
        .cloned()
        .unwrap_or_default();
    let safe: Vec<&str> = safe_defaults_for_type(type_val)
        .iter()
        .map(|(f, _)| *f)
        .collect();
    all_required
        .into_iter()
        .filter(|f| !safe.contains(&f.as_str()))
        .collect()
}

/// Run `anchor frontmatter add-required <PATH> [--auto] [--batch]`. Returns exit code.
pub fn run(
    path: &str,
    auto: bool,
    batch: bool,
    schema_path: Option<&str>,
    workspace_root: &Path,
    cwd: &Path,
) -> i32 {
    let target = {
        let cwd_rel = cwd.join(path);
        if cwd_rel.exists() {
            cwd_rel
        } else {
            let ws_rel = workspace_root.join(path);
            if ws_rel.exists() { ws_rel } else { cwd_rel }
        }
    };

    let s_path = schema_path
        .map(|s| cwd.join(s))
        .unwrap_or_else(|| SchemaRules::default_path(workspace_root));

    let schema = match SchemaRules::load(&s_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error loading schema: {e}");
            return 1;
        }
    };

    let files = if batch || target.is_dir() {
        walk_md_files(&target)
    } else {
        vec![target]
    };

    let mut changes: Vec<(PathBuf, Value, String)> = Vec::new();
    let mut gap_reports: Vec<(PathBuf, Vec<String>)> = Vec::new();
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

        let fm = match parsed.frontmatter {
            None => continue,
            Some(fm) => fm,
        };

        let type_val = match get_str(&fm, "type") {
            Some(t) => t.to_string(),
            None => continue,
        };

        if !auto {
            eprintln!("hint: use --auto to apply safe defaults, or --interactive for prompts");
            return 1;
        }

        let (new_fm, gaps) = apply_auto_defaults(fm, &type_val, &schema, file_path);
        if !gaps.is_empty() {
            gap_reports.push((file_path.clone(), gaps));
        }
        changes.push((file_path.clone(), new_fm, parsed.body.clone()));
    }

    if errors > 0 {
        eprintln!("error: {errors} file(s) could not be read — aborting");
        return 1;
    }

    // Print gap reports
    if !gap_reports.is_empty() {
        println!("GAP REPORT — fields requiring human judgment (not auto-filled):");
        for (path, gaps) in &gap_reports {
            println!("  {}", path.display());
            for g in gaps {
                println!("    '{g}' — non-deterministic; provide manually");
            }
        }
        println!();
    }

    // Write changes
    let mut write_errors = 0usize;
    for (path, fm, body) in &changes {
        if let Err(e) = write_atomic(path, fm, body) {
            eprintln!("error writing {}: {e}", path.display());
            write_errors += 1;
        }
    }

    if write_errors > 0 {
        eprintln!("error: {write_errors} file(s) failed to write");
        return 1;
    }

    println!("✓ add-required: updated {} file(s).", changes.len());
    if !gap_reports.is_empty() {
        println!(
            "  {} file(s) have non-deterministic fields requiring manual entry.",
            gap_reports.len()
        );
    }
    0
}

/// Apply safe auto-defaults to `fm`. Returns (new_fm, gap_field_names).
pub fn apply_auto_defaults(
    mut fm: Value,
    type_val: &str,
    schema: &SchemaRules,
    _path: &Path,
) -> (Value, Vec<String>) {
    // Ensure schema_version: 1 if absent
    if !has_key(&fm, "schema_version") {
        insert_i64(&mut fm, "schema_version", 1);
    }

    let safe = safe_defaults_for_type(type_val);
    for (field, default) in safe {
        if !has_key(&fm, field) {
            match default {
                SafeDefault::Str(v) => insert_str(&mut fm, field, v),
                SafeDefault::EmptyArray => insert_empty_array(&mut fm, field),
            }
        }
    }

    let gaps = non_deterministic_for_type(type_val, schema)
        .into_iter()
        .filter(|f| !has_key(&fm, f))
        .collect();

    (fm, gaps)
}

pub fn run_from_env(
    path: &str,
    auto: bool,
    batch: bool,
    schema_path: Option<&str>,
) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| workspace_root.clone());
    run(path, auto, batch, schema_path, &workspace_root, &cwd)
}
