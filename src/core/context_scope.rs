// src/core/context_scope.rs — context-scoped rewrite domain resolution (AENG-001)
//
// A move op's rewrite scope is bounded to the deepest domain that contains its source path.
// This prevents bare-name substring matches from crossing repo boundaries — e.g., moving
// `org-workspace/foundations/engine/workflows` must not rewrite `workflows/`
// mentions in unrelated repos like `org-sibling-repo/`.
//
// Domain hierarchy (deepest match wins):
//   1. `.anchorscope` boundaries — directories containing a `.anchorscope` marker file
//      (e.g. each foundation root inside a multi-foundation monorepo)
//   2. Repo roots — directories at depth 1 under workspace_root that contain `.git`
//   3. Workspace root (fallback when neither applies)
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
    /// A workspace-defined sub-boundary, discovered via a `.anchorscope` marker file.
    Defined(CanonicalPath),
}

/// The computed scope for a single move op.
#[derive(Debug, Clone)]
pub(crate) struct Scope {
    pub domain: RewriteDomain,
    /// Workspace-relative canonical root of this scope. Empty string for Workspace scope.
    pub root: CanonicalPath,
}

/// Caches repo-root and `.anchorscope` topology for one `plan()` call.
///
/// Created once per `plan()` invocation (before the `workspace_files` loop) so that
/// `scope_for_move` and `is_in_scope` don't re-scan the filesystem per reference.
pub(crate) struct ScopeResolver {
    repo_roots: Vec<CanonicalPath>,
    /// Workspace-relative canonical paths of directories containing `.anchorscope`,
    /// sorted by descending length so the deepest (most specific) ancestor matches first.
    scope_boundaries: Vec<CanonicalPath>,
}

impl ScopeResolver {
    /// Build a resolver by scanning depth-1 subdirectories of `workspace_root` for `.git`,
    /// and recursively discovering `.anchorscope` markers.
    pub(crate) fn new(workspace_root: &Path) -> Self {
        Self {
            repo_roots: discover_repo_roots(workspace_root),
            scope_boundaries: discover_scope_boundaries(workspace_root),
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

/// Recursively scan `workspace_root` for `.anchorscope` marker files.
/// Returns workspace-relative canonical paths of their parent directories, sorted by
/// descending length (deepest first) so prefix-matching in `scope_for_move` finds the
/// most-specific boundary.
///
/// Skips well-known high-fanout directories (`.git`, `target`, `node_modules`) that
/// never legitimately host scope markers — these would otherwise dominate the walk
/// cost in mixed workspaces. Uses `entry.file_type()` (Rule 12) to avoid following
/// symlinks into ancestors.
fn discover_scope_boundaries(workspace_root: &Path) -> Vec<CanonicalPath> {
    let mut boundaries = Vec::new();
    let walker = walkdir::WalkDir::new(workspace_root)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name();
            name != ".git" && name != "target" && name != "node_modules"
        });
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.file_name() != ".anchorscope" {
            continue;
        }
        let Some(parent) = entry.path().parent() else {
            continue;
        };
        let Ok(rel) = parent.strip_prefix(workspace_root) else {
            continue;
        };
        let canonical = rel.to_string_lossy().replace('\\', "/");
        if canonical.is_empty() {
            // Marker at the workspace root itself is meaningless — equals Workspace scope.
            continue;
        }
        boundaries.push(canonical);
    }
    boundaries.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    boundaries
}

