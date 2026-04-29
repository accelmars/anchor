// src/core/diagnostics.rs — shared broken-ref per-entry formatter

use crate::core::suggest;

/// Format diagnostic lines for a single broken reference.
///
/// Output format matches `anchor file validate` human output:
///
///   {file}:{line}
///     → {target}  (target not found)
///     similar: {top}   (omitted when no close match exists)
///
/// A blank line is appended after each entry.
///
/// `candidates` is the workspace file list for similarity lookup. Pass a capped
/// slice — e.g. `&workspace_files[..200.min(workspace_files.len())]` — to bound
/// lookup cost on large workspaces.
pub fn format_broken_ref(file: &str, line: usize, target: &str, candidates: &[String]) -> String {
    let suggestions = suggest::suggest_similar(target, candidates);
    let mut s = format!("  {file}:{line}\n    → {target}  (target not found)\n");
    if let Some(top) = suggestions.first() {
        s.push_str(&format!("    similar: {top}\n"));
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_broken_ref_no_similar() {
        // "xyz-qwerty-9k3j.md" has no close match in the candidate list
        let candidates = vec!["completely-unrelated-zxywv.md".to_string()];
        let result = format_broken_ref("src/file.md", 5, "xyz-qwerty-9k3j.md", &candidates);
        assert!(result.contains("  src/file.md:5\n"));
        assert!(result.contains("    → xyz-qwerty-9k3j.md  (target not found)\n"));
        assert!(
            !result.contains("similar:"),
            "no similar when no close match"
        );
        assert!(result.ends_with('\n'), "trailing blank line appended");
    }

    #[test]
    fn test_format_broken_ref_with_similar() {
        // Target "docs/gudie.md" (typo) — candidate "docs/guide.md" (existing) is close match
        let candidates = vec!["docs/guide.md".to_string()];
        let result = format_broken_ref("src/note.md", 3, "docs/gudie.md", &candidates);
        assert!(
            result.contains("    similar: docs/guide.md"),
            "similar suggestion must appear"
        );
    }

    #[test]
    fn test_format_broken_ref_trailing_blank_line() {
        let result = format_broken_ref("a.md", 1, "b.md", &[]);
        assert!(
            result.ends_with("\n\n"),
            "output must end with blank line (two newlines); got: {result:?}"
        );
    }
}
