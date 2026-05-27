// src/cli/text/find.rs — anchor text find
//
// Mechanical enumeration of literal-or-regex occurrences across markdown files
// in the workspace, with surrounding context lines per match.
//
// Read-only — no lock, no temp directory, no mutation. The mechanical primitive
// that future anchor-engine `--review` modes call to enumerate candidate sites
// for AI-judged per-site review.
//
// Exit codes:
//   0 = found ≥1 occurrences
//   1 = no occurrences found
//   2 = error (workspace not initialized, invalid regex, I/O failure)

use crate::core::scanner;
use crate::infra::workspace;
use ignore::overrides::OverrideBuilder;
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub enum FindFormat {
    #[default]
    Text,
    Json,
}

/// Args struct mirroring the CLI surface for `anchor text find`.
#[derive(Debug, Clone)]
pub struct FindArgs {
    pub pattern: String,
    pub regex: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub file_types: Vec<String>,
    pub context: usize,
    pub format: FindFormat,
    pub include_code_blocks: bool,
    pub include_frontmatter: bool,
}

impl Default for FindArgs {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            regex: false,
            include: Vec::new(),
            exclude: Vec::new(),
            file_types: vec!["md".to_string()],
            context: 2,
            format: FindFormat::Text,
            include_code_blocks: false,
            include_frontmatter: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub match_text: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
    pub in_code_block: bool,
    pub in_frontmatter: bool,
    pub compound_match: bool,
}

/// Execute `anchor text find <pattern> ...`. Resolves workspace root, delegates to `run_on_root`.
pub fn run(args: FindArgs) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_on_root(&workspace_root, args)
}

/// Core find logic on an explicit workspace root. Public for integration testing.
pub fn run_on_root(workspace_root: &Path, args: FindArgs) -> i32 {
    match do_find(workspace_root, &args) {
        Ok(occurrences) => {
            let result = match args.format {
                FindFormat::Json => format_json(&mut io::stdout(), &args, &occurrences),
                FindFormat::Text => format_human(&mut io::stdout(), &args, &occurrences),
            };
            if let Err(e) = result {
                eprintln!("error: {e}");
                return 2;
            }
            if occurrences.is_empty() {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn do_find(workspace_root: &Path, args: &FindArgs) -> Result<Vec<Occurrence>, String> {
    if args.pattern.is_empty() {
        return Err("pattern must not be empty".to_string());
    }

    let matcher: Box<dyn Matcher> = if args.regex {
        let re =
            regex::Regex::new(&args.pattern).map_err(|e| format!("invalid regex pattern: {e}"))?;
        Box::new(RegexMatcher(re))
    } else {
        Box::new(LiteralMatcher(args.pattern.clone()))
    };

    let include_overrides = build_overrides(workspace_root, &args.include)?;
    let exclude_overrides = build_overrides(workspace_root, &args.exclude)?;
    let extensions: BTreeSet<&str> = args.file_types.iter().map(|s| s.as_str()).collect();

    let files =
        scanner::scan_workspace(workspace_root).map_err(|e| format!("scanner error: {e}"))?;

    let mut occurrences = Vec::new();

    for file_path in &files {
        if !extension_matches(file_path, &extensions) {
            continue;
        }
        if !include_overrides.is_empty() && !path_matches_any(file_path, &include_overrides) {
            continue;
        }
        if !exclude_overrides.is_empty() && path_matches_any(file_path, &exclude_overrides) {
            continue;
        }

        let abs = workspace_root.join(file_path.as_str());
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: skipping {file_path}: {e}");
                continue;
            }
        };

        find_in_file(
            file_path,
            &content,
            args,
            matcher.as_ref(),
            &mut occurrences,
        );
    }

    Ok(occurrences)
}

fn find_in_file(
    file: &str,
    content: &str,
    args: &FindArgs,
    matcher: &dyn Matcher,
    out: &mut Vec<Occurrence>,
) {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let lines_trimmed: Vec<&str> = lines.iter().map(|l| l.trim_end_matches('\n')).collect();
    let frontmatter_end_line = detect_frontmatter_end(&lines_trimmed);
    let code_block_lines = detect_code_block_lines(&lines_trimmed);

    for (i, line) in lines_trimmed.iter().enumerate() {
        let line_no = i + 1;
        let in_frontmatter = frontmatter_end_line.is_some_and(|end| line_no <= end);
        let in_code_block = code_block_lines.contains(&line_no);

        if in_frontmatter && !args.include_frontmatter {
            continue;
        }
        if in_code_block && !args.include_code_blocks {
            continue;
        }

        let spans = matcher.find_all(line);
        if spans.is_empty() {
            continue;
        }

        for span in spans {
            let match_text = &line[span.0..span.1];
            let compound_match = if args.regex {
                false
            } else {
                is_compound(line, span.0, span.1)
            };

            let context_before = collect_context(&lines_trimmed, i, args.context, true);
            let context_after = collect_context(&lines_trimmed, i, args.context, false);

            out.push(Occurrence {
                file: file.to_string(),
                line: line_no,
                column: span.0 + 1,
                match_text: match_text.to_string(),
                context_before,
                context_after,
                in_code_block,
                in_frontmatter,
                compound_match,
            });
        }
    }
}

trait Matcher {
    fn find_all(&self, line: &str) -> Vec<(usize, usize)>;
}

struct LiteralMatcher(String);

impl Matcher for LiteralMatcher {
    fn find_all(&self, line: &str) -> Vec<(usize, usize)> {
        let needle = self.0.as_str();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut spans = Vec::new();
        let mut start = 0;
        while let Some(pos) = line[start..].find(needle) {
            let absolute = start + pos;
            spans.push((absolute, absolute + needle.len()));
            start = absolute + needle.len();
            if start >= line.len() {
                break;
            }
        }
        spans
    }
}

struct RegexMatcher(regex::Regex);

impl Matcher for RegexMatcher {
    fn find_all(&self, line: &str) -> Vec<(usize, usize)> {
        self.0
            .find_iter(line)
            .map(|m| (m.start(), m.end()))
            .collect()
    }
}

fn is_compound(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    matches!(before, Some(c) if is_ident(c)) || matches!(after, Some(c) if is_ident(c))
}

fn detect_frontmatter_end(lines: &[&str]) -> Option<usize> {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return None;
    }
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            return Some(i + 1);
        }
    }
    None
}

