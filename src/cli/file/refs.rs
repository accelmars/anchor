// src/cli/file/refs.rs — mind file refs (MF-007)
//
// List all files in workspace that reference a given file.
// Pure read-only — no lock, no temp directory.
//
// Exit codes:
//   0 = always (zero refs is not an error — file may be a leaf node)
//   2 = system error (I/O, workspace not found)

use crate::core::{parser, resolver, scanner};
use crate::infra::workspace;
use crate::model::reference::RefForm;
use std::collections::HashSet;
use std::path::Path;

/// Execute `mind file refs <file>`. Returns exit code for process::exit.
pub fn run(target: &str) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_on_root(&workspace_root, target)
}

/// Core refs logic on an explicit workspace root. Public for integration testing.
pub fn run_on_root(workspace_root: &Path, target: &str) -> i32 {
    match do_refs(workspace_root, target) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn do_refs(workspace_root: &Path, target: &str) -> Result<(), String> {
    // Normalize target to workspace-root-relative canonical form
    let target_canonical = normalize_target(workspace_root, target);

    let files =
        scanner::scan_workspace(workspace_root).map_err(|e| format!("scanner error: {e}"))?;

    let mut hits: Vec<(String, usize)> = Vec::new(); // (source_file, line)

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
                    resolver::ResolveResult::BrokenRef | resolver::ResolveResult::Ambiguous(_) => {
                        continue
                    }
                },
            };

            if canonical == target_canonical {
                let line = byte_offset_to_line(&content, reference.span.0);
                hits.push((file_path.clone(), line));
            }
        }
    }

    if hits.is_empty() {
        println!("No files reference {target_canonical}.");
    } else {
        let unique_files = hits
            .iter()
            .map(|(f, _)| f.as_str())
            .collect::<HashSet<_>>()
            .len();

        println!("References to: {target_canonical}");
        println!();
        for (file, line) in &hits {
            println!("  {file}:{line}");
        }
        println!();
        println!("{unique_files} files reference this file.");
    }

    Ok(())
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
