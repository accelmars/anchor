// src/core/transaction.rs — transaction phases: PLAN, APPLY, VALIDATE, COMMIT, ROLLBACK (MF-005+MF-006)
#![allow(dead_code)]
//
// PLAN phase: MF-005 — scan workspace, classify references (A/B/C), build rewrite plan.
// APPLY, VALIDATE, COMMIT, ROLLBACK phases: MF-006.
//
// Anti-pattern from HANDOVER.md (Case C detection bug):
//   WRONG: skip if reference SOURCE is inside src/ — this skips Case B (which needs rewriting).
//   CORRECT: Case C requires BOTH source file AND target inside src/.
//   Test: is_case_c = inside_src(ref.source_file) AND inside_src(ref.target)
//
// COMMIT order (HANDOVER.md anti-pattern): rename rewrites over originals FIRST,
// then rename moved/src → dst. Reversing this order breaks recovery on crash.

use crate::core::{parser, resolver, rewriter};
use crate::infra::temp::{self, TempOpDir};
use crate::model::{
    manifest::{self, Manifest},
    reference::RefForm,
    rewrite::{RewriteEntry, RewritePlan},
    CanonicalPath,
};
use std::path::Path;

/// Error returned by transaction operations.
#[derive(Debug)]
pub enum TransactionError {
    Io(std::io::Error),
    Manifest(crate::model::manifest::ManifestError),
    Temp(crate::infra::temp::TempError),
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionError::Io(e) => write!(f, "transaction I/O error: {e}"),
            TransactionError::Manifest(e) => write!(f, "manifest error: {e}"),
            TransactionError::Temp(e) => write!(f, "temp dir error: {e}"),
        }
    }
}

impl From<std::io::Error> for TransactionError {
    fn from(e: std::io::Error) -> Self {
        TransactionError::Io(e)
    }
}

impl From<crate::model::manifest::ManifestError> for TransactionError {
    fn from(e: crate::model::manifest::ManifestError) -> Self {
        TransactionError::Manifest(e)
    }
}

impl From<crate::infra::temp::TempError> for TransactionError {
    fn from(e: crate::infra::temp::TempError) -> Self {
        TransactionError::Temp(e)
    }
}

/// A reference that failed to resolve after rewrite.
#[derive(Debug, Clone)]
pub struct BrokenRef {
    /// Canonical path of the file containing the broken reference (post-move path).
    pub file: CanonicalPath,
    /// 1-based line number of the broken reference.
    pub line: usize,
    /// Raw target string (as it appears in the file).
    pub target: String,
}

/// Error returned by the VALIDATE phase.
#[derive(Debug)]
pub enum ValidationError {
    /// One or more references failed to resolve after rewrite.
    BrokenRefs(Vec<BrokenRef>),
    /// I/O error during validation.
    Io(std::io::Error),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::BrokenRefs(refs) => {
                write!(f, "{} broken reference(s) after rewrite", refs.len())
            }
            ValidationError::Io(e) => write!(f, "validation I/O error: {e}"),
        }
    }
}

impl From<std::io::Error> for ValidationError {
    fn from(e: std::io::Error) -> Self {
        ValidationError::Io(e)
    }
}

/// Error returned by the COMMIT phase.
#[derive(Debug)]
pub enum CommitError {
    Io(std::io::Error),
    Manifest(crate::model::manifest::ManifestError),
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::Io(e) => write!(f, "commit I/O error: {e}"),
            CommitError::Manifest(e) => write!(f, "commit manifest error: {e}"),
        }
    }
}

impl From<std::io::Error> for CommitError {
    fn from(e: std::io::Error) -> Self {
        CommitError::Io(e)
    }
}

impl From<crate::model::manifest::ManifestError> for CommitError {
    fn from(e: crate::model::manifest::ManifestError) -> Self {
        CommitError::Manifest(e)
    }
}

/// Returns true if `path` is `src` or is a file/directory inside `src/`.
///
/// Handles both file moves (`path == src`) and directory moves (`path` starts with `src/`).
///
/// IMPORTANT (HANDOVER.md anti-pattern): Case C requires BOTH source_file AND target
/// to satisfy inside_src. Checking only one side produces incorrect classification.
fn inside_src(path: &CanonicalPath, src: &CanonicalPath) -> bool {
    path == src || path.starts_with(&format!("{src}/"))
}

/// Remap a canonical path from under `src` to under `dst`.
///
/// If `canonical == src`, returns `dst`.
/// If `canonical` starts with `src/`, substitutes the prefix.
///
/// Example: src="projects/foo", dst="projects/archive/foo"
///   "projects/foo/bar.md" → "projects/archive/foo/bar.md"
fn remap_path(
    canonical: &CanonicalPath,
    src: &CanonicalPath,
    dst: &CanonicalPath,
) -> CanonicalPath {
    if canonical == src {
        dst.clone()
    } else {
        // canonical starts with "src/" (guaranteed by caller via inside_src check)
        format!("{dst}{}", &canonical[src.len()..])
    }
}

/// Compute the relative path from `from_file`'s directory to `to_file`.
///
/// Both arguments are workspace-root-relative canonical paths. The result is the
/// relative path string that should appear in a Markdown Form 1 reference
/// (e.g. `"../../docs/guide.md"`).
fn compute_relative_path(from_file: &CanonicalPath, to_file: &CanonicalPath) -> String {
    // Determine the directory containing from_file
    let from_dir: Vec<&str> = match from_file.rfind('/') {
        Some(idx) => from_file[..idx].split('/').collect(),
        None => vec![], // file at workspace root
    };

    let to_parts: Vec<&str> = to_file.split('/').collect();

    // Find common prefix length
    let common_len = from_dir
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let up_count = from_dir.len() - common_len;
    let down_parts = &to_parts[common_len..];

    let mut rel: Vec<&str> = (0..up_count).map(|_| "..").collect();
    rel.extend_from_slice(down_parts);

    if rel.is_empty() {
        // Same file — should not occur in valid move operations
        to_file.clone()
    } else {
        rel.join("/")
    }
}

/// Reconstruct a Form 1 Markdown reference with an updated path.
///
/// When link text is an exact byte match of `old_target_raw` (the path the author copied
/// as display text), updates the text to `new_rel_path` so the display stays in sync.
/// Any other link text is preserved unchanged.
///
/// Input:  `old_text` = `"[old/path.md](old/path.md)"`, new_rel_path = `"new/path.md"`, old_target_raw = `"old/path.md"`
/// Output: `"[new/path.md](new/path.md)"`
fn rebuild_form1_ref(
    old_text: &str,
    new_rel_path: &str,
    anchor: &Option<String>,
    old_target_raw: &str,
) -> String {
    // Find the ]( boundary separating link text from path
    let bracket_paren = old_text
        .find("](")
        .expect("Form 1 reference must contain ](");
    let link_text = &old_text[1..bracket_paren]; // between [ and ](

    // REF-001: if link text is an exact copy of the raw path, keep it in sync with the new path
    let updated_link_text = if link_text == old_target_raw {
        new_rel_path.to_string()
    } else {
        link_text.to_string()
    };

    let path_with_anchor = match anchor {
        Some(a) => format!("{new_rel_path}#{a}"),
        None => new_rel_path.to_string(),
    };

    format!("[{updated_link_text}]({path_with_anchor})")
}

