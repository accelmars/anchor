#![allow(dead_code)]

use crate::model::CanonicalPath;

/// How a reference is written in Markdown source.
#[derive(Debug, Clone, PartialEq)]
pub enum RefForm {
    /// Standard Markdown link: `[text](path.md)`
    Standard,
    /// Wiki link: `[[path]]` or `[[path|alias]]`
    Wiki,
}

/// A parsed reference from a Markdown file.
#[derive(Debug, Clone)]
pub struct Reference {
    /// Workspace-root-relative path of the file containing this reference.
    pub source_file: CanonicalPath,
    /// Raw path as written in the file, before resolution.
    /// For Form 1: the path part only (no `#anchor`).
    /// For Form 2: the stem (no `[[`, no `]]`, no `.md` extension).
    pub target_raw: String,
    /// Byte offsets (start, end) of the full reference in `source_file` content.
    pub span: (usize, usize),
    /// Form of this reference.
    pub form: RefForm,
    /// Anchor fragment for Form 1 references (e.g. `"section-heading"` from `path.md#section-heading`).
    /// Always `None` for Form 2.
    pub anchor: Option<String>,
}
