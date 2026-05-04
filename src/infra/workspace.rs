use std::io;
use std::path::{Path, PathBuf};

use accelmars_resolver_env::{ResolveResult, ResolverMode};

use crate::model::config::WorkspaceConfig;

/// Errors returned by workspace discovery and config loading.
#[derive(Debug)]
pub enum WorkspaceError {
    /// No `.accelmars/` directory was found anywhere up the directory tree.
    NotFound,
    /// An I/O error occurred while traversing the filesystem.
    IoError(io::Error),
    /// The workspace config contains an unsupported schema_version.
    #[allow(dead_code)]
    UnsupportedSchemaVersion(String),
    /// The workspace config.json could not be parsed.
    #[allow(dead_code)]
    InvalidConfig(serde_json::Error),
    /// Integrated mode, multiple tenants exist, and no slug could be determined.
    AmbiguousTenant(Vec<String>),
    /// A slug was specified (via flag or env var) but does not exist in `.accelmars/`.
    TenantNotFound(String),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceError::NotFound => {
                write!(f, "no workspace found. Run 'anchor init' to configure.")
            }
            WorkspaceError::IoError(e) => write!(f, "I/O error: {}", e),
            WorkspaceError::UnsupportedSchemaVersion(v) => {
                write!(
                    f,
                    "anchor workspace schema version \"{}\" is not supported by this version of anchor.\nPlease upgrade: https://github.com/accelmars/anchor",
                    v
                )
            }
            WorkspaceError::InvalidConfig(e) => write!(f, "invalid config.json: {}", e),
            WorkspaceError::AmbiguousTenant(slugs) => write!(
                f,
                "multiple tenants exist ({}); specify --tenant=<slug> or cd into one",
                slugs.join(", ")
            ),
            WorkspaceError::TenantNotFound(slug) => {
                write!(f, "tenant \"{}\" not found in .accelmars/", slug)
            }
        }
    }
}

/// Hints that influence slug selection in integrated mode.
pub struct ResolveHints {
    /// Value of `--tenant=<slug>` CLI flag, if provided.
    pub tenant_flag: Option<String>,
}

/// Detect whether a `.accelmars/` directory contains integrated-mode tenants.
/// Returns the mode and the list of slug names found (empty in standalone).
fn detect_mode(dot_accelmars: &Path) -> (ResolverMode, Vec<String>) {
    let mut slugs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dot_accelmars) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("MANIFEST.toml").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    slugs.push(name.to_string());
                }
            }
        }
    }
    slugs.sort(); // deterministic ordering
    if !slugs.is_empty() {
        (ResolverMode::Integrated, slugs)
    } else {
        (ResolverMode::Standalone, slugs)
    }
}

/// Resolve the active tenant slug using the 5-priority precedence ladder.
fn resolve_slug(
    hints: &ResolveHints,
    slugs: &[String],
    dot_accelmars: &Path,
    cwd: &Path,
) -> Result<String, WorkspaceError> {
    // Priority 1: --tenant flag
    if let Some(flag) = &hints.tenant_flag {
        if slugs.contains(flag) {
            return Ok(flag.clone());
        } else {
            return Err(WorkspaceError::TenantNotFound(flag.clone()));
        }
    }

    // Priority 2: ACCELMARS_TENANT env var
    if let Ok(env_slug) = std::env::var("ACCELMARS_TENANT") {
        if !env_slug.is_empty() {
            if slugs.contains(&env_slug) {
                return Ok(env_slug);
            } else {
                return Err(WorkspaceError::TenantNotFound(env_slug));
            }
        }
    }

    // Priority 3: cwd is inside .accelmars/<slug>/
    for slug in slugs {
        let slug_root = dot_accelmars.join(slug);
        if cwd.starts_with(&slug_root) {
            return Ok(slug.clone());
        }
    }

    // Priority 4: outer .accelmars/MANIFEST.toml declares default_tenant
    let outer_manifest = dot_accelmars.join("MANIFEST.toml");
    if outer_manifest.exists() {
        if let Ok(content) = std::fs::read_to_string(&outer_manifest) {
            if let Ok(table) = content.parse::<toml::Table>() {
                if let Some(toml::Value::String(default)) = table.get("default_tenant") {
                    if slugs.contains(default) {
                        return Ok(default.clone());
                    }
                }
            }
        }
    }

    // Priority 5: single-tenant shortcut — one slug, no ambiguity
    if slugs.len() == 1 {
        return Ok(slugs[0].clone());
    }

    Err(WorkspaceError::AmbiguousTenant(slugs.to_vec()))
}

