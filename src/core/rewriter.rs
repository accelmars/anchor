// src/core/rewriter.rs — span-based reference rewriter (MF-006)
#![allow(dead_code)]

use crate::model::{rewrite::RewriteEntry, CanonicalPath};
use relative_path::RelativePath;

/// Apply span-based rewrites to `content`, returning the fully rewritten string.
///
/// Rewrites are applied in **descending** order of span start offset so that
/// each replacement preserves the byte offsets of earlier spans. The span list
/// must contain non-overlapping spans (guaranteed by PLAN phase).
///
/// From 05-PARSER.md §Rewrite Engine: the engine modifies only parsed spans —
/// never global string replacement.
pub fn apply_rewrites(content: &str, entries: &[RewriteEntry]) -> String {
    // Sort descending so we apply from end to start — preserves earlier span validity.
    let mut sorted: Vec<&RewriteEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| b.span.0.cmp(&a.span.0));

    let mut result = content.to_string();
    for entry in sorted {
        result.replace_range(entry.span.0..entry.span.1, &entry.new_text);
    }
    result
}

/// Compute the Form 1 (standard Markdown link) path text to place inside `[text]({here})`.
///
/// Computes the relative path from the parent directory of `source_file` to `dst`,
/// using the `relative-path` crate. Appends `#anchor` if present.
///
/// # Arguments
/// - `source_file`: canonical path of the file that contains the reference
///   (the file is being rewritten, NOT the file being moved)
/// - `dst`: canonical destination path of the file being moved
/// - `anchor`: optional anchor fragment to append (e.g. `"section-heading"`)
///
/// # Returns
/// The new raw path string for inside `[text](...)`, e.g. `"../../people/anna/SKILL.md"`.
pub fn compute_form1_new_text(
    source_file: &CanonicalPath,
    dst: &CanonicalPath,
    anchor: &Option<String>,
) -> String {
    // Parent directory of the source file (the `from` anchor for relative computation)
    let parent = match source_file.rfind('/') {
        Some(idx) => source_file[..idx].to_string(),
        None => String::new(), // source file is at workspace root
    };

    // relative-path: from.relative(to) = path to get from `from` dir to `to`
    let from = RelativePath::new(&parent);
    let to = RelativePath::new(dst.as_str());
    let rel = from.relative(to);
    let path_str = rel.to_string();

    match anchor {
        Some(a) => format!("{path_str}#{a}"),
        None => path_str,
    }
}

/// Compute the Form 2 (wiki link) stem replacement.
///
/// From 05-PARSER.md §Form 2 rewrite: `[[old-stem]]` → `[[new-stem]]` where
/// `new-stem = stem_of(dst)`. Returns the stem only (no `[[` or `]]`).
///
/// # Example
/// `dst = "projects/archive/my-decision.md"` → `"my-decision"`
pub fn compute_form2_new_text(dst: &CanonicalPath) -> String {
    let filename = dst.rsplit('/').next().unwrap_or(dst.as_str());
    filename.strip_suffix(".md").unwrap_or(filename).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::rewrite::RewriteEntry;

    // ─── apply_rewrites ───────────────────────────────────────────────────────

    /// Single rewrite at the start of content.
    #[test]
    fn test_apply_single_rewrite_at_start() {
        let content = "[text](old/path.md) other content";
        let entries = vec![RewriteEntry {
            file: "source.md".to_string(),
            span: (0, "[text](old/path.md)".len()),
            old_text: "[text](old/path.md)".to_string(),
            new_text: "[text](new/path.md)".to_string(),
        }];
        let result = apply_rewrites(content, &entries);
        assert_eq!(result, "[text](new/path.md) other content");
    }

    /// Multiple rewrites in the same file — applied end-to-start so spans stay valid.
    #[test]
    fn test_apply_multiple_rewrites_preserves_spans() {
        let content = "[a](old/a.md) some text [b](old/b.md) end";
        // span of [b](old/b.md): starts at 21
        let b_start = content.find("[b](old/b.md)").unwrap();
        let b_end = b_start + "[b](old/b.md)".len();
        let a_start = 0usize;
        let a_end = "[a](old/a.md)".len();

        let entries = vec![
            RewriteEntry {
                file: "s.md".to_string(),
                span: (a_start, a_end),
                old_text: "[a](old/a.md)".to_string(),
                new_text: "[a](new/a.md)".to_string(),
            },
            RewriteEntry {
                file: "s.md".to_string(),
                span: (b_start, b_end),
                old_text: "[b](old/b.md)".to_string(),
                new_text: "[b](new/b.md)".to_string(),
            },
        ];
        let result = apply_rewrites(content, &entries);
        assert_eq!(result, "[a](new/a.md) some text [b](new/b.md) end");
    }

    /// Zero entries returns content unchanged.
    #[test]
    fn test_apply_no_entries_unchanged() {
        let content = "no rewrites here";
        let result = apply_rewrites(content, &[]);
        assert_eq!(result, content);
    }

    // ─── compute_form1_new_text ───────────────────────────────────────────────

    /// Source in subdirectory, dst in a different branch — verify relative path.
    #[test]
    fn test_form1_cross_directory() {
        let rel = compute_form1_new_text(
            &"projects/foo/notes.md".to_string(),
            &"people/anna/SKILL.md".to_string(),
            &None,
        );
        assert_eq!(rel, "../../people/anna/SKILL.md");
    }

    /// Source at workspace root — no leading `..` in relative path.
    #[test]
    fn test_form1_source_at_root() {
        let rel = compute_form1_new_text(
            &"CLAUDE.md".to_string(),
            &"projects/foo/bar.md".to_string(),
            &None,
        );
        assert_eq!(rel, "projects/foo/bar.md");
    }

    /// Anchor is appended with `#`.
    #[test]
    fn test_form1_with_anchor() {
        let rel = compute_form1_new_text(
            &"docs/README.md".to_string(),
            &"projects/foo/bar.md".to_string(),
            &Some("section".to_string()),
        );
        assert_eq!(rel, "../projects/foo/bar.md#section");
    }

    /// Same directory — no `..` needed.
    #[test]
    fn test_form1_same_directory() {
        let rel = compute_form1_new_text(
            &"a/b/source.md".to_string(),
            &"a/b/target.md".to_string(),
            &None,
        );
        assert_eq!(rel, "target.md");
    }

    // ─── compute_form2_new_text ───────────────────────────────────────────────

    /// Deep path → stem of filename.
    #[test]
    fn test_form2_deep_path() {
        let stem = compute_form2_new_text(&"projects/archive/my-decision.md".to_string());
        assert_eq!(stem, "my-decision");
    }

    /// Root-level file.
    #[test]
    fn test_form2_root_file() {
        let stem = compute_form2_new_text(&"README.md".to_string());
        assert_eq!(stem, "README");
    }

    /// File without `.md` extension (unusual but shouldn't panic).
    #[test]
    fn test_form2_no_md_extension() {
        let stem = compute_form2_new_text(&"projects/foo/bar".to_string());
        assert_eq!(stem, "bar");
    }
}
