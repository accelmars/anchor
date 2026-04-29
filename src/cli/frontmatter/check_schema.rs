// src/cli/frontmatter/check_schema.rs — CI diff guard
//
// Detects drift between FRONTMATTER.md (human-readable narrative) and
// FRONTMATTER.schema.json (machine-readable authority).
//
// Documented diff rule (this comment is the auditable specification):
//
//   1. BASE REQUIRED: Every field listed in the "Base layer — required" table in
//      FRONTMATTER.md has a corresponding entry in the schema's `required` array.
//      Any field in schema `required` must also appear in FRONTMATTER.md's required table.
//
//   2. TYPE CONDITIONALS: For each "### `type: X`" section in FRONTMATTER.md listing
//      a field as "Required", a matching `allOf` if/then entry in the schema must exist
//      with the same type and field in the `then.required` array.
//
//   3. STATUS SYNONYMS (structural check only): The schema `x-synonyms.status` object
//      must be non-empty when FRONTMATTER.md mentions status normalization. This check
//      does not enumerate individual synonyms — synonym tables are managed in the schema.
//
// Exit codes: 0 = in-sync, 1 = diverged (mismatch list printed), 2 = error
//
// Usage: anchor frontmatter check-schema [<spec>] [<schema>]
//   spec   — path to FRONTMATTER.md  (default: workspace-relative)
//   schema — path to FRONTMATTER.schema.json (default: workspace-relative)

use crate::infra::workspace;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Mismatch {
    pub rule: &'static str,
    pub detail: String,
}

