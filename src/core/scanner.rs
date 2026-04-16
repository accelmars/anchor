#![allow(dead_code)]

use crate::model::CanonicalPath;
use ignore::WalkBuilder;
use std::path::Path;

/// Error type for workspace scanning.
#[derive(Debug)]
pub enum ScannerError {
    Io(std::io::Error),
    Ignore(ignore::Error),
}

impl std::fmt::Display for ScannerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScannerError::Io(e) => write!(f, "I/O error: {e}"),
            ScannerError::Ignore(e) => write!(f, "walk error: {e}"),
        }
    }
}

impl std::error::Error for ScannerError {}

impl From<ignore::Error> for ScannerError {
    fn from(e: ignore::Error) -> Self {
        ScannerError::Ignore(e)
    }
}

/// Walk `root` recursively and return all `.md` files as workspace-root-relative canonical paths.
///
/// - Returns paths with forward slashes and no `./` prefix.
/// - Skips `.mind/` directory entirely (hardcoded — system directory, not user-configurable).
/// - Respects `.mindignore` at the workspace root (gitignore-compatible pattern syntax).
///   If `.mindignore` does not exist, scanner behavior is unchanged.
/// - Silently skips non-`.md` files and non-file entries.
pub fn scan_workspace(root: &Path) -> Result<Vec<CanonicalPath>, ScannerError> {
    let mut paths = Vec::new();

    let walker = WalkBuilder::new(root)
        // Disable all automatic ignore file loading — .mindignore only.
        // mind should not silently exclude files due to unrelated git configuration.
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        // Load .mindignore (gitignore-compatible) from the workspace root.
        // If the file is absent, this is a no-op — no hard error.
        .add_custom_ignore_filename(".mindignore")
        .build();

    for result in walker {
        let entry = result?;

        // Hardcoded .mind/ exclusion — system directory.
        // Two-layer defense: hardcoded here AND user cannot accidentally remove it
        // from .mindignore (it's never written there by mind init).
        if entry.path().components().any(|c| c.as_os_str() == ".mind") {
            continue;
        }

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
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

    /// .mindignore absent → scanner returns all .md files (no regression).
    #[test]
    fn test_no_mindignore_returns_all_md() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("a.md"), b"").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("b.md"), b"").unwrap();

        // No .mindignore file — behavior must be identical to prior WalkDir implementation
        assert!(!root.join(".mindignore").exists());

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["a.md", "sub/b.md"]);
    }

    /// .mindignore with `node_modules/` → entries under node_modules/ excluded.
    #[test]
    fn test_mindignore_excludes_node_modules() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("README.md"), b"").unwrap();
        fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        fs::write(root.join("node_modules").join("pkg").join("README.md"), b"").unwrap();

        fs::write(root.join(".mindignore"), b"node_modules/\n").unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["README.md"]);
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
    }

    /// .mindignore with `target/` → entries under target/ excluded.
    #[test]
    fn test_mindignore_excludes_target() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("CHANGELOG.md"), b"").unwrap();
        fs::create_dir_all(root.join("target").join("debug")).unwrap();
        fs::write(root.join("target").join("debug").join("notes.md"), b"").unwrap();

        fs::write(root.join(".mindignore"), b"target/\n").unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["CHANGELOG.md"]);
        assert!(!paths.iter().any(|p| p.contains("target")));
    }

    /// .mindignore negation pattern: file-pattern exclusion with specific re-inclusion.
    ///
    /// Standard gitignore semantics: when a DIRECTORY is excluded (e.g., `notes/`), the
    /// walker prunes the directory and files inside it cannot be re-included via `!`. However,
    /// negation DOES work for file-pattern exclusions (e.g., `*.bak` then `!keep.bak`).
    ///
    /// This test verifies the working negation case: exclude all `.bak.md` files but
    /// re-include one specific file via `!`.
    #[test]
    fn test_mindignore_negation_reinclude_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::create_dir(root.join("docs")).unwrap();
        fs::write(root.join("docs").join("draft.bak.md"), b"").unwrap();
        fs::write(root.join("docs").join("keep.bak.md"), b"").unwrap();
        fs::write(root.join("docs").join("final.md"), b"").unwrap();
        fs::write(root.join("root.md"), b"").unwrap();

        // Exclude *.bak.md files but re-include one specific file via negation
        fs::write(root.join(".mindignore"), b"*.bak.md\n!docs/keep.bak.md\n").unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        // keep.bak.md is re-included via negation; draft.bak.md remains excluded
        assert!(
            paths.contains(&"docs/keep.bak.md".to_string()),
            "negation should re-include docs/keep.bak.md; got: {:?}",
            paths
        );
        assert!(
            !paths.contains(&"docs/draft.bak.md".to_string()),
            "docs/draft.bak.md should remain excluded; got: {:?}",
            paths
        );
        assert!(paths.contains(&"docs/final.md".to_string()));
        assert!(paths.contains(&"root.md".to_string()));
    }

    /// .mindignore absent AND .mind/ present → .mind/ is still excluded (hardcoded exclusion).
    #[test]
    fn test_mind_dir_excluded_without_mindignore() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("README.md"), b"").unwrap();
        fs::create_dir(root.join(".mind")).unwrap();
        fs::write(root.join(".mind").join("config.json.md"), b"").unwrap();
        fs::write(root.join(".mind").join("secret.md"), b"").unwrap();

        // No .mindignore — .mind/ must be excluded by hardcoded logic alone
        assert!(!root.join(".mindignore").exists());

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["README.md"]);
        assert!(!paths.iter().any(|p| p.contains(".mind")));
    }
}
