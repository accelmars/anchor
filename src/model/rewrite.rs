// src/model/rewrite.rs — RewriteEntry, RewritePlan, ProseSkip structs (MF-005, AENG-010)
#![allow(dead_code)]
//
// RewriteEntry represents a single span-level reference rewrite in one file.
// RewritePlan collects all rewrites needed for a single move operation.
// ProseSkip records a backtick ref classified as a prose mention — not rewritten.

use crate::model::CanonicalPath;

/// A single reference rewrite: the span in `file` to replace `old_text` with `new_text`.
///
/// Produced by the PLAN phase (core::transaction::plan) for every Case A and Case B
/// reference. Applied during the APPLY phase (MF-006, core::rewriter).
#[derive(Debug, Clone, PartialEq)]
pub struct RewriteEntry {
    /// Workspace-root-relative path of the file containing the reference.
    pub file: CanonicalPath,
    /// Byte offsets (start, end) of the full reference text in the original file content.
    /// The range `content[span.0..span.1]` equals `old_text`.
    pub span: (usize, usize),
    /// Original reference text at the span (e.g. `"[link](../projects/foo/bar.md)"`).
    pub old_text: String,
    /// Replacement reference text (e.g. `"[link](../projects/archive/foo/bar.md)"`).
    pub new_text: String,
}

/// A backtick reference classified as a prose mention by the AENG-010 heuristic.
///
/// These references are NOT rewritten during `anchor file mv`. They are reported as
/// `[prose?]` entries in `anchor diff --verbose` so the operator can review them.
/// Use `--allow-prose-rewrites` to revert to the pre-AENG-010 behavior.
#[derive(Debug, Clone)]
pub struct ProseSkip {
    /// Workspace-root-relative path of the file containing the prose mention.
    pub file: CanonicalPath,
    /// Byte offsets (start, end) of the backtick expression in the source content.
    pub span: (usize, usize),
    /// 1-based line number of the prose mention in the source file.
    pub line: usize,
    /// The backtick expression as written (e.g. `` `old-path/` ``).
    pub old_text: String,
}

/// The complete rewrite plan for one `anchor file mv` operation.
///
/// Contains all RewriteEntry values for Case A and Case B references.
/// Case C references are excluded — their relative paths are stable after the move.
/// Backtick refs classified as prose mentions are in `prose_skips` — not rewritten.
#[derive(Debug, Clone)]
pub struct RewritePlan {
    /// Canonical source path being moved.
    pub src: CanonicalPath,
    /// Canonical destination path.
    pub dst: CanonicalPath,
    /// All span rewrites to apply, across all affected files.
    pub entries: Vec<RewriteEntry>,
    /// Backtick refs excluded from rewriting due to prose-mention classification (AENG-010).
    pub prose_skips: Vec<ProseSkip>,
}
