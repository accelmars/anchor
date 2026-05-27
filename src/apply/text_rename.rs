use crate::core::fence_state::FenceState;
use crate::infra::atomic;
use crate::model::plan::TextRenameOp;
use ignore::overrides::{Override, OverrideBuilder};
use ignore::WalkBuilder;
use regex::Regex;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct TextRenameResult {
    pub files_changed: Vec<PathBuf>,
    pub total_substitutions: usize,
    pub warnings: Vec<String>,
}

pub fn execute_text_rename(
    workspace_root: &Path,
    op: &TextRenameOp,
) -> Result<TextRenameResult, String> {
    let include = build_override(workspace_root, &op.include_paths)?;
    let exclude = build_override(workspace_root, &op.exclude_paths)?;
    let regex = if op.literal {
        None
    } else {
        Some(Regex::new(&op.from).map_err(|e| format!("invalid regex: {e}"))?)
    };

    let mut files_changed = Vec::new();
    let mut total_substitutions = 0usize;
    let mut warnings = Vec::new();

    for path in candidate_files(workspace_root, op, &include, &exclude)? {
        let original =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let had_to_before = original.contains(&op.to);
        let had_compound = op.literal && has_compound_match(&original, &op.from);
        let (rewritten, substitutions) = rewrite_content(&original, op, regex.as_ref());

        if substitutions == 0 {
            continue;
        }

        atomic::atomic_write(&path, &rewritten).map_err(|e| format!("{}: {e}", path.display()))?;
        let rel = relative_path(workspace_root, &path);
        if had_to_before {
            warnings.push(format!("to already present in {rel}"));
        }
        if had_compound {
            warnings.push(format!("compound match in {rel}"));
        }
        files_changed.push(PathBuf::from(rel));
        total_substitutions += substitutions;
    }

    files_changed.sort();
    warnings.sort();

    Ok(TextRenameResult {
        files_changed,
        total_substitutions,
        warnings,
    })
}

pub(crate) fn validate_globs(workspace_root: &Path, patterns: &[String]) -> Result<(), String> {
    build_override(workspace_root, patterns).map(|_| ())
}

pub(crate) fn preview_text_rename(
    workspace_root: &Path,
    op: &TextRenameOp,
) -> Result<TextRenameResult, String> {
    let include = build_override(workspace_root, &op.include_paths)?;
    let exclude = build_override(workspace_root, &op.exclude_paths)?;
    let regex = if op.literal {
        None
    } else {
        Some(Regex::new(&op.from).map_err(|e| format!("invalid regex: {e}"))?)
    };

    let mut files_changed = Vec::new();
    let mut total_substitutions = 0usize;
    let mut warnings = Vec::new();

    for path in candidate_files(workspace_root, op, &include, &exclude)? {
        let original =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let had_to_before = original.contains(&op.to);
        let had_compound = op.literal && has_compound_match(&original, &op.from);
        let (_, substitutions) = rewrite_content(&original, op, regex.as_ref());
        if substitutions == 0 {
            continue;
        }
        let rel = relative_path(workspace_root, &path);
        if had_to_before {
            warnings.push(format!("to already present in {rel}"));
        }
        if had_compound {
            warnings.push(format!("compound match in {rel}"));
        }
        files_changed.push(PathBuf::from(rel));
        total_substitutions += substitutions;
    }

    files_changed.sort();
    warnings.sort();

    Ok(TextRenameResult {
        files_changed,
        total_substitutions,
        warnings,
    })
}

fn candidate_files(
    workspace_root: &Path,
    op: &TextRenameOp,
    include: &Override,
    exclude: &Override,
) -> Result<Vec<PathBuf>, String> {
    let ignore_file_path = workspace_root
        .join(".accelmars")
        .join("anchor")
        .join("ignore");
    let gitignore = if ignore_file_path.exists() {
        let mut gib = ignore::gitignore::GitignoreBuilder::new(workspace_root);
        gib.add(&ignore_file_path);
        gib.build()
            .unwrap_or_else(|_| ignore::gitignore::Gitignore::empty())
    } else {
        ignore::gitignore::Gitignore::empty()
    };

    let mut builder = WalkBuilder::new(workspace_root);
    builder
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false);

    let mut files = Vec::new();
    for result in builder.build() {
        let entry = result.map_err(|e| format!("walk error: {e}"))?;
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == ".accelmars") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(workspace_root) else {
            continue;
        };
        let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
        if gitignore
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
        {
            continue;
        }
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        if !has_allowed_extension(path, &op.file_types) {
            continue;
        }
        if !include.is_empty() && !include.matched(rel, false).is_whitelist() {
            continue;
        }
        if !exclude.is_empty() && exclude.matched(rel, false).is_whitelist() {
            continue;
        }
        files.push(path.to_path_buf());
    }
    files.sort();
    Ok(files)
}

