// src/core/context_scope.rs — context-scoped rewrite domain resolution (AENG-001)
//
// A move op's rewrite scope is bounded to the deepest domain that contains its source path.
// This prevents bare-name substring matches from crossing repo boundaries — e.g., moving
// `org-workspace/foundations/engine/workflows` must not rewrite `workflows/`
// mentions in unrelated repos like `org-sibling-repo/`.
//
// Domain hierarchy (deepest match wins):
//   1. `scope_boundaries` from `.accelmars/anchor/config.json` — workspace-declared boundary
//      paths (e.g. `["foundations/*"]` expands to each foundation root in a monorepo)
//   2. Repo roots — directories at depth 1 under workspace_root that contain `.git`
//   3. Workspace root (fallback when neither applies)
//
// The "inward ref" rule: out-of-scope files may still hold workspace-relative paths that
// directly address the moved location (e.g. `$(anchor root)/org-workspace/.../workflows`).
// These are rewritten regardless of scope. Short suffix matches (e.g. bare `workflows`) from
// out-of-scope files are the false-positive class this module prevents.

use crate::model::config::WorkspaceConfig;
use crate::model::CanonicalPath;
use std::path::Path;

/// A rewrite domain — one level in the scope hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RewriteDomain {
    /// Entire workspace (fallback when no repo root contains the source path).
    Workspace,
    /// A git repository root at depth 1 under the workspace root.
    Repo(CanonicalPath),
    /// A workspace-declared sub-boundary from `scope_boundaries` in config.json.
    Defined(CanonicalPath),
}

/// The computed scope for a single move op.
#[derive(Debug, Clone)]
pub(crate) struct Scope {
    pub domain: RewriteDomain,
    /// Workspace-relative canonical root of this scope. Empty string for Workspace scope.
    pub root: CanonicalPath,
}

/// Caches repo-root and scope-boundary topology for one `plan()` call.
///
/// Created once per `plan()` invocation (before the `workspace_files` loop) so that
/// `scope_for_move` and `is_in_scope` don't re-scan the filesystem per reference.
pub(crate) struct ScopeResolver {
    repo_roots: Vec<CanonicalPath>,
    /// Workspace-relative canonical paths of declared scope boundaries,
    /// sorted by descending length so the deepest (most specific) ancestor matches first.
    scope_boundaries: Vec<CanonicalPath>,
}

impl ScopeResolver {
    /// Build a resolver by scanning depth-1 subdirectories of `workspace_root` for `.git`,
    /// and loading scope boundaries from `.accelmars/anchor/config.json`.
    pub(crate) fn new(workspace_root: &Path) -> Self {
        Self {
            repo_roots: discover_repo_roots(workspace_root),
            scope_boundaries: load_boundaries_from_config(workspace_root),
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

/// Expand a single glob pattern into workspace-relative canonical paths.
///
/// Two forms supported:
/// - `prefix/*` — returns direct child directories of `workspace_root/prefix`,
///   as `"prefix/{name}"` strings, sorted by descending length then ascending name.
/// - Literal path (no `*`) — returns `[pattern]` if `workspace_root/pattern` is a
///   directory; `[]` otherwise.
///
/// Uses `entry.file_type()` (Rule 12) to avoid following symlinks.
/// No recursive descent — `prefix/*` covers direct children only.
fn expand_glob_pattern(workspace_root: &Path, pattern: &str) -> Vec<CanonicalPath> {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        let dir = workspace_root.join(prefix);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return vec![];
        };
        let mut result = Vec::new();
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                result.push(format!("{prefix}/{name}"));
            }
        }
        result.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        result
    } else if workspace_root.join(pattern).is_dir() {
        vec![pattern.to_string()]
    } else {
        vec![]
    }
}

/// Load scope boundaries from `.accelmars/anchor/config.json`.
///
/// On any I/O or parse error, returns `vec![]` — `ScopeResolver` with empty boundaries
/// falls through to Repo scope (v0.6.0 behavior). Missing `scope_boundaries` key also
/// yields `vec![]`.
///
/// Returns deduplicated paths sorted by descending length then ascending name (deepest first),
/// matching the ordering expected by `scope_for_move`.
fn load_boundaries_from_config(workspace_root: &Path) -> Vec<CanonicalPath> {
    let config_path = workspace_root
        .join(".accelmars")
        .join("anchor")
        .join("config.json");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return vec![];
    };
    let Ok(config) = serde_json::from_str::<WorkspaceConfig>(&content) else {
        return vec![];
    };
    let patterns = config.scope_boundaries.unwrap_or_default();
    let mut all = Vec::new();
    for pattern in &patterns {
        all.extend(expand_glob_pattern(workspace_root, pattern));
    }
    all.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    all.dedup();
    all
}

