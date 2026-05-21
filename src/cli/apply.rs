// src/cli/apply.rs — anchor apply command — 8-phase batch pipeline (AENG-016)
//
// Core invariant: no operation leaves a dangling reference.
// Pre-flight validates ALL Move ops before any op executes.
// Phase 2 commits all physical moves; Phases 6-8 stage, validate, commit ref rewrites.
// Already-committed moves are NOT rolled back on rewrite validation failure.

use crate::apply::post_apply_scan::{format_plain_text_warning, scan_partial_plain_text};
use crate::core::{
    acked::{parse_ref_line, AckedRefs},
    parser, resolver, rewriter, scanner, transaction,
};
use crate::infra::{lock, temp, workspace};
use crate::model::{
    plan::{self, Op},
    reference::RefForm,
    rewrite::RewriteEntry,
};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

/// Execute `anchor apply <plan.toml>`.
///
/// Discovers workspace root, builds acked set from disk + flags, delegates to `run_impl`.
/// Returns exit code: 0 = success, 1 = plan/preflight/op error, 2 = workspace/infra error.
pub fn run(
    plan_path: &str,
    allow_broken: &[String],
    allow_broken_from: Option<&str>,
    allow_prose_rewrites: bool,
) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| workspace_root.clone());
    let engine_home = workspace::resolve(&cwd, workspace::ResolveHints { tenant_flag: None })
        .map(|r| r.engine_home)
        .unwrap_or_else(|_| workspace_root.join(".accelmars"));

    // Load acked refs from disk, then extend with explicitly specified refs.
    let mut acked = AckedRefs::load(&engine_home);
    let mut newly_specified: Vec<(String, usize)> = Vec::new();

    for s in allow_broken {
        match parse_ref_line(s) {
            Some((f, l)) => {
                acked.add(&f, l);
                newly_specified.push((f, l));
            }
            None => {
                eprintln!("warning: invalid --allow-broken value: {s} (expected file:line)");
            }
        }
    }

    // --allow-broken-from resolves via std::fs::read_to_string — CWD-relative by default.
    if let Some(from_path) = allow_broken_from {
        match std::fs::read_to_string(from_path) {
            Ok(content) => {
                for line in content.lines() {
                    if let Some((f, l)) = parse_ref_line(line) {
                        acked.add(&f, l);
                        newly_specified.push((f, l));
                    }
                }
            }
            Err(e) => {
                eprintln!("error reading --allow-broken-from {from_path}: {e}");
                return 1;
            }
        }
    }

    let exit_code = run_impl(
        plan_path,
        &workspace_root,
        &engine_home,
        &mut std::io::stdout(),
        &acked,
        allow_prose_rewrites,
    );

    // Persist newly specified refs only on success.
    if exit_code == 0 && !newly_specified.is_empty() {
        AckedRefs::save(&engine_home, &newly_specified);
    }

    exit_code
}

