// src/cli/file/validate.rs — anchor file validate
//
// Scan all .md files in workspace and report broken references.
// Pure read-only — no lock, no temp directory.
//
// Exit codes:
//   0 = all references valid
//   1 = broken references found
//   2 = system error (I/O, workspace not found)

use crate::cli::file::refs::OutputFormat;
use crate::core::{acked::AckedPatterns, parser, resolver, scanner};
use crate::infra::workspace;
use crate::model::reference::RefForm;
use std::io::{self, IsTerminal, Write};
use std::path::Path;

/// Result of scanning the workspace for broken references.
struct ValidateResult {
    files_scanned: usize,
    /// Unresolved (non-acknowledged) broken refs: (file, line, raw_target)
    broken: Vec<(String, usize, String)>,
    acknowledged: usize,
}

/// Execute `anchor file validate`. Returns exit code for process::exit.
pub fn run(format: Option<OutputFormat>) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_on_root(&workspace_root, format)
}

/// Core validate logic on an explicit workspace root. Public for integration testing.
pub fn run_on_root(workspace_root: &Path, format: Option<OutputFormat>) -> i32 {
    match do_validate(workspace_root) {
        Ok(result) => {
            let clean = result.broken.is_empty();
            if format == Some(OutputFormat::Json) {
                // PHASE 2 STABLE CONTRACT: This JSON schema is a stable interface for AI agents
                // and machine consumers. Do not change field names without a design session.
                // Schema: {"clean":bool,"files_scanned":N,"broken":[{"file":"...","line":N,"ref":"..."}],"acknowledged":N}
                match write_json_output(&mut io::stdout(), &result) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("error writing output: {e}");
                        return 2;
                    }
                }
            } else if let Err(e) = write_human_output(&mut io::stdout(), workspace_root, &result) {
                eprintln!("error writing output: {e}");
                return 2;
            }
            if clean {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// Scan workspace and return broken reference data. Returns Err on system errors.
fn do_validate(workspace_root: &Path) -> Result<ValidateResult, String> {
    let files =
        scanner::scan_workspace(workspace_root).map_err(|e| format!("scanner error: {e}"))?;
    let file_count = files.len();

    let acked = AckedPatterns::load(workspace_root);
    let mut broken: Vec<(String, usize, String)> = Vec::new();

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
                    resolver::ResolveResult::BrokenRef => {
                        let line = byte_offset_to_line(&content, reference.span.0);
                        broken.push((
                            file_path.clone(),
                            line,
                            format!("[[{}]]", reference.target_raw),
                        ));
                        continue;
                    }
                    resolver::ResolveResult::Ambiguous(_) => {
                        continue;
                    }
                },
            };

            let target_abs = workspace_root.join(&canonical);
            match target_abs.try_exists() {
                Ok(true) => {}
                Ok(false) => {
                    let line = byte_offset_to_line(&content, reference.span.0);
                    broken.push((file_path.clone(), line, reference.target_raw.clone()));
                }
                Err(e) => {
                    return Err(format!("I/O error checking {canonical}: {e}"));
                }
            }
        }
    }

    let (acked_refs, unresolved): (Vec<_>, Vec<_>) = broken
        .into_iter()
        .partition(|(file, _, _)| acked.is_acked(file));

    Ok(ValidateResult {
        files_scanned: file_count,
        broken: unresolved,
        acknowledged: acked_refs.len(),
    })
}

/// Write human-readable output to `w`.
fn write_human_output<W: Write>(
    w: &mut W,
    workspace_root: &Path,
    result: &ValidateResult,
) -> io::Result<()> {
    let unresolved_count = result.broken.len();
    let acked_count = result.acknowledged;
    let file_count = result.files_scanned;

    if unresolved_count == 0 {
        if acked_count == 0 {
            writeln!(w, "✓ {file_count} files scanned. No broken references.")
        } else {
            writeln!(
                w,
                "✓ {file_count} files scanned. No broken references.  \
                 ({acked_count} acknowledged — see .accelmars/anchor/acked)"
            )
        }
    } else {
        let workspace_str = workspace_root.display().to_string();
        if io::stdout().is_terminal() {
            writeln!(
                w,
                "Scanning workspace: \x1b[36m{workspace_str}\x1b[0m  ({file_count} files)"
            )?;
        } else {
            writeln!(
                w,
                "Scanning workspace: {workspace_str}  ({file_count} files)"
            )?;
        }

        writeln!(w)?;
        writeln!(w, "BROKEN REFERENCES ({unresolved_count}):")?;
        writeln!(w)?;
        for (file, line, raw) in &result.broken {
            writeln!(w, "  {file}:{line}")?;
            writeln!(w, "    → {raw}  (not found)")?;
            writeln!(w)?;
        }
        if acked_count > 0 {
            writeln!(
                w,
                "{unresolved_count} broken references in {file_count} files.  \
                 ({acked_count} acknowledged — see .accelmars/anchor/acked)"
            )
        } else {
            writeln!(
                w,
                "{unresolved_count} broken references in {file_count} files."
            )
        }
    }
}

