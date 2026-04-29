// src/core/context_scope.rs — context-scoped rewrite domain resolution (AENG-001)
//
// A move op's rewrite scope is bounded to the deepest domain that contains its source path.
// This prevents bare-name substring matches from crossing repo boundaries — e.g., moving
// `org-workspace/foundations/engine/workflows` must not rewrite `workflows/`
// mentions in unrelated repos like `org-sibling-repo/`.
//
// Domain hierarchy:
//   1. Workspace root (fallback when no repo root contains the source)
//   2. Repo roots — directories at depth 1 under workspace_root that contain `.git`
//   3. Workspace-defined sub-boundaries (future: from WorkspaceConfig.scopes)
//
// The "inward ref" rule: out-of-scope files may still hold workspace-relative paths that
// directly address the moved location (e.g. `$(anchor root)/org-workspace/.../workflows`).
// These are rewritten regardless of scope. Short suffix matches (e.g. bare `workflows`) from
// out-of-scope files are the false-positive class this module prevents.

use crate::model::CanonicalPath;
use std::path::Path;

/// A rewrite domain — one level in the scope hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RewriteDomain {
    /// Entire workspace (fallback when no repo root contains the source path).
    Workspace,
    /// A git repository root at depth 1 under the workspace root.
    Repo(CanonicalPath),
    /// A workspace-defined sub-boundary (future use; from config).
    #[allow(dead_code)]
    Defined(String),
}

/// The computed scope for a single move op.
#[derive(Debug, Clone)]
pub(crate) struct Scope {
    pub domain: RewriteDomain,
    /// Workspace-relative canonical root of this scope. Empty string for Workspace scope.
    pub root: CanonicalPath,
}

/// Caches repo-root topology for one `plan()` call.
///
/// Created once per `plan()` invocation (before the `workspace_files` loop) so that
/// `scope_for_move` and `is_in_scope` don't re-scan the filesystem per reference.
pub(crate) struct ScopeResolver {
    repo_roots: Vec<CanonicalPath>,
}

impl ScopeResolver {
    /// Build a resolver by scanning depth-1 subdirectories of `workspace_root` for `.git`.
    pub(crate) fn new(workspace_root: &Path) -> Self {
        Self {
            repo_roots: discover_repo_roots(workspace_root),
        }
    }
}

/// Scan `workspace_root` for depth-1 children that contain a `.git` entry.
/// Returns sorted workspace-relative canonical names (e.g. `["org-sibling-repo", "anchor", ...]`).
fn discover_repo_roots(workspace_root: &Path) -> Vec<CanonicalPath> {
    let mut roots = Vec::new();
    let Ok(entries) = std::fs::read_dir(workspace_root) else {
        return roots;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        if entry.path().join(".git").exists() {
            if let Some(name) = entry.file_name().to_str() {
                roots.push(name.to_string());
            }
        }
    }
    roots.sort();
    roots
}

/// Return the deepest-domain scope for a move op whose source is at `src`.
///
/// Walks up the components of `src` and returns the first matching repo root.
/// Falls back to `Workspace` if no repo root contains `src`.
pub(crate) fn scope_for_move(resolver: &ScopeResolver, src: &CanonicalPath) -> Scope {
    let parts: Vec<&str> = src.split('/').collect();
    // Walk from most-specific ancestor up to the top-level component.
    for depth in (1..=parts.len()).rev() {
        let candidate = parts[..depth].join("/");
        if resolver.repo_roots.contains(&candidate) {
            return Scope {
                domain: RewriteDomain::Repo(candidate.clone()),
                root: candidate,
            };
        }
    }
    Scope {
        domain: RewriteDomain::Workspace,
        root: String::new(),
    }
}

/// Returns true if `file_canonical` is within the given scope.
///
/// Workspace scope covers everything. Repo/Defined scope is satisfied by files whose
/// canonical path equals or is prefixed by `scope.root/`.
pub(crate) fn is_in_scope(file_canonical: &CanonicalPath, scope: &Scope) -> bool {
    match scope.domain {
        RewriteDomain::Workspace => true,
        _ => {
            file_canonical == &scope.root || file_canonical.starts_with(&format!("{}/", scope.root))
        }
    }
}

