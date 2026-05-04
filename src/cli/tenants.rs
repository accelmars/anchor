use crate::infra::workspace::list_tenants;

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

    match list_tenants(&start) {
        Ok(slugs) => {
            for slug in slugs {
                println!("{}", slug);
            }
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

    #[test]
    fn tenants_integrated_lists_all() {
        let dir = tempfile::tempdir().unwrap();
        for slug in &["AOS", "acme"] {
            let slug_dir = dir.path().join(".accelmars").join(slug);
            fs::create_dir_all(&slug_dir).unwrap();
            fs::write(slug_dir.join("MANIFEST.toml"), "").unwrap();
        }
        let cwd = dir.path().to_str().unwrap().to_string();
        assert_eq!(run(Some(&cwd)), 0);
    }

    #[test]
    fn tenants_standalone_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".accelmars").join("anchor")).unwrap();
        let cwd = dir.path().to_str().unwrap().to_string();
        assert_eq!(run(Some(&cwd)), 0);
    }
}