/// Compute the stem (filename without `.md` extension) of a canonical path.
fn stem_of(canonical: &CanonicalPath) -> &str {
    let filename = canonical.rsplit('/').next().unwrap_or(canonical.as_str());
    filename.strip_suffix(".md").unwrap_or(filename)
}

/// Returns `Some(new_target_raw)` if `target_raw` is a partial-path backtick match for `src`.
///
/// A partial-path match occurs when `target_raw` equals a valid suffix of `src`, or is a
/// path under such a suffix — where a valid suffix must begin after a `/` in `src` (prevents
/// `bar` from matching `foobar`). Returns `None` if no match or if the rewrite is a no-op.
///
/// Example: src="a/b/c", dst="x/y/c", target_raw="b/c/d.md" → Some("y/c/d.md")
fn rewrite_partial_backtick(target_raw: &str, src: &str, dst: &str) -> Option<String> {
    let src_parts: Vec<&str> = src.split('/').collect();
    let dst_parts: Vec<&str> = dst.split('/').collect();

    for n in 1..src_parts.len() {
        let src_suffix = src_parts[n..].join("/");

        let tail: &str = if target_raw == src_suffix {
            ""
        } else if target_raw.starts_with(&format!("{src_suffix}/")) {
            &target_raw[src_suffix.len()..]
        } else {
            continue;
        };

        if n >= dst_parts.len() {
            continue;
        }
        let dst_suffix = dst_parts[n..].join("/");
        let new_target_raw = format!("{dst_suffix}{tail}");
        if new_target_raw == target_raw {
            continue;
        }
        return Some(new_target_raw);
    }

    None
}

/// PLAN phase: scan workspace, classify all references (A/B/C), build the rewrite plan.
///
/// Algorithm from 04-TRANSACTIONS.md §PLAN Phase Detail:
/// 1. For each file in `workspace_files`: read content, parse all references
/// 2. For Form 1 references: resolve to canonical path, classify (A/B/C), compute new_text
/// 3. For Form 2 (wiki links): resolve via workspace stem scan; rewrite only if stem changes
///    (Case A where stem_of(src) != stem_of(dst); Case B not applicable for wiki links;
///    Case C: both inside src — skip)
/// 4. Classify each reference that touches `src`:
///    - Case A: !inside_src(source_file) && inside_src(target) → rewrite target path
///    - Case B:  inside_src(source_file) && !inside_src(target) → rewrite relative path (source moved)
///    - Case C:  inside_src(source_file) &&  inside_src(target) → skip (relative path stable)
/// 5. For each Case A and Case B: compute old_text + new_text via path remapping
/// 6. Return RewritePlan (manifest.phase update → "APPLY" is the orchestrator's responsibility)
///
/// # Arguments
/// - `workspace_root`: absolute path to the workspace root directory
/// - `src`: canonical path of the file or directory being moved
/// - `dst`: canonical path of the destination
/// - `workspace_files`: pre-computed list of all `.md` files in the workspace
pub fn plan(
    workspace_root: &Path,
    src: &CanonicalPath,
    dst: &CanonicalPath,
    workspace_files: &[CanonicalPath],
) -> Result<RewritePlan, TransactionError> {
    let mut entries: Vec<RewriteEntry> = Vec::new();

    for file_canonical in workspace_files {
        let file_path = workspace_root.join(file_canonical.as_str());
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File disappeared between scan and plan — skip silently
                continue;
            }
            Err(e) => return Err(TransactionError::Io(e)),
        };

        let refs = parser::parse_references(file_canonical, &content);

        for reference in refs {
            if reference.form == RefForm::Backtick {
                // Backtick path refs: match target_raw against src using exact or partial-path
                // suffix matching. target_raw is the backtick content with trailing slash
                // stripped (parser invariant).
                //
                // Gap 2 + Gap 5: strip $(anchor root)/ prefix before matching so that
                // `$(anchor root)/accelmars-guild/projects/os-council/...` matches src
                // `accelmars-guild/projects/os-council`. Prefix is preserved in new_text.
                let anchor_root_prefix = "$(anchor root)/";
                let has_anchor_prefix = reference.target_raw.starts_with(anchor_root_prefix);
                let target_normalized: CanonicalPath = if has_anchor_prefix {
                    reference.target_raw[anchor_root_prefix.len()..].to_string()
                } else {
                    reference.target_raw.clone()
                };

                // Gap 3: resolve relative backtick paths (starts with ./ or ../) to
                // workspace-relative canonical before matching — same normalization as Form 1.
                // When a relative ref matches, new_text is recomputed as a relative path from
                // the source file to the new target location (source stays put; target moves).
                let was_relative = target_normalized.starts_with("./")
                    || target_normalized.starts_with("../");
                let target_to_match: CanonicalPath = if was_relative {
                    resolver::resolve_form1(&reference.source_file, &target_normalized)
                } else {
                    target_normalized.clone()
                };

                let new_normalized = if inside_src(&target_to_match, src) {
                    remap_path(&target_to_match, src, dst)
                } else {
                    match rewrite_partial_backtick(&target_to_match, src, dst) {
                        Some(t) => t,
                        None => continue, // no match — not related to this move
                    }
                };

                // Case C: source file also inside src.
                // For non-prefixed refs: relative path is stable — skip.
                // For $(anchor root)/-prefixed refs: absolute path must still be rewritten — do not skip.
                if !has_anchor_prefix && inside_src(file_canonical, src) {
                    continue;
                }

                let old_text = content[reference.span.0..reference.span.1].to_string();
                let inner = &old_text[1..old_text.len() - 1]; // strip surrounding backticks
                let had_slash = inner.ends_with('/');
                let new_text = if has_anchor_prefix {
                    if had_slash {
                        format!("`$(anchor root)/{new_normalized}/`")
                    } else {
                        format!("`$(anchor root)/{new_normalized}`")
                    }
                } else if was_relative {
                    // Source file does not move; target did — recompute relative path from source to new target.
                    let new_rel = compute_relative_path(file_canonical, &new_normalized);
                    if had_slash {
                        format!("`{new_rel}/`")
                    } else {
                        format!("`{new_rel}`")
                    }
                } else if had_slash {
                    format!("`{new_normalized}/`")
                } else {
                    format!("`{new_normalized}`")
                };
                entries.push(RewriteEntry {
                    file: file_canonical.clone(),
                    span: reference.span,
                    old_text,
                    new_text,
                });
                continue;
            }

            if reference.form == RefForm::Wiki {
                // Form 2 (wiki links): stem-based. Rewrite only if the target is inside src
                // and the stem actually changes (i.e., dst has a different filename).
                // Case B not applicable — wiki links don't use relative paths; moving the
                // source file doesn't affect the stem's global resolution.
                let resolve_result =
                    resolver::resolve_form2(&reference.target_raw, workspace_files);
                let target_canonical = match resolve_result {
                    resolver::ResolveResult::Resolved(c) => c,
                    // Ambiguous or broken: skip — already broken before this move
                    _ => continue,
                };

                let target_inside = inside_src(&target_canonical, src);
                if !target_inside {
                    continue; // Target not affected by this move
                }

                // Case C: source also inside src → stem is stable (both move together)
                if inside_src(file_canonical, src) {
                    continue;
                }

                // Case A: external file has wiki link pointing into src
                let new_target = remap_path(&target_canonical, src, dst);
                let old_stem = stem_of(&target_canonical);
                let new_stem_str = rewriter::compute_form2_new_text(&new_target);
                if old_stem == new_stem_str.as_str() {
                    continue; // Stem unchanged — no rewrite needed (e.g., directory move)
                }

                let old_text = content[reference.span.0..reference.span.1].to_string();
                // Preserve alias if present: [[old-stem]] → [[new-stem]]
                // or [[old-stem|alias]] → [[new-stem|alias]]
                let prefix = format!("[[{old_stem}");
                let new_prefix = format!("[[{new_stem_str}");
                let new_text = old_text.replacen(&prefix, &new_prefix, 1);
                entries.push(RewriteEntry {
                    file: file_canonical.clone(),
                    span: reference.span,
                    old_text,
                    new_text,
                });
                continue;
            }

            if reference.form == RefForm::HtmlHref {
                // HtmlHref: href paths are relative to the containing file — same resolution as Form 1.
                // resolve_form1 strips any #fragment from target_raw automatically.
                let target_canonical =
                    resolver::resolve_form1(file_canonical, &reference.target_raw);

                let source_inside = inside_src(file_canonical, src);
                let target_inside = inside_src(&target_canonical, src);

                match (source_inside, target_inside) {
                    (true, true) | (false, false) => {
                        continue; // Case C or unrelated — skip
                    }
                    (false, true) | (true, false) => {
                        let (new_rel, old_text) = if !source_inside {
                            // Case A: external file has href pointing into src
                            let new_target = remap_path(&target_canonical, src, dst);
                            let rel = compute_relative_path(file_canonical, &new_target);
                            (rel, content[reference.span.0..reference.span.1].to_string())
                        } else {
                            // Case B: file inside src has href pointing to external target
                            let new_source = remap_path(file_canonical, src, dst);
                            let rel = compute_relative_path(&new_source, &target_canonical);
                            (rel, content[reference.span.0..reference.span.1].to_string())
                        };
                        // Preserve original quote style (old_text = `href="path"` or `href='path'`)
                        let quote = old_text.as_bytes().get(5).copied().unwrap_or(b'"') as char;
                        let new_text = format!("href={quote}{new_rel}{quote}");
                        entries.push(RewriteEntry {
                            file: file_canonical.clone(),
                            span: reference.span,
                            old_text,
                            new_text,
                        });
                    }
                }
                continue;
            }

            // Form 1: resolve to canonical target
            let target_canonical = resolver::resolve_form1(file_canonical, &reference.target_raw);

            let source_inside = inside_src(file_canonical, src);
            let target_inside = inside_src(&target_canonical, src);

            match (source_inside, target_inside) {
                // Case C: both inside src — relative path between them is stable after move
                (true, true) => continue,

                // Neither touches src — not relevant to this move
                (false, false) => continue,

                // Case A: external file references a file inside src/
                //   → rewrite the target path in the external file
                (false, true) => {
                    let new_target = remap_path(&target_canonical, src, dst);
                    let new_rel = compute_relative_path(file_canonical, &new_target);
                    let old_text = content[reference.span.0..reference.span.1].to_string();
                    let new_text = rebuild_form1_ref(
                        &old_text,
                        &new_rel,
                        &reference.anchor,
                        reference.target_raw.as_str(),
                    );
                    entries.push(RewriteEntry {
                        file: file_canonical.clone(),
                        span: reference.span,
                        old_text,
                        new_text,
                    });
                }

                // Case B: file inside src/ references an external file
                //   → the source file will move, so recompute relative path from its new location
                (true, false) => {
                    let new_source = remap_path(file_canonical, src, dst);
                    let new_rel = compute_relative_path(&new_source, &target_canonical);
                    let old_text = content[reference.span.0..reference.span.1].to_string();
                    let new_text = rebuild_form1_ref(
                        &old_text,
                        &new_rel,
                        &reference.anchor,
                        reference.target_raw.as_str(),
                    );
                    entries.push(RewriteEntry {
                        file: file_canonical.clone(),
                        span: reference.span,
                        old_text,
                        new_text,
                    });
                }
            }
        }
    }

    Ok(RewritePlan {
        src: src.clone(),
        dst: dst.clone(),
        entries,
    })
}

