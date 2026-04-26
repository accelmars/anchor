// src/core/reference/yaml.rs — YAML frontmatter reference extractor (GAP-A1)
//
// Extracts path references from YAML frontmatter in Markdown files.
// Only strings starting with `$(anchor root)/` are treated as path references;
// all other YAML values are silently ignored.
//
// Path detection heuristic: (a) `$(anchor root)/` prefix — see AP-002 architecture review.
// Scope: `.md` files only (frontmatter `---` blocks). Standalone `.yaml` files: deferred.
#![allow(dead_code)]

use crate::model::{
    reference::{RefForm, Reference},
    CanonicalPath,
};

const ANCHOR_ROOT_PREFIX: &str = "$(anchor root)/";

/// Extract path references from YAML frontmatter in a Markdown file.
///
/// Parses the leading `---` … `---` block, recursively walks all string values,
/// and returns a `Reference` for each string starting with `$(anchor root)/`.
///
/// - `target_raw` is the **full** original YAML value (prefix included).
/// - Non-path strings are silently ignored.
/// - Malformed YAML frontmatter is silently ignored (returns empty vec).
/// - Files without a `---` frontmatter block return empty vec.
pub fn extract_yaml_refs(content: &str, source_file: &CanonicalPath) -> Vec<Reference> {
    let Some(fm) = extract_frontmatter(content) else {
        return Vec::new();
    };

    let value: serde_yaml::Value = match serde_yaml::from_str(fm) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut refs = Vec::new();
    collect_refs(&value, source_file, content, &mut refs);
    refs
}

/// Extract the raw YAML text between the opening and closing `---` delimiters.
///
/// Returns `None` if the content does not start with `---\n` (or `---\r\n`),
/// or if no closing `---` (or `...`) delimiter is found on its own line.
fn extract_frontmatter(content: &str) -> Option<&str> {
    let rest = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))?;

    // Find closing delimiter: a line beginning with "---" or "..."
    let bytes = rest.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' && i + 3 <= bytes.len() {
            let next = &rest[i + 1..];
            if next.starts_with("---") || next.starts_with("...") {
                return Some(&rest[..i]);
            }
        }
    }
    None
}

/// Recursively walk a `serde_yaml::Value`, emitting a `Reference` for each
/// string value that starts with `$(anchor root)/`.
fn collect_refs(
    value: &serde_yaml::Value,
    source_file: &CanonicalPath,
    full_content: &str,
    refs: &mut Vec<Reference>,
) {
    match value {
        serde_yaml::Value::String(s) if s.starts_with(ANCHOR_ROOT_PREFIX) => {
            let span = find_value_span(full_content, s);
            refs.push(Reference {
                source_file: source_file.clone(),
                target_raw: s.clone(),
                span,
                form: RefForm::Yaml,
                anchor: None,
            });
        }
        serde_yaml::Value::String(_) => {}
        serde_yaml::Value::Mapping(map) => {
            for (_, v) in map {
                collect_refs(v, source_file, full_content, refs);
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                collect_refs(item, source_file, full_content, refs);
            }
        }
        _ => {}
    }
}

/// Find the byte span of `value` in `full_content`.
///
/// Returns `(start, end)` byte offsets. Used for 1-based line number computation
/// in the broken-ref reporter. Falls back to `(0, 0)` if not found.
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
        "test/source.md".to_string()
    }

    /// Frontmatter with a `$(anchor root)/` path value → reference extracted with full value.
    #[test]
    fn test_extract_path_from_frontmatter() {
        let content = "---\nstart_dir: \"$(anchor root)/foo\"\n---\n# Body\n";
        let refs = extract_yaml_refs(content, &src());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_raw, "$(anchor root)/foo");
        assert_eq!(refs[0].form, RefForm::Yaml);
        assert_eq!(refs[0].source_file, src());
    }

    /// Non-path YAML fields (id, title, state, etc.) must produce no references.
    #[test]
    fn test_skip_non_path_values() {
        let content = "---\nid: \"AP-001\"\ntitle: \"Some title\"\nstate: READY\n---\n";
        let refs = extract_yaml_refs(content, &src());
        assert!(
            refs.is_empty(),
            "non-path YAML values must not produce refs; got: {refs:?}"
        );
    }

    /// Nested path value under `output_artifacts[].path` → extracted correctly.
    #[test]
    fn test_extract_nested_path() {
        let content = concat!(
            "---\n",
            "output_artifacts:\n",
            "  - path: \"$(anchor root)/some/file.md\"\n",
            "    description: \"test artifact\"\n",
            "---\n",
            "# Body\n",
        );
        let refs = extract_yaml_refs(content, &src());
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_raw, "$(anchor root)/some/file.md");
    }

    /// Plain Markdown file without a `---` frontmatter block → empty result.
    #[test]
    fn test_no_frontmatter() {
        let content = "# A plain Markdown file\n\nNo frontmatter here.\n";
        let refs = extract_yaml_refs(content, &src());
        assert!(
            refs.is_empty(),
            "files without frontmatter must return empty vec"
        );
    }
}
