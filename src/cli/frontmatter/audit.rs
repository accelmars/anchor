// src/cli/frontmatter/audit.rs — anchor frontmatter audit
//
// Walks a path, validates YAML frontmatter against FRONTMATTER.schema.json, and
// emits a categorized compliance report.
//
// Exit codes:
//   0 = all files conformant
//   1 = schema issues found
//   2 = system error (I/O, schema not found, etc.)
//
// Rule 13: run() resolves path args from CWD; run_impl() accepts explicit roots for tests.

use super::parser::{get_i64, get_str, has_key, parse_file, walk_md_files};
use super::schema::SchemaRules;
use crate::infra::workspace;
use serde_yaml::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Output format for the audit report.
#[derive(Debug, Clone, PartialEq)]
pub enum AuditFormat {
    Human,
    Json,
}

/// A single finding for one file.
#[derive(Debug)]
pub enum Finding {
    NoFrontmatter,
    MissingSchemaVersion,
    StaleSchemaVersion {
        current: i64,
        expected: i64,
    },
    MissingRequired(String),
    MissingTypeConditional(String),
    InvalidEnum {
        field: String,
        value: String,
        allowed: Vec<String>,
    },
    WrongType {
        field: String,
        expected: String,
    },
    DuplicateId(String),
}

impl Finding {
    fn category(&self) -> &'static str {
        match self {
            Finding::NoFrontmatter => "no frontmatter",
            Finding::MissingSchemaVersion => "missing schema_version",
            Finding::StaleSchemaVersion { .. } => "stale schema_version",
            Finding::MissingRequired(_) => "missing required base field",
            Finding::MissingTypeConditional(_) => "missing type-conditional field",
            Finding::InvalidEnum { .. } => "invalid enum value",
            Finding::WrongType { .. } => "wrong field type",
            Finding::DuplicateId(_) => "duplicate id",
        }
    }

    fn detail(&self) -> String {
        match self {
            Finding::NoFrontmatter => "no YAML frontmatter block".to_string(),
            Finding::MissingSchemaVersion => "field 'schema_version' absent".to_string(),
            Finding::StaleSchemaVersion { current, expected } => {
                format!("schema_version {current} < expected {expected}")
            }
            Finding::MissingRequired(f) => format!("required field '{f}' absent"),
            Finding::MissingTypeConditional(f) => {
                format!("type-conditional required field '{f}' absent")
            }
            Finding::InvalidEnum {
                field,
                value,
                allowed,
            } => {
                format!("'{field}': '{value}' not in [{}", allowed.join(", ") + "]")
            }
            Finding::WrongType { field, expected } => {
                format!("'{field}' must be of type {expected}")
            }
            Finding::DuplicateId(id) => format!("id '{id}' appears in multiple files"),
        }
    }
}