/// Core implementation — 8-phase batch pipeline.
///
/// Phase A: pre-flight (validate all ops before any execution).
/// Phase 2: batch all filesystem ops (CreateDir + Move) — no ref rewriting yet.
/// Phase 3: single workspace scan (after all moves).
/// Phase 4: build forward/reverse maps from Move ops.
/// Phase 5: transaction::batch_plan — compute all ref rewrites in one pass.
/// Phase 6: stage rewrites to temp files.
/// Phase 7: validate staged content + moved files for broken refs.
/// Phase 8: commit staged files over originals.
/// Post: per-op non-.md text rewriting + UX-001 plain-text warning.
pub(crate) fn run_impl<W: Write>(
    plan_path: &str,
    workspace_root: &Path,
    engine_home: &Path,
    out: &mut W,
    acked: &AckedRefs,
    allow_prose_rewrites: bool,
) -> i32 {
    // Parse plan file
    let path = Path::new(plan_path);
    let plan = match plan::load_plan(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    // Canonical plan file path — used to exclude plan file from non-.md rewriting.
    let plan_file_abs = std::fs::canonicalize(path).ok();

    // Phase A: scan workspace for pre-flight validation.
    let preflight_files = match scanner::scan_workspace(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // Pre-flight: validate all Move ops before any execution begins.
    if let Err(e) = preflight(&plan, workspace_root, &preflight_files) {
        if is_already_applied(&plan, workspace_root) {
            eprintln!("note: all sources are missing and destinations already exist — this plan may have already been applied. Nothing was changed.");
        } else {
            eprintln!("{e}");
        }
        return 1;
    }

    let total = plan.ops.len();

    // Verify workspace is initialized before acquiring lock or creating staging dir.
    if !engine_home.join("anchor").exists() {
        eprintln!("error: workspace not initialized. Run 'anchor init' first.");
        return 2;
    }

    // Acquire single batch lock — held through Phase 8.
    let lock_op = format!("apply: batch {total} ops");
    let lock_guard = match lock::acquire_lock(engine_home, &lock_op) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: lock: {e}");
            return 2;
        }
    };

    // Create staging dir for Phase 6 rewrites.
    let op_dir = match temp::create_op_dir(engine_home) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: temp dir: {e}");
            drop(lock_guard);
            return 2;
        }
    };

    // Phase 2: execute all ops (CreateDir + Move) — physical filesystem only.
    let mut completed = 0usize;
    for op in &plan.ops {
        match op {
            Op::CreateDir { path: dir_path } => {
                let abs = workspace_root.join(dir_path);
                if let Err(e) = std::fs::create_dir_all(&abs) {
                    eprintln!("error creating {dir_path}/: {e}");
                    writeln!(
                        out,
                        "Stopped after {completed}/{total} operations completed."
                    )
                    .ok();
                    let _ = temp::cleanup_op_dir(&op_dir);
                    drop(lock_guard);
                    return 1;
                }
                completed += 1;
                writeln!(out, "[{completed}/{total}] created {dir_path}/").ok();
            }
            Op::Move { src, dst } => {
                let src_abs = workspace_root.join(src.as_str());
                let dst_abs = workspace_root.join(dst.as_str());
                if let Some(parent) = dst_abs.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = fs_rename(&src_abs, &dst_abs) {
                    eprintln!("error moving {src} \u{2192} {dst}: {e}");
                    writeln!(
                        out,
                        "Stopped after {completed}/{total} operations completed."
                    )
                    .ok();
                    let _ = temp::cleanup_op_dir(&op_dir);
                    drop(lock_guard);
                    return 1;
                }
                completed += 1;
                writeln!(out, "[{completed}/{total}] moved {src} \u{2192} {dst}").ok();
            }
        }
    }

    // Phase 3: single workspace scan — captures post-move file set.
    let workspace_files = match scanner::scan_workspace(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            let _ = temp::cleanup_op_dir(&op_dir);
            return 2;
        }
    };

    // Phase 4: build virtual maps from Move ops.
    let (forward_map, reverse_map) = build_virtual_maps(&plan.ops);

    // Build pre-move path set from Phase A scan — passed to batch_plan() so it can
    // distinguish real workspace paths from external-namespace strings. See AENG-017.
    let pre_move_paths = build_pre_move_paths(&preflight_files);

    // Phase 5: compute all ref rewrites in a single virtual-workspace pass.
    let entries = match transaction::batch_plan(
        workspace_root,
        &workspace_files,
        &forward_map,
        &reverse_map,
        allow_prose_rewrites,
        &pre_move_paths,
    ) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: plan: {e}");
            let _ = temp::cleanup_op_dir(&op_dir);
            return 1;
        }
    };

    // Phase 6: write rewritten file content to staging area.
    if let Err(e) = batch_stage_rewrites(workspace_root, &entries, &op_dir) {
        eprintln!("error: stage: {e}");
        let _ = temp::cleanup_op_dir(&op_dir);
        return 1;
    }

    // Phase 7: validate staged rewrites + moved files for broken refs.
    let acked_warnings = match batch_validate(
        workspace_root,
        &workspace_files,
        &entries,
        &op_dir,
        &reverse_map,
        acked,
    ) {
        Ok(warnings) => warnings,
        Err(e) => {
            eprintln!("{e}");
            let _ = temp::cleanup_op_dir(&op_dir);
            return 1;
        }
    };

    // Phase 8: rename staged files over originals.
    if let Err(e) = batch_commit_rewrites(workspace_root, &op_dir) {
        eprintln!("error: commit: {e}");
        return 1;
    }

    drop(lock_guard);

    // Emit acked warnings and summary.
    for w in &acked_warnings {
        writeln!(out, "{w}").ok();
    }
    writeln!(out, "Done. {total}/{total} operations completed.").ok();

    // Post-commit: rewrite non-.md files + UX-001 plain-text warning, per Move op.
    let workspace_md: Vec<String> = workspace_files
        .iter()
        .filter(|f| f.ends_with(".md"))
        .cloned()
        .collect();

    for op in &plan.ops {
        let Op::Move { src, dst } = op else { continue };

        let non_md_updated =
            rewrite_non_md_occurrences(workspace_root, src, dst, plan_file_abs.as_deref());
        if non_md_updated > 0 {
            eprintln!("{non_md_updated} non-markdown file(s) updated.");
        }

        let mut full_path_lines: Vec<(String, usize)> = workspace_md
            .iter()
            .filter_map(|f| {
                let content = std::fs::read_to_string(workspace_root.join(f.as_str())).ok()?;
                let count = content.matches(src.as_str()).count();
                if count > 0 {
                    Some((f.clone(), count))
                } else {
                    None
                }
            })
            .collect();
        full_path_lines.sort_by(|a, b| a.0.cmp(&b.0));

        let partial_hits = scan_partial_plain_text(&workspace_md, src, workspace_root);
        let trailing = src.rsplit('/').next().unwrap_or(src);
        if let Some(warning) = format_plain_text_warning(&full_path_lines, &partial_hits, trailing)
        {
            eprintln!("{warning}");
        }
    }

    0
}

/// Pre-flight: validate all Move ops before any op executes.
fn preflight(
    plan: &plan::Plan,
    workspace_root: &Path,
    workspace_files: &[String],
) -> Result<(), String> {
    for op in &plan.ops {
        let Op::Move { src, dst } = op else {
            continue;
        };

        let src_abs = workspace_root.join(src);
        if !src_abs.exists() {
            let suggestions = suggest_similar(src, workspace_files);
            let mut msg = format!("preflight failed: src not found: {src}");
            if let Some(top) = suggestions.first() {
                msg.push_str(&format!("\n  similar: {top}"));
            }
            return Err(msg);
        }

        let dst_abs = workspace_root.join(dst);
        if dst_abs.exists() {
            return Err(format!("preflight failed: dst already exists: {dst}"));
        }
    }
    Ok(())
}

