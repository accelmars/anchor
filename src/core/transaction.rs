// src/core/transaction.rs — PLAN phase logic + Case A/B/C classification (MF-005)
#![allow(dead_code)]
//
// APPLY, VALIDATE, COMMIT phases are in MF-006 (cli/file/mv.rs + core/rewriter.rs).
//
// Anti-pattern from HANDOVER.md (Case C detection bug):
//   WRONG: skip if reference SOURCE is inside src/ — this skips Case B (which needs rewriting).
//   CORRECT: Case C requires BOTH source file AND target inside src/.
//   Test: is_case_c = inside_src(ref.source_file) AND inside_src(ref.target)

use crate::core::{parser, resolver};
use crate::model::{
    reference::RefForm,
    rewrite::{RewriteEntry, RewritePlan},
    CanonicalPath,
};
use std::path::Path;

/// Error returned by transaction operations.
#[derive(Debug)]
pub enum TransactionError {
    Io(std::io::Error),
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionError::Io(e) => write!(f, "transaction I/O error: {e}"),
        }
    }
}

impl From<std::io::Error> for TransactionError {
    fn from(e: std::io::Error) -> Self {
        TransactionError::Io(e)
    }
}

/// Returns true if `path` is `src` or is a file/directory inside `src/`.
///
/// Handles both file moves (`path == src`) and directory moves (`path` starts with `src/`).
///
/// IMPORTANT (HANDOVER.md anti-pattern): Case C requires BOTH source_file AND target
/// to satisfy inside_src. Checking only one side produces incorrect classification.
fn inside_src(path: &CanonicalPath, src: &CanonicalPath) -> bool {
    path == src || path.starts_with(&format!("{src}/"))
}

/// Remap a canonical path from under `src` to under `dst`.
///
/// If `canonical == src`, returns `dst`.
/// If `canonical` starts with `src/`, substitutes the prefix.
///
/// Example: src="projects/foo", dst="projects/archive/foo"
///   "projects/foo/bar.md" → "projects/archive/foo/bar.md"
fn remap_path(
    canonical: &CanonicalPath,
    src: &CanonicalPath,
    dst: &CanonicalPath,
) -> CanonicalPath {
    if canonical == src {
        dst.clone()
    } else {
        // canonical starts with "src/" (guaranteed by caller via inside_src check)
        format!("{dst}{}", &canonical[src.len()..])
    }
}

/// Compute the relative path from `from_file`'s directory to `to_file`.
///
/// Both arguments are workspace-root-relative canonical paths. The result is the
/// relative path string that should appear in a Markdown Form 1 reference
/// (e.g. `"../../people/anna/SKILL.md"`).
fn compute_relative_path(from_file: &CanonicalPath, to_file: &CanonicalPath) -> String {
    // Determine the directory containing from_file
    let from_dir: Vec<&str> = match from_file.rfind('/') {
        Some(idx) => from_file[..idx].split('/').collect(),
        None => vec![], // file at workspace root
    };

    let to_parts: Vec<&str> = to_file.split('/').collect();

    // Find common prefix length
    let common_len = from_dir
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let up_count = from_dir.len() - common_len;
    let down_parts = &to_parts[common_len..];

    let mut rel: Vec<&str> = (0..up_count).map(|_| "..").collect();
    rel.extend_from_slice(down_parts);

    if rel.is_empty() {
        // Same file — should not occur in valid move operations
        to_file.clone()
    } else {
        rel.join("/")
    }
}

/// Reconstruct a Form 1 Markdown reference with an updated path.
///
/// Preserves the original link text and anchor (if any); replaces only the path.
///
/// Input:  `old_text` = `"[link text](old/path.md#anchor)"`, new_rel_path = `"new/path.md"`
/// Output: `"[link text](new/path.md#anchor)"`
fn rebuild_form1_ref(old_text: &str, new_rel_path: &str, anchor: &Option<String>) -> String {
    // Find the ]( boundary separating link text from path
    let bracket_paren = old_text
        .find("](")
        .expect("Form 1 reference must contain ](");
    let link_text = &old_text[1..bracket_paren]; // between [ and ](

    let path_with_anchor = match anchor {
        Some(a) => format!("{new_rel_path}#{a}"),
        None => new_rel_path.to_string(),
    };

    format!("[{link_text}]({path_with_anchor})")
}

