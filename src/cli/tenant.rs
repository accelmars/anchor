use crate::infra::workspace::{resolve, ResolveHints, WorkspaceError};

pub fn run(slug: &str, cwd: Option<&str>) -> i32 {
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
        tenant_flag: Some(slug.to_string()),
    };

    match resolve(&start, hints) {
        Ok(result) => {
            println!("{}", result.tenant_root.display());
            0
        }
        Err(WorkspaceError::NotFound) => {
            eprintln!("no workspace found. Run 'anchor init' to configure.");
            1
        }
        Err(e @ WorkspaceError::TenantNotFound(_)) => {
            eprintln!("error: {}", e);
            1
        }
        Err(e) => {
            eprintln!("error: {}", e);
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_integrated(dir: &std::path::Path, slug: &str) {
        let slug_dir = dir.join(".accelmars").join(slug);
        fs::create_dir_all(&slug_dir).unwrap();
        fs::write(slug_dir.join("MANIFEST.toml"), "").unwrap();
    }

    #[test]
    fn tenant_found() {
        let dir = tempfile::tempdir().unwrap();
        make_integrated(dir.path(), "AOS");
        let cwd = dir.path().to_str().unwrap().to_string();
        assert_eq!(run("AOS", Some(&cwd)), 0);
    }

    #[test]
    fn tenant_not_found() {
        let dir = tempfile::tempdir().unwrap();
        make_integrated(dir.path(), "AOS");
        let cwd = dir.path().to_str().unwrap().to_string();
        assert_eq!(run("missing", Some(&cwd)), 1);
    }
}
