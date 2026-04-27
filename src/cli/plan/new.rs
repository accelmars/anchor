// src/cli/plan/new.rs — anchor plan new wizard (AN-018)
//
// run_wizard<R,W> is parameterized on I/O so every template handler is unit-testable
// with mock stdin. pub fn run() wraps it with real stdin/stdout for the CLI entry point.

use crate::cli::plan::templates::TEMPLATES;
use crate::infra::workspace;
use crate::model::plan::{write_plan, Op, Plan};
use std::io::{BufRead, Write};
use std::path::Path;

/// CLI entry point — wraps run_wizard with real stdin/stdout.
pub fn run(output: Option<&str>) -> i32 {
    let stdin = std::io::stdin();
    let mut stdin_lock = stdin.lock();
    let mut stdout = std::io::stdout();
    let ws_root = workspace::find_workspace_root().ok();
    run_wizard(&mut stdin_lock, &mut stdout, output, ws_root.as_deref())
}

/// Wizard logic — parameterized on I/O for testability.
///
/// Displays templates, reads selection, dispatches to template handler,
/// collects description, writes plan file, prints follow-up hints.
pub fn run_wizard<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    out_path: Option<&str>,
    workspace_root: Option<&Path>,
) -> i32 {
    let _ = writeln!(
        output,
        "The wizard generates a starting plan file. For complex plans,"
    );
    let _ = writeln!(
        output,
        "edit the file directly — then run 'anchor plan validate' to check your edits."
    );
    let _ = writeln!(output);
    let _ = writeln!(output, "Available templates:");
    for (i, t) in TEMPLATES.iter().enumerate() {
        let _ = writeln!(output, "  {}. {} — {}", i + 1, t.name, t.description);
    }
    let _ = write!(output, "Select a template (1-{}): ", TEMPLATES.len());
    let _ = output.flush();

    let selection = match read_line(input) {
        Some(s) => s,
        None => {
            let _ = writeln!(output, "error: no input");
            return 1;
        }
    };

    let idx: usize = match selection.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= TEMPLATES.len() => n - 1,
        _ => {
            let _ = writeln!(output, "error: invalid selection '{}'", selection.trim());
            return 1;
        }
    };

    let template = &TEMPLATES[idx];
    let ops = match template.id {
        "batch-move" => wizard_batch_move(input, output, workspace_root),
        "categorize" => wizard_categorize(input, output),
        "archive" => wizard_archive(input, output),
        "rename" => wizard_rename(input, output),
        "scaffold" => wizard_scaffold(input, output),
        _ => {
            let _ = writeln!(output, "error: unknown template id '{}'", template.id);
            return 1;
        }
    };

    let _ = write!(output, "Plan description (optional, Enter to skip): ");
    let _ = output.flush();
    let description = read_line(input).and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    let plan = Plan {
        version: "1".to_string(),
        description,
        ops,
    };

    let path_str = out_path.unwrap_or("anchor-plan.toml");
    let path = Path::new(path_str);

    if let Err(e) = write_plan(path, &plan) {
        let _ = writeln!(output, "error: could not write plan: {}", e);
        return 1;
    }

    let _ = writeln!(output, "Written:  {}", path_str);
    let _ = writeln!(
        output,
        "Tip:      edit {} directly for complex plans",
        path_str
    );
    let _ = writeln!(output, "Validate: anchor plan validate {}", path_str);
    let _ = writeln!(output, "Preview:  anchor diff {}", path_str);
    let _ = writeln!(output, "Execute:  anchor apply {}", path_str);

    0
}

fn wizard_batch_move<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    workspace_root: Option<&Path>,
) -> Vec<Op> {
    let _ = write!(output, "How many moves? ");
    let _ = output.flush();
    let n: usize = read_line(input)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let mut ops = Vec::new();
    for i in 1..=n {
        let _ = write!(output, "Move {}/{} source: ", i, n);
        let _ = output.flush();
        let src = read_line(input).unwrap_or_default();

        let _ = write!(output, "Move {}/{} destination: ", i, n);
        let _ = output.flush();
        let dst = read_line(input).unwrap_or_default();

        let src = src.trim().to_string();
        let dst = dst.trim().to_string();
        if !src.is_empty() && !dst.is_empty() {
            ops.push(Op::Move { src, dst });
        }
    }

    // Collect unique parent dirs that need creation. Sort for deterministic prompt order.
    let parents_to_create: Vec<String> = {
        let mut seen = std::collections::BTreeSet::new();
        for op in &ops {
            if let Op::Move { dst, .. } = op {
                if let Some(parent) = Path::new(dst.as_str()).parent() {
                    if !parent.as_os_str().is_empty() {
                        let p = parent.to_str().unwrap_or("").to_string();
                        let needs_create = workspace_root
                            .map(|root| !root.join(&p).exists())
                            .unwrap_or(true);
                        if needs_create {
                            seen.insert(p);
                        }
                    }
                }
            }
        }
        seen.into_iter().collect()
    };

    let mut create_ops: Vec<Op> = Vec::new();
    for parent in &parents_to_create {
        let _ = write!(
            output,
            "Destination '{parent}/' does not exist. Add a create_dir op? [Y/n] "
        );
        let _ = output.flush();
        let answer = read_line(input).unwrap_or_default();
        let answer = answer.trim().to_lowercase();
        if answer.is_empty() || answer == "y" || answer == "yes" {
            create_ops.push(Op::CreateDir {
                path: parent.clone(),
            });
        }
    }

    let mut result = create_ops;
    result.extend(ops);
    result
}

