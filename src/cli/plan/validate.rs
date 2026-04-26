// src/cli/plan/validate.rs — anchor plan validate command (AP-001)

use crate::infra::workspace;
use crate::model::plan::{self, Op};
use std::path::Path;

/// Execute `anchor plan validate <plan.toml>`.
///
/// Discovers workspace root, loads the plan, then validates each operation:
/// Move ops must have an existing src and absent dst. Returns 0 on all-pass,
/// 1 on validation failures, 2 on file read/parse error.
pub fn run(plan_path: &str) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_impl(plan_path, &workspace_root)
}

pub(crate) fn run_impl(plan_path: &str, workspace_root: &Path) -> i32 {
    let plan = match plan::load_plan(Path::new(plan_path)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let mut errors: Vec<String> = Vec::new();

    for (i, op) in plan.ops.iter().enumerate() {
        let n = i + 1;
        match op {
            Op::Move { src, dst } => {
                if !workspace_root.join(src).exists() {
                    errors.push(format!("operation {n}: src not found: {src}"));
                }
                if workspace_root.join(dst).exists() {
                    errors.push(format!("operation {n}: dst already exists: {dst}"));
                }
            }
            Op::CreateDir { .. } => {}
        }
    }

    if errors.is_empty() {
        let count = plan.ops.len();
        println!("Plan is valid. {count} operations ready to apply.");
        0
    } else {
        for e in &errors {
            eprintln!("error: {e}");
        }
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &Path, rel: &str) {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, "").unwrap();
    }

    fn plan_file(dir: &Path, content: &str) -> String {
        let path = dir.join("test.toml");
        fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn test_validate_valid_plan() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "docs/guide.md");

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "docs/guide.md"
dst = "docs/renamed.md"
"#,
        );

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 0);
    }

    #[test]
    fn test_validate_missing_src() {
        let ws = TempDir::new().unwrap();

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "nonexistent/file.md"
dst = "other.md"
"#,
        );

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 1);
    }

    #[test]
    fn test_validate_dst_exists() {
        let ws = TempDir::new().unwrap();
        write_file(ws.path(), "src/file.md");
        write_file(ws.path(), "dst/file.md");

        let plan = plan_file(
            ws.path(),
            r#"version = "1"
[[ops]]
type = "move"
src = "src/file.md"
dst = "dst/file.md"
"#,
        );

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 1);
    }

    #[test]
    fn test_validate_invalid_toml() {
        let ws = TempDir::new().unwrap();
        let plan = plan_file(ws.path(), "not valid toml [[[");

        let code = run_impl(&plan, ws.path());
        assert_eq!(code, 2);
    }
}