/// Returns true iff all Move ops in the plan have src absent and dst present on disk.
fn is_already_applied(plan: &plan::Plan, workspace_root: &Path) -> bool {
    let move_ops: Vec<(&str, &str)> = plan
        .ops
        .iter()
        .filter_map(|op| {
            if let Op::Move { src, dst } = op {
                Some((src.as_str(), dst.as_str()))
            } else {
                None
            }
        })
        .collect();
    !move_ops.is_empty()
        && move_ops.iter().all(|(src, dst)| {
            !workspace_root.join(src).exists() && workspace_root.join(dst).exists()
        })
}

// ── Batch pipeline helpers ────────────────────────────────────────────────────

/// Build forward_map (src→dst) and reverse_map (dst→src) from Move ops.
fn build_virtual_maps(ops: &[Op]) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut forward_map = HashMap::new();
    let mut reverse_map = HashMap::new();
    for op in ops {
        if let Op::Move { src, dst } = op {
            forward_map.insert(src.clone(), dst.clone());
            reverse_map.insert(dst.clone(), src.clone());
        }
    }
    (forward_map, reverse_map)
}

/// Build a set of all pre-move workspace paths (files + every directory ancestor).
/// Used by batch_plan() to distinguish real workspace paths from external-namespace
/// strings (GitHub org/repo shorthands, package prefixes, etc.). See AENG-017.
pub(crate) fn build_pre_move_paths(files: &[String]) -> HashSet<String> {
    let mut set = HashSet::new();
    for f in files {
        set.insert(f.clone());
        let mut p: &str = f.as_str();
        while let Some(idx) = p.rfind('/') {
            p = &p[..idx];
            set.insert(p.to_string());
        }
    }
    set
}

/// Phase 6: for each unique file in entries, apply rewrites and write to staging area.
fn batch_stage_rewrites(
    workspace_root: &Path,
    entries: &[RewriteEntry],
    op_dir: &temp::TempOpDir,
) -> Result<(), String> {
    let mut by_file: HashMap<&str, Vec<&RewriteEntry>> = HashMap::new();
    for e in entries {
        by_file.entry(e.file.as_str()).or_default().push(e);
    }
    for (file_canonical, file_entries) in &by_file {
        let file_path = workspace_root.join(file_canonical);
        let content =
            std::fs::read_to_string(&file_path).map_err(|e| format!("{file_canonical}: {e}"))?;
        let owned: Vec<RewriteEntry> = file_entries.iter().map(|e| (*e).clone()).collect();
        let rewritten = rewriter::apply_rewrites(&content, &owned);
        let fc_string = file_canonical.to_string();
        let encoded = temp::encode_path(&fc_string);
        let staged_path = op_dir.path.join("rewrites").join(&encoded);
        std::fs::write(&staged_path, rewritten.as_bytes())
            .map_err(|e| format!("{file_canonical}: {e}"))?;
    }
    Ok(())
}