fn wizard_categorize<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Vec<Op> {
    let _ = write!(output, "Parent folder name: ");
    let _ = output.flush();
    let parent = read_line(input).unwrap_or_default().trim().to_string();

    let _ = writeln!(
        output,
        "Items to categorize (one per line, blank to finish):"
    );
    let _ = output.flush();
    let items = collect_lines(input);

    let mut ops = vec![Op::CreateDir {
        path: parent.clone(),
    }];

    for item in &items {
        let basename = Path::new(item.as_str())
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(item.as_str())
            .to_string();

        let _ = write!(output, "New name for {} [Enter to keep]: ", basename);
        let _ = output.flush();
        let new_name_input = read_line(input).unwrap_or_default();
        let new_name = new_name_input.trim().to_string();
        let dst_name = if new_name.is_empty() {
            basename.clone()
        } else {
            new_name
        };
        let dst = format!("{}/{}", parent, dst_name);
        ops.push(Op::Move {
            src: item.clone(),
            dst,
        });
    }

    ops
}

fn wizard_archive<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Vec<Op> {
    let _ = write!(output, "Archive folder: ");
    let _ = output.flush();
    let archive = read_line(input).unwrap_or_default().trim().to_string();

    let _ = writeln!(output, "Items to archive (one per line, blank to finish):");
    let _ = output.flush();
    let items = collect_lines(input);

    let mut ops = vec![Op::CreateDir {
        path: archive.clone(),
    }];

    for item in &items {
        let basename = Path::new(item.as_str())
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(item.as_str())
            .to_string();
        let dst = format!("{}/{}", archive, basename);
        ops.push(Op::Move {
            src: item.clone(),
            dst,
        });
    }

    ops
}

fn wizard_rename<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Vec<Op> {
    let _ = writeln!(output, "Items to rename (one per line, blank to finish):");
    let _ = output.flush();
    let items = collect_lines(input);

    let mut ops = Vec::new();

    for item in &items {
        let _ = write!(output, "New name for {} [Enter to skip]: ", item);
        let _ = output.flush();
        let new_name_input = read_line(input).unwrap_or_default();
        let new_name = new_name_input.trim().to_string();
        if new_name.is_empty() {
            continue;
        }
        let dst = match Path::new(item.as_str()).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                format!("{}/{}", parent.display(), new_name)
            }
            _ => new_name,
        };
        ops.push(Op::Move {
            src: item.clone(),
            dst,
        });
    }

    ops
}

fn wizard_scaffold<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Vec<Op> {
    let _ = writeln!(
        output,
        "Directories to create (one per line, blank to finish):"
    );
    let _ = output.flush();
    collect_lines(input)
        .into_iter()
        .map(|d| Op::CreateDir { path: d })
        .collect()
}

/// Read lines until a blank line or EOF. Returns trimmed, non-empty entries.
fn collect_lines<R: BufRead>(input: &mut R) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        match read_line(input) {
            Some(line) if !line.trim().is_empty() => lines.push(line.trim().to_string()),
            _ => break,
        }
    }
    lines
}

