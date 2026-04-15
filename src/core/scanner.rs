#![allow(dead_code)]

use crate::model::CanonicalPath;
use std::path::Path;
use walkdir::{DirEntry, WalkDir};

/// Error type for workspace scanning.
#[derive(Debug)]
pub enum ScannerError {
    WalkDir(walkdir::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for ScannerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScannerError::WalkDir(e) => write!(f, "directory walk error: {e}"),
            ScannerError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for ScannerError {}

impl From<walkdir::Error> for ScannerError {
    fn from(e: walkdir::Error) -> Self {
        ScannerError::WalkDir(e)
    }
}

fn is_mind_dir(entry: &DirEntry) -> bool {
    entry.file_name().to_str() == Some(".mind")
}

/// Walk `root` recursively and return all `.md` files as workspace-root-relative canonical paths.
///
/// - Returns paths with forward slashes and no `./` prefix.
/// - Skips `.mind/` directory entirely.
/// - Silently skips non-`.md` files and non-file entries.
pub fn scan_workspace(root: &Path) -> Result<Vec<CanonicalPath>, ScannerError> {
    let mut paths = Vec::new();

    let walker = WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !is_mind_dir(e));

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|e| ScannerError::Io(std::io::Error::other(e)))?;
        // Build canonical form: forward slashes, no leading ./
        let canonical = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        paths.push(canonical);
    }

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Test 10 (contract): temp workspace with .md files at root and subdir,
    /// non-.md files, and .mind/ directory. Verifies only .md paths returned,
    /// no ./ prefix, and .mind/ excluded.
    #[test]
    fn test_scan_workspace() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // .md files to include
        fs::write(root.join("README.md"), b"").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("design.md"), b"").unwrap();

        // Non-.md files — must be excluded
        fs::write(root.join("notes.txt"), b"").unwrap();
        fs::write(root.join("sub").join("image.png"), b"").unwrap();

        // .mind/ directory — must be excluded entirely
        fs::create_dir(root.join(".mind")).unwrap();
        fs::write(root.join(".mind").join("hidden.md"), b"").unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["README.md", "sub/design.md"]);

        // Verify .mind contents are absent
        assert!(!paths.iter().any(|p| p.contains(".mind")));
        // Verify no ./ prefix
        assert!(!paths.iter().any(|p| p.starts_with("./")));
    }
}