/// Resolve the workspace root from `start`, honoring multi-tenant slug selection.
///
/// In standalone mode, `tenant_root` is the `.accelmars/` directory itself.
/// In integrated mode, `tenant_root` is `.accelmars/<slug>/`.
pub fn resolve(start: &Path, hints: ResolveHints) -> Result<ResolveResult, WorkspaceError> {
    let dot_accelmars = find_dot_accelmars(start)?;
    let (mode, slugs) = detect_mode(&dot_accelmars);

    match mode {
        ResolverMode::Standalone => Ok(ResolveResult {
            tenant_root: dot_accelmars.clone(),
            tenant_slug: "standalone".to_string(),
            engine_home: dot_accelmars,
            mode: ResolverMode::Standalone,
            spec_version: 1,
        }),
        ResolverMode::Integrated => {
            let slug = resolve_slug(&hints, &slugs, &dot_accelmars, start)?;
            let tenant_root = dot_accelmars.join(&slug);
            Ok(ResolveResult {
                engine_home: tenant_root.clone(),
                tenant_root,
                tenant_slug: slug,
                mode: ResolverMode::Integrated,
                spec_version: 1,
            })
        }
    }
}

/// List all tenant slugs found in the `.accelmars/` directory nearest to `start`.
/// Returns an empty vec if the workspace is standalone or no workspace exists.
pub fn list_tenants(start: &Path) -> Result<Vec<String>, WorkspaceError> {
    match find_dot_accelmars(start) {
        Ok(dot_accelmars) => {
            let (_, slugs) = detect_mode(&dot_accelmars);
            Ok(slugs)
        }
        Err(WorkspaceError::NotFound) => Ok(vec![]),
        Err(e) => Err(e),
    }
}

/// Walk up from `start` and return the `.accelmars/` directory path (not its parent).
fn find_dot_accelmars(start: &Path) -> Result<PathBuf, WorkspaceError> {
    let mut current = start.to_path_buf();
    loop {
        let marker = current.join(".accelmars");
        if marker.is_dir() {
            return Ok(marker);
        }
        match current.parent().map(|p| p.to_path_buf()) {
            Some(p) if p != current => current = p,
            _ => return Err(WorkspaceError::NotFound),
        }
    }
}

impl From<io::Error> for WorkspaceError {
    fn from(e: io::Error) -> Self {
        WorkspaceError::IoError(e)
    }
}

/// Walk up the directory tree from `start`, looking for a `.accelmars/` directory.
/// Returns the path of the directory containing `.accelmars/`, with no trailing slash.
///
/// Algorithm (verbatim from 260425-anchor-workspace-layout.md §4 Root Discovery):
/// 1. Start at `start`
/// 2. Check if .accelmars/ directory exists in current directory
/// 3. If yes → workspace root found, return this path
/// 4. If no → move to parent directory
/// 5. If reached filesystem root (/) with no .accelmars/ found:
///    → hard error: "no workspace found. Run 'anchor init' to configure."
/// 6. Repeat from step 2
///
/// Extracted from `find_workspace_root` so callers that already know their start
/// directory (e.g., tests) can call this directly without touching the global cwd.
pub(crate) fn find_workspace_root_from(start: &Path) -> Result<PathBuf, WorkspaceError> {
    let mut current = start.to_path_buf();

    loop {
        let marker = current.join(".accelmars");
        match marker.is_dir() {
            true => return Ok(current),
            false => {
                let parent = current.parent().map(|p| p.to_path_buf());
                match parent {
                    Some(p) if p != current => {
                        current = p;
                    }
                    _ => return Err(WorkspaceError::NotFound),
                }
            }
        }
    }
}

/// Walk up the directory tree from the current working directory, looking for a
/// `.accelmars/` directory. Returns the path of the directory containing it,
/// with no trailing slash.
pub fn find_workspace_root() -> Result<PathBuf, WorkspaceError> {
    let start = std::env::current_dir().map_err(WorkspaceError::IoError)?;
    find_workspace_root_from(&start)
}

