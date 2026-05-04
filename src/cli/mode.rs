use accelmars_resolver_env::ResolverMode;

use crate::infra::workspace::{resolve, ResolveHints, WorkspaceError};

pub fn run(cwd: Option<&str>) -> i32 {
    let start = match cwd {
        Some(p) => std::path::PathBuf::from(p),
        None => match std::env::current_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error: {}", e);
                return 1;
            }
        },
    };

    let hints = ResolveHints { tenant_flag: None };
    match resolve(&start, hints) {
        Ok(result) => {
            match result.mode {
                ResolverMode::Standalone => println!("standalone"),
                ResolverMode::Integrated => println!("integrated"),
            }
            0
        }
        // Ambiguous tenant still means the mode IS integrated.
        Err(WorkspaceError::AmbiguousTenant(_)) => {
            println!("integrated");
            0
        }
        Err(WorkspaceError::NotFound) => {
            println!("none");
            0
        }
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_standalone(dir: &std::path::Path) {
        fs::create_dir_all(dir.join(".accelmars").join("anchor")).unwrap();
    }

    fn make_integrated(dir: &std::path::Path, slug: &str) {
        let slug_dir = dir.join(".accelmars").join(slug);
        fs::create_dir_all(&slug_dir).unwrap();
        fs::write(slug_dir.join("MANIFEST.toml"), "").unwrap();
    }

    #[test]
    fn mode_standalone() {
        let dir = tempfile::tempdir().unwrap();
        make_standalone(dir.path());
        let cwd = dir.path().to_str().unwrap().to_string();
        assert_eq!(run(Some(&cwd)), 0);
    }

    #[test]
    fn mode_integrated() {
        let dir = tempfile::tempdir().unwrap();
        make_integrated(dir.path(), "AOS");
        let cwd = dir.path().to_str().unwrap().to_string();
        assert_eq!(run(Some(&cwd)), 0);
    }

    #[test]
    fn mode_none() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap().to_string();
        assert_eq!(run(Some(&cwd)), 0);
    }
}