/// APPLY phase: copy src to temp, write rewritten files to temp/rewrites/, update manifest.
///
/// IMPORTANT: originals are NOT touched during APPLY. All writes go to op_dir.
/// From 04-TRANSACTIONS.md §APPLY Phase Detail.
pub fn apply(
    workspace_root: &Path,
    plan: &RewritePlan,
    op_dir: &TempOpDir,
    manifest: &mut Manifest,
) -> Result<(), TransactionError> {
    // Step 1: copy src (file or directory tree) to op_dir/moved/{src_name}
    let src_path = workspace_root.join(plan.src.as_str());
    let src_name = std::path::Path::new(plan.src.as_str())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| plan.src.clone());
    let moved_dst = op_dir.path.join("moved").join(&src_name);

    if src_path.is_dir() {
        copy_dir_recursive(&src_path, &moved_dst)?;
    } else {
        std::fs::copy(&src_path, &moved_dst)?;
    }

    // Step 2: for each file in the rewrite plan, apply rewrites and write to op_dir/rewrites/
    // Group entries by file so apply_rewrites sees all entries for a file at once.
    let mut files_to_rewrite: Vec<&CanonicalPath> = plan
        .entries
        .iter()
        .map(|e| &e.file)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    files_to_rewrite.sort(); // deterministic order

    for file_canonical in files_to_rewrite {
        let file_path = workspace_root.join(file_canonical.as_str());
        let content = std::fs::read_to_string(&file_path)?;

        let file_entries: Vec<&RewriteEntry> = plan
            .entries
            .iter()
            .filter(|e| &e.file == file_canonical)
            .collect();

        // Collect by value for apply_rewrites
        let owned: Vec<RewriteEntry> = file_entries.iter().map(|e| (*e).clone()).collect();
        let rewritten = rewriter::apply_rewrites(&content, &owned);

        let encoded = temp::encode_path(file_canonical);
        let rewrite_path = op_dir.path.join("rewrites").join(&encoded);
        std::fs::write(&rewrite_path, rewritten)?;
    }

    // Step 3: update manifest phase → VALIDATE
    manifest.phase = "VALIDATE".to_string();
    manifest::write_manifest(&op_dir.path, manifest)?;

    Ok(())
}

