// src/apply/post_apply_scan.rs — post-apply UX-001 partial-path plain-text scanner
//
// After `anchor apply` rewrites all backtick and link refs, bare-prose remainder
// references are invisible to the rewriter. This module scans the apply-touched
// file list for partial-path plain-text occurrences and emits them alongside the
// existing full-path UX-001 warning lines.
//
// Design decision (AENG-008): scan input is limited to `workspace_files` (the list
// already obtained by the apply scan) rather than walking the workspace again.
// This bounds scan cost to the set anchor already knows about.

use crate::refs::partial_path_segments;
use std::path::Path;

/// A single file-level hit: the file path, the matched segment, and occurrence count.
#[derive(Debug, PartialEq, Eq)]
pub struct PlainTextHit {
    pub file: String,
    pub segment: String,
    pub count: usize,
}

/// Scan `workspace_files` for plain-text occurrences of partial-path segments of `src`.
///
/// For each valid partial-path suffix of `src` (from `crate::refs::partial_path_segments`),
/// counts how many times the suffix appears as a plain-text substring in each `.md` file.
/// Deduplicates so each (file, segment) pair is reported at most once (the first/longest
/// matching suffix that has occurrences wins — avoids reporting `os-council` AND `councils/os-council`
/// for the same file when both would match the same occurrences).
///
/// Returns a list of `PlainTextHit` items sorted by file path, then by segment.
/// Returns an empty list when `src` has no partial suffixes (single-component path)
/// or when no occurrences are found.
pub fn scan_partial_plain_text(
    workspace_files: &[String],
    src: &str,
    workspace_root: &Path,
) -> Vec<PlainTextHit> {
    let segments = partial_path_segments(src);
    if segments.is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<PlainTextHit> = Vec::new();

    for file in workspace_files.iter().filter(|f| f.ends_with(".md")) {
        let content = match std::fs::read_to_string(workspace_root.join(file.as_str())) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Track which segments already reported for this file (longest-first wins).
        // When a longer suffix (e.g. "councils/os-council") has N occurrences, we skip
        // the trailing subsegment ("os-council") if its count equals N — same occurrences.
        let mut reported_count: Option<usize> = None;

        for segment in &segments {
            let count = content.matches(segment.as_str()).count();
            if count == 0 {
                continue;
            }
            // Skip if a longer suffix already accounts for the same count (subset match).
            if let Some(prev) = reported_count {
                if count == prev {
                    continue;
                }
            }
            hits.push(PlainTextHit {
                file: file.clone(),
                segment: segment.clone(),
                count,
            });
            reported_count = Some(count);
        }
    }

    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.segment.cmp(&b.segment)));
    hits
}