fn build_override(workspace_root: &Path, patterns: &[String]) -> Result<Override, String> {
    let mut builder = OverrideBuilder::new(workspace_root);
    for pattern in patterns {
        builder
            .add(pattern)
            .map_err(|e| format!("invalid glob pattern {pattern:?}: {e}"))?;
    }
    builder.build().map_err(|e| format!("invalid glob: {e}"))
}

fn has_allowed_extension(path: &Path, file_types: &[String]) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    file_types.iter().any(|file_type| {
        let file_type = file_type.strip_prefix('.').unwrap_or(file_type);
        ext == file_type
    })
}

fn rewrite_content(content: &str, op: &TextRenameOp, regex: Option<&Regex>) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
    let mut substitutions = 0usize;
    let mut fence = FenceState::default();
    let mut frontmatter = FrontmatterState::Start;

    for line in split_lines_inclusive(content) {
        let in_frontmatter = frontmatter.observe_line(line);
        fence.observe_line(line);
        let in_code_block = fence.in_code_block();
        let skip = (!op.match_in_frontmatter && in_frontmatter)
            || (!op.match_in_code_blocks && in_code_block);
        if skip {
            out.push_str(line);
            continue;
        }
        let (rewritten, count) = rewrite_segment(line, &op.from, &op.to, op.literal, regex);
        out.push_str(&rewritten);
        substitutions += count;
    }

    (out, substitutions)
}

fn rewrite_segment(
    segment: &str,
    from: &str,
    to: &str,
    literal: bool,
    regex: Option<&Regex>,
) -> (String, usize) {
    if literal {
        let count = segment.matches(from).count();
        if count == 0 {
            (segment.to_string(), 0)
        } else {
            (segment.replace(from, to), count)
        }
    } else {
        let Some(regex) = regex else {
            return (segment.to_string(), 0);
        };
        let count = regex.find_iter(segment).count();
        if count == 0 {
            (segment.to_string(), 0)
        } else {
            (regex.replace_all(segment, to).into_owned(), count)
        }
    }
}