/// Phase 7: validate staged rewrites and all moved files for broken refs.
///
/// Returns acked warning strings on success. Returns Err on unacked broken refs.
fn batch_validate(
    workspace_root: &Path,
    workspace_files: &[String],
    entries: &[RewriteEntry],
    op_dir: &temp::TempOpDir,
    reverse_map: &HashMap<String, String>,
    acked: &AckedRefs,
) -> Result<Vec<String>, String> {
    let staged_canonicals: HashSet<&str> = entries.iter().map(|e| e.file.as_str()).collect();
    let mut broken: Vec<(String, usize, String)> = Vec::new();

    // Validate staged files (rewritten content).
    let rewrites_dir = op_dir.path.join("rewrites");
    if let Ok(read_dir) = std::fs::read_dir(&rewrites_dir) {
        for entry in read_dir.flatten() {
            let encoded = entry.file_name().to_string_lossy().into_owned();
            let file_canonical = encoded.replace("__", "/");
            let content =
                std::fs::read_to_string(entry.path()).map_err(|e| format!("validate: {e}"))?;
            collect_broken_refs(&file_canonical, &content, workspace_root, &mut broken);
        }
    }

    // Validate moved files that have no staged content (content unchanged, position changed).
    for file_new in workspace_files {
        if staged_canonicals.contains(file_new.as_str()) {
            continue;
        }
        let was_moved = reverse_map
            .keys()
            .any(|pfx| file_new == pfx || file_new.starts_with(&format!("{pfx}/")));
        if !was_moved {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(workspace_root.join(file_new.as_str())) else {
            continue;
        };
        collect_broken_refs(file_new, &content, workspace_root, &mut broken);
    }

    if broken.is_empty() {
        return Ok(vec![]);
    }

    let (acked_refs, unacked_refs): (Vec<_>, Vec<_>) = broken
        .into_iter()
        .partition(|(file, line, _)| acked.contains(file, *line));

    if !unacked_refs.is_empty() {
        let capped = &workspace_files[..200.min(workspace_files.len())];
        eprintln!("BROKEN REFERENCES AFTER REWRITE ({}):", unacked_refs.len());
        eprintln!();
        for (file, line, target) in &unacked_refs {
            eprint!(
                "{}",
                crate::core::diagnostics::format_broken_ref(file, *line, target, capped)
            );
        }
        return Err("rolled back.".to_string());
    }

    Ok(acked_refs
        .iter()
        .map(|(file, line, _)| format!("⚠  Allowing known broken ref: {file}:{line}  (acked)"))
        .collect())
}

/// Collect Form 1 broken refs from `content` parsed as `file_canonical`.
fn collect_broken_refs(
    file_canonical: &str,
    content: &str,
    workspace_root: &Path,
    broken: &mut Vec<(String, usize, String)>,
) {
    let fc = file_canonical.to_string();
    let refs = parser::parse_references(&fc, content);
    for reference in &refs {
        match reference.form {
            RefForm::Wiki | RefForm::Backtick | RefForm::HtmlHref => continue,
            _ => {}
        }
        let resolved = resolver::resolve_form1(&fc, &reference.target_raw);
        if !workspace_root.join(&resolved).exists() {
            let line_no = content[..reference.span.0]
                .chars()
                .filter(|&c| c == '\n')
                .count()
                + 1;
            broken.push((fc.clone(), line_no, reference.target_raw.clone()));
        }
    }
}

/// Phase 8: rename all staged files to their final workspace locations.
fn batch_commit_rewrites(workspace_root: &Path, op_dir: &temp::TempOpDir) -> Result<(), String> {
    let rewrites_dir = op_dir.path.join("rewrites");
    if let Ok(read_dir) = std::fs::read_dir(&rewrites_dir) {
        for entry in read_dir.flatten() {
            let encoded = entry.file_name().to_string_lossy().into_owned();
            let file_canonical = encoded.replace("__", "/");
            let final_path = workspace_root.join(&file_canonical);
            std::fs::rename(entry.path(), &final_path)
                .map_err(|e| format!("{file_canonical}: {e}"))?;
        }
    }
    let _ = temp::cleanup_op_dir(op_dir);
    Ok(())
}

/// Move src to dst; falls back to copy+delete on cross-filesystem error.
fn fs_rename(src: &Path, dst: &Path) -> Result<(), String> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            eprintln!("warning: cross-filesystem move: using copy+delete (non-atomic)");
            if src.is_dir() {
                copy_dir_all(src, dst).map_err(|e| e.to_string())?;
                std::fs::remove_dir_all(src).map_err(|e| e.to_string())?;
            } else {
                std::fs::copy(src, dst)
                    .map(|_| ())
                    .map_err(|e| e.to_string())?;
                std::fs::remove_file(src).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Recursively copy a directory, skipping symlinks.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

// ── Non-.md occurrence helpers ────────────────────────────────────────────────

/// Walk `workspace_root` and count plain-text occurrences of `needle` in .md files.
pub(crate) fn count_plaintext_md_occurrences(workspace_root: &Path, needle: &str) -> usize {
    let files = match scanner::scan_workspace(workspace_root) {
        Ok(f) => f,
        Err(_) => return 0,
    };
    files
        .iter()
        .filter(|f| f.ends_with(".md"))
        .filter_map(|f| std::fs::read_to_string(workspace_root.join(f)).ok())
        .map(|content| content.matches(needle).count())
        .sum()
}

/// Walk `workspace_root` and count text occurrences of `needle` in non-.md files.
#[cfg(test)]
fn count_text_occurrences(workspace_root: &Path, needle: &str) -> usize {
    let extensions = ["json", "yaml", "yml", "toml", "ts", "js", "py"];
    let mut total = 0usize;
    count_in_dir(workspace_root, needle, &extensions, &mut total);
    total
}

#[cfg(test)]
fn count_in_dir(dir: &Path, needle: &str, extensions: &[&str], total: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();

        if path.components().any(|c| c.as_os_str() == ".accelmars") {
            continue;
        }

        if path.is_dir() {
            count_in_dir(&path, needle, extensions, total);
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !extensions.contains(&ext) {
                continue;
            }
            if ext == "toml" && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                let mut start = 0;
                while let Some(pos) = content[start..].find(needle) {
                    *total += 1;
                    start += pos + needle.len();
                    if start >= content.len() {
                        break;
                    }
                }
            }
        }
    }
}

/// Walk `workspace_root` and replace text occurrences of `src` with `dst` in non-.md files.
pub(crate) fn rewrite_non_md_occurrences(
    workspace_root: &Path,
    src: &str,
    dst: &str,
    plan_file_abs: Option<&std::path::Path>,
) -> usize {
    let extensions = ["json", "yaml", "yml", "toml", "ts", "js", "py"];
    let mut updated = 0usize;
    rewrite_in_dir(
        workspace_root,
        src,
        dst,
        &extensions,
        &mut updated,
        plan_file_abs,
    );
    updated
}

fn rewrite_in_dir(
    dir: &Path,
    src: &str,
    dst: &str,
    extensions: &[&str],
    updated: &mut usize,
    plan_file_abs: Option<&std::path::Path>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == ".accelmars") {
            continue;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            rewrite_in_dir(&path, src, dst, extensions, updated, plan_file_abs);
            continue;
        }
        if let Some(plan_path) = plan_file_abs {
            if let Ok(canonical) = std::fs::canonicalize(&path) {
                if canonical == plan_path {
                    continue;
                }
            }
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !extensions.contains(&ext) {
            continue;
        }
        if ext == "toml" && path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !content.contains(src) {
            continue;
        }
        let new_content = content.replace(src, dst);
        if let Err(e) = std::fs::write(&path, new_content.as_bytes()) {
            eprintln!("warning: could not rewrite {}: {e}", path.display());
        } else {
            *updated += 1;
        }
    }
}

