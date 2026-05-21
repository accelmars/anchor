use std::path::{Path, PathBuf};

pub const ENV_TENANT_ROOT: &str = "ACCELMARS_TENANT_ROOT";
pub const ENV_TENANT_SLUG: &str = "ACCELMARS_TENANT_SLUG";
pub const ENV_ENGINE_HOME: &str = "ACCELMARS_ENGINE_HOME";
pub const ENV_MODE: &str = "ACCELMARS_MODE";
pub const ENV_SPEC_VERSION: &str = "ACCELMARS_SPEC_VERSION";

#[derive(Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolverMode {
    Standalone,
    Integrated,
}

#[derive(Debug, PartialEq)]
pub struct ResolveResult {
    pub tenant_root: PathBuf,
    pub tenant_slug: String,
    pub engine_home: PathBuf,
    pub mode: ResolverMode,
    pub spec_version: u32,
}

#[derive(Debug, PartialEq)]
pub enum EnvError {
    MissingVar(String),
    InvalidValue {
        var: String,
        value: String,
        reason: String,
    },
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvError::MissingVar(v) => write!(f, "missing env var: {}", v),
            EnvError::InvalidValue { var, value, reason } => {
                write!(f, "invalid value for {}: {:?} — {}", var, value, reason)
            }
        }
    }
}

pub fn read_from_env() -> Result<ResolveResult, EnvError> {
    let tenant_root = PathBuf::from(require_var(ENV_TENANT_ROOT)?);
    let tenant_slug = require_var(ENV_TENANT_SLUG)?;
    let engine_home = PathBuf::from(require_var(ENV_ENGINE_HOME)?);

    let mode_str = require_var(ENV_MODE)?;
    let mode = match mode_str.as_str() {
        "standalone" => ResolverMode::Standalone,
        "integrated" => ResolverMode::Integrated,
        _ => {
            return Err(EnvError::InvalidValue {
                var: ENV_MODE.to_string(),
                value: mode_str,
                reason: "expected \"standalone\" or \"integrated\"".to_string(),
            })
        }
    };

    let version_str = require_var(ENV_SPEC_VERSION)?;
    let spec_version = version_str
        .parse::<u32>()
        .map_err(|_| EnvError::InvalidValue {
            var: ENV_SPEC_VERSION.to_string(),
            value: version_str,
            reason: "expected a non-negative integer".to_string(),
        })?;

    Ok(ResolveResult {
        tenant_root,
        tenant_slug,
        engine_home,
        mode,
        spec_version,
    })
}

fn require_var(name: &str) -> Result<String, EnvError> {
    std::env::var(name).map_err(|_| EnvError::MissingVar(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// fallback_standalone injects the `default/` slug layer per OS-ARC22.
    /// `.accelmars/` on disk → `tenant_root = .accelmars/default/`, slug = "default".
    #[test]
    fn fallback_injects_default_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let dot = tmp.path().join(".accelmars");
        fs::create_dir_all(&dot).unwrap();

        let result = fallback_standalone(tmp.path()).expect("walk-up should find .accelmars");

        assert_eq!(result.tenant_slug, STANDALONE_SLUG);
        assert_eq!(result.tenant_slug, "default");
        assert_eq!(result.tenant_root, dot.join(STANDALONE_SLUG));
        assert_eq!(result.engine_home, dot.join(STANDALONE_SLUG));
        assert_eq!(result.mode, ResolverMode::Standalone);
        assert_eq!(result.spec_version, 1);
    }

    /// fallback_standalone walks parent directories until it finds .accelmars/.
    #[test]
    fn fallback_walks_up_to_find_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let dot = tmp.path().join(".accelmars");
        fs::create_dir_all(&dot).unwrap();

        let deep = tmp.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();

        let result = fallback_standalone(&deep).expect("should find via ascent");
        assert_eq!(result.tenant_root, dot.join(STANDALONE_SLUG));
    }

    /// fallback_standalone errors when no `.accelmars/` is found up the tree.
    #[test]
    fn fallback_errors_when_no_marker() {
        let tmp = tempfile::tempdir().unwrap();
        // No .accelmars/ created.
        let result = fallback_standalone(tmp.path());
        match result {
            Err(EnvError::MissingVar(v)) => assert_eq!(v, ENV_TENANT_ROOT),
            other => panic!("expected MissingVar, got {:?}", other),
        }
    }

    /// read_from_env returns InvalidValue for an unrecognized mode.
    #[test]
    fn read_from_env_rejects_unknown_mode() {
        // Use temp env-var scope to avoid global pollution.
        // SAFETY: tests in this module are single-threaded by default; we set all required vars.
        unsafe {
            std::env::set_var(ENV_TENANT_ROOT, "/tmp/x");
            std::env::set_var(ENV_TENANT_SLUG, "acme");
            std::env::set_var(ENV_ENGINE_HOME, "/tmp/x");
            std::env::set_var(ENV_MODE, "garbage");
            std::env::set_var(ENV_SPEC_VERSION, "1");
        }
        let result = read_from_env();
        unsafe {
            std::env::remove_var(ENV_TENANT_ROOT);
            std::env::remove_var(ENV_TENANT_SLUG);
            std::env::remove_var(ENV_ENGINE_HOME);
            std::env::remove_var(ENV_MODE);
            std::env::remove_var(ENV_SPEC_VERSION);
        }
        match result {
            Err(EnvError::InvalidValue { var, .. }) => assert_eq!(var, ENV_MODE),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }
}

/// Default slug for the unnamed-tenant case in standalone mode.
///
/// Convention matches Kubernetes namespaces, AWS CLI profiles, Terraform workspaces,
/// and Docker Compose project names. Per OS-ARC22, every tenant layout includes a
/// slug subdirectory (`.accelmars/<slug>/`); standalone installs use `default` until
/// the operator renames via `os tenant rename default <new-slug>`.
pub const STANDALONE_SLUG: &str = "default";

pub fn fallback_standalone(cwd: &Path) -> Result<ResolveResult, EnvError> {
    let mut current = cwd.to_path_buf();
    loop {
        let marker = current.join(".accelmars");
        if marker.is_dir() {
            let tenant_root = marker.join(STANDALONE_SLUG);
            return Ok(ResolveResult {
                engine_home: tenant_root.clone(),
                tenant_root,
                tenant_slug: STANDALONE_SLUG.to_string(),
                mode: ResolverMode::Standalone,
                spec_version: 1,
            });
        }
        match current.parent().map(|p| p.to_path_buf()) {
            Some(p) if p != current => current = p,
            _ => return Err(EnvError::MissingVar(ENV_TENANT_ROOT.to_string())),
        }
    }
}