fn split_lines_inclusive(content: &str) -> Vec<&str> {
    let mut lines = content.split_inclusive('\n').collect::<Vec<_>>();
    if lines.is_empty() || content.ends_with('\n') {
        return lines;
    }
    if let Some(last) = content.rsplit('\n').next() {
        if !last.is_empty() && lines.last().copied() != Some(last) {
            lines.push(last);
        }
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontmatterState {
    Start,
    In,
    Done,
}

impl FrontmatterState {
    fn observe_line(&mut self, line: &str) -> bool {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        match self {
            FrontmatterState::Start if trimmed == "---" => {
                *self = FrontmatterState::In;
                true
            }
            FrontmatterState::Start => {
                *self = FrontmatterState::Done;
                false
            }
            FrontmatterState::In if trimmed == "---" => {
                *self = FrontmatterState::Done;
                true
            }
            FrontmatterState::In => true,
            FrontmatterState::Done => false,
        }
    }
}

fn has_compound_match(content: &str, from: &str) -> bool {
    if from.is_empty() {
        return false;
    }
    content.match_indices(from).any(|(idx, matched)| {
        let before = content[..idx].chars().next_back();
        let after = content[idx + matched.len()..].chars().next();
        before.is_some_and(is_compound_char) || after.is_some_and(is_compound_char)
    })
}

fn is_compound_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'
}

fn relative_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn op(from: &str, to: &str) -> TextRenameOp {
        TextRenameOp {
            from: from.to_string(),
            to: to.to_string(),
            include_paths: vec![],
            exclude_paths: vec![],
            file_types: vec!["md".to_string()],
            literal: true,
            match_in_code_blocks: false,
            match_in_frontmatter: true,
        }
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    #[test]
    fn test_literal_substitution_in_single_file() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "old old\n");
        let result = execute_text_rename(ws.path(), &op("old", "new")).unwrap();
        assert_eq!(result.total_substitutions, 2);
        assert_eq!(
            fs::read_to_string(ws.path().join("a.md")).unwrap(),
            "new new\n"
        );
    }

    #[test]
    fn test_regex_substitution() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "ticket-12 ticket-34\n");
        let mut rename = op(r"ticket-(\d+)", "issue-$1");
        rename.literal = false;
        let result = execute_text_rename(ws.path(), &rename).unwrap();
        assert_eq!(result.total_substitutions, 2);
        assert_eq!(
            fs::read_to_string(ws.path().join("a.md")).unwrap(),
            "issue-12 issue-34\n"
        );
    }

    #[test]
    fn test_multiple_files_include_paths_filter() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "docs/a.md", "old\n");
        write_file(ws.path(), "notes/b.md", "old\n");
        let mut rename = op("old", "new");
        rename.include_paths = vec!["docs/**".to_string()];
        let result = execute_text_rename(ws.path(), &rename).unwrap();
        assert_eq!(result.files_changed, vec![PathBuf::from("docs/a.md")]);
        assert_eq!(
            fs::read_to_string(ws.path().join("docs/a.md")).unwrap(),
            "new\n"
        );
        assert_eq!(
            fs::read_to_string(ws.path().join("notes/b.md")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn test_exclude_paths_filter() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "docs/a.md", "old\n");
        write_file(ws.path(), "docs/archive/b.md", "old\n");
        let mut rename = op("old", "new");
        rename.exclude_paths = vec!["docs/archive/**".to_string()];
        let result = execute_text_rename(ws.path(), &rename).unwrap();
        assert_eq!(result.files_changed, vec![PathBuf::from("docs/a.md")]);
        assert_eq!(
            fs::read_to_string(ws.path().join("docs/archive/b.md")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn test_file_types_filter() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "old\n");
        write_file(ws.path(), "b.markdown", "old\n");
        let mut rename = op("old", "new");
        rename.file_types = vec!["markdown".to_string()];
        let result = execute_text_rename(ws.path(), &rename).unwrap();
        assert_eq!(result.files_changed, vec![PathBuf::from("b.markdown")]);
        assert_eq!(fs::read_to_string(ws.path().join("a.md")).unwrap(), "old\n");
    }

    #[test]
    fn test_skips_fenced_code_blocks_by_default() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "old\n```\nold\n```\n");
        let result = execute_text_rename(ws.path(), &op("old", "new")).unwrap();
        assert_eq!(result.total_substitutions, 1);
        assert_eq!(
            fs::read_to_string(ws.path().join("a.md")).unwrap(),
            "new\n```\nold\n```\n"
        );
    }

    #[test]
    fn test_match_in_code_blocks_true_includes_code_blocks() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "old\n```\nold\n```\n");
        let mut rename = op("old", "new");
        rename.match_in_code_blocks = true;
        let result = execute_text_rename(ws.path(), &rename).unwrap();
        assert_eq!(result.total_substitutions, 2);
        assert_eq!(
            fs::read_to_string(ws.path().join("a.md")).unwrap(),
            "new\n```\nnew\n```\n"
        );
    }

    #[test]
    fn test_match_in_frontmatter_false_skips_frontmatter() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "---\ntitle: old\n---\nold\n");
        let mut rename = op("old", "new");
        rename.match_in_frontmatter = false;
        let result = execute_text_rename(ws.path(), &rename).unwrap();
        assert_eq!(result.total_substitutions, 1);
        assert_eq!(
            fs::read_to_string(ws.path().join("a.md")).unwrap(),
            "---\ntitle: old\n---\nnew\n"
        );
    }

    #[test]
    fn test_no_op_when_no_matches_leaves_file_untouched() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "clean\n");
        let modified_before = fs::metadata(ws.path().join("a.md"))
            .unwrap()
            .modified()
            .unwrap();
        let result = execute_text_rename(ws.path(), &op("old", "new")).unwrap();
        let modified_after = fs::metadata(ws.path().join("a.md"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(result.total_substitutions, 0);
        assert_eq!(modified_after, modified_before);
    }

    #[test]
    fn test_idempotent_second_run_has_no_changes() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "old\n");
        let first = execute_text_rename(ws.path(), &op("old", "new")).unwrap();
        let second = execute_text_rename(ws.path(), &op("old", "new")).unwrap();
        assert_eq!(first.total_substitutions, 1);
        assert_eq!(second.total_substitutions, 0);
        assert!(second.files_changed.is_empty());
    }

    #[test]
    fn test_compound_match_warning() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "apps-monorepo-deploy\n");
        let result = execute_text_rename(ws.path(), &op("apps-monorepo", "apps")).unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("compound match in a.md")));
    }

    #[test]
    fn test_to_already_in_file_warning() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "new\nold\n");
        let result = execute_text_rename(ws.path(), &op("old", "new")).unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("to already present in a.md")));
    }

    #[test]
    fn test_respects_anchor_ignore() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), ".accelmars/anchor/ignore", "ignored/**\n");
        write_file(ws.path(), "kept/a.md", "old\n");
        write_file(ws.path(), "ignored/b.md", "old\n");
        let result = execute_text_rename(ws.path(), &op("old", "new")).unwrap();
        assert_eq!(result.files_changed, vec![PathBuf::from("kept/a.md")]);
    }

    #[test]
    fn test_preview_does_not_write() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "a.md", "old\n");
        let result = preview_text_rename(ws.path(), &op("old", "new")).unwrap();
        assert_eq!(result.total_substitutions, 1);
        assert_eq!(fs::read_to_string(ws.path().join("a.md")).unwrap(), "old\n");
    }
}
