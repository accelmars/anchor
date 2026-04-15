// src/cli/file/validate.rs — mind file validate (MF-007)
//
// Scan all .md files in workspace and report broken references.
// Pure read-only — no lock, no temp directory.
//
// Exit codes:
//   0 = all references valid
//   1 = broken references found
//   2 = system error (I/O, workspace not found)

use crate::core::{parser, resolver, scanner};
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

    if broken.is_empty() {
        println!("✓ {file_count} files scanned. No broken references.");
        Ok(true)
    } else {
        let count = broken.len();

        // Print header (03-COMMANDS.md §Output — broken refs found)
        let workspace_str = workspace_root.display().to_string();
        if std::io::stdout().is_terminal() {
            println!("Scanning workspace: \x1b[36m{workspace_str}\x1b[0m  ({file_count} files)");
        } else {
            println!("Scanning workspace: {workspace_str}  ({file_count} files)");
        }

        println!();
        println!("BROKEN REFERENCES ({count}):");
        println!();
        for (file, line, raw) in &broken {
            println!("  {file}:{line}");
            println!("    → {raw}  (not found)");
            println!();
        }
        println!("{count} broken references in {file_count} files.");
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
