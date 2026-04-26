// src/cli/file/refs.rs — anchor file refs
//
// List all files in workspace that reference a given file.
// Pure read-only — no lock, no temp directory.
//
// Exit codes:
//   0 = success (file exists in workspace, may have zero refs)
//   2 = system error (I/O, workspace not found)
//   2 = target not found in workspace (shows suggestions via suggest_similar)

use crate::core::{parser, resolver, scanner, suggest};
use crate::infra::workspace;
use crate::model::reference::RefForm;
use std::collections::HashSet;
use std::io;
use std::path::Path;

/// Output format for `anchor file refs`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum OutputFormat {
    Json,
}

/// Execute `anchor file refs <file>`. Returns exit code for process::exit.
pub fn run(target: &str, format: Option<OutputFormat>) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_on_root(&workspace_root, target, format)
}

/// Core refs logic on an explicit workspace root. Public for integration testing.
pub fn run_on_root(workspace_root: &Path, target: &str, format: Option<OutputFormat>) -> i32 {
    match do_refs(workspace_root, target) {
        Ok((target_canonical, hits, absent)) => {
            if absent {
                // Target not present in workspace — show suggestions instead of "No references found."
                let workspace_files: Vec<String> =
                    scanner::scan_workspace(workspace_root).unwrap_or_default();
                let suggestions = suggest::suggest_similar(&target_canonical, &workspace_files);
                eprintln!(
                    "{}",
                    suggest::format_suggestions(&target_canonical, &suggestions, None)
                );
                return 2;
            }
            let result = if format == Some(OutputFormat::Json) {
                format_json(&mut io::stdout(), &target_canonical, &hits)
            } else {
                format_human(&mut io::stdout(), &target_canonical, &hits)
            };
            match result {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// Return type for `do_refs`: (canonical target path, hits (file, line), absent flag).
type RefsResult = (String, Vec<(String, usize)>, bool);

fn do_refs(workspace_root: &Path, target: &str) -> Result<RefsResult, String> {
    // Normalize target to workspace-root-relative canonical form
    let target_canonical = normalize_target(workspace_root, target);

    let files =
        scanner::scan_workspace(workspace_root).map_err(|e| format!("scanner error: {e}"))?;

    // Detect absent target: if the target path is not in the workspace file list,
    // it does not exist — surface suggestions rather than "No references found."
    let absent = !files.contains(&target_canonical);

    let mut hits: Vec<(String, usize)> = Vec::new(); // (source_file, line)

    if !absent {
        for file_path in &files {
            let abs_path = workspace_root.join(file_path.as_str());
            let content = std::fs::read_to_string(&abs_path)
                .map_err(|e| format!("I/O error reading {file_path}: {e}"))?;

            let refs = parser::parse_references(file_path, &content);

            for reference in &refs {
                let canonical = match reference.form {
                    RefForm::Standard => resolver::resolve_form1(file_path, &reference.target_raw),
                    RefForm::Wiki => match resolver::resolve_form2(&reference.target_raw, &files) {
                        resolver::ResolveResult::Resolved(path) => path,
                        resolver::ResolveResult::BrokenRef
                        | resolver::ResolveResult::Ambiguous(_) => continue,
                    },
                    RefForm::Yaml | RefForm::Toml => reference
                        .target_raw
                        .strip_prefix("$(anchor root)/")
                        .unwrap_or(&reference.target_raw)
                        .to_string(),
                };

                if canonical == target_canonical {
                    let line = byte_offset_to_line(&content, reference.span.0);
                    hits.push((file_path.clone(), line));
                }
            }
        }
    }

    Ok((target_canonical, hits, absent))
}

/// Render hits in human-readable format.
fn format_human<W: io::Write>(
    w: &mut W,
    target_canonical: &str,
    hits: &[(String, usize)],
) -> io::Result<()> {
    if hits.is_empty() {
        writeln!(w, "No references found.")
    } else {
        let unique_files = hits
            .iter()
            .map(|(f, _)| f.as_str())
            .collect::<HashSet<_>>()
            .len();

        writeln!(w, "References to: {target_canonical}")?;
        writeln!(w)?;
        for (file, line) in hits {
            writeln!(w, "  {file}:{line}")?;
        }
        writeln!(w)?;
        writeln!(w, "{unique_files} files reference this file.")
    }
}

/// Render hits as JSON.
///
/// PHASE 2 STABLE CONTRACT: This JSON schema is a stable interface for AI agents and
/// machine consumers. Do not change field names ("refs", "query_path", "count") without
/// a design session. Schema: {"refs":[{"file":"relative/path.md","line":12}],"query_path":"...","count":N}
fn format_json<W: io::Write>(
    w: &mut W,
    target_canonical: &str,
    hits: &[(String, usize)],
) -> io::Result<()> {
    let refs: Vec<serde_json::Value> = hits
        .iter()
        .map(|(file, line)| serde_json::json!({"file": file, "line": line}))
        .collect();
    let output = serde_json::json!({
        "refs": refs,
        "query_path": target_canonical,
        "count": hits.len(),
    });
    writeln!(w, "{output}")
}

/// Normalize a user-provided target path to workspace-root-relative canonical form.
fn normalize_target(workspace_root: &Path, target: &str) -> String {
    let p = std::path::Path::new(target);
    if p.is_absolute() {
        p.strip_prefix(workspace_root)
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| target.to_string())
    } else {
        let normalized = target.replace('\\', "/");
        normalized
            .strip_prefix("./")
            .unwrap_or(&normalized)
            .to_string()
    }
}

/// Convert a byte offset in `content` to a 1-based line number.
fn byte_offset_to_line(content: &str, byte_offset: usize) -> usize {
    content[..byte_offset.min(content.len())]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_refs_emits_confirmation() {
        let mut out = Vec::new();
        format_human(&mut out, "docs/guide.md", &[]).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap().trim(),
            "No references found."
        );
    }

    #[test]
    fn test_format_json_with_hits() {
        let hits = vec![
            ("projects/README.md".to_string(), 3usize),
            ("docs/index.md".to_string(), 45usize),
        ];
        let mut out = Vec::new();
        format_json(&mut out, "docs/guide.md", &hits).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(String::from_utf8(out).unwrap().trim()).unwrap();
        assert_eq!(parsed["query_path"], "docs/guide.md");
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["refs"][0]["file"], "projects/README.md");
        assert_eq!(parsed["refs"][0]["line"], 3);
        assert_eq!(parsed["refs"][1]["file"], "docs/index.md");
        assert_eq!(parsed["refs"][1]["line"], 45);
    }

    #[test]
    fn test_format_json_zero_hits() {
        let mut out = Vec::new();
        format_json(&mut out, "docs/guide.md", &[]).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(String::from_utf8(out).unwrap().trim()).unwrap();
        assert_eq!(parsed["query_path"], "docs/guide.md");
        assert_eq!(parsed["count"], 0);
        assert!(parsed["refs"].as_array().unwrap().is_empty());
    }

    /// Absent target (not in workspace) → exit code 2.
    /// The "Did you mean?" output goes to stderr; we verify only the exit code here.
    #[test]
    fn test_absent_target_exits_2() {
        use std::fs;
        use tempfile::tempdir;

        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.path()
                .join(".accelmars")
                .join("anchor")
                .join("config.json"),
            r#"{"schema_version":"1"}"#,
        )
        .unwrap();
        // Create a real .md file that is a close match for the typo query.
        fs::write(root.path().join("design.md"), "# Design\n").unwrap();

        let exit_code = run_on_root(root.path(), "desig.md", None);
        assert_eq!(exit_code, 2, "absent target must exit 2, got: {exit_code}");
    }

    /// Existing file with zero inbound refs → exit 0, stdout "No references found."
    ///
    /// The absent-detection must NOT trigger when the file exists in the workspace.
    #[test]
    fn test_existing_file_zero_refs_exits_0() {
        use std::fs;
        use tempfile::tempdir;

        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".accelmars").join("anchor")).unwrap();
        fs::write(
            root.path()
                .join(".accelmars")
                .join("anchor")
                .join("config.json"),
            r#"{"schema_version":"1"}"#,
        )
        .unwrap();
        // File exists, nothing references it.
        fs::write(root.path().join("leaf.md"), "# Leaf\n").unwrap();

        let exit_code = run_on_root(root.path(), "leaf.md", None);
        assert_eq!(
            exit_code, 0,
            "existing file with zero refs must exit 0, got: {exit_code}"
        );
    }
}
