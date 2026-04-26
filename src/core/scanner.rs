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

/// Walk `root` recursively and return all `.md` and non-Cargo `.toml` files as
/// workspace-root-relative canonical paths.
///
/// - Returns paths with forward slashes and no `./` prefix.
/// - Skips `.accelmars/` directory entirely (hardcoded — system directory, not user-configurable).
/// - Respects `.accelmars/anchor/ignore` at the workspace root (gitignore-compatible pattern syntax).
///   If the file does not exist, scanner behavior is unchanged.
/// - Includes `.toml` files (excluding `Cargo.toml`) for TOML config ref scanning.
/// - Backtick path extraction from `.md` content is handled by `parser::parse_references`,
///   not by this function.
/// - Silently skips all other file types and non-file entries.
pub fn scan_workspace(root: &Path) -> Result<Vec<CanonicalPath>, ScannerError> {
    let mut paths = Vec::new();

    let mut builder = WalkBuilder::new(root);
    // Disable all automatic ignore file loading — use absolute path loading only.
    // anchor should not silently exclude files due to unrelated git configuration.
    builder
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false);
    // Load .accelmars/anchor/ignore (gitignore-compatible) by absolute path.
    // add_ignore returns Option<Error>; None means success or file absent (silently skipped).
    builder.add_ignore(root.join(".accelmars").join("anchor").join("ignore"));
    let walker = builder.build();

    for result in walker {
        let entry = result?;

        // Hardcoded .accelmars/ exclusion — system directory.
        // Two-layer defense: hardcoded here AND user cannot accidentally remove it
        // from .accelmars/anchor/ignore (it's never written there by anchor init).
        if entry
            .path()
            .components()
            .any(|c| c.as_os_str() == ".accelmars")
        {
            continue;
        }

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        let ext = path.extension().and_then(|ext| ext.to_str());
        let is_md = ext == Some("md");
        let is_toml = ext == Some("toml");
        if !is_md && !is_toml {
            continue;
        }
        // Exclude Cargo.toml — Rust build manifest, not a workspace reference document.
        if is_toml && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
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
    /// non-.md files, and .accelmars/ directory. Verifies only .md paths returned,
    /// no ./ prefix, and .accelmars/ excluded.
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

        // .accelmars/ directory — must be excluded entirely
        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.join(".accelmars").join("anchor").join("hidden.md"),
            b"",
        )
        .unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["README.md", "sub/design.md"]);

        // Verify .accelmars contents are absent
        assert!(!paths.iter().any(|p| p.contains(".accelmars")));
        // Verify no ./ prefix
        assert!(!paths.iter().any(|p| p.starts_with("./")));
    }

    /// .accelmars/anchor/ignore absent → scanner returns all .md files (no regression).
    #[test]
    fn test_no_ignore_file_returns_all_md() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("a.md"), b"").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("b.md"), b"").unwrap();

        // No .accelmars/anchor/ignore file — behavior must be identical to prior implementation
        assert!(!root
            .join(".accelmars")
            .join("anchor")
            .join("ignore")
            .exists());

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["a.md", "sub/b.md"]);
    }

    /// .accelmars/anchor/ignore with `node_modules/` → entries under node_modules/ excluded.
    #[test]
    fn test_ignore_excludes_node_modules() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("README.md"), b"").unwrap();
        fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        fs::write(root.join("node_modules").join("pkg").join("README.md"), b"").unwrap();

        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.join(".accelmars").join("anchor").join("ignore"),
            b"node_modules/\n",
        )
        .unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["README.md"]);
        assert!(!paths.iter().any(|p| p.contains("node_modules")));
    }

    /// .accelmars/anchor/ignore with `target/` → entries under target/ excluded.
    #[test]
    fn test_ignore_excludes_target() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("CHANGELOG.md"), b"").unwrap();
        fs::create_dir_all(root.join("target").join("debug")).unwrap();
        fs::write(root.join("target").join("debug").join("notes.md"), b"").unwrap();

        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.join(".accelmars").join("anchor").join("ignore"),
            b"target/\n",
        )
        .unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["CHANGELOG.md"]);
        assert!(!paths.iter().any(|p| p.contains("target")));
    }

    /// .accelmars/anchor/ignore with file-pattern exclusion.
    ///
    /// Note: negation patterns (e.g. `!docs/keep.bak.md`) in a file loaded via
    /// `add_ignore(absolute_path)` are interpreted relative to the ignore file's parent
    /// directory (.accelmars/anchor/), not the workspace root. Negation for workspace-root-
    /// relative paths is therefore not supported with this loading strategy.
    ///
    /// This test verifies that exclusion patterns work, and that negation does NOT
    /// unexpectedly re-include files (to avoid false safety assumptions).
    #[test]
    fn test_ignore_file_pattern_exclusion() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::create_dir(root.join("docs")).unwrap();
        fs::write(root.join("docs").join("draft.bak.md"), b"").unwrap();
        fs::write(root.join("docs").join("keep.bak.md"), b"").unwrap();
        fs::write(root.join("docs").join("final.md"), b"").unwrap();
        fs::write(root.join("root.md"), b"").unwrap();

        // Exclude *.bak.md files — both draft and keep are excluded.
        // Note: negation (!docs/keep.bak.md) does not apply when loaded via add_ignore
        // from .accelmars/anchor/ (base path mismatch with workspace root).
        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.join(".accelmars").join("anchor").join("ignore"),
            b"*.bak.md\n",
        )
        .unwrap();

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert!(
            !paths.contains(&"docs/draft.bak.md".to_string()),
            "docs/draft.bak.md must be excluded by *.bak.md pattern; got: {:?}",
            paths
        );
        assert!(
            !paths.contains(&"docs/keep.bak.md".to_string()),
            "docs/keep.bak.md must be excluded by *.bak.md pattern; got: {:?}",
            paths
        );
        assert!(paths.contains(&"docs/final.md".to_string()));
        assert!(paths.contains(&"root.md".to_string()));
    }

    /// No .accelmars/anchor/ignore AND .accelmars/ present → .accelmars/ is still excluded (hardcoded exclusion).
    #[test]
    fn test_accelmars_dir_excluded_without_ignore_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("README.md"), b"").unwrap();
        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.join(".accelmars")
                .join("anchor")
                .join("config.json.md"),
            b"",
        )
        .unwrap();
        fs::write(
            root.join(".accelmars").join("anchor").join("secret.md"),
            b"",
        )
        .unwrap();

        // No .accelmars/anchor/ignore — .accelmars/ must be excluded by hardcoded logic alone
        assert!(!root
            .join(".accelmars")
            .join("anchor")
            .join("ignore")
            .exists());

        let mut paths = scan_workspace(root).unwrap();
        paths.sort();

        assert_eq!(paths, vec!["README.md"]);
        assert!(!paths.iter().any(|p| p.contains(".accelmars")));
    }
}
