//! Acknowledged broken-reference entries (.accelmars/anchor/acked).
//!
//! Two distinct mechanisms share the same `.accelmars/anchor/acked` file:
//!
//! - `AckedPatterns` (gitignore-style): suppresses broken ref output for entire source files
//!   in `anchor file validate`. Used by `anchor validate` output filtering.
//! - `AckedRefs` (file:line tuples): suppresses specific broken refs during `anchor apply`.
//!   Lines of the form `<file>:<line_number>` are parsed only by `AckedRefs`; `AckedPatterns`
//!   ignores them (colon is not a gitignore special character, so `foo.md:42` matches no real
//!   file path on Unix filesystems).
//!
//! If `.accelmars/anchor/acked` does not exist or cannot be read, no suppression occurs.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

pub struct AckedPatterns {
    inner: Option<Gitignore>,
}

impl AckedPatterns {
    /// Load `.accelmars/anchor/acked` from `workspace_root`. Returns an instance with no
    /// patterns if the file is absent or unreadable (not an error).
    pub fn load(workspace_root: &Path) -> Self {
        let path = workspace_root
            .join(".accelmars")
            .join("anchor")
            .join("acked");
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
    /// (forward slashes, no leading `./`), e.g. `"my-workspace/projects/archive/foo.md"`.
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

    fn write_anchor_acked(root: &Path, content: &str) {
        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.join(".accelmars").join("anchor").join("acked"),
            content,
        )
        .unwrap();
    }

    /// No .accelmars/anchor/acked file → is_acked always returns false.
    #[test]
    fn test_absent_returns_false() {
        let tmp = TempDir::new().unwrap();
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("my-workspace/projects/archive/foo.md"));
        assert!(!acked.is_acked("any/path.md"));
    }

    /// Pattern matches source file → is_acked returns true.
    #[test]
    fn test_matching_pattern_returns_true() {
        let tmp = TempDir::new().unwrap();
        write_anchor_acked(tmp.path(), "my-workspace/projects/archive/\n");
        let acked = AckedPatterns::load(tmp.path());
        assert!(acked.is_acked("my-workspace/projects/archive/old-contract.md"));
    }

    /// Pattern does NOT match source → is_acked returns false.
    #[test]
    fn test_non_matching_pattern_returns_false() {
        let tmp = TempDir::new().unwrap();
        write_anchor_acked(tmp.path(), "my-workspace/projects/archive/\n");
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("my-workspace/projects/active/current.md"));
    }

    /// Multiple patterns — each matched independently.
    #[test]
    fn test_multiple_patterns() {
        let tmp = TempDir::new().unwrap();
        write_anchor_acked(tmp.path(), "my-workspace/projects/archive/\nother-repo/\n");
        let acked = AckedPatterns::load(tmp.path());
        assert!(acked.is_acked("my-workspace/projects/archive/foo.md"));
        assert!(acked.is_acked("other-repo/design/old.md"));
        assert!(!acked.is_acked("my-workspace/active/current.md"));
    }

    /// Empty .accelmars/anchor/acked file → no patterns → is_acked always false.
    #[test]
    fn test_empty_file_no_patterns() {
        let tmp = TempDir::new().unwrap();
        write_anchor_acked(tmp.path(), "");
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("any/path.md"));
    }

    /// Comments-only .accelmars/anchor/acked file → no active patterns → is_acked false.
    #[test]
    fn test_comments_only() {
        let tmp = TempDir::new().unwrap();
        write_anchor_acked(tmp.path(), "# this is a comment\n# another comment\n");
        let acked = AckedPatterns::load(tmp.path());
        assert!(!acked.is_acked("any/path.md"));
    }
}

// ── AckedRefs: per-apply (file, line) override set ───────────────────────────

/// Per-apply broken-reference override set for `anchor apply --allow-broken`.
///
/// Entries are `(workspace_root_relative_canonical_path, 1-based_line_number)` tuples.
/// A broken ref is suppressed during `anchor apply` if its `(file, line)` appears here.
pub(crate) struct AckedRefs {
    entries: HashSet<(String, usize)>,
}

impl AckedRefs {
    pub(crate) fn empty() -> Self {
        Self {
            entries: HashSet::new(),
        }
    }

    /// Load `file:line` entries from `.accelmars/anchor/acked`. Tolerates missing file.
    pub(crate) fn load(workspace_root: &Path) -> Self {
        let path = workspace_root.join(".accelmars").join("anchor").join("acked");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::empty(),
        };
        let entries = content.lines().filter_map(parse_ref_line).collect();
        Self { entries }
    }

    /// Append newly specified refs to `.accelmars/anchor/acked`. Skips existing duplicates.
    pub(crate) fn save(workspace_root: &Path, refs: &[(String, usize)]) {
        if refs.is_empty() {
            return;
        }
        let existing = Self::load(workspace_root);
        let new_lines: Vec<String> = refs
            .iter()
            .filter(|(f, l)| !existing.contains(f, *l))
            .map(|(f, l)| format!("{f}:{l}"))
            .collect();
        if new_lines.is_empty() {
            return;
        }
        let path = workspace_root.join(".accelmars").join("anchor").join("acked");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let text = new_lines.join("\n") + "\n";
            let _ = file.write_all(text.as_bytes());
        }
    }

    pub(crate) fn contains(&self, file: &str, line: usize) -> bool {
        self.entries.contains(&(file.to_string(), line))
    }

    pub(crate) fn add(&mut self, file: &str, line: usize) {
        self.entries.insert((file.to_string(), line));
    }
}

