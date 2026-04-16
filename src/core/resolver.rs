// MF-004: Resolver + Canonical Path Model
//
// Converts raw reference paths to canonical workspace-root-relative paths.
// Form 1 algorithm: relative-to-source-file normalization.
// Form 2 algorithm: workspace-wide stem scan.
#![allow(dead_code)]
//
// PHASE-2-BRIDGE Contract 4: canonical path form is frozen here.
// Phase 2 will index files by canonical path in libSQL.
// Any redefinition of canonical form breaks Phase 2 index integrity.

use crate::model::CanonicalPath;
use path_clean::PathClean;
use std::path::Path;

/// Result of resolving a raw reference path.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolveResult {
    /// Resolved to a unique canonical path.
    Resolved(CanonicalPath),
    /// Zero matches found for a Form 2 stem — already-broken reference.
    BrokenRef,
    /// Two or more files match the stem — hard abort; caller must display all candidates.
    Ambiguous(Vec<CanonicalPath>),
}

/// Resolve a Form 1 (standard Markdown link) raw path to a canonical path.
///
/// Algorithm (05-PARSER.md §Resolution Algorithms §Form 1):
/// 1. Strip anchor from `raw_path` (caller already does this in MF-003, but we handle it anyway)
/// 2. Join with the directory of `source_file`
/// 3. Normalize (remove `..`, remove `./`, ensure forward slashes)
///
/// # Arguments
/// - `source_file`: canonical path of the file containing this reference
/// - `raw_path`: the path as written in the file (may contain `..`, `./`, no anchor)
///
/// # Returns
/// Workspace-root-relative canonical path (no `./` prefix, no `..`, forward slashes).
pub fn resolve_form1(source_file: &CanonicalPath, raw_path: &str) -> CanonicalPath {
    // Step 1: strip anchor fragment
    let path_no_anchor = match raw_path.find('#') {
        Some(idx) => &raw_path[..idx],
        None => raw_path,
    };

    // Step 2: join with source file's parent directory
    let parent = Path::new(source_file)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let joined = if parent.is_empty() {
        path_no_anchor.to_string()
    } else {
        format!("{}/{}", parent, path_no_anchor)
    };

    // Step 3: normalize — remove `..` and `./`, ensure forward slashes
    // path_clean operates on the joined string; we then normalize OS separators.
    let cleaned = Path::new(&joined).clean();
    let canonical = cleaned.to_string_lossy().replace('\\', "/");

    // Guarantee: no `./` prefix
    let canonical = canonical
        .strip_prefix("./")
        .unwrap_or(&canonical)
        .to_string();

    canonical
}

/// Resolve a Form 2 (wiki link) stem to a canonical path via workspace-wide scan.
///
/// Algorithm (05-PARSER.md §Resolution Algorithms §Form 2):
/// 1. `stem` is already stripped of `[[]]` and `.md` extension (done by parser in MF-003)
/// 2. For each path in `all_workspace_files`: compute stem (filename without `.md`)
/// 3. Collect all paths where `stem_of(path) == stem` (case-sensitive)
/// 4. 0 matches → `BrokenRef`
/// 5. 1 match → `Resolved(canonical_path)`
/// 6. 2+ matches → `Ambiguous(all_candidates)` — hard abort; includes ALL matching paths
///
/// # Arguments
/// - `stem`: the stem to search for (e.g. `"260415-decision"`)
/// - `all_workspace_files`: complete list of canonical paths of all `.md` files in workspace
///
/// # Returns
/// `ResolveResult` indicating resolution outcome.
pub fn resolve_form2(stem: &str, all_workspace_files: &[CanonicalPath]) -> ResolveResult {
    let matches: Vec<CanonicalPath> = all_workspace_files
        .iter()
        .filter(|path| stem_of(path) == stem)
        .cloned()
        .collect();

    match matches.len() {
        0 => ResolveResult::BrokenRef,
        1 => ResolveResult::Resolved(matches.into_iter().next().unwrap()),
        _ => ResolveResult::Ambiguous(matches),
    }
}