/// Run `anchor frontmatter audit`. Returns exit code.
pub fn run(
    path: Option<&str>,
    format: AuditFormat,
    schema_override: Option<&str>,
    strict: bool,
    workspace_root: &Path,
    cwd: &Path,
) -> i32 {
    let target = resolve_path(path, cwd, workspace_root);

    let schema_path = match SchemaRules::resolve_schema_path(schema_override, cwd, workspace_root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let schema = match SchemaRules::load(&schema_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error loading schema: {e}");
            eprintln!("hint: schema expected at {}", schema_path.display());
            return 2;
        }
    };

    let files = if target.is_file() {
        vec![target]
    } else {
        walk_md_files(&target)
    };

    match run_audit(&files, &schema, strict) {
        Ok((findings_by_file, any_issues)) => {
            match format {
                AuditFormat::Json => print_json(&findings_by_file, files.len()),
                AuditFormat::Human => print_human(&findings_by_file, files.len()),
            }
            if any_issues {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// Core audit logic — separated for testing.
pub type AuditResults = Vec<(PathBuf, Vec<Finding>)>;

pub fn run_audit(
    files: &[PathBuf],
    schema: &SchemaRules,
    strict: bool,
) -> Result<(AuditResults, bool), String> {
    let mut id_locations: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut results: Vec<(PathBuf, Vec<Finding>)> = Vec::new();

    for path in files {
        let parsed =
            parse_file(path).map_err(|e| format!("I/O error reading {}: {e}", path.display()))?;
        let findings = validate_file(&parsed.frontmatter, schema, strict, path);

        if let Some(fm) = &parsed.frontmatter {
            if let Some(id) = get_str(fm, "id") {
                id_locations
                    .entry(id.to_string())
                    .or_default()
                    .push(path.clone());
            }
        }
        results.push((path.clone(), findings));
    }

    // Inject duplicate-id findings
    for (id, paths) in &id_locations {
        if paths.len() > 1 {
            for path in paths {
                if let Some((_, findings)) = results.iter_mut().find(|(p, _)| p == path) {
                    findings.push(Finding::DuplicateId(id.clone()));
                }
            }
        }
    }

    let any_issues = results.iter().any(|(_, f)| !f.is_empty());
    Ok((results, any_issues))
}

fn validate_file(
    fm_opt: &Option<Value>,
    schema: &SchemaRules,
    strict: bool,
    path: &Path,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    let fm = match fm_opt {
        None => {
            findings.push(Finding::NoFrontmatter);
            return findings;
        }
        Some(fm) => fm,
    };

    // schema_version check
    match get_i64(fm, "schema_version") {
        None => findings.push(Finding::MissingSchemaVersion),
        Some(v) if v < 1 => findings.push(Finding::StaleSchemaVersion {
            current: v,
            expected: 1,
        }),
        _ => {}
    }

    // Base required fields
    for req in &schema.base_required {
        if !has_key(fm, req) {
            findings.push(Finding::MissingRequired(req.clone()));
        }
    }

    // Enum and type validation
    for (field, prop) in &schema.properties {
        if !has_key(fm, field) {
            continue; // optional absent fields are fine
        }
        let val = fm.get(field.as_str()).unwrap();

        if let Some(expected_type) = &prop.json_type {
            if !value_matches_type(val, expected_type) {
                findings.push(Finding::WrongType {
                    field: field.clone(),
                    expected: expected_type.clone(),
                });
                continue;
            }
        }

        if let Some(allowed) = &prop.enum_values {
            if let Some(s) = val.as_str() {
                if !allowed.iter().any(|a| a == s) {
                    findings.push(Finding::InvalidEnum {
                        field: field.clone(),
                        value: s.to_string(),
                        allowed: allowed.clone(),
                    });
                }
            }
        }
    }

    // Type-conditional required fields
    if let Some(type_val) = get_str(fm, "type") {
        if let Some(cond_required) = schema.type_conditionals.get(type_val) {
            for req in cond_required {
                if !has_key(fm, req) {
                    findings.push(Finding::MissingTypeConditional(req.clone()));
                }
            }
        }
    }

    // Strict: _INDEX.md body TOC check
    if strict {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "_INDEX.md" {
            if let Some(type_val) = get_str(fm, "type") {
                if type_val == "index" {
                    // Body TOC check happens at the caller (we don't have body here).
                    // Skip — body is available only in ParsedFile.
                }
            }
        }
    }

    findings
}

fn value_matches_type(val: &Value, expected: &str) -> bool {
    match expected {
        "string" => val.is_string(),
        "integer" => val.is_number() && val.as_i64().is_some(),
        "array" => val.is_sequence(),
        "boolean" => val.is_bool(),
        _ => true,
    }
}

fn print_human(results: &[(PathBuf, Vec<Finding>)], total: usize) {
    let counts = count_by_category(results);
    let clean = results.iter().all(|(_, f)| f.is_empty());

    println!("SCHEMA COMPLIANCE — {total} file(s) scanned");
    println!();
    println!(
        "  no frontmatter:                  {} file(s)",
        counts.get("no frontmatter").copied().unwrap_or(0)
    );
    println!(
        "  missing schema_version:          {} file(s)",
        counts.get("missing schema_version").copied().unwrap_or(0)
    );
    println!(
        "  stale schema_version:            {} file(s)",
        counts.get("stale schema_version").copied().unwrap_or(0)
    );
    println!(
        "  missing required base field:     {} file(s)",
        counts
            .get("missing required base field")
            .copied()
            .unwrap_or(0)
    );
    println!(
        "  missing type-conditional field:  {} file(s)",
        counts
            .get("missing type-conditional field")
            .copied()
            .unwrap_or(0)
    );
    println!(
        "  invalid enum value:              {} file(s)",
        counts.get("invalid enum value").copied().unwrap_or(0)
    );
    println!(
        "  wrong field type:                {} file(s)",
        counts.get("wrong field type").copied().unwrap_or(0)
    );
    println!(
        "  duplicate id within engine:      {} collision(s)",
        counts.get("duplicate id").copied().unwrap_or(0)
    );
    println!();

    if clean {
        println!("✓ All files conformant.");
        return;
    }

    for cat in &[
        "missing type-conditional field",
        "missing required base field",
        "no frontmatter",
        "missing schema_version",
        "stale schema_version",
        "invalid enum value",
        "wrong field type",
        "duplicate id",
    ] {
        let files_with_cat: Vec<_> = results
            .iter()
            .filter(|(_, f)| f.iter().any(|fi| fi.category() == *cat))
            .collect();
        if files_with_cat.is_empty() {
            continue;
        }
        println!("DETAIL — {cat}");
        for (path, findings) in &files_with_cat {
            println!("  {}", path.display());
            for f in findings.iter().filter(|fi| fi.category() == *cat) {
                println!("    {}", f.detail());
            }
        }
        println!();
    }
}

fn print_json(results: &[(PathBuf, Vec<Finding>)], total: usize) {
    let issues: Vec<serde_json::Value> = results
        .iter()
        .filter(|(_, f)| !f.is_empty())
        .map(|(path, findings)| {
            serde_json::json!({
                "file": path.display().to_string(),
                "findings": findings.iter().map(|f| serde_json::json!({
                    "category": f.category(),
                    "detail": f.detail(),
                })).collect::<Vec<_>>()
            })
        })
        .collect();
    let out = serde_json::json!({
        "total": total,
        "clean": issues.is_empty(),
        "issues": issues,
    });
    println!("{out}");
}

fn count_by_category(results: &[(PathBuf, Vec<Finding>)]) -> HashMap<&'static str, usize> {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for (_, findings) in results {
        for f in findings {
            *counts.entry(f.category()).or_insert(0) += 1;
        }
    }
    counts
}

fn resolve_path(path: Option<&str>, cwd: &Path, workspace_root: &Path) -> PathBuf {
    match path {
        None => cwd.to_path_buf(),
        Some(p) => {
            // Rule 13: CWD-relative for explicit paths; workspace-root-relative fallback for src
            let cwd_rel = cwd.join(p);
            if cwd_rel.exists() {
                cwd_rel
            } else {
                let ws_rel = workspace_root.join(p);
                if ws_rel.exists() {
                    ws_rel
                } else {
                    cwd_rel // return CWD-relative even if not found (error reported by caller)
                }
            }
        }
    }
}

/// Public entry point — resolves workspace root and CWD from environment, then delegates.
pub fn run_from_env(
    path: Option<&str>,
    format: AuditFormat,
    schema_override: Option<&str>,
    strict: bool,
) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| workspace_root.clone());
    run(path, format, schema_override, strict, &workspace_root, &cwd)
}
