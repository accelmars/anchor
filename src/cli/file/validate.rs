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
use crate::core::{
    acked::AckedPatterns, parser, reference::toml as toml_parser, reference::yaml as yaml_parser,
    resolver, scanner, suggest,
};
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
    workspace_files: Vec<String>,
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
            } else if let Err(e) = write_human_output(
                &mut io::stdout(),
                workspace_root,
                &result,
                &result.workspace_files,
            ) {
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

        let refs = if file_path.ends_with(".toml") {
            toml_parser::extract_toml_refs(&content, file_path)
        } else {
            let mut r = parser::parse_references(file_path, &content);
            r.extend(yaml_parser::extract_yaml_refs(&content, file_path));
            r
        };

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
                RefForm::Yaml | RefForm::Toml => reference
                    .target_raw
                    .strip_prefix("$(anchor root)/")
                    .unwrap_or(&reference.target_raw)
                    .to_string(),
                // Backtick path: resolve relative paths (starts with ./ or ../) before existence
                // check — consistent with Form 1 and Yaml/Toml handling. Also strip
                // $(anchor root)/ prefix (Gap 4).
                RefForm::Backtick => {
                    let raw = &reference.target_raw;
                    if raw.starts_with("./") || raw.starts_with("../") {
                        resolver::resolve_form1(file_path, raw)
                    } else if let Some(stripped) = raw.strip_prefix("$(anchor root)/") {
                        stripped.to_string()
                    } else {
                        raw.clone()
                    }
                }
                // HtmlHref: resolve relative to source file (same semantics as Form 1)
                RefForm::HtmlHref => resolver::resolve_form1(file_path, &reference.target_raw),
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
        workspace_files: files,
    })
}

/// Return broken refs as structured data for HTTP server handler (server::handle_file_validate).
pub fn validate_workspace(workspace_root: &Path) -> Result<Vec<(String, usize, String)>, String> {
    do_validate(workspace_root).map(|r| r.broken)
}