fn detect_code_block_lines(lines: &[&str]) -> BTreeSet<usize> {
    let mut blocks = BTreeSet::new();
    let mut in_fence = false;
    let mut marker: Option<char> = None;
    let mut marker_len = 0usize;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let fence_start_chars = trimmed
            .chars()
            .take_while(|c| *c == '`' || *c == '~')
            .collect::<String>();
        let fence_char = fence_start_chars.chars().next();
        let fence_len = fence_start_chars.len();
        let line_no = i + 1;

        if !in_fence {
            if fence_len >= 3 && fence_char.is_some() {
                in_fence = true;
                marker = fence_char;
                marker_len = fence_len;
                blocks.insert(line_no);
            }
        } else {
            blocks.insert(line_no);
            if fence_len >= marker_len && fence_char == marker {
                let after_fence: String = trimmed.chars().skip(fence_len).collect();
                if after_fence.trim().is_empty() {
                    in_fence = false;
                    marker = None;
                    marker_len = 0;
                }
            }
        }
    }

    blocks
}

fn collect_context(lines: &[&str], idx: usize, n: usize, before: bool) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }
    if before {
        let start = idx.saturating_sub(n);
        lines[start..idx].iter().map(|s| s.to_string()).collect()
    } else {
        let end = (idx + 1 + n).min(lines.len());
        lines[idx + 1..end].iter().map(|s| s.to_string()).collect()
    }
}