/// Parse a `file:line` entry from a single line of the acked file.
///
/// Returns `None` for gitignore-style patterns (no numeric suffix after last colon),
/// comments, and empty lines — those belong to `AckedPatterns`, not `AckedRefs`.
pub(crate) fn parse_ref_line(line: &str) -> Option<(String, usize)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let colon = line.rfind(':')?;
    let num: usize = line[colon + 1..].parse().ok()?;
    let file = &line[..colon];
    if file.is_empty() {
        return None;
    }
    Some((file.to_string(), num))
}

#[cfg(test)]
mod acked_refs_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        tmp
    }

    fn write_acked(root: &Path, content: &str) {
        fs::write(
            root.join(".accelmars").join("anchor").join("acked"),
            content,
        )
        .unwrap();
    }

    // ── parse_ref_line ────────────────────────────────────────────────────────

    #[test]
    fn test_parse_ref_line_valid() {
        assert_eq!(
            parse_ref_line("foundations/engine.md:42"),
            Some(("foundations/engine.md".to_string(), 42))
        );
    }

    #[test]
    fn test_parse_ref_line_comment_skipped() {
        assert_eq!(parse_ref_line("# comment"), None);
    }

    #[test]
    fn test_parse_ref_line_gitignore_pattern_skipped() {
        // gitignore-style pattern — no numeric suffix
        assert_eq!(parse_ref_line("my-workspace/projects/archive/"), None);
    }

    #[test]
    fn test_parse_ref_line_empty_skipped() {
        assert_eq!(parse_ref_line(""), None);
        assert_eq!(parse_ref_line("   "), None);
    }

    #[test]
    fn test_parse_ref_line_no_file_skipped() {
        assert_eq!(parse_ref_line(":42"), None);
    }

    // ── AckedRefs::load ───────────────────────────────────────────────────────

    #[test]
    fn test_load_absent_file_returns_empty() {
        let ws = make_workspace();
        let acked = AckedRefs::load(ws.path());
        assert!(!acked.contains("any/file.md", 1));
    }

    #[test]
    fn test_load_parses_file_line_entries() {
        let ws = make_workspace();
        write_acked(ws.path(), "foundations/engine.md:42\nother/file.md:7\n");
        let acked = AckedRefs::load(ws.path());
        assert!(acked.contains("foundations/engine.md", 42));
        assert!(acked.contains("other/file.md", 7));
        assert!(!acked.contains("foundations/engine.md", 43));
    }

    #[test]
    fn test_load_skips_gitignore_patterns() {
        let ws = make_workspace();
        // Mix of gitignore patterns and file:line entries
        write_acked(
            ws.path(),
            "my-workspace/archive/\nfoundations/engine.md:42\n# comment\n",
        );
        let acked = AckedRefs::load(ws.path());
        assert!(acked.contains("foundations/engine.md", 42));
        assert!(!acked.contains("my-workspace/archive/", 0));
    }

    // ── AckedRefs::save ───────────────────────────────────────────────────────

    #[test]
    fn test_save_appends_new_entries() {
        let ws = make_workspace();
        AckedRefs::save(ws.path(), &[("file.md".to_string(), 1)]);
        let acked = AckedRefs::load(ws.path());
        assert!(acked.contains("file.md", 1));
    }

    #[test]
    fn test_save_deduplicates_existing_entries() {
        let ws = make_workspace();
        AckedRefs::save(ws.path(), &[("file.md".to_string(), 1)]);
        AckedRefs::save(ws.path(), &[("file.md".to_string(), 1)]);
        let content = fs::read_to_string(
            ws.path().join(".accelmars").join("anchor").join("acked"),
        )
        .unwrap();
        assert_eq!(
            content.lines().filter(|l| *l == "file.md:1").count(),
            1,
            "duplicate entry must not be written"
        );
    }

    #[test]
    fn test_save_creates_file_when_absent() {
        let ws = make_workspace();
        AckedRefs::save(ws.path(), &[("new.md".to_string(), 5)]);
        assert!(ws
            .path()
            .join(".accelmars")
            .join("anchor")
            .join("acked")
            .exists());
    }

    // ── AckedRefs::add + contains ─────────────────────────────────────────────

    #[test]
    fn test_add_and_contains() {
        let mut acked = AckedRefs::empty();
        assert!(!acked.contains("file.md", 1));
        acked.add("file.md", 1);
        assert!(acked.contains("file.md", 1));
        assert!(!acked.contains("file.md", 2));
        assert!(!acked.contains("other.md", 1));
    }
}