/// Run `anchor frontmatter check-schema [spec] [schema]`. Returns exit code.
pub fn run(
    spec_path: Option<&str>,
    schema_path_arg: Option<&str>,
    workspace_root: &Path,
    cwd: &Path,
) -> i32 {
    let spec = spec_path
        .map(|p| resolve(p, cwd, workspace_root))
        .unwrap_or_else(|| {
            workspace_root
                .join("accelmars-workspace")
                .join("FRONTMATTER.md")
        });

    let schema_path = schema_path_arg
        .map(|p| resolve(p, cwd, workspace_root))
        .unwrap_or_else(|| {
            workspace_root
                .join("accelmars-workspace")
                .join("FRONTMATTER.schema.json")
        });

    match run_check(&spec, &schema_path) {
        Ok(mismatches) if mismatches.is_empty() => {
            println!("✓ FRONTMATTER.md and FRONTMATTER.schema.json are in sync.");
            0
        }
        Ok(mismatches) => {
            eprintln!(
                "DIVERGED — {} mismatch(es) between spec and schema:",
                mismatches.len()
            );
            for m in &mismatches {
                eprintln!("  [{}] {}", m.rule, m.detail);
            }
            1
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// Core check logic — separated for testing.
pub fn run_check(spec_path: &Path, schema_path: &Path) -> Result<Vec<Mismatch>, String> {
    let spec_content = std::fs::read_to_string(spec_path)
        .map_err(|e| format!("cannot read spec {}: {e}", spec_path.display()))?;
    let schema_content = std::fs::read_to_string(schema_path)
        .map_err(|e| format!("cannot read schema {}: {e}", schema_path.display()))?;

    let schema: serde_json::Value = serde_json::from_str(&schema_content)
        .map_err(|e| format!("invalid JSON in schema: {e}"))?;

    let mut mismatches = Vec::new();

    // Rule 1: Base required fields
    let spec_required = parse_base_required_from_md(&spec_content);
    let schema_required: HashSet<String> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    for field in &spec_required {
        if !schema_required.contains(field) {
            mismatches.push(Mismatch {
                rule: "BASE_REQUIRED",
                detail: format!(
                    "'{field}' is required in FRONTMATTER.md but absent from schema required[]"
                ),
            });
        }
    }
    for field in &schema_required {
        if !spec_required.contains(field) {
            mismatches.push(Mismatch {
                rule: "BASE_REQUIRED",
                detail: format!(
                    "'{field}' is in schema required[] but not found in FRONTMATTER.md required table"
                ),
            });
        }
    }

    // Rule 2: Type conditionals
    let spec_conditionals = parse_type_conditionals_from_md(&spec_content);
    let schema_conditionals = extract_schema_conditionals(&schema);

    for (type_val, spec_fields) in &spec_conditionals {
        let schema_fields = schema_conditionals
            .get(type_val)
            .cloned()
            .unwrap_or_default();
        for field in spec_fields {
            if !schema_fields.contains(field) {
                mismatches.push(Mismatch {
                    rule: "TYPE_CONDITIONAL",
                    detail: format!(
                        "type:{type_val} requires '{field}' in FRONTMATTER.md but schema allOf has no matching entry"
                    ),
                });
            }
        }
        for field in &schema_fields {
            if !spec_fields.contains(field) {
                mismatches.push(Mismatch {
                    rule: "TYPE_CONDITIONAL",
                    detail: format!(
                        "schema allOf requires '{field}' for type:{type_val} but FRONTMATTER.md doesn't list it"
                    ),
                });
            }
        }
    }
    // Schema conditionals not present in spec
    for (type_val, schema_fields) in &schema_conditionals {
        if !spec_conditionals.contains_key(type_val) && !schema_fields.is_empty() {
            mismatches.push(Mismatch {
                rule: "TYPE_CONDITIONAL",
                detail: format!(
                    "schema allOf has conditionals for type:{type_val} but FRONTMATTER.md has no '### `type: {type_val}`' section"
                ),
            });
        }
    }

    // Rule 3: Status synonyms structural check
    let has_synonyms = schema
        .pointer("/x-synonyms/status")
        .and_then(|v| v.as_object())
        .map(|o| !o.is_empty())
        .unwrap_or(false);
    let md_mentions_synonyms =
        spec_content.contains("synonyms") || spec_content.contains("consumed");
    if md_mentions_synonyms && !has_synonyms {
        mismatches.push(Mismatch {
            rule: "STATUS_SYNONYMS",
            detail:
                "FRONTMATTER.md mentions status synonyms but schema has no x-synonyms.status table"
                    .to_string(),
        });
    }

    Ok(mismatches)
}

/// Parse the base-required field names from the "Base layer — required" section of FRONTMATTER.md.
///
/// Looks for the section "## Base layer — required" and extracts backtick-wrapped field names
/// from table rows (first column), until the next `##` heading or end of section.
fn parse_base_required_from_md(content: &str) -> HashSet<String> {
    let mut fields = HashSet::new();
    let mut in_required_section = false;
    let mut in_table = false;

    for line in content.lines() {
        if line.starts_with("## Base layer") && line.contains("required") {
            in_required_section = true;
            in_table = false;
            continue;
        }
        if in_required_section && line.starts_with("## ") {
            break; // moved past the required section
        }
        if !in_required_section {
            continue;
        }
        // Table row detection
        if line.starts_with('|') {
            in_table = true;
            let first_col = line.split('|').nth(1).unwrap_or("").trim();
            // Skip header rows and separator rows
            if first_col.starts_with('-') || first_col.to_lowercase() == "field" {
                continue;
            }
            // Extract backtick-wrapped field name
            if let Some(field) = extract_backtick(first_col) {
                fields.insert(field);
            }
        } else if in_table && !line.trim().is_empty() && !line.starts_with('|') {
            in_table = false; // table ended
        }
    }
    fields
}

/// Parse type-conditional required fields from FRONTMATTER.md "### `type: X`" sections.
fn parse_type_conditionals_from_md(content: &str) -> HashMap<String, HashSet<String>> {
    let mut result: HashMap<String, HashSet<String>> = HashMap::new();
    let mut current_type: Option<String> = None;
    let mut in_table = false;

    for line in content.lines() {
        // Detect "### `type: X`" heading
        if line.starts_with("### `type:") {
            let type_val = line
                .trim_start_matches('#')
                .trim()
                .trim_matches('`')
                .trim_start_matches("type:")
                .trim()
                .to_string();
            current_type = Some(type_val);
            in_table = false;
            continue;
        }
        // New section resets current type
        if line.starts_with("## ") || (line.starts_with("### ") && !line.contains("type:")) {
            current_type = None;
            in_table = false;
            continue;
        }
        let Some(ref type_val) = current_type else {
            continue;
        };

        // Table parsing — look for "Required" in the "Required?" column
        if line.starts_with('|') {
            in_table = true;
            let cols: Vec<&str> = line.split('|').collect();
            if cols.len() < 3 {
                continue;
            }
            let first_col = cols.get(1).map(|s| s.trim()).unwrap_or("");
            let required_col = cols.get(2).map(|s| s.trim()).unwrap_or("");
            if first_col.starts_with('-') || first_col.to_lowercase() == "field" {
                continue;
            }
            if required_col.contains("Required") && !required_col.contains("Recommended") {
                if let Some(field) = extract_backtick(first_col) {
                    result.entry(type_val.clone()).or_default().insert(field);
                }
            }
        } else if in_table && !line.trim().is_empty() && !line.starts_with('|') {
            in_table = false;
        }
    }
    result
}

fn extract_schema_conditionals(schema: &serde_json::Value) -> HashMap<String, HashSet<String>> {
    let mut map = HashMap::new();
    if let Some(all_of) = schema.get("allOf").and_then(|v| v.as_array()) {
        for entry in all_of {
            let type_val = entry
                .pointer("/if/properties/type/const")
                .and_then(|v| v.as_str());
            let required = entry
                .pointer("/then/required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<HashSet<_>>()
                });
            if let (Some(t), Some(reqs)) = (type_val, required) {
                map.insert(t.to_string(), reqs);
            }
        }
    }
    map
}

fn extract_backtick(s: &str) -> Option<String> {
    let s = s.trim();
    if s.starts_with('`') && s.ends_with('`') && s.len() > 2 {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

fn resolve(p: &str, cwd: &Path, workspace_root: &Path) -> PathBuf {
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

pub fn run_from_env(spec_path: Option<&str>, schema_path: Option<&str>) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| workspace_root.clone());
    run(spec_path, schema_path, &workspace_root, &cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_backtick_valid() {
        assert_eq!(extract_backtick("`title`"), Some("title".to_string()));
        assert_eq!(extract_backtick("  `engine`  "), Some("engine".to_string()));
    }

    #[test]
    fn extract_backtick_none_for_plain_text() {
        assert_eq!(extract_backtick("Field"), None);
    }

    #[test]
    fn parse_base_required_extracts_fields() {
        let md = "## Base layer — required for every file\n\n\
| Field | Type | Values | Purpose |\n\
|-------|------|--------|--------|\n\
| `title` | string | text | Display |\n\
| `type` | enum | ... | Class |\n\
\n## Next section\n";
        let fields = parse_base_required_from_md(md);
        assert!(fields.contains("title"), "fields: {fields:?}");
        assert!(fields.contains("type"), "fields: {fields:?}");
    }
}