/// Read a single line, stripping the trailing newline. Returns None on EOF or error.
fn read_line<R: BufRead>(input: &mut R) -> Option<String> {
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(
            line.trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string(),
        ),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::plan::load_plan;
    use std::io::Cursor;
    use tempfile::TempDir;

    fn wizard(input: &str, out_path: Option<&str>) -> (i32, String) {
        let mut reader = Cursor::new(input.as_bytes().to_vec());
        let mut writer = Vec::<u8>::new();
        let code = run_wizard(&mut reader, &mut writer, out_path, None);
        (code, String::from_utf8_lossy(&writer).into_owned())
    }

    // ── scaffold ──────────────────────────────────────────────────────────────

    /// scaffold template: 2 dir inputs → plan with 2 create_dir ops
    #[test]
    fn test_scaffold_two_dirs() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 5, two dirs, blank to finish, blank description
        let (code, _) = wizard("5\nfoundations\narchive\n\n\n", Some(&out_str));
        assert_eq!(code, 0);

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 2);
        assert_eq!(
            plan.ops[0],
            Op::CreateDir {
                path: "foundations".to_string()
            }
        );
        assert_eq!(
            plan.ops[1],
            Op::CreateDir {
                path: "archive".to_string()
            }
        );
    }

    // ── batch-move ────────────────────────────────────────────────────────────

    /// batch-move template: N=2, 2 src/dst pairs → plan with 2 move ops
    #[test]
    fn test_batch_move_two_items() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 1, N=2, pair1, pair2, blank description
        let (code, _) = wizard(
            "1\n2\nfile-a.md\nnew-a.md\nfile-b.md\nnew-b.md\n\n",
            Some(&out_str),
        );
        assert_eq!(code, 0);

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 2);
        assert_eq!(
            plan.ops[0],
            Op::Move {
                src: "file-a.md".to_string(),
                dst: "new-a.md".to_string()
            }
        );
        assert_eq!(
            plan.ops[1],
            Op::Move {
                src: "file-b.md".to_string(),
                dst: "new-b.md".to_string()
            }
        );
    }

    // ── categorize ────────────────────────────────────────────────────────────

    /// categorize: 2 items, parent=docs, custom name for item1, default for item2
    /// → CreateDir(docs) + 2 move ops
    #[test]
    fn test_categorize_two_items() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 2, parent=docs, item1=old-a.md, item2=old-b.md, blank,
        // custom name "a.md" for item1, Enter (default) for item2, blank description
        let (code, _) = wizard("2\ndocs\nold-a.md\nold-b.md\n\na.md\n\n\n", Some(&out_str));
        assert_eq!(code, 0);

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 3);
        assert_eq!(
            plan.ops[0],
            Op::CreateDir {
                path: "docs".to_string()
            }
        );
        assert_eq!(
            plan.ops[1],
            Op::Move {
                src: "old-a.md".to_string(),
                dst: "docs/a.md".to_string()
            }
        );
        assert_eq!(
            plan.ops[2],
            Op::Move {
                src: "old-b.md".to_string(),
                dst: "docs/old-b.md".to_string()
            }
        );
    }

    // ── archive ───────────────────────────────────────────────────────────────

    /// archive: 2 items → CreateDir(archive) + 2 move ops using basename as dst name
    #[test]
    fn test_archive_items() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 3, archive folder, 2 items, blank, blank description
        let (code, _) = wizard(
            "3\narchive\nproject-a.md\nproject-b.md\n\n\n",
            Some(&out_str),
        );
        assert_eq!(code, 0);

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 3);
        assert_eq!(
            plan.ops[0],
            Op::CreateDir {
                path: "archive".to_string()
            }
        );
        assert_eq!(
            plan.ops[1],
            Op::Move {
                src: "project-a.md".to_string(),
                dst: "archive/project-a.md".to_string(),
            }
        );
        assert_eq!(
            plan.ops[2],
            Op::Move {
                src: "project-b.md".to_string(),
                dst: "archive/project-b.md".to_string(),
            }
        );
    }

    // ── rename ────────────────────────────────────────────────────────────────

    /// rename: 2 items, item1 renamed, item2 skipped (Enter) → 1 move op
    #[test]
    fn test_rename_skip() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 4, item1, item2, blank to finish,
        // new name for item1, Enter to skip item2, blank description
        let (code, _) = wizard("4\nold-a.md\nold-b.md\n\nnew-a.md\n\n\n", Some(&out_str));
        assert_eq!(code, 0);

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(
            plan.ops[0],
            Op::Move {
                src: "old-a.md".to_string(),
                dst: "new-a.md".to_string()
            }
        );
    }

    // ── batch-move create_dir prompt (PLAN-002) ───────────────────────────────

    /// batch-move: dst with parent, user answers Y → plan has create_dir op before move op
    #[test]
    fn test_batch_move_dst_parent_prompt_yes() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 1, N=1, src=file.md, dst=foundations/file.md, answer Y, blank description
        let (code, _) = wizard("1\n1\nfile.md\nfoundations/file.md\nY\n\n", Some(&out_str));
        assert_eq!(code, 0);

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 2);
        assert_eq!(
            plan.ops[0],
            Op::CreateDir {
                path: "foundations".to_string()
            }
        );
        assert_eq!(
            plan.ops[1],
            Op::Move {
                src: "file.md".to_string(),
                dst: "foundations/file.md".to_string()
            }
        );
    }

    /// batch-move: dst with parent, user answers N → no create_dir op in plan
    #[test]
    fn test_batch_move_dst_parent_prompt_no() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 1, N=1, src=file.md, dst=foundations/file.md, answer N, blank description
        let (code, _) = wizard("1\n1\nfile.md\nfoundations/file.md\nN\n\n", Some(&out_str));
        assert_eq!(code, 0);

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(
            plan.ops[0],
            Op::Move {
                src: "file.md".to_string(),
                dst: "foundations/file.md".to_string()
            }
        );
    }

    /// batch-move: flat rename (dst has no parent component) → no create_dir prompt
    #[test]
    fn test_batch_move_flat_rename_no_prompt() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        // template 1, N=1, src=old.md, dst=new.md (flat, no parent), blank description
        // No prompt for create_dir — input has no extra answer line
        let (code, output) = wizard("1\n1\nold.md\nnew.md\n\n", Some(&out_str));
        assert_eq!(code, 0);
        assert!(
            !output.contains("does not exist"),
            "flat rename must not trigger a create_dir prompt; got output:\n{output}"
        );

        let plan = load_plan(&out).unwrap();
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(
            plan.ops[0],
            Op::Move {
                src: "old.md".to_string(),
                dst: "new.md".to_string()
            }
        );
    }

    // ── intro blurb + post-write hints (AR-007a) ─────────────────────────────

    /// Wizard output includes scaffold-first intro blurb before template list.
    #[test]
    fn test_wizard_output_includes_intro_blurb() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        let (code, output) = wizard("5\ntest-dir\n\n\n", Some(&out_str));
        assert_eq!(code, 0);
        assert!(
            output.contains("edit the file directly"),
            "intro blurb must appear before template list; got:\n{output}"
        );
    }

    /// Wizard output includes Tip: and Validate: lines in post-write hint block.
    #[test]
    fn test_wizard_output_includes_tip_and_validate() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("plan.toml");
        let out_str = out.to_str().unwrap().to_string();

        let (code, output) = wizard("5\ntest-dir\n\n\n", Some(&out_str));
        assert_eq!(code, 0);
        assert!(
            output.contains("Tip:"),
            "post-write block must include Tip: line; got:\n{output}"
        );
        assert!(
            output.contains("Validate: anchor plan validate"),
            "post-write block must include Validate: anchor plan validate line; got:\n{output}"
        );
    }

    // ── output path ───────────────────────────────────────────────────────────

    /// Default output path is "anchor-plan.toml" when out_path is None.
    #[test]
    fn test_default_output_path() {
        // scaffold template, 1 dir, blank to finish, blank description
        let (code, msgs) = wizard("5\ntest-dir\n\n\n", None);
        // Clean up regardless of outcome
        let _ = std::fs::remove_file("anchor-plan.toml");
        assert_eq!(code, 0);
        assert!(
            msgs.contains("anchor-plan.toml"),
            "expected 'anchor-plan.toml' in output"
        );
    }

    /// --output overrides write path; written path appears in output message.
    #[test]
    fn test_output_override() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("custom.toml");
        let out_str = out.to_str().unwrap().to_string();

        let (code, msgs) = wizard("5\nmy-dir\n\n\n", Some(&out_str));
        assert_eq!(code, 0);
        assert!(out.exists(), "plan file should exist at custom path");
        assert!(
            msgs.contains(&out_str),
            "output path should appear in Written message"
        );
    }

    // ── roundtrip ─────────────────────────────────────────────────────────────

    /// All 5 templates produce TOML parseable by plan::load_plan.
    #[test]
    fn test_all_templates_roundtrip() {
        let tmp = TempDir::new().unwrap();

        let cases: &[(&str, &str)] = &[
            ("batch-move", "1\n1\na.md\nb.md\n\n"),
            ("categorize", "2\nparent\nitem.md\n\n\n\n"),
            ("archive", "3\narc\nfile.md\n\n\n"),
            ("rename", "4\nfile.md\n\nnew.md\n\n"),
            ("scaffold", "5\ndir1\n\n\n"),
        ];

        for (name, input) in cases {
            let out = tmp.path().join(format!("{}.toml", name));
            let out_str = out.to_str().unwrap().to_string();
            let (code, _) = wizard(input, Some(&out_str));
            assert_eq!(code, 0, "template '{}' wizard returned non-zero", name);
            load_plan(&out)
                .unwrap_or_else(|e| panic!("template '{}' produced invalid TOML: {}", name, e));
        }
    }
}
