// src/cli/frontmatter/parser.rs — shared frontmatter parser
//
// Rule 7 (ENGINE-LESSONS): promoted to module because audit/migrate/normalize/add-required
// all share the same YAML extraction logic.
//
// Rule 13 (ENGINE-LESSONS): callers must resolve path args relative to CWD before passing
// to parse_file. This module does not resolve paths — callers own that.

use serde_yaml::Value;
use std::io;
use std::path::{Path, PathBuf};

/// A .md file parsed into its frontmatter and body.
pub struct ParsedFile {
    #[allow(dead_code)]
    pub path: PathBuf,
    /// Parsed YAML. None means the file has no valid frontmatter block.
    pub frontmatter: Option<Value>,
    /// Raw YAML string between the --- delimiters (delimiters not included).
    pub raw_fm: Option<String>,
    /// Everything after the closing --- delimiter.
    pub body: String,
}

/// Parse a single file's frontmatter and body.
pub fn parse_file(path: &Path) -> io::Result<ParsedFile> {
    let content = std::fs::read_to_string(path)?;
    let (raw_fm, body) = split_frontmatter(&content);
    let frontmatter = raw_fm.as_deref().and_then(|s| serde_yaml::from_str(s).ok());
    Ok(ParsedFile {
        path: path.to_path_buf(),
        frontmatter,
        raw_fm,
        body,
    })
}

/// Split content into (raw_fm, body).
///
/// Valid frontmatter: file starts with "---\n", followed by a "\n---\n" closing
/// delimiter. The leading "---\n" and "\n---\n" are stripped from the result.
pub fn split_frontmatter(content: &str) -> (Option<String>, String) {
    if !content.starts_with("---\n") {
        return (None, content.to_string());
    }
    let after_open = &content[4..]; // skip "---\n"
    if let Some(pos) = after_open.find("\n---\n") {
        let raw_fm = after_open[..pos].to_string();
        let body = after_open[pos + 5..].to_string();
        (Some(raw_fm), body)
    } else if after_open.ends_with("\n---") {
        let pos = after_open.len() - 4;
        (Some(after_open[..pos].to_string()), String::new())
    } else {
        (None, content.to_string())
    }
}

/// Write frontmatter back to a file atomically (rename-into-place).
///
/// Serializes `fm` using serde_yaml (canonical formatting). Used by normalize.
pub fn write_atomic(path: &Path, fm: &Value, body: &str) -> io::Result<()> {
    let yaml = serde_yaml::to_string(fm)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let new_content = format!("---\n{}---\n{}", yaml, body);
    write_atomic_raw(path, &new_content)
}

/// Write raw content to a file atomically (rename-into-place).
///
/// Used by migrate (preserves original formatting, only adds fields).
pub fn write_atomic_str(path: &Path, raw_fm: &str, body: &str) -> io::Result<()> {
    let fm_block = if raw_fm.ends_with('\n') {
        raw_fm.to_string()
    } else {
        format!("{raw_fm}\n")
    };
    let new_content = format!("---\n{}---\n{}", fm_block, body);
    write_atomic_raw(path, &new_content)
}

fn write_atomic_raw(path: &Path, content: &str) -> io::Result<()> {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, content.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Get a string field value from a YAML mapping value.
pub fn get_str<'a>(fm: &'a Value, key: &str) -> Option<&'a str> {
    fm.get(key).and_then(|v| v.as_str())
}

/// Get an integer field value from a YAML mapping value.
pub fn get_i64(fm: &Value, key: &str) -> Option<i64> {
    fm.get(key).and_then(|v| v.as_i64())
}

/// Check if a key exists in a YAML mapping value.
pub fn has_key(fm: &Value, key: &str) -> bool {
    fm.get(key).is_some()
}

/// Insert a key-value pair into a YAML mapping value, returning the new mapping.
pub fn insert_str(fm: &mut Value, key: &str, val: &str) {
    if let Value::Mapping(map) = fm {
        map.insert(
            Value::String(key.to_string()),
            Value::String(val.to_string()),
        );
    }
}

/// Insert a key → integer into a YAML mapping value.
pub fn insert_i64(fm: &mut Value, key: &str, val: i64) {
    if let Value::Mapping(map) = fm {
        map.insert(
            Value::String(key.to_string()),
            Value::Number(serde_yaml::Number::from(val)),
        );
    }
}

/// Insert a key → empty array into a YAML mapping value.
pub fn insert_empty_array(fm: &mut Value, key: &str) {
    if let Value::Mapping(map) = fm {
        map.insert(Value::String(key.to_string()), Value::Sequence(vec![]));
    }
}

/// Scan a directory tree for .md files using walkdir. Respects Rule 12 (entry.file_type()).
pub fn walk_md_files(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
        .map(|e| e.path().to_path_buf())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_no_frontmatter() {
        let (fm, body) = split_frontmatter("# Title\nBody\n");
        assert!(fm.is_none());
        assert_eq!(body, "# Title\nBody\n");
    }

    #[test]
    fn split_valid_frontmatter() {
        let (fm, body) = split_frontmatter("---\ntitle: Test\n---\n# Body\n");
        assert_eq!(fm.as_deref(), Some("title: Test"));
        assert_eq!(body, "# Body\n");
    }

    #[test]
    fn split_empty_body() {
        let (fm, body) = split_frontmatter("---\ntitle: Test\n---\n");
        assert_eq!(fm.as_deref(), Some("title: Test"));
        assert_eq!(body, "");
    }

    #[test]
    fn split_horizontal_rule_in_body_not_confused() {
        let content = "---\ntitle: Test\n---\n# Body\n\n---\n\nMore\n";
        let (fm, body) = split_frontmatter(content);
        assert_eq!(fm.as_deref(), Some("title: Test"));
        assert!(
            body.contains("---"),
            "horizontal rule in body must be preserved"
        );
    }

    #[test]
    fn get_str_returns_value() {
        let fm: Value = serde_yaml::from_str("title: Hello").unwrap();
        assert_eq!(get_str(&fm, "title"), Some("Hello"));
        assert_eq!(get_str(&fm, "missing"), None);
    }

    #[test]
    fn has_key_present_and_absent() {
        let fm: Value = serde_yaml::from_str("title: Hi\ntype: gap").unwrap();
        assert!(has_key(&fm, "title"));
        assert!(!has_key(&fm, "id"));
    }
}
