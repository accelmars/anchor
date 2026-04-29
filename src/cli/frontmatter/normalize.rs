// src/cli/frontmatter/normalize.rs — anchor frontmatter normalize
//
// Deterministic normalization:
//   - Map non-canonical status values via x-synonyms table from schema
//   - Add schema_version: 1 if absent
//   - Reorder keys to canonical order (--reorder flag)
//
// Idempotent: running twice produces zero diff on second run.
//
// Exit codes: 0 = success (including no-change), 1 = error
//
// Rule 13: run_from_env() resolves paths; run() accepts explicit roots for tests.

use super::parser::{parse_file, walk_md_files, write_atomic};
use super::schema::SchemaRules;
use crate::infra::workspace;
use serde_yaml::{Mapping, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Run `anchor frontmatter normalize [PATH] [--apply] [--reorder]`. Returns exit code.
pub fn run(
    path: Option<&str>,
    apply: bool,
    reorder: bool,
    schema_path: Option<&str>,
    workspace_root: &Path,
    cwd: &Path,
) -> i32 {
    let target = resolve_path(path, cwd, workspace_root);

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

    let files = if target.is_file() { vec![target] } else { walk_md_files(&target) };

    // Collect-then-commit: compute all transforms before writing
    let mut changes: Vec<(PathBuf, Value, String)> = Vec::new();
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
            None => continue, // no frontmatter: normalize does not add one
            Some(fm) => fm,
        };

        let new_fm = normalize_fm(fm, &schema, reorder);
        changes.push((file_path.clone(), new_fm, parsed.body.clone()));
    }

    if errors > 0 {
        eprintln!("error: {errors} file(s) could not be read — aborting");
        return 1;
    }

    if changes.is_empty() {
        println!("No files to normalize.");
        return 0;
    }

    if !apply {
        println!(
            "DRY RUN — {} file(s) would be normalized.",
            changes.len()
        );
        println!("(run with --apply to write changes)");
        for (path, _, _) in &changes {
            println!("  {}", path.display());
        }
        return 0;
    }

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

    println!("✓ Normalized {} file(s).", changes.len());
    0
}

/// Apply normalization transforms to a YAML value.
///
/// Idempotent: repeated application produces the same result.
pub fn normalize_fm(fm: Value, schema: &SchemaRules, reorder: bool) -> Value {
    let mut fm = apply_synonyms(fm, schema);
    fm = ensure_schema_version(fm);
    if reorder {
        fm = reorder_keys(fm, &schema.canonical_key_order);
    }
    fm
}

/// Apply status synonym normalization from schema x-synonyms table.
fn apply_synonyms(fm: Value, schema: &SchemaRules) -> Value {
    let Value::Mapping(mut map) = fm else { return fm };

    for (field, synonym_table) in &schema.synonyms {
        let key = Value::String(field.clone());
        if let Some(val) = map.get(&key).cloned() {
            if let Some(current) = val.as_str() {
                if let Some(canonical) = synonym_table.get(current) {
                    map.insert(key, Value::String(canonical.clone()));
                }
            }
        }
    }
    Value::Mapping(map)
}

/// Add schema_version: 1 if absent.
fn ensure_schema_version(fm: Value) -> Value {
    let Value::Mapping(mut map) = fm else { return fm };

    let sv_key = Value::String("schema_version".to_string());
    if map.get(&sv_key).is_none() {
        map.insert(sv_key, Value::Number(serde_yaml::Number::from(1i64)));
    }
    Value::Mapping(map)
}

/// Reorder mapping keys to canonical order; unknown keys appended alphabetically.
pub fn reorder_keys(fm: Value, canonical_order: &[String]) -> Value {
    let Value::Mapping(map) = fm else { return fm };

    let canonical_set: HashSet<&str> = canonical_order.iter().map(|s| s.as_str()).collect();
    let mut new_map = Mapping::new();

    // First: insert keys in canonical order
    for key_str in canonical_order {
        let key = Value::String(key_str.clone());
        if let Some(val) = map.get(&key) {
            new_map.insert(key, val.clone());
        }
    }

    // Then: append remaining keys alphabetically
    let mut extras: Vec<(String, Value)> = map
        .iter()
        .filter_map(|(k, v)| {
            k.as_str()
                .filter(|s| !canonical_set.contains(*s))
                .map(|s| (s.to_string(), v.clone()))
        })
        .collect();
    extras.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in extras {
        new_map.insert(Value::String(k), v);
    }

    Value::Mapping(new_map)
}

fn resolve_path(path: Option<&str>, cwd: &Path, workspace_root: &Path) -> PathBuf {
    match path {
        None => cwd.to_path_buf(),
        Some(p) => {
            let cwd_rel = cwd.join(p);
            if cwd_rel.exists() { cwd_rel } else {
                let ws_rel = workspace_root.join(p);
                if ws_rel.exists() { ws_rel } else { cwd_rel }
            }
        }
    }
}

pub fn run_from_env(
    path: Option<&str>,
    apply: bool,
    reorder: bool,
    schema_path: Option<&str>,
) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| workspace_root.clone());
    run(path, apply, reorder, schema_path, &workspace_root, &cwd)
}
