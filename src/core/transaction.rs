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
/// Preserves the original link text and anchor (if any); replaces only the path.
///
/// Input:  `old_text` = `"[link text](old/path.md#anchor)"`, new_rel_path = `"new/path.md"`
/// Output: `"[link text](new/path.md#anchor)"`
fn rebuild_form1_ref(old_text: &str, new_rel_path: &str, anchor: &Option<String>) -> String {
    // Find the ]( boundary separating link text from path
    let bracket_paren = old_text
        .find("](")
        .expect("Form 1 reference must contain ](");
    let link_text = &old_text[1..bracket_paren]; // between [ and ](

    let path_with_anchor = match anchor {
        Some(a) => format!("{new_rel_path}#{a}"),
        None => new_rel_path.to_string(),
    };

    format!("[{link_text}]({path_with_anchor})")
}

/// Compute the stem (filename without `.md` extension) of a canonical path.
fn stem_of(canonical: &CanonicalPath) -> &str {
    let filename = canonical.rsplit('/').next().unwrap_or(canonical.as_str());
    filename.strip_suffix(".md").unwrap_or(filename)
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
                    let new_text = rebuild_form1_ref(&old_text, &new_rel, &reference.anchor);
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
                    let new_text = rebuild_form1_ref(&old_text, &new_rel, &reference.anchor);
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
fn copy_dir_recursive(
    src: &std::path::Path,
    dst: &std::path::Path,
) -> Result<(), TransactionError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_child = entry.path();
        let dst_child = dst.join(entry.file_name());
        if src_child.is_dir() {
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
            if reference.form == RefForm::Wiki {
                // Wiki links resolve via stem scan of the workspace; skip in VALIDATE
                // (the workspace state during VALIDATE is complex to simulate accurately)
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
    // _lock is dropped here, triggering LockGuard::drop → deletes .mind/lock
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
    std::fs::rename(&original_src, &final_dst)?;

    // Step 4: remove op_dir
    let _ = temp::cleanup_op_dir(op_dir);

    // Step 5: _lock is dropped here → LockGuard::drop deletes .mind/lock

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
}
