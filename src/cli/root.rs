use accelmars_os_env::ResolverMode;

use crate::infra::workspace::{resolve, ResolveHints, WorkspaceError};

// PHASE-2-BRIDGE Contract 1: anchor root output format is frozen.
// Output: absolute path, no trailing slash, on stdout.
// Exit codes: 0 = found, 1 = not found / tenant error, 2 = system error.
// Value changes in integrated mode (returns .accelmars/<slug>/ instead of parent dir),
// but format is preserved.
pub fn run(cwd: Option<&str>, tenant: Option<&str>) -> i32 {
    let start = match cwd {
        Some(p) => std::path::PathBuf::from(p),
        None => match std::env::current_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: {}", e);
                return 2;
            }
        },
    };

    let hints = ResolveHints {
        tenant_flag: tenant.map(|s| s.to_string()),
    };

    match resolve(&start, hints) {
        Ok(result) => {
            let output = match result.mode {
                // Standalone: print parent of .accelmars/ — same value as before v0.8.0.
                ResolverMode::Standalone => result
                    .tenant_root
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(result.tenant_root),
                // Integrated: print .accelmars/<slug>/ (the tenant root).
                ResolverMode::Integrated => result.tenant_root,
            };
            println!("{}", output.display());
            0
        }
        Err(WorkspaceError::NotFound) => {
            eprintln!("no workspace found. Run 'anchor init' to configure.");
            1
        }
        Err(e @ WorkspaceError::AmbiguousTenant(_))
        | Err(e @ WorkspaceError::TenantNotFound(_)) => {
            eprintln!("error: {}", e);
            1
        }
        Err(e) => {
            eprintln!("error: {}", e);
            2
        }
    }
}
