// src/cli/file/validate.rs — mind file validate (MF-007)
//
// Scan all .md files in workspace and report broken references.
// Pure read-only — no lock, no temp directory.
//
// Exit codes:
//   0 = all references valid
//   1 = broken references found
//   2 = system error (I/O, workspace not found)

use crate::core::{acked::AckedPatterns, parser, resolver, scanner};
use crate::infra::workspace;
use crate::model::reference::RefForm;
use std::io::IsTerminal;
use std::path::Path;

/// Execute `mind file validate`. Returns exit code for process::exit.
pub fn run() -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_on_root(&workspace_root)
}

/// Core validate logic on an explicit workspace root. Public for integration testing.
pub fn run_on_root(workspace_root: &Path) -> i32 {
    match do_validate(workspace_root) {
        Ok(clean) => {
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

/// Returns Ok(true) = clean, Ok(false) = broken refs found, Err = system error.
fn do_validate(workspace_root: &Path) -> Result<bool, String> {
    let files =
        scanner::scan_workspace(workspace_root).map_err(|e| format!("scanner error: {e}"))?;
    let file_count = files.len();

    // Load acknowledged patterns once before the main loop.
    let acked = AckedPatterns::load(workspace_root);

    let mut broken: Vec<(String, usize, String)> = Vec::new(); // (file, line, raw_display)

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
                        // Ambiguous wiki link: not a broken ref — skip
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

    // Partition broken refs into acknowledged and unresolved.
    let (acked_refs, unresolved): (Vec<_>, Vec<_>) = broken
        .into_iter()
        .partition(|(file, _, _)| acked.is_acked(file));
    let acked_count = acked_refs.len();
    let unresolved_count = unresolved.len();

    if unresolved_count == 0 {
        if acked_count == 0 {
            println!("✓ {file_count} files scanned. No broken references.");
        } else {
            println!(
                "✓ {file_count} files scanned. No broken references.  \
                 ({acked_count} acknowledged — see .mindacked)"
            );
        }
        Ok(true)
    } else {
        // Print header (03-COMMANDS.md §Output — broken refs found)
        let workspace_str = workspace_root.display().to_string();
        if std::io::stdout().is_terminal() {
            println!("Scanning workspace: \x1b[36m{workspace_str}\x1b[0m  ({file_count} files)");
        } else {
            println!("Scanning workspace: {workspace_str}  ({file_count} files)");
        }

        println!();
        println!("BROKEN REFERENCES ({unresolved_count}):");
        println!();
        for (file, line, raw) in &unresolved {
            println!("  {file}:{line}");
            println!("    → {raw}  (not found)");
            println!();
        }
        if acked_count > 0 {
            println!(
                "{unresolved_count} broken references in {file_count} files.  \
                 ({acked_count} acknowledged — see .mindacked)"
            );
        } else {
            println!("{unresolved_count} broken references in {file_count} files.");
        }
        Ok(false)
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
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &Path, path: &str, content: &str) {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    /// .mindacked absent → broken refs still reported (Ok(false)).
    /// Output is unchanged from pre-MF-010 behavior — no acked count line.
    #[test]
    fn test_no_mindacked_broken_refs_reported() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "source.md", "[broken](missing.md)\n");
        let result = do_validate(tmp.path());
        assert!(
            matches!(result, Ok(false)),
            "broken refs must be reported when no .mindacked exists"
        );
    }

    /// .mindacked pattern matches source file → broken refs suppressed (Ok(true)).
    #[test]
    fn test_mindacked_matches_source_suppresses_broken_refs() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "archive/old.md", "[broken](missing.md)\n");
        fs::write(tmp.path().join(".mindacked"), "archive/\n").unwrap();
        let result = do_validate(tmp.path());
        assert!(
            matches!(result, Ok(true)),
            "broken refs from acked source must be suppressed from output"
        );
    }

    /// .mindacked pattern does NOT match source → refs still reported (Ok(false)).
    #[test]
    fn test_mindacked_non_matching_pattern_refs_still_reported() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "active/current.md", "[broken](missing.md)\n");
        fs::write(tmp.path().join(".mindacked"), "archive/\n").unwrap();
        let result = do_validate(tmp.path());
        assert!(
            matches!(result, Ok(false)),
            "broken refs from non-acked source must still be reported"
        );
    }

    /// All broken refs acknowledged → exit code 0 (Ok(true)).
    #[test]
    fn test_all_broken_refs_acknowledged_exit_zero() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "archive/foo.md", "[broken](missing-a.md)\n");
        write_file(
            tmp.path(),
            "archive/bar.md",
            "[also broken](missing-b.md)\n",
        );
        fs::write(tmp.path().join(".mindacked"), "archive/\n").unwrap();
        let result = do_validate(tmp.path());
        assert!(
            matches!(result, Ok(true)),
            "all acked → must return Ok(true) (exit code 0)"
        );
    }

    /// Mixed: some acked, some unresolved → Ok(false) (exit code 1).
    #[test]
    fn test_mixed_acked_and_unresolved_exit_one() {
        let tmp = TempDir::new().unwrap();
        // archive/ is acked
        write_file(tmp.path(), "archive/old.md", "[broken](missing.md)\n");
        // active/ is NOT acked → unresolved
        write_file(
            tmp.path(),
            "active/current.md",
            "[also broken](also-missing.md)\n",
        );
        fs::write(tmp.path().join(".mindacked"), "archive/\n").unwrap();
        let result = do_validate(tmp.path());
        assert!(
            matches!(result, Ok(false)),
            "unresolved refs still present → must return Ok(false) (exit code 1)"
        );
    }
}