/// Write human-readable output to `w`.
fn write_human_output<W: Write>(
    w: &mut W,
    workspace_root: &Path,
    result: &ValidateResult,
    workspace_files: &[String],
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
            let suggestions = suggest::suggest_similar(raw, workspace_files);
            if let Some(top) = suggestions.first() {
                writeln!(w, "    similar: {top}")?;
            }
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

    /// `anchor validate` alias (Commands::Validate) dispatches to run_on_root — exits 0 on clean workspace.
    #[test]
    fn test_validate_alias_exits_0_on_clean_workspace() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "a.md", "[b](b.md)\n");
        write_file(tmp.path(), "b.md", "# B\n");
        let code = run_on_root(tmp.path(), None);
        assert_eq!(
            code, 0,
            "clean workspace must exit 0 via validate alias dispatch"
        );
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

    /// Broken ref where a close match (single-character typo) exists → "similar:" hint in output.
    #[test]
    fn test_human_output_suggests_similar_for_typo() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "source.md",
            "[broken](anchor-foundtion/file.md)\n",
        );
        fs::create_dir_all(tmp.path().join("anchor-foundation")).unwrap();
        write_file(tmp.path(), "anchor-foundation/file.md", "# Target\n");

        let result = do_validate(tmp.path()).unwrap();
        let mut out = Vec::new();
        write_human_output(&mut out, tmp.path(), &result, &result.workspace_files).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("similar: anchor-foundation/file.md"),
            "output must contain 'similar: anchor-foundation/file.md', got: {output}"
        );
    }

    /// Broken ref with no close match → no "similar:" line in output.
    #[test]
    fn test_human_output_no_similar_when_no_close_match() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "source.md", "[broken](xyz123qwerty.md)\n");

        let result = do_validate(tmp.path()).unwrap();
        let mut out = Vec::new();
        write_human_output(&mut out, tmp.path(), &result, &result.workspace_files).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(
            !output.contains("similar:"),
            "output must not contain 'similar:' when no close match exists, got: {output}"
        );
    }

    /// `--format json` output has no "similar" field even when suggestions exist.
    #[test]
    fn test_json_output_no_similar_field_when_suggestions_exist() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "source.md",
            "[broken](anchor-foundtion/file.md)\n",
        );
        fs::create_dir_all(tmp.path().join("anchor-foundation")).unwrap();
        write_file(tmp.path(), "anchor-foundation/file.md", "# Target\n");

        let result = do_validate(tmp.path()).unwrap();
        let mut out = Vec::new();
        write_json_output(&mut out, &result).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(String::from_utf8(out).unwrap().trim()).unwrap();
        assert!(
            parsed.get("similar").is_none(),
            "top-level JSON must not have 'similar' field"
        );
        for entry in parsed["broken"].as_array().unwrap() {
            assert!(
                entry.get("similar").is_none(),
                "broken entry must not have 'similar' field, got: {entry}"
            );
        }
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

    /// YAML frontmatter with a broken `$(anchor root)/` path → reported as broken ref.
    #[test]
    fn test_yaml_frontmatter_broken_path_detected() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "contract.md",
            "---\nstart_dir: \"$(anchor root)/nonexistent-path\"\n---\n# Body\n",
        );
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            !result.broken.is_empty(),
            "broken YAML frontmatter path must be reported as broken ref"
        );
        assert!(
            result
                .broken
                .iter()
                .any(|(_, _, raw)| raw.contains("$(anchor root)/nonexistent-path")),
            "broken ref target must include the full YAML path value; got: {:?}",
            result.broken
        );
    }

    /// YAML frontmatter with non-path fields (id, title, state) → no broken refs reported.
    #[test]
    fn test_yaml_non_path_fields_not_flagged() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "contract.md",
            "---\nid: \"AP-001\"\ntitle: \"Test contract\"\nstate: READY\n---\n# Body\n",
        );
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            result.broken.is_empty(),
            "non-path YAML fields must not be reported as broken refs; got: {:?}",
            result.broken
        );
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

    /// TOML config file with a broken `$(anchor root)/` path → reported as broken ref.
    #[test]
    fn test_toml_broken_path_detected() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "config.toml",
            "start_dir = \"$(anchor root)/nonexistent-toml-path\"\n",
        );
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            !result.broken.is_empty(),
            "broken TOML path must be reported as broken ref"
        );
        assert!(
            result
                .broken
                .iter()
                .any(|(_, _, raw)| raw.contains("$(anchor root)/nonexistent-toml-path")),
            "broken ref target must include the full TOML path value; got: {:?}",
            result.broken
        );
    }

    /// Gap 4: relative backtick path (../../) that resolves to an existing file → NOT reported as broken.
    #[test]
    fn test_relative_backtick_valid_target_not_reported_broken() {
        let tmp = TempDir::new().unwrap();
        // Target file: accelmars-guild/councils/os-council/decisions/foo.md
        write_file(
            tmp.path(),
            "accelmars-guild/councils/os-council/decisions/foo.md",
            "# Decision\n",
        );
        // Source file: accelmars-guild/projects/accelmars-gtm/STATUS.md
        // Relative backtick: ../../councils/os-council/decisions/foo.md
        // Resolves from accelmars-guild/projects/accelmars-gtm/ → accelmars-guild/councils/os-council/decisions/foo.md ✓
        write_file(
            tmp.path(),
            "accelmars-guild/projects/accelmars-gtm/STATUS.md",
            "See `../../councils/os-council/decisions/foo.md` for context.\n",
        );
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            result.broken.is_empty(),
            "relative backtick resolving to existing file must not be reported as broken; got: {:?}",
            result.broken
        );
    }

    /// Gap 4 regression: relative backtick path that resolves to a NON-EXISTING file IS reported as broken.
    #[test]
    fn test_relative_backtick_broken_target_still_reported() {
        let tmp = TempDir::new().unwrap();
        // Source file with relative backtick ref to a file that does not exist.
        write_file(
            tmp.path(),
            "accelmars-guild/projects/accelmars-gtm/STATUS.md",
            "See `../../councils/os-council/decisions/missing.md` for context.\n",
        );
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            !result.broken.is_empty(),
            "relative backtick resolving to missing file must be reported as broken"
        );
    }

    /// Gap 4 regression: `$(anchor root)/` backtick path where target EXISTS → NOT reported as broken.
    #[test]
    fn test_anchor_root_backtick_valid_target_not_reported_broken() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "accelmars-guild/councils/os-council/decisions/foo.md",
            "# Decision\n",
        );
        write_file(
            tmp.path(),
            "proposals/MKT-144.md",
            "Path: `$(anchor root)/accelmars-guild/councils/os-council/decisions/foo.md`\n",
        );
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            result.broken.is_empty(),
            "$(anchor root)/ backtick resolving to existing file must not be reported as broken; got: {:?}",
            result.broken
        );
    }

    /// TOML plan file with src/dst fields using relative paths → no broken refs reported.
    #[test]
    fn test_toml_plan_src_dst_not_flagged() {
        let tmp = TempDir::new().unwrap();
        write_file(
            tmp.path(),
            "ops.toml",
            concat!(
                "version = \"1\"\n",
                "description = \"batch-move\"\n",
                "\n",
                "[[ops]]\n",
                "type = \"move\"\n",
                "src = \"anchor-foundation\"\n",
                "dst = \"foundations/anchor-engine\"\n",
            ),
        );
        let result = do_validate(tmp.path()).unwrap();
        assert!(
            result.broken.is_empty(),
            "plan file src/dst relative paths must not be flagged as broken refs; got: {:?}",
            result.broken
        );
    }
}
