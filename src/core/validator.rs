// src/core/validator.rs — post-rewrite reference validation (MF-006)
#![allow(dead_code)]

use crate::core::{parser, resolver};
use crate::model::CanonicalPath;
use std::path::Path;

/// A reference that failed to resolve (target file not found).
#[derive(Debug, Clone)]
pub struct BrokenRef {
    /// Canonical path of the file containing the reference.
    pub file: CanonicalPath,
    /// 1-based line number of the broken reference.
    pub line: usize,
    /// Raw target string as written in the file.
    pub target: String,
}

/// Validate that every Form 1 reference in the given files resolves to an existing file.
///
/// Returns all references whose resolved canonical path does not exist on disk.
/// Form 2 (wiki links) are not validated here — their stem-based resolution requires
/// the full workspace file list (out of scope for this function's use in MF-007).
///
/// # Arguments
/// - `workspace_root`: absolute path to the workspace root
/// - `files`: slice of `(canonical_path, content)` tuples to validate
///
/// # Returns
/// `Vec<BrokenRef>` of all references that do not resolve to an existing file.
/// Empty vec = all references valid.
pub fn validate_files(workspace_root: &Path, files: &[(CanonicalPath, String)]) -> Vec<BrokenRef> {
    let mut broken = Vec::new();

    for (canonical, content) in files {
        let refs = parser::parse_references(canonical, content);

        for reference in refs {
            // Wiki and Backtick refs: skip (not relative paths, cannot use resolve_form1)
            if reference.form == crate::model::reference::RefForm::Wiki
                || reference.form == crate::model::reference::RefForm::Backtick
            {
                continue;
            }

            let resolved = resolver::resolve_form1(canonical, &reference.target_raw);
            if !workspace_root.join(resolved.as_str()).exists() {
                let line = content[..reference.span.0]
                    .chars()
                    .filter(|&c| c == '\n')
                    .count()
                    + 1;
                broken.push(BrokenRef {
                    file: canonical.clone(),
                    line,
                    target: reference.target_raw.clone(),
                });
            }
        }
    }

    broken
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &Path, path: &str, content: &str) {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    /// All references valid → empty vec returned.
    #[test]
    fn test_all_valid_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(root, "target.md", "# Target\n");
        write_file(root, "source.md", "[link](target.md)\n");

        let content = fs::read_to_string(root.join("source.md")).unwrap();
        let files = vec![("source.md".to_string(), content)];

        let result = validate_files(root, &files);
        assert!(
            result.is_empty(),
            "expected no broken refs, got: {result:?}"
        );
    }

    /// Broken reference → returns one BrokenRef with correct file and target.
    #[test]
    fn test_broken_ref_reported() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // target.md does NOT exist
        let content = "[missing](missing.md)\n".to_string();
        let files = vec![("source.md".to_string(), content)];

        let result = validate_files(root, &files);
        assert_eq!(result.len(), 1, "expected one broken ref");
        assert_eq!(result[0].file, "source.md");
        assert_eq!(result[0].target, "missing.md");
        assert_eq!(result[0].line, 1);
    }

    /// Mixed: one valid, one broken → only broken ref returned.
    #[test]
    fn test_mixed_returns_only_broken() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(root, "exists.md", "# Exists\n");
        let content = "[ok](exists.md)\n[broken](ghost.md)\n".to_string();
        let files = vec![("source.md".to_string(), content)];

        let result = validate_files(root, &files);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].target, "ghost.md");
        assert_eq!(result[0].line, 2);
    }

    /// Line numbers are 1-based and correctly computed.
    #[test]
    fn test_line_number_accuracy() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let content = "# Header\n\nSome text.\n[broken](missing.md)\nmore text.\n".to_string();
        let files = vec![("doc.md".to_string(), content)];

        let result = validate_files(root, &files);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line, 4, "broken ref is on line 4");
    }
}
