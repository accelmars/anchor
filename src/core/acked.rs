//! Acknowledged broken-reference patterns (.mindacked).
//!
//! Reads `.mindacked` at the workspace root. Source canonical paths matching
//! any pattern have their broken outbound refs suppressed from validate output.
//! Files are still scanned and indexed — suppression applies to output only.
//!
//! If `.mindacked` does not exist or cannot be read, no suppression occurs.
//!
//! Note: `.mindignore` (exclude from index) and `.mindacked` (suppress output) are
//! orthogonal. A path in both is valid; `.mindignore` wins (not scanned = no refs
//! produced). Adding the same path to `.mindacked` is harmless but redundant.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

pub struct AckedPatterns {
    inner: Option<Gitignore>,
}

impl AckedPatterns {
    /// Load `.mindacked` from `workspace_root`. Returns an instance with no
    /// patterns if the file is absent or unreadable (not an error).
    pub fn load(workspace_root: &Path) -> Self {
        let path = workspace_root.join(".mindacked");
        if !path.exists() {
            return Self { inner: None };
        }
        let mut builder = GitignoreBuilder::new(workspace_root);
        // add() returns Option<Error> for parse errors per line; ignore silently.
        builder.add(&path);
        match builder.build() {
            Ok(gi) => Self { inner: Some(gi) },
            // Unreadable file: treat as absent — no suppression, no crash.
            Err(_) => Self { inner: None },
        }
    }

    /// Returns true if the broken refs from `source_canonical` should be
    /// suppressed from validate output.
    ///
    /// `source_canonical` must be a workspace-root-relative canonical path
    /// (forward slashes, no leading `./`), e.g. `"accelmars-guild/projects/archive/foo.md"`.
    ///
    /// Uses `matched_path_or_any_parents` so that directory patterns like `archive/`
    /// correctly suppress files inside the directory, not just the directory itself.
    pub fn is_acked(&self, source_canonical: &str) -> bool {
        match &self.inner {
            None => false,
            Some(gi) => gi
                .matched_path_or_any_parents(Path::new(source_canonical), false)
                .is_ignore(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_mindacked(root: &Path, content: &str) {
        fs::write(root.join(".mindacked"), content).unwrap();
    }

    /// No .mindacked file → is_acked always returns false.
    #[test]
    fn test_absent_returns_false() {
        let tmp = TempDir::new().unwrap();
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("accelmars-guild/projects/archive/foo.md"));
        assert!(!acked.is_acked("any/path.md"));
    }

    /// Pattern matches source file → is_acked returns true.
    #[test]
    fn test_matching_pattern_returns_true() {
        let tmp = TempDir::new().unwrap();
        write_mindacked(tmp.path(), "accelmars-guild/projects/archive/\n");
        let acked = AckedPatterns::load(tmp.path());
        assert!(acked.is_acked("accelmars-guild/projects/archive/old-contract.md"));
    }

    /// Pattern does NOT match source → is_acked returns false.
    #[test]
    fn test_non_matching_pattern_returns_false() {
        let tmp = TempDir::new().unwrap();
        write_mindacked(tmp.path(), "accelmars-guild/projects/archive/\n");
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("accelmars-guild/projects/active/current.md"));
    }

    /// Multiple patterns — each matched independently.
    #[test]
    fn test_multiple_patterns() {
        let tmp = TempDir::new().unwrap();
        write_mindacked(
            tmp.path(),
            "accelmars-guild/projects/archive/\nschole-meta/\n",
        );
        let acked = AckedPatterns::load(tmp.path());
        assert!(acked.is_acked("accelmars-guild/projects/archive/foo.md"));
        assert!(acked.is_acked("schole-meta/design/old.md"));
        assert!(!acked.is_acked("accelmars-guild/active/current.md"));
    }

    /// Empty .mindacked file → no patterns → is_acked always false.
    #[test]
    fn test_empty_file_no_patterns() {
        let tmp = TempDir::new().unwrap();
        write_mindacked(tmp.path(), "");
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("any/path.md"));
    }

    /// Comments-only .mindacked file → no active patterns → is_acked false.
    #[test]
    fn test_comments_only() {
        let tmp = TempDir::new().unwrap();
        write_mindacked(tmp.path(), "# this is a comment\n# another comment\n");
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("any/path.md"));
    }
}