/// Returns true if `target_raw` is a workspace-relative path that has `src` as a prefix
/// component — meaning it genuinely points at or into the moved source.
///
/// This is the "inward ref" rule: a file outside the scope may hold a fully-qualified
/// workspace-relative reference to the moved location (e.g. via `$(anchor root)/…/src/`)
/// and that reference must be rewritten. Short suffix matches (bare `workflows`) do NOT
/// qualify as inward refs.
///
/// `target_raw` should be the normalized canonical form (no `$(anchor root)/` prefix,
/// no trailing slash) — the same value used for matching in `plan()`.
pub(crate) fn is_inward_ref(target_raw: &str, src: &CanonicalPath) -> bool {
    let normalized = target_raw.trim_end_matches('/');
    normalized == src.as_str() || normalized.starts_with(&format!("{}/", src.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace_with_repos(repo_names: &[&str]) -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".accelmars").join("anchor")).unwrap();
        for name in repo_names {
            let repo = root.join(name);
            fs::create_dir_all(&repo).unwrap();
            fs::create_dir_all(repo.join(".git")).unwrap();
        }
        dir
    }

    // ── scope_for_move ────────────────────────────────────────────────────────

    /// Cross-repo scenario: src inside org-workspace, sibling repo is a peer.
    /// scope_for_move should return Repo("org-workspace").
    #[test]
    fn test_scope_for_move_repo_root() {
        let ws = make_workspace_with_repos(&["org-workspace", "org-sibling-repo"]);
        let resolver = ScopeResolver::new(ws.path());

        let src = "org-workspace/foundations/engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(scope.root, "org-workspace");
        assert_eq!(
            scope.domain,
            RewriteDomain::Repo("org-workspace".to_string())
        );
    }

    /// src is directly a repo root (e.g. moving the entire repo — unusual but valid).
    /// scope_for_move should find it at depth 1 and return Repo(src).
    #[test]
    fn test_scope_for_move_src_is_repo_root() {
        let ws = make_workspace_with_repos(&["org-sibling-repo"]);
        let resolver = ScopeResolver::new(ws.path());

        let src = "org-sibling-repo".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(scope.root, "org-sibling-repo");
        assert_eq!(
            scope.domain,
            RewriteDomain::Repo("org-sibling-repo".to_string())
        );
    }

    /// Workspace fallback: no repo root contains src.
    /// scope_for_move should return Workspace scope.
    #[test]
    fn test_scope_for_move_workspace_fallback() {
        let ws = make_workspace_with_repos(&["repo-a"]);
        let resolver = ScopeResolver::new(ws.path());

        // src is at the workspace root level — no repo root matches
        let src = "docs".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(scope.domain, RewriteDomain::Workspace);
        assert!(scope.root.is_empty());
    }

    /// Empty workspace boundary set: only repo-root scoping applies.
    #[test]
    fn test_scope_no_repos_gives_workspace() {
        let ws = make_workspace_with_repos(&[]);
        let resolver = ScopeResolver::new(ws.path());

        let src = "some/deep/path".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(scope.domain, RewriteDomain::Workspace);
    }

    // ── is_in_scope ───────────────────────────────────────────────────────────

    /// File inside repo scope → in scope.
    #[test]
    fn test_is_in_scope_inside_repo() {
        let scope = Scope {
            domain: RewriteDomain::Repo("org-workspace".to_string()),
            root: "org-workspace".to_string(),
        };

        assert!(is_in_scope(
            &"org-workspace/foundations/engine/workflows/foo.md".to_string(),
            &scope
        ));
        assert!(is_in_scope(
            &"org-workspace/README.md".to_string(),
            &scope
        ));
    }

    /// File outside repo scope → not in scope.
    #[test]
    fn test_is_in_scope_outside_repo() {
        let scope = Scope {
            domain: RewriteDomain::Repo("org-workspace".to_string()),
            root: "org-workspace".to_string(),
        };

        assert!(!is_in_scope(
            &"org-sibling-repo/BOUNDARY.md".to_string(),
            &scope
        ));
        assert!(!is_in_scope(
            &"org-guild/projects/foo.md".to_string(),
            &scope
        ));
    }

    /// Workspace scope → everything is in scope.
    #[test]
    fn test_is_in_scope_workspace_includes_all() {
        let scope = Scope {
            domain: RewriteDomain::Workspace,
            root: String::new(),
        };

        assert!(is_in_scope(
            &"org-sibling-repo/BOUNDARY.md".to_string(),
            &scope
        ));
        assert!(is_in_scope(
            &"org-workspace/foundations/engine/workflows/foo.md".to_string(),
            &scope
        ));
    }

    // ── is_inward_ref ─────────────────────────────────────────────────────────

    /// Full canonical path matching src → inward ref.
    #[test]
    fn test_is_inward_ref_exact_match() {
        let src = "org-workspace/foundations/engine/workflows".to_string();
        assert!(is_inward_ref(
            "org-workspace/foundations/engine/workflows",
            &src
        ));
    }

    /// Path under src → inward ref.
    #[test]
    fn test_is_inward_ref_subpath() {
        let src = "org-workspace/foundations/engine/workflows".to_string();
        assert!(is_inward_ref(
            "org-workspace/foundations/engine/workflows/design.md",
            &src
        ));
    }

    /// Bare name suffix (the AENG-001 false-positive class) → NOT inward ref.
    #[test]
    fn test_is_inward_ref_bare_name_not_inward() {
        let src = "org-workspace/foundations/engine/workflows".to_string();
        assert!(!is_inward_ref("workflows", &src));
        assert!(!is_inward_ref("engine/workflows", &src));
        assert!(!is_inward_ref("foundations/engine/workflows", &src));
    }

    /// Trailing slash stripped before comparison.
    #[test]
    fn test_is_inward_ref_trailing_slash_stripped() {
        let src = "org-workspace/foundations/engine/workflows".to_string();
        assert!(is_inward_ref(
            "org-workspace/foundations/engine/workflows/",
            &src
        ));
    }

    /// Prefix-component boundary: `workflowsExtra` must not match `workflows`.
    #[test]
    fn test_is_inward_ref_no_false_prefix_match() {
        let src = "a/workflows".to_string();
        assert!(!is_inward_ref("a/workflowsExtra", &src));
        assert!(!is_inward_ref("a/workflowsExtra/foo.md", &src));
    }
}