/// Format a UX-001 warning block combining full-path and partial-path plain-text hits.
///
/// Returns `None` when both lists are empty (suppress the entire block).
///
/// Output format (matches existing UX-001 style):
/// ```text
/// ⚠ Plain-text occurrences not rewritten (may be bare prose refs):
///   <file>: <N> occurrence(s) of '<segment>'
///   ...
/// Run 'anchor refs --plain <segment>' to inspect before closing.
/// ```
pub fn format_plain_text_warning(
    full_path_lines: &[(String, usize)],
    partial_hits: &[PlainTextHit],
    trailing_segment: &str,
) -> Option<String> {
    if full_path_lines.is_empty() && partial_hits.is_empty() {
        return None;
    }

    let mut lines = Vec::new();
    lines.push("⚠ Plain-text occurrences not rewritten (may be bare prose refs):".to_string());

    for (file, count) in full_path_lines {
        lines.push(format!(
            "  {file}: {count} occurrence(s) of '{trailing_segment}'"
        ));
    }

    for hit in partial_hits {
        lines.push(format!(
            "  {}: {} occurrence(s) of '{}'",
            hit.file, hit.count, hit.segment
        ));
    }

    lines.push(format!(
        "Run 'anchor refs --plain {trailing_segment}' to inspect before closing."
    ));

    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        tmp
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    // ── scan_partial_plain_text ───────────────────────────────────────────────

    /// Single-component src has no partial segments — returns empty.
    #[test]
    fn test_single_component_src_returns_empty() {
        let ws = make_workspace();
        write_file(ws.path(), "STATUS.md", "See os-council for details.\n");
        let files = vec!["STATUS.md".to_string()];
        let hits = scan_partial_plain_text(&files, "os-council", ws.path());
        assert!(hits.is_empty(), "single-component src: no partial segments");
    }

    /// Two-component src: trailing segment found in a file.
    #[test]
    fn test_two_component_src_trailing_segment_found() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "STATUS.md",
            "The os-council folder holds decisions.\n",
        );
        let files = vec!["STATUS.md".to_string()];
        let hits = scan_partial_plain_text(&files, "councils/os-council", ws.path());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, "STATUS.md");
        assert_eq!(hits[0].segment, "os-council");
        assert_eq!(hits[0].count, 1);
    }

    /// Multiple files with occurrences — sorted by file path.
    #[test]
    fn test_multiple_files_sorted_by_path() {
        let ws = make_workspace();
        write_file(ws.path(), "z.md", "os-council os-council\n");
        write_file(ws.path(), "a.md", "os-council\n");
        let mut files = vec!["z.md".to_string(), "a.md".to_string()];
        files.sort();
        let hits = scan_partial_plain_text(&files, "councils/os-council", ws.path());
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].file, "a.md");
        assert_eq!(hits[0].count, 1);
        assert_eq!(hits[1].file, "z.md");
        assert_eq!(hits[1].count, 2);
    }

    /// Non-.md files are excluded.
    #[test]
    fn test_non_md_files_excluded() {
        let ws = make_workspace();
        write_file(ws.path(), "config.yaml", "path: os-council\n");
        let files = vec!["config.yaml".to_string()];
        let hits = scan_partial_plain_text(&files, "councils/os-council", ws.path());
        assert!(hits.is_empty(), "yaml files must be excluded");
    }

    /// Zero occurrences — returns empty.
    #[test]
    fn test_no_occurrences_returns_empty() {
        let ws = make_workspace();
        write_file(ws.path(), "README.md", "# Clean document\n");
        let files = vec!["README.md".to_string()];
        let hits = scan_partial_plain_text(&files, "councils/os-council", ws.path());
        assert!(hits.is_empty());
    }

    /// Three-component src: longer suffix wins when counts match.
    #[test]
    fn test_longer_suffix_wins_when_counts_equal() {
        let ws = make_workspace();
        // File mentions "councils/os-council" twice — "os-council" also appears twice (same occurrences).
        write_file(
            ws.path(),
            "STATUS.md",
            "councils/os-council and councils/os-council\n",
        );
        let files = vec!["STATUS.md".to_string()];
        let hits =
            scan_partial_plain_text(&files, "accelmars-guild/councils/os-council", ws.path());
        // "councils/os-council" has 2 hits; "os-council" also has 2 — skip shorter as duplicate.
        assert_eq!(
            hits.len(),
            1,
            "longer suffix must suppress same-count shorter suffix"
        );
        assert_eq!(hits[0].segment, "councils/os-council");
    }

    /// Three-component src: shorter suffix has MORE occurrences — both reported.
    #[test]
    fn test_shorter_suffix_with_more_occurrences_also_reported() {
        let ws = make_workspace();
        // "os-council" appears 3 times; "councils/os-council" only once.
        write_file(
            ws.path(),
            "STATUS.md",
            "councils/os-council and os-council and os-council\n",
        );
        let files = vec!["STATUS.md".to_string()];
        let hits =
            scan_partial_plain_text(&files, "accelmars-guild/councils/os-council", ws.path());
        assert_eq!(hits.len(), 2, "both segments reported when counts differ");
        let segs: Vec<&str> = hits.iter().map(|h| h.segment.as_str()).collect();
        assert!(segs.contains(&"councils/os-council"));
        assert!(segs.contains(&"os-council"));
    }

    // ── format_plain_text_warning ─────────────────────────────────────────────

    /// Both lists empty → None.
    #[test]
    fn test_format_returns_none_when_both_empty() {
        let result = format_plain_text_warning(&[], &[], "os-council");
        assert!(result.is_none());
    }

    /// Full-path lines only → warning block with them only.
    #[test]
    fn test_format_full_path_lines_only() {
        let full = vec![("STATUS.md".to_string(), 3usize)];
        let result = format_plain_text_warning(&full, &[], "os-council").unwrap();
        assert!(result.contains("⚠ Plain-text occurrences"));
        assert!(result.contains("STATUS.md: 3 occurrence(s) of 'os-council'"));
        assert!(result.contains("anchor refs --plain os-council"));
    }

    /// Partial hits only → warning block.
    #[test]
    fn test_format_partial_hits_only() {
        let hits = vec![PlainTextHit {
            file: "docs/guide.md".to_string(),
            segment: "os-council".to_string(),
            count: 2,
        }];
        let result = format_plain_text_warning(&[], &hits, "os-council").unwrap();
        assert!(result.contains("docs/guide.md: 2 occurrence(s) of 'os-council'"));
    }

    /// Both full-path and partial hits → combined block.
    #[test]
    fn test_format_combined() {
        let full = vec![("a.md".to_string(), 1usize)];
        let partial = vec![PlainTextHit {
            file: "b.md".to_string(),
            segment: "os-council".to_string(),
            count: 4,
        }];
        let result = format_plain_text_warning(&full, &partial, "os-council").unwrap();
        assert!(result.contains("a.md: 1 occurrence(s) of 'os-council'"));
        assert!(result.contains("b.md: 4 occurrence(s) of 'os-council'"));
        assert!(result.contains("anchor refs --plain os-council"));
    }
}
