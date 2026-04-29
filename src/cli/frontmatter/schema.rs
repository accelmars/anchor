// src/cli/frontmatter/schema.rs — FRONTMATTER.schema.json loader
//
// Reads the JSON Schema file and produces SchemaRules, which all subcommands
// use for validation, synonym lookup, and canonical key ordering.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed rules derived from FRONTMATTER.schema.json.
pub struct SchemaRules {
    /// Fields required for every file (base required layer).
    pub base_required: Vec<String>,
    /// Per-property type definitions ("string", "integer", "array", "boolean").
    pub properties: HashMap<String, PropDef>,
    /// Per-type additional required fields (from allOf if/then).
    pub type_conditionals: HashMap<String, Vec<String>>,
    /// Synonym tables per field (from x-synonyms extension).
    pub synonyms: HashMap<String, HashMap<String, String>>,
    /// Canonical key order (from x-canonical-key-order extension).
    pub canonical_key_order: Vec<String>,
}

/// Property definition from the schema.
pub struct PropDef {
    pub json_type: Option<String>,
    pub enum_values: Option<Vec<String>>,
}

impl SchemaRules {
    /// Resolve the schema path: explicit flag → workspace-local config → error.
    ///
    /// Resolution order:
    ///   1. `explicit_path` when provided (resolved relative to `cwd`)
    ///   2. `<workspace_root>/.accelmars/anchor/frontmatter-schema.json`
    ///   3. Error — names every path tried
    pub fn resolve_schema_path(
        explicit_path: Option<&str>,
        cwd: &Path,
        workspace_root: &Path,
    ) -> Result<PathBuf, String> {
        if let Some(p) = explicit_path {
            let resolved = if std::path::Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                cwd.join(p)
            };
            return Ok(resolved);
        }
        let workspace_local = workspace_root
            .join(".accelmars")
            .join("anchor")
            .join("frontmatter-schema.json");
        if workspace_local.exists() {
            return Ok(workspace_local);
        }
        Err(format!(
            "schema not found — tried:\n  {}\nUse --schema <path> or place the schema at .accelmars/anchor/frontmatter-schema.json",
            workspace_local.display()
        ))
    }

    /// Load and parse the schema from `path`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read schema at {}: {e}", path.display()))?;
        let schema: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| format!("invalid JSON in schema: {e}"))?;

        let base_required = extract_string_array(&schema, "required");

        let properties = extract_properties(&schema);

        let type_conditionals = extract_type_conditionals(&schema);

        let synonyms = extract_synonyms(&schema);

        let canonical_key_order = schema
            .get("x-canonical-key-order")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Ok(SchemaRules {
            base_required,
            properties,
            type_conditionals,
            synonyms,
            canonical_key_order,
        })
    }
}

fn extract_string_array(obj: &serde_json::Value, key: &str) -> Vec<String> {
    obj.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_properties(schema: &serde_json::Value) -> HashMap<String, PropDef> {
    let mut map = HashMap::new();
    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        for (key, prop) in props {
            let json_type = prop.get("type").and_then(|v| v.as_str()).map(String::from);
            let enum_values = prop.get("enum").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });
            map.insert(
                key.clone(),
                PropDef {
                    json_type,
                    enum_values,
                },
            );
        }
    }
    map
}

/// Parse allOf if/then conditionals of the form:
///   if.properties.type.const = "<type>" → then.required = [...]
fn extract_type_conditionals(schema: &serde_json::Value) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    if let Some(all_of) = schema.get("allOf").and_then(|v| v.as_array()) {
        for entry in all_of {
            let type_val = entry
                .pointer("/if/properties/type/const")
                .and_then(|v| v.as_str());
            let required_fields = entry
                .pointer("/then/required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                });
            if let (Some(t), Some(reqs)) = (type_val, required_fields) {
                map.insert(t.to_string(), reqs);
            }
        }
    }
    map
}

/// Parse x-synonyms extension:
///   { "field": { "synonym": "canonical", ... }, ... }
fn extract_synonyms(schema: &serde_json::Value) -> HashMap<String, HashMap<String, String>> {
    let mut outer = HashMap::new();
    if let Some(syns) = schema.get("x-synonyms").and_then(|v| v.as_object()) {
        for (field, table) in syns {
            if let Some(entries) = table.as_object() {
                let mut inner = HashMap::new();
                for (syn, canonical) in entries {
                    if let Some(c) = canonical.as_str() {
                        inner.insert(syn.clone(), c.to_string());
                    }
                }
                outer.insert(field.clone(), inner);
            }
        }
    }
    outer
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace() -> TempDir {
        TempDir::new().unwrap()
    }

    fn make_workspace_with_schema() -> TempDir {
        let dir = TempDir::new().unwrap();
        let schema_dir = dir.path().join(".accelmars").join("anchor");
        fs::create_dir_all(&schema_dir).unwrap();
        fs::write(
            schema_dir.join("frontmatter-schema.json"),
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","required":["title"],"properties":{"title":{"type":"string"}}}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn resolve_schema_path_explicit_wins() {
        let ws = make_workspace();
        let explicit = ws.path().join("my-schema.json");
        fs::write(&explicit, "{}").unwrap();

        let result = SchemaRules::resolve_schema_path(
            Some(explicit.to_str().unwrap()),
            ws.path(),
            ws.path(),
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), explicit);
    }

    #[test]
    fn resolve_schema_path_workspace_local_fallback() {
        let ws = make_workspace_with_schema();
        let expected = ws
            .path()
            .join(".accelmars")
            .join("anchor")
            .join("frontmatter-schema.json");

        let result = SchemaRules::resolve_schema_path(None, ws.path(), ws.path());
        assert!(result.is_ok(), "should find workspace-local schema");
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn resolve_schema_path_error_names_paths_tried() {
        let ws = make_workspace();

        let result = SchemaRules::resolve_schema_path(None, ws.path(), ws.path());
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains(".accelmars/anchor/frontmatter-schema.json"),
            "error must name path tried: {msg}"
        );
        assert!(
            msg.contains("--schema"),
            "error must suggest --schema flag: {msg}"
        );
    }

    #[test]
    fn resolve_schema_path_explicit_relative_resolved_from_cwd() {
        let ws = make_workspace();
        let schema_file = ws.path().join("sub").join("schema.json");
        fs::create_dir_all(schema_file.parent().unwrap()).unwrap();
        fs::write(&schema_file, "{}").unwrap();

        let result =
            SchemaRules::resolve_schema_path(Some("sub/schema.json"), ws.path(), ws.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ws.path().join("sub").join("schema.json"));
    }
}