/// Compute the stem (filename without `.md` extension) of a canonical path.
/// e.g. `"docs/decisions/2026-decision.md"` → `"2026-decision"`
fn stem_of(canonical: &str) -> &str {
    let filename = canonical.rsplit('/').next().unwrap_or(canonical);
    filename.strip_suffix(".md").unwrap_or(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Form 1 tests ────────────────────────────────────────────────────────

    // Test 1: Same directory — source and target in same dir
    #[test]
    fn test_form1_same_directory() {
        let result = resolve_form1(&"a/b/c.md".to_string(), "d.md");
        assert_eq!(result, "a/b/d.md");
    }

    // Test 2: Subdirectory — target is in a subdirectory of source's parent
    #[test]
    fn test_form1_subdirectory() {
        let result = resolve_form1(&"a/b.md".to_string(), "sub/c.md");
        assert_eq!(result, "a/sub/c.md");
    }

    // Test 3: Parent directory — one level up with `..`
    #[test]
    fn test_form1_parent_directory() {
        let result = resolve_form1(&"a/b/c.md".to_string(), "../d.md");
        assert_eq!(result, "a/d.md");
    }

    // Test 4: Multi-level `..` — two levels up
    #[test]
    fn test_form1_multilevel_dotdot() {
        let result = resolve_form1(&"a/b/c/d.md".to_string(), "../../e.md");
        assert_eq!(result, "a/e.md");
    }

    // Test 5: Root-level source file — no parent directory component
    #[test]
    fn test_form1_root_level_source() {
        let result = resolve_form1(&"CLAUDE.md".to_string(), "protocols/FOO.md");
        assert_eq!(result, "protocols/FOO.md");
    }

    // ─── Form 2 tests ────────────────────────────────────────────────────────

    // Test 6: 0 matches → BrokenRef
    #[test]
    fn test_form2_zero_matches() {
        let workspace = vec![
            "projects/team/STATUS.md".to_string(),
            "docs/members/alice.md".to_string(),
        ];
        let result = resolve_form2("nonexistent-decision", &workspace);
        assert_eq!(result, ResolveResult::BrokenRef);
    }

    // Test 7: 1 match → Resolved with full canonical path
    #[test]
    fn test_form2_one_match() {
        let workspace = vec![
            "docs/decisions/2026-decision.md".to_string(),
            "docs/members/alice.md".to_string(),
        ];
        let result = resolve_form2("2026-decision", &workspace);
        assert_eq!(
            result,
            ResolveResult::Resolved("docs/decisions/2026-decision.md".to_string())
        );
    }

    // Test 8: 2+ matches → Ambiguous with ALL candidates (vec length == 2)
    #[test]
    fn test_form2_ambiguous() {
        let workspace = vec![
            "projects/alpha/shared-stem.md".to_string(),
            "projects/beta/shared-stem.md".to_string(),
            "docs/members/alice.md".to_string(),
        ];
        let result = resolve_form2("shared-stem", &workspace);
        match result {
            ResolveResult::Ambiguous(candidates) => {
                assert_eq!(candidates.len(), 2, "must include ALL matching candidates");
                assert!(candidates.contains(&"projects/alpha/shared-stem.md".to_string()));
                assert!(candidates.contains(&"projects/beta/shared-stem.md".to_string()));
            }
            other => panic!("expected Ambiguous, got {:?}", other),
        }
    }

    // ─── Canonical form invariant tests ──────────────────────────────────────

    // Test 9: Raw `./foo/../bar/baz.md` → canonical `bar/baz.md` (no `./`, no `..`)
    #[test]
    fn test_canonical_form_normalization() {
        // Source at root so parent is empty; raw path starts with ./
        let result = resolve_form1(&"ROOT.md".to_string(), "./foo/../bar/baz.md");
        assert_eq!(result, "bar/baz.md");
        assert!(
            !result.starts_with("./"),
            "canonical must not start with ./"
        );
        assert!(!result.contains(".."), "canonical must not contain ..");
    }

    // Test 10: Two different raw forms that resolve to the same file → identical canonical strings
    #[test]
    fn test_canonical_form_identical_for_same_target() {
        // Both raw paths refer to `a/b/target.md` from different angles
        let canonical_a = resolve_form1(&"a/b/source.md".to_string(), "target.md");
        let canonical_b = resolve_form1(&"a/b/other/source2.md".to_string(), "../target.md");
        assert_eq!(
            canonical_a, canonical_b,
            "different raw paths to same file must produce identical canonical paths"
        );
        assert_eq!(canonical_a, "a/b/target.md");
    }
}