fn build_overrides(workspace_root: &Path, patterns: &[String]) -> Result<Vec<String>, String> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    // Validate by trying to build an OverrideBuilder; collect into a Vec of normalized
    // patterns. We re-walk per-file with glob matching for simplicity (workspaces are
    // small enough that this is fine; if needed, can be optimized later).
    let mut ob = OverrideBuilder::new(workspace_root);
    for pat in patterns {
        ob.add(pat)
            .map_err(|e| format!("invalid glob pattern '{pat}': {e}"))?;
    }
    ob.build()
        .map_err(|e| format!("failed to build override matcher: {e}"))?;
    Ok(patterns.to_vec())
}

fn path_matches_any(path: &str, patterns: &[String]) -> bool {
    use globset::{Glob, GlobSetBuilder};
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            builder.add(g);
        }
    }
    if let Ok(set) = builder.build() {
        set.is_match(path)
    } else {
        false
    }
}

fn extension_matches(file: &str, allowed: &BTreeSet<&str>) -> bool {
    if allowed.is_empty() {
        return true;
    }
    match Path::new(file).extension().and_then(|e| e.to_str()) {
        Some(ext) => allowed.contains(ext),
        None => false,
    }
}

fn format_human<W: io::Write>(
    w: &mut W,
    args: &FindArgs,
    occurrences: &[Occurrence],
) -> io::Result<()> {
    if occurrences.is_empty() {
        return writeln!(w, "No occurrences of {:?} found.", args.pattern);
    }

    let mut files: BTreeSet<&str> = BTreeSet::new();
    for occ in occurrences {
        files.insert(occ.file.as_str());
    }

    for occ in occurrences {
        writeln!(w, "{}:{}", occ.file, occ.line)?;
        let start_line = occ.line.saturating_sub(occ.context_before.len());
        for (i, ctx) in occ.context_before.iter().enumerate() {
            writeln!(w, "  > {}: {}", start_line + i, ctx)?;
        }
        let marker = if occ.compound_match { "*" } else { ">" };
        writeln!(w, "  {} {}: {}", marker, occ.line, line_for_display(occ))?;
        for (i, ctx) in occ.context_after.iter().enumerate() {
            writeln!(w, "  > {}: {}", occ.line + 1 + i, ctx)?;
        }
        writeln!(w)?;
    }

    writeln!(
        w,
        "Found {} occurrence(s) in {} file(s).",
        occurrences.len(),
        files.len(),
    )
}

fn line_for_display(occ: &Occurrence) -> String {
    // Render the matching line content. Since we don't store the line content directly,
    // we reconstruct an indicator instead. For full content, callers can use --format json.
    let mut s = String::new();
    if occ.in_frontmatter {
        s.push_str("[frontmatter] ");
    }
    if occ.in_code_block {
        s.push_str("[code-block] ");
    }
    s.push_str(&occ.match_text);
    if occ.compound_match {
        s.push_str("  (compound match)");
    }
    s
}

