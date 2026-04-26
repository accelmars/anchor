// src/core/reference/toml.rs — TOML config reference extractor (GAP-A1, AP-003)
//
// Extracts path references from TOML config files in the workspace.
// Only strings starting with `$(anchor root)/` are treated as path references;
// all other TOML values are silently ignored.
//
// Path detection heuristic: `$(anchor root)/` prefix — same as the YAML extractor (AP-002).
// Plan operation `src`/`dst` fields use workspace-relative paths (no `$(anchor root)/` prefix)
// and are therefore naturally filtered by this heuristic — no explicit key-name check needed.
#![allow(dead_code)]

use crate::model::{
    reference::{RefForm, Reference},
    CanonicalPath,
};

const ANCHOR_ROOT_PREFIX: &str = "$(anchor root)/";

/// Extract path references from a TOML config file.
///
/// Walks all TOML values (including nested tables and arrays) and returns a
/// `Reference` for each string value starting with `$(anchor root)/`.
///
/// - `target_raw` is the **full** original TOML string value (prefix included).
/// - Non-path strings are silently ignored.
/// - Malformed TOML is silently ignored (returns empty vec).
pub fn extract_toml_refs(content: &str, source_file: &CanonicalPath) -> Vec<Reference> {
    let value: toml::Value = match toml::from_str(content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut refs = Vec::new();
    collect_refs(&value, source_file, content, &mut refs);
    refs
}

/// Recursively walk a `toml::Value`, emitting a `Reference` for each string
/// value that starts with `$(anchor root)/`.
fn collect_refs(
    value: &toml::Value,
    source_file: &CanonicalPath,
    full_content: &str,
    refs: &mut Vec<Reference>,
) {
    match value {
        toml::Value::String(s) if s.starts_with(ANCHOR_ROOT_PREFIX) => {
            let span = find_value_span(full_content, s);
            refs.push(Reference {
                source_file: source_file.clone(),
                target_raw: s.clone(),
                span,
                form: RefForm::Toml,
                anchor: None,
            });
        }
        toml::Value::Table(map) => {
            for (_, v) in map {
                collect_refs(v, source_file, full_content, refs);
            }
        }
        toml::Value::Array(arr) => {
            for item in arr {
                collect_refs(item, source_file, full_content, refs);
            }
        }
        _ => {}
    }
}

/// Find the byte span of `value` in `full_content`.
///
/// Returns `(start, end)` byte offsets. Falls back to `(0, 0)` if not found.
fn find_value_span(content: &str, value: &str) -> (usize, usize) {
    if let Some(pos) = content.find(value) {
        (pos, pos + value.len())
    } else {
        (0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> CanonicalPath {
        "test/config.toml".to_string()
    }

    /// TOML with a `$(anchor root)/` path value → reference extracted with full value.
    #[test]
    fn test_extract_path_from_toml() {
        let content = "start_dir = \"$(anchor root)/foo/bar\"\n";
        let refs = extract_toml_refs(content, &src());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_raw, "$(anchor root)/foo/bar");
        assert_eq!(refs[0].form, RefForm::Toml);
        assert_eq!(refs[0].source_file, src());
    }

    /// TOML with non-path strings (e.g. operation type, version) → nothing extracted.
    #[test]
    fn test_skip_non_path_toml_values() {
        let content = concat!(
            "version = \"1\"\n",
            "description = \"batch-move\"\n",
            "\n",
            "[[ops]]\n",
            "type = \"move\"\n",
            "src = \"anchor-foundation\"\n",
            "dst = \"foundations/anchor-engine\"\n",
        );
        let refs = extract_toml_refs(content, &src());
        assert!(
            refs.is_empty(),
            "non-path TOML values must produce no refs; got: {refs:?}"
        );
    }

    /// Deeply nested TOML path value → extracted correctly.
    #[test]
    fn test_toml_nested_path() {
        let content = concat!(
            "[section]\n",
            "[section.subsection]\n",
            "target = \"$(anchor root)/some/nested/file.md\"\n",
        );
        let refs = extract_toml_refs(content, &src());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_raw, "$(anchor root)/some/nested/file.md");
        assert_eq!(refs[0].form, RefForm::Toml);
    }
}