/// Return the deepest-domain scope for a move op whose source is at `src`.
///
/// Resolution order:
///   1. The deepest scope boundary that is `src` or an ancestor of `src` (`Defined`)
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

    /// Write a scope_boundaries config to `.accelmars/anchor/config.json`.
    fn write_scope_boundaries(ws: &TempDir, patterns: &[&str]) {
        let config_dir = ws.path().join(".accelmars").join("anchor");
        fs::create_dir_all(&config_dir).unwrap();
        let patterns_json: Vec<String> = patterns.iter().map(|p| format!("\"{p}\"")).collect();
        let json = format!(
            r#"{{"schema_version":"1","scope_boundaries":[{}]}}"#,
            patterns_json.join(",")
        );
        fs::write(config_dir.join("config.json"), json).unwrap();
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

    // ── scope_boundaries (AENG-001 complete; config-driven) ───────────────────

    /// scope_boundaries boundary takes precedence over the enclosing repo root.
    /// A move inside foundations/gateway-engine/ scopes to that foundation, not the
    /// whole test-workspace repo.
    #[test]
    fn test_scope_for_move_boundary_beats_repo() {
        let ws = make_workspace_with_repos(&["test-workspace"]);
        fs::create_dir_all(ws.path().join("test-workspace/foundations/gateway-engine")).unwrap();
        fs::create_dir_all(ws.path().join("test-workspace/foundations/anchor-engine")).unwrap();
        write_scope_boundaries(
            &ws,
            &[
                "test-workspace/foundations/gateway-engine",
                "test-workspace/foundations/anchor-engine",
            ],
        );
        let resolver = ScopeResolver::new(ws.path());

        let src = "test-workspace/foundations/gateway-engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Defined("test-workspace/foundations/gateway-engine".to_string())
        );
        assert_eq!(scope.root, "test-workspace/foundations/gateway-engine");
    }

    /// Deepest scope boundary wins when multiple ancestors are declared.
    #[test]
    fn test_scope_for_move_deepest_boundary_wins() {
        let ws = make_workspace_with_repos(&["test-workspace"]);
        fs::create_dir_all(ws.path().join("test-workspace/foundations")).unwrap();
        fs::create_dir_all(ws.path().join("test-workspace/foundations/gateway-engine")).unwrap();
        write_scope_boundaries(
            &ws,
            &[
                "test-workspace/foundations",
                "test-workspace/foundations/gateway-engine",
            ],
        );
        let resolver = ScopeResolver::new(ws.path());

        let src = "test-workspace/foundations/gateway-engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Defined("test-workspace/foundations/gateway-engine".to_string())
        );
    }

    /// Boundary exists elsewhere in the workspace but not as an ancestor of `src`
    /// → fall back to repo-root scope (backward compat with v0.6.0).
    #[test]
    fn test_scope_for_move_boundary_not_ancestor_falls_through() {
        let ws = make_workspace_with_repos(&["test-workspace"]);
        fs::create_dir_all(ws.path().join("test-workspace/foundations/gateway-engine")).unwrap();
        write_scope_boundaries(&ws, &["test-workspace/foundations/gateway-engine"]);
        let resolver = ScopeResolver::new(ws.path());

        // src is in test-workspace but NOT inside any declared boundary ancestor
        let src = "test-workspace/docs/some-folder".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Repo("test-workspace".to_string())
        );
    }

    /// No scope_boundaries in config → behavior identical to v0.6.0 (Repo scope).
    #[test]
    fn test_scope_for_move_no_boundaries_in_config_repo_fallback() {
        let ws = make_workspace_with_repos(&["test-workspace"]);
        // No write_scope_boundaries call — config.json has no scope_boundaries key
        let resolver = ScopeResolver::new(ws.path());

        let src = "test-workspace/foundations/gateway-engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);

        assert_eq!(
            scope.domain,
            RewriteDomain::Repo("test-workspace".to_string())
        );
    }

    /// `is_in_scope` correctly admits files inside a `Defined` boundary and rejects
    /// sibling-foundation files (the v0.6.0 false-positive class).
    #[test]
    fn test_is_in_scope_defined_admits_inside_rejects_sibling() {
        let scope = Scope {
            domain: RewriteDomain::Defined("test-workspace/foundations/gateway-engine".to_string()),
            root: "test-workspace/foundations/gateway-engine".to_string(),
        };

        // In-scope file inside the foundation
        assert!(is_in_scope(
            &"test-workspace/foundations/gateway-engine/workflows/foo.md".to_string(),
            &scope
        ));
        // Sibling foundation — out of scope (this is the AENG-001 fix)
        assert!(!is_in_scope(
            &"test-workspace/foundations/anchor-engine/41-gaps/AENG-001.md".to_string(),
            &scope
        ));
        // File outside test-workspace entirely — out of scope
        assert!(!is_in_scope(&"test-codex/BOUNDARY.md".to_string(), &scope));
    }

    /// `scope_boundaries: ["foundations/*"]` expands to all direct child directories.
    #[test]
    fn test_scope_boundaries_glob_expands_direct_children() {
        let ws = make_workspace_with_repos(&["test-workspace"]);
        fs::create_dir_all(ws.path().join("foundations/engine-a")).unwrap();
        fs::create_dir_all(ws.path().join("foundations/engine-b")).unwrap();
        write_scope_boundaries(&ws, &["foundations/*"]);

        let resolver = ScopeResolver::new(ws.path());

        // Both foundations must appear in scope_boundaries
        let src_a = "foundations/engine-a/workflows".to_string();
        let scope_a = scope_for_move(&resolver, &src_a);
        assert_eq!(
            scope_a.domain,
            RewriteDomain::Defined("foundations/engine-a".to_string())
        );

        let src_b = "foundations/engine-b/docs".to_string();
        let scope_b = scope_for_move(&resolver, &src_b);
        assert_eq!(
            scope_b.domain,
            RewriteDomain::Defined("foundations/engine-b".to_string())
        );
    }

    /// A literal path pointing to a file (not a directory) is excluded from scope_boundaries.
    #[test]
    fn test_scope_boundaries_literal_path_is_file_excluded() {
        let ws = make_workspace_with_repos(&["test-workspace"]);
        // Write a regular file at the literal path — not a directory
        let file_path = ws.path().join("test-workspace/not-a-dir.md");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, "# content").unwrap();
        write_scope_boundaries(&ws, &["test-workspace/not-a-dir.md"]);

        let resolver = ScopeResolver::new(ws.path());

        // The file path is not a valid scope boundary — must fall through to Repo scope
        let src = "test-workspace/foundations/engine/workflows".to_string();
        let scope = scope_for_move(&resolver, &src);
        assert_eq!(
            scope.domain,
            RewriteDomain::Repo("test-workspace".to_string())
        );
    }
}