/// Write JSON output to `w`.
fn write_json_output<W: Write>(w: &mut W, result: &ValidateResult) -> io::Result<()> {
    let broken: Vec<serde_json::Value> = result
        .broken
        .iter()
        .map(|(file, line, raw)| serde_json::json!({"file": file, "line": line, "ref": raw}))
        .collect();
    let output = serde_json::json!({
        "clean": result.broken.is_empty(),
        "files_scanned": result.files_scanned,
        "broken": broken,
        "acknowledged": result.acknowledged,
    });
    writeln!(w, "{output}")
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
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &Path, path: &str, content: &str) {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    /// .accelmars/anchor/acked absent → broken refs still reported (unresolved non-empty).
    #[test]
    fn test_no_anchor_acked_broken_refs_reported() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "source.md", "[broken](missing.md)\n");
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            !result.broken.is_empty(),
            "broken refs must be reported when no .accelmars/anchor/acked exists"
        );
    }

    /// .accelmars/anchor/acked pattern matches source file → broken refs suppressed (broken empty).
    #[test]
    fn test_anchor_acked_matches_source_suppresses_broken_refs() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "archive/old.md", "[broken](missing.md)\n");
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        fs::write(
            tmp.path().join(".accelmars").join("anchor").join("acked"),
            "archive/\n",
        )
        .unwrap();
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            result.broken.is_empty(),
            "broken refs from acked source must be suppressed"
        );
        assert_eq!(result.acknowledged, 1);
    }

    /// .accelmars/anchor/acked pattern does NOT match source → refs still reported.
    #[test]
    fn test_anchor_acked_non_matching_pattern_refs_still_reported() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "active/current.md", "[broken](missing.md)\n");
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        fs::write(
            tmp.path().join(".accelmars").join("anchor").join("acked"),
            "archive/\n",
        )
        .unwrap();
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            !result.broken.is_empty(),
            "broken refs from non-acked source must still be reported"
        );
    }

    /// All broken refs acknowledged → broken empty, acknowledged count correct.
    #[test]
    fn test_all_broken_refs_acknowledged_exit_zero() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "archive/foo.md", "[broken](missing-a.md)\n");
        write_file(
            tmp.path(),
            "archive/bar.md",
            "[also broken](missing-b.md)\n",
        );
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        fs::write(
            tmp.path().join(".accelmars").join("anchor").join("acked"),
            "archive/\n",
        )
        .unwrap();
        let result = do_validate(tmp.path()).unwrap();
        assert!(result.broken.is_empty(), "all acked → broken must be empty");
        assert_eq!(result.acknowledged, 2);
    }

    /// Mixed: some acked, some unresolved → broken non-empty.
    #[test]
    fn test_mixed_acked_and_unresolved_exit_one() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "archive/old.md", "[broken](missing.md)\n");
        write_file(
            tmp.path(),
            "active/current.md",
            "[also broken](also-missing.md)\n",
        );
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        fs::write(
            tmp.path().join(".accelmars").join("anchor").join("acked"),
            "archive/\n",
        )
        .unwrap();
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            !result.broken.is_empty(),
            "unresolved refs still present → broken must be non-empty"
        );
    }

    /// `--format json` on a clean workspace outputs {"clean":true,...,"broken":[]}.
    #[test]
    fn test_validate_format_json_clean() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "a.md", "[link](b.md)\n");
        write_file(tmp.path(), "b.md", "# B\n");
        let result = do_validate(tmp.path()).unwrap();
        let mut out = Vec::new();
        write_json_output(&mut out, &result).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(String::from_utf8(out).unwrap().trim()).unwrap();
        assert_eq!(
            parsed["clean"], true,
            "clean workspace must have clean:true"
        );
        assert!(
            parsed["broken"].as_array().unwrap().is_empty(),
            "broken array must be empty for clean workspace"
        );
        assert_eq!(parsed["files_scanned"], 2);
        assert_eq!(parsed["acknowledged"], 0);
    }

    /// `--format json` with broken refs outputs populated `broken` array.
    #[test]
    fn test_validate_format_json_broken() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "docs/index.md", "[missing](../missing.md)\n");
        let result = do_validate(tmp.path()).unwrap();
        let mut out = Vec::new();
        write_json_output(&mut out, &result).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(String::from_utf8(out).unwrap().trim()).unwrap();
        assert_eq!(
            parsed["clean"], false,
            "broken workspace must have clean:false"
        );
        let broken_arr = parsed["broken"].as_array().unwrap();
        assert!(!broken_arr.is_empty(), "broken array must be non-empty");
        let entry = &broken_arr[0];
        assert_eq!(entry["file"], "docs/index.md");
        assert!(entry["line"].as_u64().unwrap() >= 1);
        assert!(entry["ref"].as_str().is_some(), "ref field must be present");
    }
}
