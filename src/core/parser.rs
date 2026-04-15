#![allow(dead_code)]

use crate::model::{
    reference::{RefForm, Reference},
    CanonicalPath,
};
use regex::Regex;
use std::sync::OnceLock;

// Static regex compilation — never recompile per call (see 05-PARSER.md §Regexes).
// Patterns are compile-time-known-valid constants; unwrap in initialization is accepted.
static FORM1_RE: OnceLock<Regex> = OnceLock::new();
static FORM2_RE: OnceLock<Regex> = OnceLock::new();

fn form1_re() -> &'static Regex {
    FORM1_RE.get_or_init(|| Regex::new(r"\[([^\]]*)\]\(([^)]+\.md[^)]*)\)").unwrap())
}

fn form2_re() -> &'static Regex {
    FORM2_RE.get_or_init(|| Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]*)?\]\]").unwrap())
}

/// Returns the byte-range spans of backtick-delimited inline code on `line`.
/// Pairs opening and closing single backticks; content within each pair is a span.
fn find_backtick_spans(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                i += 1;
            }
            if i < bytes.len() {
                // Found closing backtick
                spans.push((start, i + 1));
                i += 1;
            }
            // Unmatched opening backtick — no span produced, continue
        } else {
            i += 1;
        }
    }
    spans
}

/// Returns true if the byte range `[start, end)` is fully contained within any backtick span.
fn in_backtick_span(spans: &[(usize, usize)], start: usize, end: usize) -> bool {
    spans.iter().any(|&(s, e)| s <= start && end <= e)
}

/// Returns true if `line` is a fenced code block delimiter (``` or ~~~).
fn is_fence_delimiter(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Split `path#anchor` into `(path, Some(anchor))` or `(path, None)`.
fn strip_anchor(path: &str) -> (String, Option<String>) {
    match path.find('#') {
        Some(idx) => (path[..idx].to_string(), Some(path[idx + 1..].to_string())),
        None => (path.to_string(), None),
    }
}

/// Parse all Markdown references from `content`, returning them with byte-offset spans.
///
/// Rules (from 05-PARSER.md):
/// - Skips all content inside fenced code blocks (` ``` ` or `~~~` boundaries)
/// - Skips references inside backtick-delimited inline code
/// - Skips Form 1 matches where the path is an external URL
/// - Produces byte offsets into `content` (not character offsets)
///
/// Does NOT resolve references — that is MF-004 (resolver).
pub fn parse_references(source_file: &CanonicalPath, content: &str) -> Vec<Reference> {
    let form1 = form1_re();
    let form2 = form2_re();

    let mut refs = Vec::new();
    let mut in_fence = false;
    let mut pos = 0usize;

    while pos < content.len() {
        // Find end of current line
        let newline_pos = content[pos..].find('\n').map(|p| pos + p);
        let line_end = newline_pos.unwrap_or(content.len());

        // Line content: strip trailing \r for \r\n files, but keep positions relative to pos
        let line_raw = &content[pos..line_end];
        let line = line_raw.trim_end_matches('\r');

        if is_fence_delimiter(line) {
            in_fence = !in_fence;
        } else if !in_fence {
            let backtick_spans = find_backtick_spans(line);

            // Form 1: [text](path.md) or [text](path.md#anchor)
            for caps in form1.captures_iter(line) {
                let full_match = caps.get(0).unwrap();
                let path_with_anchor = caps.get(2).unwrap().as_str();

                // Defense-in-depth: skip external URLs (regex requires .md but URL could
                // contain .md, e.g. https://example.com/readme.md)
                if path_with_anchor.starts_with("http://")
                    || path_with_anchor.starts_with("https://")
                    || path_with_anchor.starts_with("mailto:")
                    || path_with_anchor.starts_with("//")
                {
                    continue;
                }

                // Skip if inside inline code
                if in_backtick_span(&backtick_spans, full_match.start(), full_match.end()) {
                    continue;
                }

                let (target_raw, anchor) = strip_anchor(path_with_anchor);
                let span = (pos + full_match.start(), pos + full_match.end());

                refs.push(Reference {
                    source_file: source_file.clone(),
                    target_raw,
                    span,
                    form: RefForm::Standard,
                    anchor,
                });
            }

            // Form 2: [[path]] or [[path|alias]]
            for caps in form2.captures_iter(line) {
                let full_match = caps.get(0).unwrap();
                let path_part = caps.get(1).unwrap().as_str();

                // Skip if inside inline code
                if in_backtick_span(&backtick_spans, full_match.start(), full_match.end()) {
                    continue;
                }

                // Strip .md extension — Form 2 uses stem-only for resolution
                let target_raw = path_part
                    .strip_suffix(".md")
                    .unwrap_or(path_part)
                    .to_string();

                let span = (pos + full_match.start(), pos + full_match.end());

                refs.push(Reference {
                    source_file: source_file.clone(),
                    target_raw,
                    span,
                    form: RefForm::Wiki,
                    anchor: None,
                });
            }
        }

        // Advance past line + newline character
        pos = newline_pos.map(|p| p + 1).unwrap_or(content.len());
    }

    refs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> CanonicalPath {
        "test/source.md".to_string()
    }

    // Test 1: Standard relative link parsed; span matches byte range of full [text](path.md)
    #[test]
    fn test_form1_basic() {
        let content = "[text](path.md)";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.target_raw, "path.md");
        assert_eq!(r.form, RefForm::Standard);
        assert_eq!(r.anchor, None);
        assert_eq!(r.span, (0, content.len()));
        assert_eq!(&content[r.span.0..r.span.1], "[text](path.md)");
    }

    // Test 2: Anchor stored separately; target_raw has no anchor
    #[test]
    fn test_form1_anchor() {
        let content = "[text](path.md#section)";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.target_raw, "path.md");
        assert_eq!(r.anchor, Some("section".to_string()));
        assert_eq!(r.span, (0, content.len()));
    }

    // Test 3: External URL skipped
    #[test]
    fn test_form1_external_url_skipped() {
        let content = "[text](https://example.com)";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 0);
    }

    // Test 4: Reference inside backticks not parsed
    #[test]
    fn test_form1_inside_backticks_skipped() {
        let content = "`[text](path.md)`";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 0);
    }

    // Test 5: Reference inside fenced code block not parsed
    #[test]
    fn test_form1_inside_fence_skipped() {
        let content = "```\n[text](path.md)\n```";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 0);
    }

    // Test 6: Relative path with .. parsed; target_raw preserves ../
    #[test]
    fn test_form1_relative_with_dotdot() {
        let content = "[text](../other/path.md)";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_raw, "../other/path.md");
    }

    // Test 7: Basic wiki link parsed
    #[test]
    fn test_form2_basic() {
        let content = "[[260405-decision]]";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.target_raw, "260405-decision");
        assert_eq!(r.form, RefForm::Wiki);
        assert_eq!(r.anchor, None);
    }

    // Test 8: Wiki link with alias — alias discarded, target_raw is stem
    #[test]
    fn test_form2_with_alias() {
        let content = "[[260405-decision|click here]]";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_raw, "260405-decision");
    }

    // Test 9: Wiki link with .md extension stripped; target_raw is stem
    #[test]
    fn test_form2_md_extension_stripped() {
        let content = "[[path.md]]";
        let refs = parse_references(&src(), content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_raw, "path");
    }
}
