// src/cli/text/verify.rs — anchor text verify
//
// Read-only absence gate for text occurrences. This intentionally delegates
// enumeration to `anchor text find` so the scanned scope stays identical.

use crate::cli::text::find::{self, FindArgs, FindFormat, Occurrence};
use crate::infra::workspace;
use std::collections::{BTreeSet, HashSet};
use std::io;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct VerifyArgs {
    pub pattern: String,
    pub regex: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub file_types: Vec<String>,
    pub include_code_blocks: bool,
    pub include_frontmatter: bool,
    pub allow: Vec<String>,
    pub allow_from: Option<String>,
    pub format: FindFormat,
}

impl Default for VerifyArgs {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            regex: false,
            include: Vec::new(),
            exclude: Vec::new(),
            file_types: vec!["md".to_string()],
            include_code_blocks: false,
            include_frontmatter: true,
            allow: Vec::new(),
            allow_from: None,
            format: FindFormat::Text,
        }
    }
}

/// Execute `anchor text verify <pattern> ...`. Resolves workspace root, delegates to `run_on_root`.
pub fn run(args: VerifyArgs) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_on_root(&workspace_root, args)
}

/// Core verify logic on an explicit workspace root. Public for integration testing.
pub fn run_on_root(workspace_root: &Path, args: VerifyArgs) -> i32 {
    let allow_set = match parse_allowlist(&args) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let find_args = FindArgs {
        pattern: args.pattern.clone(),
        regex: args.regex,
        include: args.include.clone(),
        exclude: args.exclude.clone(),
        file_types: args.file_types.clone(),
        context: 0,
        format: args.format,
        include_code_blocks: args.include_code_blocks,
        include_frontmatter: args.include_frontmatter,
    };

    match find::do_find(workspace_root, &find_args) {
        Ok(occurrences) => {
            let violations = collect_violations(&occurrences, &allow_set);
            let allowed_count = occurrences.len().saturating_sub(violations.len());
            let result = match args.format {
                FindFormat::Json => {
                    format_json(&mut io::stdout(), &args, &violations, allowed_count)
                }
                FindFormat::Text => {
                    format_human(&mut io::stdout(), &args, &violations, allowed_count)
                }
            };
            if let Err(e) = result {
                eprintln!("error: {e}");
                return 2;
            }
            if violations.is_empty() {
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

fn collect_violations<'a>(
    occurrences: &'a [Occurrence],
    allow_set: &HashSet<(String, usize)>,
) -> Vec<&'a Occurrence> {
    occurrences
        .iter()
        .filter(|occ| !allow_set.contains(&(occ.file.clone(), occ.line)))
        .collect()
}

fn parse_allowlist(args: &VerifyArgs) -> Result<HashSet<(String, usize)>, String> {
    let mut allow_set = HashSet::new();

    if let Some(from_path) = args.allow_from.as_deref() {
        let content = std::fs::read_to_string(from_path)
            .map_err(|e| format!("reading --allow-from {from_path}: {e}"))?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match parse_allow_value(trimmed) {
                Some(value) => {
                    allow_set.insert(value);
                }
                None => eprintln!("warning: invalid --allow value: {trimmed}"),
            }
        }
    }

    for s in &args.allow {
        match parse_allow_value(s.trim()) {
            Some(value) => {
                allow_set.insert(value);
            }
            None => eprintln!("warning: invalid --allow value: {s}"),
        }
    }

    Ok(allow_set)
}

fn parse_allow_value(s: &str) -> Option<(String, usize)> {
    let (file, line) = s.rsplit_once(':')?;
    if file.is_empty() {
        return None;
    }
    let line = line.parse::<usize>().ok()?;
    if line == 0 {
        return None;
    }
    Some((file.to_string(), line))
}

fn format_human<W: io::Write>(
    w: &mut W,
    args: &VerifyArgs,
    violations: &[&Occurrence],
    allowed_count: usize,
) -> io::Result<()> {
    if violations.is_empty() {
        return writeln!(
            w,
            "✓ verified: no remaining occurrences of \"{}\" in scope ({allowed_count} allow-listed)",
            args.pattern
        );
    }

    for occ in violations {
        writeln!(w, "  {}:{}: {}", occ.file, occ.line, occ.match_text)?;
    }
    writeln!(
        w,
        "✗ FAILED: {} occurrence(s) of \"{}\" remain ({allowed_count} allow-listed)",
        violations.len(),
        args.pattern
    )
}

fn format_json<W: io::Write>(
    w: &mut W,
    args: &VerifyArgs,
    violations: &[&Occurrence],
    allowed_count: usize,
) -> io::Result<()> {
    let files: BTreeSet<&str> = violations.iter().map(|o| o.file.as_str()).collect();
    let violation_values: Vec<serde_json::Value> = violations
        .iter()
        .map(|o| {
            serde_json::json!({
                "file": o.file,
                "line": o.line,
                "column": o.column,
                "match_text": o.match_text,
            })
        })
        .collect();

    let output = serde_json::json!({
        "pattern": args.pattern,
        "regex": args.regex,
        "verified": violations.is_empty(),
        "violations": violation_values,
        "total_violations": violations.len(),
        "allowed_count": allowed_count,
        "total_files": files.len(),
    });
    writeln!(w, "{output}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
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

    fn args(pattern: &str) -> VerifyArgs {
        VerifyArgs {
            pattern: pattern.to_string(),
            ..VerifyArgs::default()
        }
    }

    fn find_args_from_verify(args: &VerifyArgs) -> FindArgs {
        FindArgs {
            pattern: args.pattern.clone(),
            regex: args.regex,
            include: args.include.clone(),
            exclude: args.exclude.clone(),
            file_types: args.file_types.clone(),
            context: 0,
            format: args.format,
            include_code_blocks: args.include_code_blocks,
            include_frontmatter: args.include_frontmatter,
        }
    }

    #[test]
    fn scope_parity() {
        let ws = make_workspace(&[
            ("a.md", "term here\n```\nterm hidden\n```\n"),
            ("b.txt", "term ignored\n"),
            ("c.md", "---\ntitle: term\n---\nbody term\n"),
        ]);
        let a = args("term");
        let occurrences = find::do_find(ws.path(), &find_args_from_verify(&a)).unwrap();
        let violations = collect_violations(&occurrences, &HashSet::new());

        let find_set: BTreeSet<_> = occurrences
            .iter()
            .map(|o| (o.file.clone(), o.line))
            .collect();
        let verify_set: BTreeSet<_> = violations
            .iter()
            .map(|o| (o.file.clone(), o.line))
            .collect();
        assert_eq!(find_set, verify_set);
        assert_eq!(verify_set.len(), 3);
    }

    #[test]
    fn exit_0_when_clean() {
        let ws = make_workspace(&[("a.md", "nothing\n")]);
        assert_eq!(run_on_root(ws.path(), args("missing")), 0);
    }

    #[test]
    fn exit_1_when_occurrence_remains() {
        let ws = make_workspace(&[("a.md", "term\n")]);
        assert_eq!(run_on_root(ws.path(), args("term")), 1);
    }

    #[test]
    fn exit_2_on_invalid_regex() {
        let ws = make_workspace(&[("a.md", "term\n")]);
        let mut a = args("[");
        a.regex = true;
        assert_eq!(run_on_root(ws.path(), a), 2);
    }

    #[test]
    fn allow_exempts_exact_line() {
        let ws = make_workspace(&[("a.md", "term\n"), ("b.md", "term\n")]);
        let mut one_allowed = args("term");
        one_allowed.allow = vec!["a.md:1".to_string()];
        assert_eq!(run_on_root(ws.path(), one_allowed.clone()), 1);

        let occurrences = find::do_find(ws.path(), &find_args_from_verify(&one_allowed)).unwrap();
        let allow_set = parse_allowlist(&one_allowed).unwrap();
        let violations = collect_violations(&occurrences, &allow_set);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].file, "b.md");
        assert_eq!(violations[0].line, 1);

        let mut both_allowed = args("term");
        both_allowed.allow = vec!["a.md:1".to_string(), "b.md:1".to_string()];
        assert_eq!(run_on_root(ws.path(), both_allowed), 0);
    }

    #[test]
    fn allow_from_file_parsed() {
        let ws = make_workspace(&[("a.md", "term\n"), ("b.md", "term\n")]);
        let allow_path = ws.path().join("allow.txt");
        fs::write(&allow_path, "\na.md:1\n").unwrap();

        let mut a = args("term");
        a.allow_from = Some(allow_path.to_string_lossy().to_string());
        let occurrences = find::do_find(ws.path(), &find_args_from_verify(&a)).unwrap();
        let allow_set = parse_allowlist(&a).unwrap();
        let violations = collect_violations(&occurrences, &allow_set);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].file, "b.md");
    }

    #[test]
    fn malformed_allow_warns_not_fails() {
        let ws = make_workspace(&[("a.md", "term\n")]);
        let mut a = args("term");
        a.allow = vec!["garbage".to_string()];
        let allow_set = parse_allowlist(&a).unwrap();
        assert!(allow_set.is_empty());
        assert_eq!(run_on_root(ws.path(), a), 1);
    }

    #[test]
    fn json_verified_true_iff_clean() {
        let ws = make_workspace(&[("a.md", "term\n")]);
        let mut clean = args("missing");
        clean.format = FindFormat::Json;
        let occurrences = find::do_find(ws.path(), &find_args_from_verify(&clean)).unwrap();
        let violations = collect_violations(&occurrences, &HashSet::new());
        let mut buf = Vec::new();
        format_json(&mut buf, &clean, &violations, 0).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed["verified"], true);
        assert_eq!(run_on_root(ws.path(), clean), 0);

        let mut dirty = args("term");
        dirty.format = FindFormat::Json;
        let occurrences = find::do_find(ws.path(), &find_args_from_verify(&dirty)).unwrap();
        let violations = collect_violations(&occurrences, &HashSet::new());
        let mut buf = Vec::new();
        format_json(&mut buf, &dirty, &violations, 0).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&buf).unwrap().trim()).unwrap();
        assert_eq!(parsed["verified"], false);
        assert_eq!(parsed["violations"][0]["file"], "a.md");
        assert_eq!(parsed["violations"][0]["line"], 1);
        assert_eq!(parsed["violations"][0]["match_text"], "term");
        assert_eq!(run_on_root(ws.path(), dirty), 1);
    }

    #[test]
    fn respects_include_exclude_and_frontmatter() {
        let ws = make_workspace(&[
            ("excluded.md", "term\n"),
            ("front.md", "---\ntitle: term\n---\nbody\n"),
        ]);

        let mut excluded = args("term");
        excluded.exclude = vec!["excluded.md".to_string()];
        excluded.include_frontmatter = false;
        assert_eq!(run_on_root(ws.path(), excluded), 0);

        let ws = make_workspace(&[("front.md", "---\ntitle: term\n---\nbody\n")]);
        let mut frontmatter = args("term");
        frontmatter.include_frontmatter = false;
        assert_eq!(run_on_root(ws.path(), frontmatter), 0);
    }
}