/// PLAN phase: scan workspace, classify all references (A/B/C), build the rewrite plan.
///
/// Algorithm from 04-TRANSACTIONS.md §PLAN Phase Detail:
/// 1. For each file in `workspace_files`: read content, parse all references
/// 2. Resolve each Form 1 reference to a canonical path
/// 3. Skip Form 2 (wiki links): stem doesn't change when a file is moved; no rewrite needed
/// 4. Classify each reference that touches `src`:
///    - Case A: !inside_src(source_file) && inside_src(target) → rewrite target path
///    - Case B:  inside_src(source_file) && !inside_src(target) → rewrite relative path (source moved)
///    - Case C:  inside_src(source_file) &&  inside_src(target) → skip (relative path stable)
/// 5. For each Case A and Case B: compute old_text + new_text via path remapping
/// 6. Return RewritePlan (manifest.phase update → "APPLY" is the orchestrator's responsibility)
///
/// # Arguments
/// - `workspace_root`: absolute path to the workspace root directory
/// - `src`: canonical path of the file or directory being moved
/// - `dst`: canonical path of the destination
/// - `workspace_files`: pre-computed list of all `.md` files in the workspace
pub fn plan(
    workspace_root: &Path,
    src: &CanonicalPath,
    dst: &CanonicalPath,
    workspace_files: &[CanonicalPath],
) -> Result<RewritePlan, TransactionError> {
    let mut entries: Vec<RewriteEntry> = Vec::new();

    for file_canonical in workspace_files {
        let file_path = workspace_root.join(file_canonical.as_str());
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File disappeared between scan and plan — skip silently
                continue;
            }
            Err(e) => return Err(TransactionError::Io(e)),
        };

        let refs = parser::parse_references(file_canonical, &content);

        for reference in refs {
            // Form 2 (wiki links): stem-based resolution means the stem doesn't change
            // when a file is moved. No rewrite needed for any wiki link.
            if reference.form == RefForm::Wiki {
                continue;
            }

            // Form 1: resolve to canonical target
            let target_canonical = resolver::resolve_form1(file_canonical, &reference.target_raw);

            let source_inside = inside_src(file_canonical, src);
            let target_inside = inside_src(&target_canonical, src);

            match (source_inside, target_inside) {
                // Case C: both inside src — relative path between them is stable after move
                (true, true) => continue,

                // Neither touches src — not relevant to this move
                (false, false) => continue,

                // Case A: external file references a file inside src/
                //   → rewrite the target path in the external file
                (false, true) => {
                    let new_target = remap_path(&target_canonical, src, dst);
                    let new_rel = compute_relative_path(file_canonical, &new_target);
                    let old_text = content[reference.span.0..reference.span.1].to_string();
                    let new_text = rebuild_form1_ref(&old_text, &new_rel, &reference.anchor);
                    entries.push(RewriteEntry {
                        file: file_canonical.clone(),
                        span: reference.span,
                        old_text,
                        new_text,
                    });
                }

                // Case B: file inside src/ references an external file
                //   → the source file will move, so recompute relative path from its new location
                (true, false) => {
                    let new_source = remap_path(file_canonical, src, dst);
                    let new_rel = compute_relative_path(&new_source, &target_canonical);
                    let old_text = content[reference.span.0..reference.span.1].to_string();
                    let new_text = rebuild_form1_ref(&old_text, &new_rel, &reference.anchor);
                    entries.push(RewriteEntry {
                        file: file_canonical.clone(),
                        span: reference.span,
                        old_text,
                        new_text,
                    });
                }
            }
        }
    }

    Ok(RewritePlan {
        src: src.clone(),
        dst: dst.clone(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Set up a temp workspace and return its root path.
    fn make_workspace() -> TempDir {
        TempDir::new().unwrap()
    }

    /// Write a file at `root/canonical_path` with the given content, creating parent dirs.
    fn write_file(root: &Path, canonical: &str, content: &str) {
        let path = root.join(canonical);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
    }

    /// Test 6: External file references file inside src/ → classified as Case A.
    ///         A RewriteEntry must be generated for the external file.
    #[test]
    fn test_case_a_external_references_inside_src() {
        let tmp = make_workspace();
        let root = tmp.path();

        // src directory being moved
        let src = "projects/foo".to_string();
        let dst = "projects/archive/foo".to_string();

        // File INSIDE src
        write_file(root, "projects/foo/bar.md", "# Bar\n");

        // File OUTSIDE src that references something inside src (Case A)
        // From "docs/README.md", the path to "projects/foo/bar.md" is "../projects/foo/bar.md"
        write_file(
            root,
            "docs/README.md",
            "See [bar](../projects/foo/bar.md).\n",
        );

        let workspace_files = vec![
            "projects/foo/bar.md".to_string(),
            "docs/README.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        // Must have exactly one RewriteEntry for docs/README.md (Case A)
        assert_eq!(plan.entries.len(), 1, "exactly one Case A entry expected");
        let entry = &plan.entries[0];
        assert_eq!(
            entry.file, "docs/README.md",
            "entry must be in the external file"
        );
        assert!(
            entry.old_text.contains("projects/foo/bar.md"),
            "old_text must contain old path, got: {}",
            entry.old_text
        );
        assert!(
            entry.new_text.contains("projects/archive/foo/bar.md"),
            "new_text must contain new path after remap, got: {}",
            entry.new_text
        );
    }

    /// Test 7: File inside src/ references external file → classified as Case B.
    ///         A RewriteEntry must be generated for the inside-src file.
    #[test]
    fn test_case_b_inside_src_references_external() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "projects/foo".to_string();
        let dst = "projects/archive/foo".to_string();

        // External file (target of the reference)
        write_file(root, "people/anna/SKILL.md", "# Anna\n");

        // File INSIDE src that references the external file (Case B)
        // From "projects/foo/notes.md", path to "people/anna/SKILL.md" is "../../people/anna/SKILL.md"
        write_file(
            root,
            "projects/foo/notes.md",
            "See [Anna](../../people/anna/SKILL.md).\n",
        );

        let workspace_files = vec![
            "people/anna/SKILL.md".to_string(),
            "projects/foo/notes.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        // Must have exactly one RewriteEntry for projects/foo/notes.md (Case B)
        assert_eq!(plan.entries.len(), 1, "exactly one Case B entry expected");
        let entry = &plan.entries[0];
        assert_eq!(
            entry.file, "projects/foo/notes.md",
            "entry must be in the inside-src file"
        );
        assert!(
            entry.old_text.contains("../../people/anna/SKILL.md"),
            "old_text must contain old relative path, got: {}",
            entry.old_text
        );
        // After move: "projects/archive/foo/notes.md" → "../../../people/anna/SKILL.md"
        assert!(
            entry.new_text.contains("people/anna/SKILL.md"),
            "new_text must reference the external target, got: {}",
            entry.new_text
        );
    }

    /// Test 8: File inside src/ references another file inside src/ → Case C.
    ///         NO RewriteEntry must appear for this reference (relative path stable).
    #[test]
    fn test_case_c_both_inside_src_skipped() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "projects/foo".to_string();
        let dst = "projects/archive/foo".to_string();

        // Both files are inside src/
        write_file(root, "projects/foo/a.md", "# A\n");
        // File b.md references a.md — both inside src/ → Case C
        write_file(root, "projects/foo/b.md", "See [a](a.md).\n");

        let workspace_files = vec![
            "projects/foo/a.md".to_string(),
            "projects/foo/b.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        // Case C: relative path between a.md and b.md is stable — NO entries
        assert!(
            plan.entries.is_empty(),
            "Case C references must not produce RewriteEntry, got: {:?}",
            plan.entries
        );
    }

    /// Verify inside_src correctly handles file move (exact match) and directory move (prefix).
    #[test]
    fn test_inside_src() {
        let src = "projects/foo".to_string();

        // Exact match — file move case
        assert!(inside_src(&"projects/foo".to_string(), &src));
        // File inside directory
        assert!(inside_src(&"projects/foo/bar.md".to_string(), &src));
        // Nested directory inside src
        assert!(inside_src(&"projects/foo/sub/baz.md".to_string(), &src));
        // Same prefix but different name — must NOT match (projects/foobar != projects/foo/)
        assert!(!inside_src(&"projects/foobar.md".to_string(), &src));
        // Completely different path
        assert!(!inside_src(&"people/anna/SKILL.md".to_string(), &src));
    }

    /// Verify compute_relative_path produces correct relative paths.
    #[test]
    fn test_compute_relative_path() {
        // Same directory
        assert_eq!(
            compute_relative_path(&"a/b/source.md".to_string(), &"a/b/target.md".to_string()),
            "target.md"
        );
        // One level up
        assert_eq!(
            compute_relative_path(&"a/b/source.md".to_string(), &"a/target.md".to_string()),
            "../target.md"
        );
        // Two levels up, different branch
        assert_eq!(
            compute_relative_path(
                &"projects/foo/source.md".to_string(),
                &"people/anna/SKILL.md".to_string()
            ),
            "../../people/anna/SKILL.md"
        );
        // Source at root level
        assert_eq!(
            compute_relative_path(&"ROOT.md".to_string(), &"projects/foo/bar.md".to_string()),
            "projects/foo/bar.md"
        );
    }
}