/// Recursively copy a directory tree from `src` to `dst`.
///
/// Uses `entry.file_type()` (symlink-aware, does NOT follow symlinks) instead of
/// `path.is_dir()` (follows symlinks). A symlink-to-directory via `is_dir()` would
/// recurse infinitely if the symlink points to an ancestor. Symlinks are skipped —
/// anchor operates on .md files and has no use case for preserving symlinks.
fn copy_dir_recursive(
    src: &std::path::Path,
    dst: &std::path::Path,
) -> Result<(), TransactionError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            // Skip symlinks — following them can create infinite recursion cycles.
            continue;
        }
        let src_child = entry.path();
        let dst_child = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_child, &dst_child)?;
        } else {
            std::fs::copy(&src_child, &dst_child)?;
        }
    }
    Ok(())
}

/// Check if a target canonical path "exists" in the post-move filesystem state.
///
/// Returns true if:
/// - The file exists on disk at workspace_root/canonical, OR
/// - The canonical path is plan.dst or is inside plan.dst/ (will exist after commit)
fn target_exists_post_move(
    workspace_root: &Path,
    plan: &RewritePlan,
    canonical: &CanonicalPath,
) -> bool {
    if workspace_root.join(canonical.as_str()).exists() {
        return true;
    }
    // Will exist after commit: plan.dst itself or anything inside it
    canonical == &plan.dst || canonical.starts_with(&format!("{}/", plan.dst))
}

/// VALIDATE phase: resolve all references in rewritten files; any broken → ValidationError.
///
/// Uses post-move filesystem state: plan.dst and its subtree are treated as existing.
/// From 04-TRANSACTIONS.md §VALIDATE Phase Detail.
pub fn validate(
    workspace_root: &Path,
    plan: &RewritePlan,
    op_dir: &TempOpDir,
) -> Result<(), ValidationError> {
    let rewrites_dir = op_dir.path.join("rewrites");
    let Ok(read_dir) = std::fs::read_dir(&rewrites_dir) else {
        // No rewritten files — nothing to validate
        return Ok(());
    };

    let mut broken: Vec<BrokenRef> = Vec::new();

    for entry in read_dir.flatten() {
        let encoded_name = entry.file_name().to_string_lossy().into_owned();
        // Decode canonical path: __ → /
        let original_canonical: CanonicalPath = encoded_name.replace("__", "/");

        // Determine the post-move canonical path for reference resolution.
        // Files inside plan.src will be at plan.dst after commit.
        let resolve_canonical = if inside_src(&original_canonical, &plan.src) {
            remap_path(&original_canonical, &plan.src, &plan.dst)
        } else {
            original_canonical.clone()
        };

        let content = std::fs::read_to_string(entry.path())?;
        let refs = parser::parse_references(&resolve_canonical, &content);

        for line_no in compute_line_numbers(&content, &refs) {
            let (reference, line) = line_no;
            // Only validate Form 1 (relative links) — their new paths must resolve
            if reference.form == RefForm::Wiki
                || reference.form == RefForm::Backtick
                || reference.form == RefForm::HtmlHref
            {
                // Wiki links resolve via stem scan — skip (workspace state too complex to simulate).
                // Backtick refs are directory/file name mentions, not relative paths — skip.
                // HtmlHref paths are rewritten by plan() but not validated post-move (same as Backtick).
                continue;
            }

            let resolved = resolver::resolve_form1(&resolve_canonical, &reference.target_raw);
            if !target_exists_post_move(workspace_root, plan, &resolved) {
                broken.push(BrokenRef {
                    file: resolve_canonical.clone(),
                    line,
                    target: reference.target_raw.clone(),
                });
            }
        }
    }

    if broken.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::BrokenRefs(broken))
    }
}

/// Compute (Reference, line_number) pairs by walking content line-by-line.
///
/// Line numbers are 1-based. Each reference is matched to the line whose byte range
/// contains its span start.
fn compute_line_numbers(
    content: &str,
    refs: &[crate::model::reference::Reference],
) -> Vec<(crate::model::reference::Reference, usize)> {
    let mut result = Vec::new();
    for reference in refs {
        let byte_pos = reference.span.0;
        let line_no = content[..byte_pos].chars().filter(|&c| c == '\n').count() + 1;
        result.push((reference.clone(), line_no));
    }
    result
}

/// ROLLBACK: remove op_dir and drop lock (via LockGuard's Drop impl).
///
/// Called on any failure before COMMIT. Originals are untouched — workspace is
/// byte-for-byte identical to before the operation.
/// From 04-TRANSACTIONS.md §Rollback.
pub fn rollback(op_dir: &TempOpDir, _lock: crate::infra::lock::LockGuard) {
    // Best-effort removal — we are in an error path, ignore errors.
    let _ = temp::cleanup_op_dir(op_dir);
    // _lock is dropped here, triggering LockGuard::drop → deletes .accelmars/anchor/lock
}

/// Move `src` to `dst`. On EXDEV (cross-filesystem), falls back to copy+delete.
///
/// The fallback is non-atomic — a crash between copy and delete leaves a partial state.
/// The caller (COMMIT) logs a warning to stderr before entering the fallback path.
fn move_with_crossfs_fallback(src: &Path, dst: &Path) -> Result<(), CommitError> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            eprintln!("warning: cross-filesystem move: using copy+delete (non-atomic)");
            crossfs_fallback_copy(src, dst)
        }
        Err(e) => Err(CommitError::Io(e)),
    }
}

/// Copy `src` to `dst` then delete `src`. Handles both files and directories.
///
/// Used by move_with_crossfs_fallback when rename(2) returns EXDEV.
fn crossfs_fallback_copy(src: &Path, dst: &Path) -> Result<(), CommitError> {
    if src.is_dir() {
        copy_dir_recursive(src, dst).map_err(|e| match e {
            TransactionError::Io(io) => CommitError::Io(io),
            _ => unreachable!(),
        })?;
        std::fs::remove_dir_all(src)?;
    } else {
        std::fs::copy(src, dst).map(|_| ())?;
        std::fs::remove_file(src)?;
    }
    Ok(())
}