/// Read `.accelmars/anchor/config.json` from the given workspace root, deserialize it,
/// and enforce schema version compatibility.
#[allow(dead_code)]
///
/// PHASE-2-BRIDGE Contract 2: schema_version is required and hard-enforced.
/// Any unknown version causes a hard stop — never degrade silently.
///
/// Error message format (exact, per 260425-anchor-workspace-layout.md §4):
/// `anchor workspace schema version "{v}" is not supported by this version of anchor.`
pub fn load_and_check_config(workspace_root: &Path) -> Result<WorkspaceConfig, WorkspaceError> {
    let config_path = workspace_root
        .join(".accelmars")
        .join("anchor")
        .join("config.json");
    let content = std::fs::read_to_string(&config_path).map_err(WorkspaceError::IoError)?;
    let config: WorkspaceConfig =
        serde_json::from_str(&content).map_err(WorkspaceError::InvalidConfig)?;
    if config.schema_version != "1" {
        return Err(WorkspaceError::UnsupportedSchemaVersion(
            config.schema_version,
        ));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Happy path: `.accelmars/` directory exists in the start directory.
    /// Verifies: returns Ok(path) matching the temp dir, no trailing slash.
    #[test]
    fn test_found_happy_path() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        fs::create_dir_all(dir.path().join(".accelmars")).unwrap();

        let root = find_workspace_root_from(dir.path()).expect("should find workspace root");
        // No trailing slash
        let as_str = root.to_string_lossy();
        assert!(
            !as_str.ends_with('/'),
            "root path must not have a trailing slash, got: {}",
            as_str
        );
        // Path must match the temp dir (canonicalized)
        assert_eq!(
            root.canonicalize().expect("canonicalize root"),
            dir.path().canonicalize().expect("canonicalize dir")
        );
    }

    /// Not found: no `.accelmars/` anywhere up the tree from a fresh temp directory.
    /// Verifies: returns Err(WorkspaceError::NotFound), does not loop infinitely.
    #[test]
    fn test_not_found_filesystem_root_stop() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        // No .accelmars/ in dir or its ancestors (tempdir is not inside the workspace).
        let result = find_workspace_root_from(dir.path());
        match result {
            Err(WorkspaceError::NotFound) => {}
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    /// Nested ascent: `.accelmars/` at root of temp dir, start is a deep subdirectory.
    /// Verifies: walk-up finds `.accelmars/` at the correct root level.
    #[test]
    fn test_nested_subdirectory_ascent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        fs::create_dir_all(dir.path().join(".accelmars")).unwrap();

        let deep = dir.path().join("2").join("3").join("4");
        fs::create_dir_all(&deep).expect("failed to create nested dirs");

        let root = find_workspace_root_from(&deep).expect("should find workspace root via ascent");
        assert_eq!(
            root.canonicalize().expect("canonicalize root"),
            dir.path().canonicalize().expect("canonicalize dir"),
            "walk-up should find .accelmars/ at the workspace root, not at the deep subdirectory"
        );
    }

    /// Unknown schema_version causes hard stop with exact error message.
    /// PHASE-2-BRIDGE Contract 2: hard-stop, never degrade silently.
    #[test]
    fn test_unsupported_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let anchor_dir = dir.path().join(".accelmars").join("anchor");
        fs::create_dir_all(&anchor_dir).unwrap();
        fs::write(anchor_dir.join("config.json"), r#"{"schema_version":"99"}"#).unwrap();

        let result = load_and_check_config(dir.path());
        match result {
            Err(WorkspaceError::UnsupportedSchemaVersion(v)) => {
                assert_eq!(v, "99");
                let msg = format!("{}", WorkspaceError::UnsupportedSchemaVersion(v));
                assert!(
                    msg.contains("anchor workspace schema version"),
                    "error message must reference 'anchor workspace schema version', got: {}",
                    msg
                );
                assert!(
                    msg.contains("not supported"),
                    "error message must contain 'not supported', got: {}",
                    msg
                );
            }
            other => panic!("expected UnsupportedSchemaVersion, got: {:?}", other),
        }
    }

    // --- resolve() tests ---

    fn make_standalone_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        // Flat engine dirs, no slug subdirs with MANIFEST.toml
        fs::create_dir_all(dir.path().join(".accelmars").join("anchor")).unwrap();
        fs::create_dir_all(dir.path().join(".accelmars").join("canon")).unwrap();
        dir
    }

    fn make_integrated_workspace(slugs: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for slug in slugs {
            let slug_dir = dir.path().join(".accelmars").join(slug);
            fs::create_dir_all(&slug_dir).unwrap();
            fs::write(
                slug_dir.join("MANIFEST.toml"),
                format!("slug = \"{}\"", slug),
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn resolve_standalone_compat() {
        let dir = make_standalone_workspace();
        let hints = ResolveHints { tenant_flag: None };
        let result = resolve(dir.path(), hints).expect("should resolve");
        assert_eq!(
            result.mode,
            accelmars_resolver_env::ResolverMode::Standalone
        );
        assert_eq!(result.tenant_slug, "standalone");
        // tenant_root should be the .accelmars/ dir
        assert!(result.tenant_root.ends_with(".accelmars"));
        // backward-compat check: parent of tenant_root == dir.path()
        assert_eq!(
            result.tenant_root.parent().unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn resolve_integrated_happy_path() {
        let dir = make_integrated_workspace(&["AOS"]);
        let hints = ResolveHints { tenant_flag: None };
        let result = resolve(dir.path(), hints).expect("should resolve");
        assert_eq!(
            result.mode,
            accelmars_resolver_env::ResolverMode::Integrated
        );
        assert_eq!(result.tenant_slug, "AOS");
        assert!(
            result.tenant_root.ends_with(".accelmars/AOS"),
            "tenant_root should end with .accelmars/AOS, got {:?}",
            result.tenant_root
        );
    }

    #[test]
    fn resolve_slug_via_tenant_flag() {
        let dir = make_integrated_workspace(&["AOS", "acme"]);
        let hints = ResolveHints {
            tenant_flag: Some("AOS".to_string()),
        };
        let result = resolve(dir.path(), hints).expect("should resolve");
        assert_eq!(result.tenant_slug, "AOS");
    }

    #[test]
    fn resolve_slug_via_env_var() {
        let dir = make_integrated_workspace(&["AOS", "acme"]);
        std::env::set_var("ACCELMARS_TENANT", "acme");
        let hints = ResolveHints { tenant_flag: None };
        let result = resolve(dir.path(), hints).expect("should resolve");
        std::env::remove_var("ACCELMARS_TENANT");
        assert_eq!(result.tenant_slug, "acme");
    }

    #[test]
    fn resolve_slug_via_cwd_context() {
        let dir = make_integrated_workspace(&["AOS", "acme"]);
        // cwd is inside .accelmars/AOS/anchor/
        let inner = dir.path().join(".accelmars").join("AOS").join("anchor");
        fs::create_dir_all(&inner).unwrap();
        let hints = ResolveHints { tenant_flag: None };
        let result = resolve(&inner, hints).expect("should resolve");
        assert_eq!(result.tenant_slug, "AOS");
    }

    #[test]
    fn resolve_slug_via_default_manifest() {
        let dir = make_integrated_workspace(&["AOS", "acme"]);
        // Outer .accelmars/MANIFEST.toml with default_tenant
        let outer = dir.path().join(".accelmars").join("MANIFEST.toml");
        fs::write(outer, "default_tenant = \"acme\"\n").unwrap();
        let hints = ResolveHints { tenant_flag: None };
        let result = resolve(dir.path(), hints).expect("should resolve");
        assert_eq!(result.tenant_slug, "acme");
    }

    #[test]
    fn resolve_ambiguous_error() {
        let dir = make_integrated_workspace(&["AOS", "acme"]);
        let hints = ResolveHints { tenant_flag: None };
        let err = resolve(dir.path(), hints).expect_err("should fail — ambiguous");
        match err {
            WorkspaceError::AmbiguousTenant(slugs) => {
                assert!(slugs.contains(&"AOS".to_string()));
                assert!(slugs.contains(&"acme".to_string()));
            }
            other => panic!("expected AmbiguousTenant, got: {:?}", other),
        }
    }

    #[test]
    fn resolve_tenant_not_found() {
        let dir = make_integrated_workspace(&["AOS"]);
        let hints = ResolveHints {
            tenant_flag: Some("missing".to_string()),
        };
        let err = resolve(dir.path(), hints).expect_err("should fail — tenant not found");
        match err {
            WorkspaceError::TenantNotFound(slug) => assert_eq!(slug, "missing"),
            other => panic!("expected TenantNotFound, got: {:?}", other),
        }
    }
}