fn format_json<W: io::Write>(
    w: &mut W,
    args: &FindArgs,
    occurrences: &[Occurrence],
) -> io::Result<()> {
    let files: BTreeSet<&str> = occurrences.iter().map(|o| o.file.as_str()).collect();
    let results: Vec<serde_json::Value> = occurrences
        .iter()
        .map(|o| {
            serde_json::json!({
                "file": o.file,
                "line": o.line,
                "column": o.column,
                "match_text": o.match_text,
                "context_before": o.context_before,
                "context_after": o.context_after,
                "in_code_block": o.in_code_block,
                "in_frontmatter": o.in_frontmatter,
                "compound_match": o.compound_match,
            })
        })
        .collect();

    let output = serde_json::json!({
        "pattern": args.pattern,
        "regex": args.regex,
        "results": results,
        "total_occurrences": occurrences.len(),
        "total_files": files.len(),
    });
    writeln!(w, "{output}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".accelmars").join("anchor")).unwrap();
        fs::write(
            dir.path()
                .join(".accelmars")
                .join("anchor")
                .join("config.json"),
            r#"{"schema_version":"1"}"#,
        )
        .unwrap();
        for (path, content) in files {
            let abs = dir.path().join(path);
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&abs, content).unwrap();
        }
        dir
    }

    fn args(pattern: &str) -> FindArgs {
        FindArgs {
            pattern: pattern.to_string(),
            ..FindArgs::default()
        }
    }

    #[test]
    fn finds_literal_matches_across_files() {
        let ws = make_workspace(&[
            ("a.md", "hello apps-monorepo today\n"),
            (
                "b.md",
                "another apps-monorepo line\nand a second apps-monorepo here\n",
            ),
        ]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 3);
        let files: Vec<_> = result.iter().map(|o| o.file.as_str()).collect();
        assert!(files.iter().any(|f| f.contains("a.md")));
        assert!(files.iter().any(|f| f.contains("b.md")));
    }

    #[test]
    fn no_match_returns_empty() {
        let ws = make_workspace(&[("a.md", "hello world\n")]);
        let result = do_find(ws.path(), &args("missing-string")).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn regex_search_finds_pattern() {
        let ws = make_workspace(&[("a.md", "foo-123 and foo-456\n")]);
        let mut a = args(r"foo-\d+");
        a.regex = true;
        let result = do_find(ws.path(), &a).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].match_text, "foo-123");
        assert_eq!(result[1].match_text, "foo-456");
    }

    #[test]
    fn invalid_regex_returns_error() {
        let ws = make_workspace(&[("a.md", "x\n")]);
        let mut a = args("foo(unclosed");
        a.regex = true;
        let result = do_find(ws.path(), &a);
        assert!(result.is_err());
    }

    #[test]
    fn include_paths_filters_files() {
        let ws = make_workspace(&[
            ("docs/a.md", "apps-monorepo\n"),
            ("src/b.md", "apps-monorepo\n"),
        ]);
        let mut a = args("apps-monorepo");
        a.include = vec!["docs/**".to_string()];
        let result = do_find(ws.path(), &a).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].file.starts_with("docs/"));
    }

    #[test]
    fn exclude_paths_filters_files() {
        let ws = make_workspace(&[
            ("docs/a.md", "apps-monorepo\n"),
            ("decisions/b.md", "apps-monorepo\n"),
        ]);
        let mut a = args("apps-monorepo");
        a.exclude = vec!["decisions/**".to_string()];
        let result = do_find(ws.path(), &a).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].file.starts_with("docs/"));
    }

    #[test]
    fn file_types_filter_skips_non_matching_extensions() {
        let ws = make_workspace(&[
            ("a.md", "apps-monorepo\n"),
            ("config.toml", "apps-monorepo\n"),
        ]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        // Default file_types = ["md"], so toml should be skipped
        assert_eq!(result.len(), 1);
        assert!(result[0].file.ends_with(".md"));
    }

    #[test]
    fn skips_fenced_code_blocks_by_default() {
        let ws = make_workspace(&[(
            "a.md",
            "apps-monorepo in prose\n```\napps-monorepo in code\n```\napps-monorepo again\n",
        )]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        // Should find 2 (prose and again), not the one inside code fence
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|o| !o.in_code_block));
    }

    #[test]
    fn include_code_blocks_finds_matches_inside_fences() {
        let ws = make_workspace(&[("a.md", "before\n```\napps-monorepo inside\n```\nafter\n")]);
        let mut a = args("apps-monorepo");
        a.include_code_blocks = true;
        let result = do_find(ws.path(), &a).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].in_code_block);
    }

    #[test]
    fn frontmatter_matches_are_flagged() {
        let ws = make_workspace(&[(
            "a.md",
            "---\ntitle: apps-monorepo\n---\n\n# body apps-monorepo\n",
        )]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 2);
        let fm = result.iter().find(|o| o.in_frontmatter);
        let body = result.iter().find(|o| !o.in_frontmatter);
        assert!(fm.is_some());
        assert!(body.is_some());
    }

    #[test]
    fn exclude_frontmatter_skips_yaml_block() {
        let ws = make_workspace(&[(
            "a.md",
            "---\ntitle: apps-monorepo\n---\n\n# body apps-monorepo\n",
        )]);
        let mut a = args("apps-monorepo");
        a.include_frontmatter = false;
        let result = do_find(ws.path(), &a).unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].in_frontmatter);
    }

    #[test]
    fn context_lines_captured_before_and_after() {
        let ws = make_workspace(&[(
            "a.md",
            "line1\nline2\nline3 apps-monorepo here\nline4\nline5\n",
        )]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 1);
        let occ = &result[0];
        assert_eq!(
            occ.context_before,
            vec!["line1".to_string(), "line2".to_string()]
        );
        assert_eq!(
            occ.context_after,
            vec!["line4".to_string(), "line5".to_string()]
        );
    }

    #[test]
    fn match_at_file_start_truncates_context_before() {
        let ws = make_workspace(&[("a.md", "apps-monorepo first\nline2\nline3\n")]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].context_before.is_empty());
    }

    #[test]
    fn match_at_file_end_truncates_context_after() {
        let ws = make_workspace(&[("a.md", "line1\nline2\nlast apps-monorepo\n")]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].context_after.is_empty());
    }

    #[test]
    fn compound_match_detected_when_followed_by_ident_char() {
        let ws = make_workspace(&[("a.md", "see apps-monorepo-deploy.yml file\n")]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].compound_match);
    }

    #[test]
    fn compound_match_false_when_followed_by_slash() {
        let ws = make_workspace(&[("a.md", "see accelmars/apps-monorepo/foo path\n")]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].compound_match);
    }

    #[test]
    fn regex_matches_skip_compound_detection() {
        let ws = make_workspace(&[("a.md", "see apps-monorepo-deploy.yml\n")]);
        let mut a = args(r"apps-monorepo");
        a.regex = true;
        let result = do_find(ws.path(), &a).unwrap();
        // Regex authors define their own boundaries; compound_match always false
        assert!(!result[0].compound_match);
    }

    #[test]
    fn json_output_roundtrips_through_parser() {
        let ws = make_workspace(&[("a.md", "find apps-monorepo here\n")]);
        let a = args("apps-monorepo");
        let result = do_find(ws.path(), &a).unwrap();
        let mut buf = Vec::new();
        format_json(&mut buf, &a, &result).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed["pattern"], "apps-monorepo");
        assert_eq!(parsed["regex"], false);
        assert_eq!(parsed["total_occurrences"], 1);
        assert_eq!(parsed["total_files"], 1);
        assert_eq!(parsed["results"][0]["match_text"], "apps-monorepo");
    }

    #[test]
    fn json_output_with_zero_results() {
        let ws = make_workspace(&[("a.md", "no matches\n")]);
        let a = args("apps-monorepo");
        let result = do_find(ws.path(), &a).unwrap();
        let mut buf = Vec::new();
        format_json(&mut buf, &a, &result).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed["total_occurrences"], 0);
        assert!(parsed["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn empty_pattern_returns_error() {
        let ws = make_workspace(&[("a.md", "x\n")]);
        let result = do_find(ws.path(), &args(""));
        assert!(result.is_err());
    }

    #[test]
    fn run_on_root_exits_0_when_matches_found() {
        let ws = make_workspace(&[("a.md", "apps-monorepo\n")]);
        let exit = run_on_root(ws.path(), args("apps-monorepo"));
        assert_eq!(exit, 0);
    }

    #[test]
    fn run_on_root_exits_1_when_no_matches() {
        let ws = make_workspace(&[("a.md", "nothing\n")]);
        let exit = run_on_root(ws.path(), args("missing"));
        assert_eq!(exit, 1);
    }

    #[test]
    fn run_on_root_exits_2_on_invalid_regex() {
        let ws = make_workspace(&[("a.md", "x\n")]);
        let mut a = args("foo(unclosed");
        a.regex = true;
        let exit = run_on_root(ws.path(), a);
        assert_eq!(exit, 2);
    }

    #[test]
    fn multiple_matches_on_same_line_all_captured() {
        let ws = make_workspace(&[(
            "a.md",
            "apps-monorepo and apps-monorepo and apps-monorepo\n",
        )]);
        let result = do_find(ws.path(), &args("apps-monorepo")).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|o| o.line == 1));
    }
}