/// COMMIT phase: rename rewrites over originals, then rename moved/src → dst.
///
/// CRITICAL ORDER (HANDOVER.md anti-pattern): rename rewrite files FIRST, then src→dst.
/// If process dies after rewrites but before src rename, old src is still valid.
/// If process dies after src rename but before rewrites, references would be broken.
/// From 04-TRANSACTIONS.md §COMMIT Phase Detail.
pub fn commit(
    workspace_root: &Path,
    plan: &RewritePlan,
    op_dir: &TempOpDir,
    manifest: &mut Manifest,
    _lock: crate::infra::lock::LockGuard,
) -> Result<(), CommitError> {
    // Step 1: update manifest phase → COMMIT
    manifest.phase = "COMMIT".to_string();
    manifest::write_manifest(&op_dir.path, manifest)?;

    // Step 2: rename each rewritten file over its original (FIRST — preserves recovery)
    // manifest.rewrites contains the canonical paths of files that were rewritten
    let rewrites_dir = op_dir.path.join("rewrites");
    for original_canonical in &manifest.rewrites {
        let encoded = temp::encode_path(original_canonical);
        let tmp_path = rewrites_dir.join(&encoded);
        let original_path = workspace_root.join(original_canonical.as_str());
        std::fs::rename(&tmp_path, &original_path)?;
    }

    // Step 3: rename src → dst  (SECOND — after rewrites are committed)
    // Per 04-TRANSACTIONS.md: "rename src → dst" — the actual source path, not from tmp/moved.
    // tmp/moved is a safety copy for recovery; it gets cleaned up in step 4.
    // Crash recovery: if we die here, referencing files are updated but src still exists.
    // manifest.json shows phase=COMMIT, so the stale cleanup path knows what happened.
    let original_src = workspace_root.join(plan.src.as_str());
    let final_dst = workspace_root.join(plan.dst.as_str());
    // Ensure dst parent directory exists
    if let Some(parent) = final_dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    move_with_crossfs_fallback(&original_src, &final_dst)?;

    // Step 4: remove op_dir
    let _ = temp::cleanup_op_dir(op_dir);

    // Step 5: _lock is dropped here → LockGuard::drop deletes .accelmars/anchor/lock

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Set up a temp workspace and return its root path.
    fn make_workspace() -> TempDir {
        TempDir::new().unwrap()
    }

    /// Write a file at `root/canonical_path` with the given content, creating parent dirs.
    fn write_file(root: &Path, canonical: &str, content: &str) {
        let path = root.join(canonical);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
    }

    /// Test 6: External file references file inside src/ → classified as Case A.
    ///         A RewriteEntry must be generated for the external file.
    #[test]
    fn test_case_a_external_references_inside_src() {
        let tmp = make_workspace();
        let root = tmp.path();

        // src directory being moved
        let src = "projects/foo".to_string();
        let dst = "projects/archive/foo".to_string();

        // File INSIDE src
        write_file(root, "projects/foo/bar.md", "# Bar\n");

        // File OUTSIDE src that references something inside src (Case A)
        // From "docs/README.md", the path to "projects/foo/bar.md" is "../projects/foo/bar.md"
        write_file(
            root,
            "docs/README.md",
            "See [bar](../projects/foo/bar.md).\n",
        );

        let workspace_files = vec![
            "projects/foo/bar.md".to_string(),
            "docs/README.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        // Must have exactly one RewriteEntry for docs/README.md (Case A)
        assert_eq!(plan.entries.len(), 1, "exactly one Case A entry expected");
        let entry = &plan.entries[0];
        assert_eq!(
            entry.file, "docs/README.md",
            "entry must be in the external file"
        );
        assert!(
            entry.old_text.contains("projects/foo/bar.md"),
            "old_text must contain old path, got: {}",
            entry.old_text
        );
        assert!(
            entry.new_text.contains("projects/archive/foo/bar.md"),
            "new_text must contain new path after remap, got: {}",
            entry.new_text
        );
    }

    /// Test 7: File inside src/ references external file → classified as Case B.
    ///         A RewriteEntry must be generated for the inside-src file.
    #[test]
    fn test_case_b_inside_src_references_external() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "projects/foo".to_string();
        let dst = "projects/archive/foo".to_string();

        // External file (target of the reference)
        write_file(root, "docs/guide.md", "# Guide\n");

        // File INSIDE src that references the external file (Case B)
        // From "projects/foo/notes.md", path to "docs/guide.md" is "../../docs/guide.md"
        write_file(
            root,
            "projects/foo/notes.md",
            "See [guide](../../docs/guide.md).\n",
        );

        let workspace_files = vec![
            "docs/guide.md".to_string(),
            "projects/foo/notes.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        // Must have exactly one RewriteEntry for projects/foo/notes.md (Case B)
        assert_eq!(plan.entries.len(), 1, "exactly one Case B entry expected");
        let entry = &plan.entries[0];
        assert_eq!(
            entry.file, "projects/foo/notes.md",
            "entry must be in the inside-src file"
        );
        assert!(
            entry.old_text.contains("../../docs/guide.md"),
            "old_text must contain old relative path, got: {}",
            entry.old_text
        );
        // After move: "projects/archive/foo/notes.md" → "../../../docs/guide.md"
        assert!(
            entry.new_text.contains("docs/guide.md"),
            "new_text must reference the external target, got: {}",
            entry.new_text
        );
    }

    /// Test 8: File inside src/ references another file inside src/ → Case C.
    ///         NO RewriteEntry must appear for this reference (relative path stable).
    #[test]
    fn test_case_c_both_inside_src_skipped() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "projects/foo".to_string();
        let dst = "projects/archive/foo".to_string();

        // Both files are inside src/
        write_file(root, "projects/foo/a.md", "# A\n");
        // File b.md references a.md — both inside src/ → Case C
        write_file(root, "projects/foo/b.md", "See [a](a.md).\n");

        let workspace_files = vec![
            "projects/foo/a.md".to_string(),
            "projects/foo/b.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        // Case C: relative path between a.md and b.md is stable — NO entries
        assert!(
            plan.entries.is_empty(),
            "Case C references must not produce RewriteEntry, got: {:?}",
            plan.entries
        );
    }

    /// Verify inside_src correctly handles file move (exact match) and directory move (prefix).
    #[test]
    fn test_inside_src() {
        let src = "projects/foo".to_string();

        // Exact match — file move case
        assert!(inside_src(&"projects/foo".to_string(), &src));
        // File inside directory
        assert!(inside_src(&"projects/foo/bar.md".to_string(), &src));
        // Nested directory inside src
        assert!(inside_src(&"projects/foo/sub/baz.md".to_string(), &src));
        // Same prefix but different name — must NOT match (projects/foobar != projects/foo/)
        assert!(!inside_src(&"projects/foobar.md".to_string(), &src));
        // Completely different path
        assert!(!inside_src(&"docs/guide.md".to_string(), &src));
    }

    /// Verify compute_relative_path produces correct relative paths.
    #[test]
    fn test_compute_relative_path() {
        // Same directory
        assert_eq!(
            compute_relative_path(&"a/b/source.md".to_string(), &"a/b/target.md".to_string()),
            "target.md"
        );
        // One level up
        assert_eq!(
            compute_relative_path(&"a/b/source.md".to_string(), &"a/target.md".to_string()),
            "../target.md"
        );
        // Two levels up, different branch
        assert_eq!(
            compute_relative_path(
                &"projects/foo/source.md".to_string(),
                &"docs/guide.md".to_string()
            ),
            "../../docs/guide.md"
        );
        // Source at root level
        assert_eq!(
            compute_relative_path(&"ROOT.md".to_string(), &"projects/foo/bar.md".to_string()),
            "projects/foo/bar.md"
        );
    }

    /// Regression test: copy_dir_recursive must terminate and skip symlinks.
    /// Incident: anchor v0.2.0 copy_dir_recursive used is_dir() which follows symlinks.
    /// A symlink pointing to an ancestor caused infinite recursion until SIGKILL.
    /// Fixed in v0.2.1: entry.file_type().is_dir() (does not follow symlinks) + symlink skip.
    /// FS-004 in accelmars-codex.
    #[test]
    #[cfg(unix)]
    fn test_copy_dir_recursive_skips_symlinks() {
        use std::os::unix::fs::symlink;
        use tempfile::TempDir;

        let src_tmp = TempDir::new().unwrap();
        let src = src_tmp.path();

        // Create a regular file
        std::fs::write(src.join("file.md"), b"content").unwrap();

        // Create a symlink inside src/ that points back to src/ (cycle)
        symlink(src, src.join("self_link")).unwrap();

        let dst_tmp = TempDir::new().unwrap();
        let dst = dst_tmp.path().join("output");

        // Must terminate — not hang
        copy_dir_recursive(src, &dst).unwrap();

        // file.md must be copied; symlink must be skipped (no self_link/ in dst)
        assert!(dst.join("file.md").exists(), "regular file must be copied");
        assert!(!dst.join("self_link").exists(), "symlink must be skipped");
    }

    /// Test crossfs_fallback_copy for a single file: src is removed, dst has the content.
    ///
    /// This tests the copy+delete fallback logic directly. The EXDEV trigger itself
    /// (errno 18 from rename(2)) requires two distinct filesystems mounted at different
    /// paths — not reproducible in CI without system-level setup. The detection branch
    /// in move_with_crossfs_fallback is exercised by this path when EXDEV is raised;
    /// the logical correctness of the fallback is verified here on a same-FS basis.
    #[test]
    fn test_crossfs_fallback_copy_file() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("source.md");
        let dst = tmp.path().join("dest.md");
        fs::write(&src, b"content").unwrap();

        crossfs_fallback_copy(&src, &dst).unwrap();

        assert!(!src.exists(), "src must be removed after fallback copy");
        assert!(dst.exists(), "dst must exist after fallback copy");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "content");
    }

    /// Backtick Case A: external .md file with `src-dir/` in backtick span → RewriteEntry generated.
    #[test]
    fn test_backtick_case_a_produces_rewrite_entry() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "gateway-foundation".to_string();
        let dst = "foundations/gateway-engine".to_string();

        // File inside src (being moved)
        write_file(root, "gateway-foundation/README.md", "# GW\n");

        // External file with backtick mention of the moved directory
        write_file(
            root,
            "CONSTELLATION.md",
            "The `gateway-foundation/` directory holds all gateway logic.\n",
        );

        let workspace_files = vec![
            "gateway-foundation/README.md".to_string(),
            "CONSTELLATION.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt_entries: Vec<_> = plan
            .entries
            .iter()
            .filter(|e| e.old_text.contains("`gateway-foundation/`"))
            .collect();
        assert_eq!(
            bt_entries.len(),
            1,
            "expected 1 backtick rewrite entry, got: {:?}",
            plan.entries
        );
        assert_eq!(bt_entries[0].file, "CONSTELLATION.md");
        assert_eq!(bt_entries[0].old_text, "`gateway-foundation/`");
        assert_eq!(bt_entries[0].new_text, "`foundations/gateway-engine/`");
    }

    /// Backtick ref: file inside src with backtick ref to something inside src → Case C, no entry.
    #[test]
    fn test_backtick_inside_src_no_entry() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "projects/foo".to_string();
        let dst = "archive/foo".to_string();

        // Both files inside src
        write_file(root, "projects/foo/a.md", "# A\n");
        write_file(root, "projects/foo/b.md", "See `projects/foo/` for more.\n");

        let workspace_files = vec![
            "projects/foo/a.md".to_string(),
            "projects/foo/b.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        // Case C: source and target both inside src → no backtick rewrite entry
        let bt_entries: Vec<_> = plan
            .entries
            .iter()
            .filter(|e| e.old_text.starts_with('`'))
            .collect();
        assert!(
            bt_entries.is_empty(),
            "Case C backtick: no entry expected, got: {:?}",
            bt_entries
        );
    }

    /// Backtick ref: path unrelated to the move → no RewriteEntry generated.
    #[test]
    fn test_backtick_non_src_path_no_entry() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "projects/foo".to_string();
        let dst = "archive/foo".to_string();

        write_file(root, "projects/foo/note.md", "# Note\n");
        // External file with backtick ref to something UNRELATED to the move
        write_file(
            root,
            "docs/overview.md",
            "See `other-project/` for details.\n",
        );

        let workspace_files = vec![
            "projects/foo/note.md".to_string(),
            "docs/overview.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt_entries: Vec<_> = plan
            .entries
            .iter()
            .filter(|e| e.old_text.starts_with('`'))
            .collect();
        assert!(
            bt_entries.is_empty(),
            "unrelated backtick path: no entry expected, got: {:?}",
            bt_entries
        );
    }

    /// Test crossfs_fallback_copy for a directory: src tree is removed, dst tree has the files.
    #[test]
    fn test_crossfs_fallback_copy_dir() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src_dir");
        let dst_dir = tmp.path().join("dst_dir");
        fs::create_dir(&src_dir).unwrap();
        fs::write(src_dir.join("a.md"), b"hello").unwrap();
        fs::create_dir(src_dir.join("sub")).unwrap();
        fs::write(src_dir.join("sub").join("b.md"), b"world").unwrap();

        crossfs_fallback_copy(&src_dir, &dst_dir).unwrap();

        assert!(!src_dir.exists(), "src dir must be removed after fallback");
        assert!(
            dst_dir.join("a.md").exists(),
            "top-level file must be copied"
        );
        assert!(
            dst_dir.join("sub").join("b.md").exists(),
            "nested file must be copied"
        );
    }

    /// REF-001: link text that is an exact copy of target_raw is updated to new_rel_path.
    /// Before: `[gateway-foundation/README.md](gateway-foundation/README.md)`
    /// After:  `[foundations/gateway-engine/README.md](foundations/gateway-engine/README.md)`
    #[test]
    fn test_link_text_sync_when_text_equals_path() {
        let old_text = "[gateway-foundation/README.md](gateway-foundation/README.md)";
        let new_rel = "foundations/gateway-engine/README.md";
        let old_target_raw = "gateway-foundation/README.md";
        let result = rebuild_form1_ref(old_text, new_rel, &None, old_target_raw);
        assert_eq!(
            result, "[foundations/gateway-engine/README.md](foundations/gateway-engine/README.md)",
            "link text matching target_raw must be updated"
        );
    }

    /// REF-001: link text that differs from target_raw is preserved unchanged.
    /// Before: `[Gateway Foundation](gateway-foundation/README.md)`
    /// After:  `[Gateway Foundation](foundations/gateway-engine/README.md)`
    #[test]
    fn test_link_text_preserved_when_text_differs() {
        let old_text = "[Gateway Foundation](gateway-foundation/README.md)";
        let new_rel = "foundations/gateway-engine/README.md";
        let old_target_raw = "gateway-foundation/README.md";
        let result = rebuild_form1_ref(old_text, new_rel, &None, old_target_raw);
        assert_eq!(
            result, "[Gateway Foundation](foundations/gateway-engine/README.md)",
            "link text that differs from target_raw must be preserved"
        );
    }

    /// SIM-F: external .md file with `<a href="...">` pointing into src → RewriteEntry generated (Case A).
    #[test]
    fn test_href_case_a_produces_rewrite_entry() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "docs-foundation".to_string();
        let dst = "foundations/docs-engine".to_string();

        // File inside src being moved
        write_file(root, "docs-foundation/guide.md", "# Guide\n");

        // External file with html href pointing into src
        write_file(
            root,
            "README.md",
            r#"See <a href="docs-foundation/guide.md">Guide</a>."#,
        );

        let workspace_files = vec![
            "docs-foundation/guide.md".to_string(),
            "README.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let href_entries: Vec<_> = plan
            .entries
            .iter()
            .filter(|e| e.old_text.starts_with("href="))
            .collect();
        assert_eq!(href_entries.len(), 1, "expected 1 HtmlHref rewrite entry");
        assert_eq!(href_entries[0].file, "README.md");
        assert!(
            href_entries[0]
                .old_text
                .contains("docs-foundation/guide.md"),
            "old_text must contain old path, got: {}",
            href_entries[0].old_text
        );
        assert!(
            href_entries[0]
                .new_text
                .contains("foundations/docs-engine/guide.md"),
            "new_text must contain new path, got: {}",
            href_entries[0].new_text
        );
    }

    /// Gap 1: rewrite_partial_backtick unit test — verifies matching and no-op filtering.
    #[test]
    fn test_rewrite_partial_backtick_unit() {
        let src = "accelmars-guild/projects/os-council";
        let dst = "accelmars-guild/councils/os-council";

        // Directory partial match (n=1: omit "accelmars-guild/")
        assert_eq!(
            rewrite_partial_backtick("projects/os-council", src, dst),
            Some("councils/os-council".to_string()),
            "directory partial match must rewrite suffix"
        );

        // File under directory partial match
        assert_eq!(
            rewrite_partial_backtick("projects/os-council/decisions/foo.md", src, dst),
            Some("councils/os-council/decisions/foo.md".to_string()),
            "file-under-dir partial match must rewrite suffix"
        );

        // Unrelated path — no match
        assert_eq!(
            rewrite_partial_backtick("projects/other-dir/foo.md", src, dst),
            None,
            "unrelated path must not match"
        );

        // No-op (stem unchanged across move) — filtered out
        let src2 = "a/b/same";
        let dst2 = "x/y/same";
        assert_eq!(
            rewrite_partial_backtick("same", src2, dst2),
            None,
            "no-op rewrite (new == old) must return None"
        );
    }

    /// Gap 1 — partial-path directory ref: external file with `projects/os-council/`
    /// (guild-relative partial path) is matched and rewritten during the move of
    /// `accelmars-guild/projects/os-council` → `accelmars-guild/councils/os-council`.
    #[test]
    fn test_partial_backtick_directory_ref_matched() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(root, "accelmars-guild/projects/os-council/README.md", "# OS Council\n");
        write_file(
            root,
            "accelmars-guild/projects/accelmars-gtm/proposals/MKT-039.md",
            "See `projects/os-council/` for decisions.\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/README.md".to_string(),
            "accelmars-guild/projects/accelmars-gtm/proposals/MKT-039.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert_eq!(bt.len(), 1, "expected 1 partial-path backtick entry, got: {:?}", plan.entries);
        assert_eq!(bt[0].old_text, "`projects/os-council/`");
        assert_eq!(bt[0].new_text, "`councils/os-council/`");
    }

    /// Gap 1 — partial-path file ref: `projects/os-council/decisions/foo.md` is matched and
    /// rewritten to `councils/os-council/decisions/foo.md`.
    #[test]
    fn test_partial_backtick_file_ref_matched() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(
            root,
            "accelmars-guild/projects/os-council/decisions/foo.md",
            "# Decision\n",
        );
        write_file(
            root,
            "accelmars-guild/projects/accelmars-gtm/STATUS.md",
            "See `projects/os-council/decisions/foo.md` for context.\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/decisions/foo.md".to_string(),
            "accelmars-guild/projects/accelmars-gtm/STATUS.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert_eq!(bt.len(), 1, "expected 1 partial-path file ref entry, got: {:?}", plan.entries);
        assert_eq!(bt[0].old_text, "`projects/os-council/decisions/foo.md`");
        assert_eq!(bt[0].new_text, "`councils/os-council/decisions/foo.md`");
    }

    /// Gap 1 — unrelated partial path: `projects/other-dir/foo.md` must NOT be matched when
    /// moving `accelmars-guild/projects/os-council`.
    #[test]
    fn test_partial_backtick_unrelated_path_not_matched() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(root, "accelmars-guild/projects/os-council/README.md", "# OS Council\n");
        write_file(
            root,
            "docs/overview.md",
            "See `projects/other-dir/foo.md` for more.\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/README.md".to_string(),
            "docs/overview.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert!(
            bt.is_empty(),
            "unrelated partial path must not produce entry, got: {:?}",
            bt
        );
    }

    /// Gap 2: external file with `$(anchor root)/src-dir/` backtick ref → matched and rewritten with prefix preserved.
    #[test]
    fn test_anchor_root_prefix_external_dir_ref() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(root, "accelmars-guild/projects/os-council/README.md", "# OS Council\n");
        write_file(
            root,
            "accelmars-guild/projects/accelmars-gtm/proposals/MKT-144.md",
            "See `$(anchor root)/accelmars-guild/projects/os-council/` for decisions.\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/README.md".to_string(),
            "accelmars-guild/projects/accelmars-gtm/proposals/MKT-144.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert_eq!(bt.len(), 1, "expected 1 $(anchor root)/ dir ref entry, got: {:?}", plan.entries);
        assert_eq!(
            bt[0].old_text,
            "`$(anchor root)/accelmars-guild/projects/os-council/`"
        );
        assert_eq!(
            bt[0].new_text,
            "`$(anchor root)/accelmars-guild/councils/os-council/`"
        );
    }

    /// Gap 2: external file with `$(anchor root)/src-dir/sub/file.md` backtick ref → rewritten, prefix preserved.
    #[test]
    fn test_anchor_root_prefix_external_file_ref() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(
            root,
            "accelmars-guild/projects/os-council/decisions/foo.md",
            "# Decision\n",
        );
        write_file(
            root,
            "accelmars-guild/contracts/SKW-002.md",
            "start_dir: `$(anchor root)/accelmars-guild/projects/os-council/decisions/foo.md`\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/decisions/foo.md".to_string(),
            "accelmars-guild/contracts/SKW-002.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert_eq!(bt.len(), 1, "expected 1 $(anchor root)/ file ref entry, got: {:?}", plan.entries);
        assert_eq!(
            bt[0].old_text,
            "`$(anchor root)/accelmars-guild/projects/os-council/decisions/foo.md`"
        );
        assert_eq!(
            bt[0].new_text,
            "`$(anchor root)/accelmars-guild/councils/os-council/decisions/foo.md`"
        );
    }

    /// Gap 5: file INSIDE moved dir with `$(anchor root)/` self-ref → rewritten (Case C inverted for absolute refs).
    #[test]
    fn test_anchor_root_prefix_internal_self_ref() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/skill-council".to_string();
        let dst = "accelmars-guild/councils/skill-council".to_string();

        write_file(
            root,
            "accelmars-guild/projects/skill-council/contracts/SKW-002.md",
            "start_dir: `$(anchor root)/accelmars-guild/projects/skill-council/proposals`\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/skill-council/contracts/SKW-002.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert_eq!(
            bt.len(),
            1,
            "expected 1 internal self-ref entry (Gap 5), got: {:?}",
            plan.entries
        );
        assert_eq!(
            bt[0].old_text,
            "`$(anchor root)/accelmars-guild/projects/skill-council/proposals`"
        );
        assert_eq!(
            bt[0].new_text,
            "`$(anchor root)/accelmars-guild/councils/skill-council/proposals`"
        );
    }

    /// Gap 3: external file with `../os-council/decisions/foo.md` in backtick → matched and
    /// rewritten to a depth-adjusted relative path after the os-council dir moves.
    #[test]
    fn test_relative_backtick_external_file_matched_and_rewritten() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(
            root,
            "accelmars-guild/projects/os-council/decisions/foo.md",
            "# Decision\n",
        );
        // External file (sibling of os-council under projects/) with relative backtick ref.
        // From accelmars-guild/projects/accelmars-gtm/CLAUDE.md, `../os-council/decisions/foo.md`
        // resolves to accelmars-guild/projects/os-council/decisions/foo.md (inside src).
        write_file(
            root,
            "accelmars-guild/projects/accelmars-gtm/CLAUDE.md",
            "Path: `../os-council/decisions/foo.md`\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/decisions/foo.md".to_string(),
            "accelmars-guild/projects/accelmars-gtm/CLAUDE.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert_eq!(bt.len(), 1, "expected 1 relative backtick entry (Gap 3), got: {:?}", plan.entries);
        assert_eq!(bt[0].file, "accelmars-guild/projects/accelmars-gtm/CLAUDE.md");
        assert_eq!(bt[0].old_text, "`../os-council/decisions/foo.md`");
        // After move: source stays at accelmars-guild/projects/accelmars-gtm/CLAUDE.md,
        // target moves to accelmars-guild/councils/os-council/decisions/foo.md.
        // New relative: ../../councils/os-council/decisions/foo.md
        assert_eq!(bt[0].new_text, "`../../councils/os-council/decisions/foo.md`");
    }

    /// Gap 3: relative backtick dir ref `../os-council/` → matched and rewritten with trailing slash preserved.
    #[test]
    fn test_relative_backtick_dir_ref_trailing_slash_preserved() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(root, "accelmars-guild/projects/os-council/README.md", "# OS Council\n");
        write_file(
            root,
            "accelmars-guild/projects/accelmars-gtm/STATUS.md",
            "Council dir: `../os-council/`\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/README.md".to_string(),
            "accelmars-guild/projects/accelmars-gtm/STATUS.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert_eq!(bt.len(), 1, "expected 1 relative dir ref entry (Gap 3), got: {:?}", plan.entries);
        assert_eq!(bt[0].old_text, "`../os-council/`");
        assert_eq!(bt[0].new_text, "`../../councils/os-council/`");
    }

    /// Gap 3 negative: relative backtick ref that resolves to a path NOT inside src → no entry.
    #[test]
    fn test_relative_backtick_resolves_outside_src_no_entry() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(root, "accelmars-guild/projects/os-council/README.md", "# OS\n");
        write_file(
            root,
            "accelmars-guild/projects/accelmars-gtm/CLAUDE.md",
            "Other: `../other-project/docs.md`\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/README.md".to_string(),
            "accelmars-guild/projects/accelmars-gtm/CLAUDE.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert!(
            bt.is_empty(),
            "relative backtick resolving outside src must not produce entry, got: {:?}",
            bt
        );
    }

    /// Gap 2 negative: unrelated `$(anchor root)/` path must not match when moving a different src.
    #[test]
    fn test_anchor_root_prefix_unrelated_path_not_matched() {
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "accelmars-guild/projects/os-council".to_string();
        let dst = "accelmars-guild/councils/os-council".to_string();

        write_file(root, "accelmars-guild/projects/os-council/README.md", "# OS Council\n");
        write_file(
            root,
            "docs/overview.md",
            "See `$(anchor root)/accelmars-guild/projects/other-project/` for details.\n",
        );

        let workspace_files = vec![
            "accelmars-guild/projects/os-council/README.md".to_string(),
            "docs/overview.md".to_string(),
        ];

        let plan = plan(root, &src, &dst, &workspace_files).unwrap();

        let bt: Vec<_> = plan.entries.iter().filter(|e| e.old_text.starts_with('`')).collect();
        assert!(
            bt.is_empty(),
            "unrelated $(anchor root)/ path must not produce entry, got: {:?}",
            bt
        );
    }

    /// SIM-F: validate() skips HtmlHref refs — they are handled by plan() and not validated post-move.
    #[test]
    fn test_href_validate_skip() {
        use crate::infra::temp::TempOpDir;
        use crate::model::rewrite::RewritePlan;

        // Build a minimal RewritePlan and op_dir with a rewritten file containing an HtmlHref.
        // The HtmlHref target does not exist — but validate() must not report it as broken.
        let tmp = make_workspace();
        let root = tmp.path();

        let src = "old-dir".to_string();
        let dst = "new-dir".to_string();

        // Write a rewritten file that contains an HTML href (already rewritten, correct path)
        write_file(root, "README.md", r#"<a href="new-dir/guide.md">Guide</a>"#);

        // Create op_dir structure manually
        let op_path = tmp.path().join(".accelmars").join("anchor").join("op");
        std::fs::create_dir_all(op_path.join("rewrites")).unwrap();

        // Write the "rewritten" file into op_dir/rewrites/ using encoded path
        let encoded = "README.md"; // no slashes — simple encoding
        std::fs::write(
            op_path.join("rewrites").join(encoded),
            r#"<a href="new-dir/guide.md">Guide</a>"#,
        )
        .unwrap();

        let op_dir = TempOpDir { path: op_path };
        let plan = RewritePlan {
            src,
            dst,
            entries: vec![],
        };

        // validate() must not return an error despite the href target not existing on disk
        let result = validate(root, &plan, &op_dir);
        assert!(
            result.is_ok(),
            "validate must skip HtmlHref refs, got: {:?}",
            result.err()
        );
    }
}