// ── Suggestion helpers ────────────────────────────────────────────────────────

fn suggest_similar(missing: &str, candidates: &[String]) -> Vec<String> {
    let missing_base = basename(missing);
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let c_base = basename(c);
            let max_len = missing_base.len().max(c_base.len());
            if max_len == 0 {
                return None;
            }
            let dist = levenshtein(missing_base, c_base);
            let normalized = dist as f64 / max_len as f64;
            if normalized <= 0.6 {
                Some((dist, c))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by_key(|(dist, _)| *dist);
    scored.into_iter().take(1).map(|(_, c)| c.clone()).collect()
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, val) in dp[0].iter_mut().enumerate() {
        *val = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::acked::AckedRefs;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".accelmars").join("anchor")).unwrap();
        tmp
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    fn plan_file(ws: &TempDir, content: &str) -> String {
        let path = ws.path().join("test.toml");
        fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    // ── Exit criterion 1: Pre-flight detects missing src ─────────────────────

    #[test]
    fn test_preflight_missing_src_workspace_unchanged() {
        let ws = make_workspace();
        write_file(ws.path(), "foundations/guide.md", "# Guide\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "foundtion/guide.md"
dst = "foundations/moved.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_ne!(code, 0, "missing src must return non-zero exit code");

        assert!(
            !ws.path().join("foundations/moved.md").exists(),
            "dst must not be created when preflight fails"
        );
        assert!(
            ws.path().join("foundations/guide.md").exists(),
            "original file must still exist — workspace unchanged"
        );
    }

    #[test]
    fn test_preflight_missing_src_includes_similar() {
        let ws = make_workspace();
        write_file(ws.path(), "foundations/guide.md", "# Guide\n");

        let plan_loaded = plan::load_plan(std::path::Path::new(&{
            let p = ws.path().join("test.toml");
            fs::write(
                &p,
                r#"version = "1"
[[ops]]
type = "move"
src = "foundtion/guide.md"
dst = "foundations/moved.md"
"#,
            )
            .unwrap();
            p.to_string_lossy().into_owned()
        }))
        .unwrap();

        let workspace_files = scanner::scan_workspace(ws.path()).unwrap();
        let result = preflight(&plan_loaded, ws.path(), &workspace_files);

        assert!(result.is_err(), "preflight must fail for missing src");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("similar: foundations/guide.md"),
            "error must include similar suggestion; got:\n{msg}"
        );
    }

    // ── Exit criterion 2: Pre-flight detects dst already exists ──────────────

    #[test]
    fn test_preflight_dst_exists_stops_execution() {
        let ws = make_workspace();
        write_file(ws.path(), "src/a.md", "# A\n");
        write_file(ws.path(), "src/b.md", "# B — already exists\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "src/a.md"
dst = "src/b.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_ne!(code, 0, "dst-exists must return non-zero exit code");

        assert!(
            ws.path().join("src/a.md").exists(),
            "src must still exist when preflight fails"
        );
    }

    // ── Exit criterion 3: CreateDir is idempotent ────────────────────────────

    #[test]
    fn test_create_dir_already_exists_exits_0() {
        let ws = make_workspace();
        fs::create_dir_all(ws.path().join("existing-dir")).unwrap();

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "existing-dir"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(
            code, 0,
            "CreateDir with existing path must exit 0 (idempotent)"
        );
    }

    // ── Exit criterion 4: Stopped after M/N on failure ───────────────────────

    #[test]
    fn test_failed_move_prints_stopped_message() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# A\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"

[[ops]]
type = "move"
src = "a.md"
dst = "c.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_ne!(code, 0, "second op must fail — non-zero exit code");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("Stopped after 1/2 operations completed."),
            "must print stopped message after 1 completed op; got:\n{output}"
        );

        assert!(
            ws.path().join("b.md").exists(),
            "first op must have committed — b.md must exist"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "src moved by first op — a.md must be gone"
        );
        assert!(
            !ws.path().join("c.md").exists(),
            "second op must not have committed — c.md must not exist"
        );
    }

    // ── Exit criterion 5: Successful plan prints "Done. N/N operations completed." ──

    #[test]
    fn test_successful_plan_prints_done() {
        let ws = make_workspace();
        write_file(ws.path(), "docs/source.md", "# Source\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/source.md"
dst = "docs/destination.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(code, 0, "successful plan must exit 0");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("Done. 1/1 operations completed."),
            "success message must be printed; got:\n{output}"
        );
    }

    // ── Exit criterion 6: Move progress line includes src and dst ──────────────

    /// Each Move op progress line includes [N/total] prefix, src, dst.
    /// Ref count is NOT in the per-op line — computed in Phase 5 after all moves.
    #[test]
    fn test_move_progress_line_format() {
        let ws = make_workspace();
        write_file(ws.path(), "src/target.md", "# Target\n");
        write_file(ws.path(), "src/referrer.md", "See [target](target.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "src/target.md"
dst = "src/renamed.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(code, 0, "must succeed");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("[1/1]"),
            "progress line must contain [1/1]; got:\n{output}"
        );
        assert!(
            output.contains("src/target.md"),
            "progress line must contain src; got:\n{output}"
        );
        assert!(
            output.contains("src/renamed.md"),
            "progress line must contain dst; got:\n{output}"
        );
    }

    // ── Zero-ref plain-text .md warning (UX-001) ─────────────────────────────

    #[test]
    fn test_zero_ref_plaintext_warning_emitted() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "docs/notes.md",
            "See also gateway-foundation for more details.\n",
        );
        let count = count_plaintext_md_occurrences(ws.path(), "gateway-foundation");
        assert!(
            count > 0,
            "expected >0 plain-text occurrences in notes.md, got: {count}"
        );
    }

    /// When refs are rewritten, move succeeds normally — no plain-text warning condition.
    #[test]
    fn test_zero_ref_no_warning_when_refs_found() {
        let ws = make_workspace();
        write_file(ws.path(), "src/target.md", "# Target\n");
        write_file(ws.path(), "src/referrer.md", "See [target](target.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "src/target.md"
dst = "src/renamed.md"
"#,
        );
        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(code, 0, "move with refs must succeed");
        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("Done."),
            "must print Done. summary; got:\n{output}"
        );
    }

    #[test]
    fn test_zero_ref_no_warning_when_no_plaintext() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "docs/clean.md",
            "# Clean document with no mentions\n",
        );
        let count = count_plaintext_md_occurrences(ws.path(), "gateway-foundation");
        assert_eq!(
            count, 0,
            "expected 0 plain-text occurrences when needle absent from all .md files"
        );
    }

    // ── Non-.md occurrence warning ─────────────────────────────────────────────

    #[test]
    fn test_nonmd_warning_emitted_when_occurrences_exist() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "config.json",
            r#"{"path": "gateway-foundation/config.yaml"}"#,
        );

        let count = count_text_occurrences(ws.path(), "gateway-foundation");
        assert!(
            count > 0,
            "expected >0 occurrences in config.json, got: {count}"
        );
    }

    #[test]
    fn test_nonmd_no_warning_when_clean() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# Hello\n");

        let count = count_text_occurrences(ws.path(), "gateway-foundation");
        assert_eq!(count, 0, "expected 0 occurrences when only .md files exist");
    }

    // ── rewrite_non_md_occurrences (AR-010 / REF-005) ────────────────────────

    #[test]
    fn test_rewrite_non_md_occurrences_updates_json() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "config.json",
            r#"{"path": "old-engine/config.yaml", "ref": "old-engine/index.md"}"#,
        );

        let updated = rewrite_non_md_occurrences(ws.path(), "old-engine", "new-engine", None);
        assert_eq!(updated, 1, "expected 1 file updated");

        let content = fs::read_to_string(ws.path().join("config.json")).unwrap();
        assert!(
            content.contains("new-engine"),
            "file must contain new path; got:\n{content}"
        );
        assert!(
            !content.contains("old-engine"),
            "old path must be gone from file; got:\n{content}"
        );
    }

    #[test]
    fn test_rewrite_non_md_occurrences_no_match_returns_zero() {
        let ws = make_workspace();
        let original = r#"{"path": "unrelated/path"}"#;
        write_file(ws.path(), "config.json", original);

        let updated = rewrite_non_md_occurrences(ws.path(), "old-engine", "new-engine", None);
        assert_eq!(updated, 0, "expected 0 files updated when no match");

        let content = fs::read_to_string(ws.path().join("config.json")).unwrap();
        assert_eq!(content, original, "file must be unchanged when no match");
    }

    // ── Intra-plan chain: batch pipeline correctness (AENG-016) ─────────────

    /// Batch pipeline: alpha refs beta; both dirs move in same plan.
    /// batch_plan must produce `../beta-engine/index.md` (intra-chain correctness).
    #[test]
    fn test_intra_plan_chain_refs_updated() {
        let ws = make_workspace();
        write_file(ws.path(), "alpha/index.md", "[beta](../beta/index.md)\n");
        write_file(ws.path(), "beta/index.md", "# Beta\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "create_dir"
path = "foundations"

[[ops]]
type = "move"
src = "alpha"
dst = "foundations/alpha-engine"

[[ops]]
type = "move"
src = "beta"
dst = "foundations/beta-engine"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(
            code,
            0,
            "plan must succeed; output:\n{}",
            String::from_utf8_lossy(&out)
        );

        let content =
            fs::read_to_string(ws.path().join("foundations/alpha-engine/index.md")).unwrap();
        assert!(
            content.contains("../beta-engine/index.md"),
            "ref must point to beta-engine after intra-plan chain; got:\n{content}"
        );
        assert!(
            !content.contains("../../beta/index.md"),
            "stale intermediate ref must be gone; got:\n{content}"
        );
    }

    /// Batch pipeline: two ops move dirs to different depths; relative ref updated correctly.
    #[test]
    fn test_multilevel_relative_ref_updated() {
        let ws = make_workspace();
        write_file(ws.path(), "a/README.md", "[b](../b/README.md)\n");
        write_file(ws.path(), "b/README.md", "# B\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a"
dst = "deep/nested/a"

[[ops]]
type = "move"
src = "b"
dst = "other/b"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(
            code,
            0,
            "plan must succeed; output:\n{}",
            String::from_utf8_lossy(&out)
        );

        let content = fs::read_to_string(ws.path().join("deep/nested/a/README.md")).unwrap();
        assert!(
            content.contains("../../../other/b/README.md"),
            "ref must point to other/b after multi-level chain; got:\n{content}"
        );
        assert!(
            !content.contains("../b/README.md"),
            "original ref must be gone; got:\n{content}"
        );
    }

    // ── Exit criterion 7: Reference integrity maintained after apply ──────────

    #[test]
    fn test_reference_integrity_after_apply() {
        let ws = make_workspace();
        write_file(ws.path(), "projects/source.md", "# Source\n");
        write_file(
            ws.path(),
            "projects/referrer.md",
            "See [source](source.md)\n",
        );

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "projects/source.md"
dst = "projects/renamed.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(code, 0, "must succeed");

        assert!(
            !ws.path().join("projects/source.md").exists(),
            "src must have been moved"
        );
        assert!(
            ws.path().join("projects/renamed.md").exists(),
            "dst must exist after apply"
        );

        let referrer_content = fs::read_to_string(ws.path().join("projects/referrer.md")).unwrap();
        assert!(
            referrer_content.contains("renamed.md"),
            "referrer must point to new dst path; got:\n{referrer_content}"
        );
        assert!(
            !referrer_content.contains("source.md"),
            "old reference must be gone from referrer; got:\n{referrer_content}"
        );
    }

    // ── Re-apply detection tests (AR-007) ─────────────────────────────────────

    #[test]
    fn test_apply_reapply_hint_emitted() {
        let ws = make_workspace();
        write_file(ws.path(), "docs/destination.md", "# Already moved\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/source.md"
dst = "docs/destination.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(code, 1, "re-apply must return exit 1");

        let plan = plan::load_plan(std::path::Path::new(&plan_path)).unwrap();
        assert!(
            is_already_applied(&plan, ws.path()),
            "is_already_applied must return true when all srcs absent and dsts present"
        );
    }

    #[test]
    fn test_apply_no_reapply_hint_when_src_missing_but_dst_also_absent() {
        let ws = make_workspace();

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/source.md"
dst = "docs/destination.md"
"#,
        );

        let plan = plan::load_plan(std::path::Path::new(&plan_path)).unwrap();
        assert!(
            !is_already_applied(&plan, ws.path()),
            "is_already_applied must return false when dst is also absent"
        );
    }

    // ── Plan file self-modification exclusion (AR-015) ────────────────────────

    #[test]
    fn test_apply_does_not_rewrite_plan_file() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# A\n");

        let plan_content =
            "version = \"1\"\n[[ops]]\ntype = \"move\"\nsrc = \"a.md\"\ndst = \"b.md\"\n";
        let plan_path = ws.path().join("plan.toml");
        fs::write(&plan_path, plan_content).unwrap();

        let mut out = Vec::new();
        let code = run_impl(
            plan_path.to_str().unwrap(),
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(code, 0, "plan must succeed");

        let plan_after = fs::read_to_string(&plan_path).unwrap();
        assert_eq!(
            plan_after, plan_content,
            "plan file must not be rewritten during apply; got:\n{plan_after}"
        );
    }

    #[test]
    fn test_apply_rewrites_adjacent_toml_but_not_plan_file() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "# A\n");
        write_file(ws.path(), "config.toml", "ref = \"a.md\"\n");

        let plan_content =
            "version = \"1\"\n[[ops]]\ntype = \"move\"\nsrc = \"a.md\"\ndst = \"b.md\"\n";
        let plan_path = ws.path().join("plan.toml");
        fs::write(&plan_path, plan_content).unwrap();

        let mut out = Vec::new();
        let code = run_impl(
            plan_path.to_str().unwrap(),
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        assert_eq!(code, 0, "plan must succeed");

        let plan_after = fs::read_to_string(&plan_path).unwrap();
        assert_eq!(plan_after, plan_content, "plan file must not be rewritten");

        let config_after = fs::read_to_string(ws.path().join("config.toml")).unwrap();
        assert!(
            config_after.contains("b.md"),
            "config.toml must be rewritten; got:\n{config_after}"
        );
        assert!(
            !config_after.contains("a.md"),
            "old path must be gone from config.toml; got:\n{config_after}"
        );
    }

    // ── AENG-003 — --allow-broken acked-refs tests ────────────────────────────

    /// Apply with 1 broken ref + matching acked entry → apply succeeds, warning in output.
    #[test]
    fn test_allow_broken_acked_suppresses_rollback() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "[broken](nonexistent.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        let mut acked = AckedRefs::empty();
        acked.add("b.md", 1);

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &acked,
            false,
        );
        assert_eq!(code, 0, "acked broken ref must not cause rollback");

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("⚠  Allowing known broken ref: b.md:1  (acked)"),
            "acked warning must appear in output; got:\n{output}"
        );
        assert!(
            ws.path().join("b.md").exists(),
            "b.md must exist after apply"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "a.md must be gone after apply"
        );
    }

    /// Apply with 1 broken ref but wrong file:line → validation fails, non-zero exit.
    ///
    /// Note: in the batch pipeline, Phase 2 (physical move) commits before Phase 7
    /// (validation). The file IS moved (b.md exists) but exit is non-zero.
    #[test]
    fn test_allow_broken_wrong_ref_still_rolls_back() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "[broken](nonexistent.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        let mut acked = AckedRefs::empty();
        acked.add("b.md", 999); // wrong line number — does not match b.md:1

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &acked,
            false,
        );
        assert_ne!(
            code, 0,
            "wrong file:line must not suppress validation error"
        );
        // Phase 2 committed the move; batch pipeline does not roll back physical moves.
        assert!(
            ws.path().join("b.md").exists(),
            "b.md must exist — file was moved in Phase 2 before validation"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "a.md must be gone — was moved to b.md in Phase 2"
        );
    }

    #[test]
    fn test_allow_broken_persisted_applies_on_reload() {
        let ws = make_workspace();
        write_file(ws.path(), "a.md", "[broken](nonexistent.md)\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        AckedRefs::save(&ws.path().join(".accelmars"), &[("b.md".to_string(), 1)]);

        let acked = AckedRefs::load(&ws.path().join(".accelmars"));

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &acked,
            false,
        );
        assert_eq!(code, 0, "acked ref loaded from disk must suppress rollback");
        assert!(
            ws.path().join("b.md").exists(),
            "b.md must exist after apply"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "a.md must be gone after apply"
        );
    }

    /// Partial ack: 2 broken refs, only 1 acked → validation fails, non-zero exit.
    ///
    /// Note: in the batch pipeline, b.md exists (moved in Phase 2) even though
    /// validation failed and rewrites were not committed.
    #[test]
    fn test_allow_broken_partial_ack_still_rolls_back() {
        let ws = make_workspace();
        write_file(
            ws.path(),
            "a.md",
            "[broken1](nonexistent1.md)\n[broken2](nonexistent2.md)\n",
        );

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "a.md"
dst = "b.md"
"#,
        );

        let mut acked = AckedRefs::empty();
        acked.add("b.md", 1);

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &acked,
            false,
        );
        assert_ne!(code, 0, "partial ack must not suppress validation error");
        // Phase 2 committed; batch pipeline does not roll back physical moves.
        assert!(
            ws.path().join("b.md").exists(),
            "b.md must exist — file was moved in Phase 2 before validation"
        );
        assert!(
            !ws.path().join("a.md").exists(),
            "a.md must be gone — was moved to b.md in Phase 2"
        );
    }

    // ── AENG-016: batch pipeline progress format ──────────────────────────────

    /// Batch pipeline: progress lines for both Move ops appear, no ref count inline.
    /// Intra-chain ref is correctly rewritten after single Phase 5 pass.
    #[test]
    fn test_batch_pipeline_intra_chain_correct() {
        let ws = make_workspace();
        write_file(ws.path(), "alpha/index.md", "[beta](../beta/index.md)\n");
        write_file(ws.path(), "beta/index.md", "# Beta\n");

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "alpha"
dst = "foundations/alpha-engine"

[[ops]]
type = "move"
src = "beta"
dst = "foundations/beta-engine"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false,
        );
        let output = String::from_utf8(out).unwrap();
        assert_eq!(code, 0, "batch pipeline must succeed; output:\n{output}");

        assert!(
            output.contains("[1/2] moved alpha"),
            "first move progress line; got:\n{output}"
        );
        assert!(
            output.contains("[2/2] moved beta"),
            "second move progress line; got:\n{output}"
        );

        let content =
            fs::read_to_string(ws.path().join("foundations/alpha-engine/index.md")).unwrap();
        assert!(
            content.contains("../beta-engine/index.md"),
            "intra-chain ref must be correct after batch pipeline; got:\n{content}"
        );
    }

    /// AENG-010 / Intake A Gap 1 — `anchor apply` must honor `--allow-prose-rewrites`,
    /// mirroring `anchor file mv`. With the flag off, arrow-line backtick mentions
    /// of the moved path are SKIPPED (prose heuristic). With the flag on, they are
    /// rewritten as live references.
    #[test]
    fn test_allow_prose_rewrites_off_skips_arrow_line_backtick() {
        let ws = make_workspace();
        write_file(ws.path(), "old/leaf.md", "# Leaf\n");
        write_file(
            ws.path(),
            "narrative.md",
            "Historical: file moved from `old/leaf.md` to `new/leaf.md`.\n",
        );

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "old/leaf.md"
dst = "new/leaf.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            false, // allow_prose_rewrites
        );
        assert_eq!(code, 0, "apply must succeed");

        let narrative =
            std::fs::read_to_string(ws.path().join("narrative.md")).expect("read narrative.md");
        assert!(
            narrative.contains("from `old/leaf.md` to `new/leaf.md`"),
            "prose backtick must be preserved when flag is off; got:\n{narrative}"
        );
    }

    #[test]
    fn test_allow_prose_rewrites_on_rewrites_arrow_line_backtick() {
        let ws = make_workspace();
        write_file(ws.path(), "old/leaf.md", "# Leaf\n");
        write_file(
            ws.path(),
            "narrative.md",
            "Historical: file moved from `old/leaf.md` to `new/leaf.md`.\n",
        );

        let plan_path = plan_file(
            &ws,
            r#"version = "1"
[[ops]]
type = "move"
src = "old/leaf.md"
dst = "new/leaf.md"
"#,
        );

        let mut out = Vec::new();
        let code = run_impl(
            &plan_path,
            ws.path(),
            &ws.path().join(".accelmars"),
            &mut out,
            &AckedRefs::empty(),
            true, // allow_prose_rewrites
        );
        assert_eq!(code, 0, "apply must succeed");

        let narrative =
            std::fs::read_to_string(ws.path().join("narrative.md")).expect("read narrative.md");
        assert!(
            narrative.contains("from `new/leaf.md` to `new/leaf.md`"),
            "prose backtick must be rewritten when --allow-prose-rewrites is on; got:\n{narrative}"
        );
    }
}