/// Return the deepest-domain scope for a move op whose source is at `src`.
///
/// Resolution order:
///   1. The deepest `.anchorscope` boundary that is `src` or an ancestor of `src` (`Defined`)
///   2. Repo root containing `src` (`Repo`) — backward compat with v0.6.0
///   3. `Workspace` fallback
pub(crate) fn scope_for_move(resolver: &ScopeResolver, src: &CanonicalPath) -> Scope {
    for boundary in &resolver.scope_boundaries {
        if src == boundary || src.starts_with(&format!("{boundary}/")) {
            return Scope {
                domain: RewriteDomain::Defined(boundary.clone()),
                root: boundary.clone(),
            };
        }
    }
    let parts: Vec<&str> = src.split('/').collect();
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

    /// Place an empty `.anchorscope` marker at `relative_dir` under the workspace root.
    fn add_anchorscope(ws: &TempDir, relative_dir: &str) {
        let dir = ws.path().join(relative_dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".anchorscope"), "").unwrap();
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
        assert!(is_in_scope(&"org-workspace/README.md".to_string(), &scope));
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

    // ── .anchorscope discovery (AENG-001 complete) ────────────────────────────

    /// `.anchorscope` boundary takes precedence over the enclosing repo root.
    /// A move inside foundations/gateway-engine/ scopes to that foundation, not the
    /// whole accelmars-workspace repo.
    #[test]
    fn test_scope_for_move_anchorscope_beats_repo() {
        let ws = make_workspace_with_repos(&["accelmars-workspace"]);
        add_anchorscope(&ws, "accelmars-workspace/foundations/gateway-engine");
        add_anchorscope(&ws, "accelmars-workspace/foundations/anchor-engine");
        let resolver = ScopeResolver::new(ws.path());

        let src = "accelmars-workspace/foundations/gateway-engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Defined("accelmars-workspace/foundations/gateway-engine".to_string())
        );
        assert_eq!(scope.root, "accelmars-workspace/foundations/gateway-engine");
    }

    /// Deepest `.anchorscope` wins when multiple ancestors carry markers.
    #[test]
    fn test_scope_for_move_deepest_anchorscope_wins() {
        let ws = make_workspace_with_repos(&["accelmars-workspace"]);
        add_anchorscope(&ws, "accelmars-workspace/foundations");
        add_anchorscope(&ws, "accelmars-workspace/foundations/gateway-engine");
        let resolver = ScopeResolver::new(ws.path());

        let src = "accelmars-workspace/foundations/gateway-engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Defined("accelmars-workspace/foundations/gateway-engine".to_string())
        );
    }

    /// `.anchorscope` exists elsewhere in the workspace but not as an ancestor of `src`
    /// → fall back to repo-root scope (backward compat with v0.6.0).
    #[test]
    fn test_scope_for_move_anchorscope_not_ancestor_falls_through() {
        let ws = make_workspace_with_repos(&["accelmars-workspace"]);
        add_anchorscope(&ws, "accelmars-workspace/foundations/gateway-engine");
        let resolver = ScopeResolver::new(ws.path());

        // src is in accelmars-workspace but NOT inside any .anchorscope ancestor
        let src = "accelmars-workspace/docs/some-folder".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Repo("accelmars-workspace".to_string())
        );
    }

    /// No `.anchorscope` markers anywhere → behavior identical to v0.6.0 (Repo scope).
    #[test]
    fn test_scope_for_move_no_anchorscope_repo_fallback() {
        let ws = make_workspace_with_repos(&["accelmars-workspace"]);
        let resolver = ScopeResolver::new(ws.path());

        let src = "accelmars-workspace/foundations/gateway-engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Repo("accelmars-workspace".to_string())
        );
    }

    /// `is_in_scope` correctly admits files inside a `Defined` boundary and rejects
    /// sibling-foundation files (the v0.6.0 false-positive class).
    #[test]
    fn test_is_in_scope_defined_admits_inside_rejects_sibling() {
        let scope = Scope {
            domain: RewriteDomain::Defined(
                "accelmars-workspace/foundations/gateway-engine".to_string(),
            ),
            root: "accelmars-workspace/foundations/gateway-engine".to_string(),
        };

        // In-scope file inside the foundation
        assert!(is_in_scope(
            &"accelmars-workspace/foundations/gateway-engine/workflows/foo.md".to_string(),
            &scope
        ));
        // Sibling foundation — out of scope (this is the AENG-001 fix)
        assert!(!is_in_scope(
            &"accelmars-workspace/foundations/anchor-engine/41-gaps/AENG-001.md".to_string(),
            &scope
        ));
        // File outside accelmars-workspace entirely — out of scope
        assert!(!is_in_scope(
            &"accelmars-codex/BOUNDARY.md".to_string(),
            &scope
        ));
    }

    /// Discovery skips well-known high-fanout dirs (`.git`, `target`, `node_modules`).
    /// A `.anchorscope` planted inside any of these must not become a boundary.
    #[test]
    fn test_discover_scope_boundaries_skips_irrelevant_dirs() {
        let ws = make_workspace_with_repos(&["repo"]);
        // Legitimate marker
        add_anchorscope(&ws, "repo/foundations/engine");
        // Decoy markers that must be ignored
        for ignored in &["repo/.git/hooks", "repo/target/debug", "repo/node_modules/x"] {
            let dir = ws.path().join(ignored);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(".anchorscope"), "").unwrap();
        }

        let boundaries = discover_scope_boundaries(ws.path());
        assert_eq!(boundaries, vec!["repo/foundations/engine".to_string()]);
    }
}
