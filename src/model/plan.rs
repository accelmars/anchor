// src/model/plan.rs — Plan and Op types, TOML parsing, error types, write helpers (AN-015)
#![allow(dead_code)]

use serde::Deserialize;
use std::io;
use std::path::Path;

/// A plan file (`version = "1"`) listing operations to execute atomically.
#[derive(Debug, Deserialize, PartialEq)]
pub struct Plan {
    pub version: String,
    pub description: Option<String>,
    pub ops: Vec<Op>,
}

/// A single operation in a plan.
///
/// Deserialized from TOML using internally-tagged enum format:
///   `type = "create_dir"` → `Op::CreateDir`
///   `type = "move"`       → `Op::Move`
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Op {
    CreateDir { path: String },
    Move { src: String, dst: String },
}

/// Error returned by plan read/parse operations.
#[derive(Debug)]
pub enum PlanError {
    Io(io::Error),
    Parse(toml::de::Error),
    UnsupportedVersion(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Io(e) => write!(f, "plan I/O error: {e}"),
            PlanError::Parse(e) => write!(f, "plan parse error: {e}"),
            PlanError::UnsupportedVersion(v) => {
                write!(f, "unsupported plan version: {v:?} (expected \"1\")")
            }
        }
    }
}

impl From<io::Error> for PlanError {
    fn from(e: io::Error) -> Self {
        PlanError::Io(e)
    }
}

impl From<toml::de::Error> for PlanError {
    fn from(e: toml::de::Error) -> Self {
        PlanError::Parse(e)
    }
}

/// Read and parse a plan file from `path`.
///
/// Returns `PlanError::UnsupportedVersion` if `version != "1"`.
pub fn load_plan(path: &Path) -> Result<Plan, PlanError> {
    let content = std::fs::read_to_string(path)?;
    let plan: Plan = toml::from_str(&content)?;
    if plan.version != "1" {
        return Err(PlanError::UnsupportedVersion(plan.version));
    }
    Ok(plan)
}

/// Render a plan as human-readable TOML.
///
/// Uses manual rendering — not `toml::to_string` — to produce the canonical
/// plan file format with `[[ops]]` array-of-tables notation and clear spacing.
pub fn render_plan_toml(plan: &Plan) -> String {
    let mut out = String::new();
    out.push_str(&format!("version = {}\n", toml_string(&plan.version)));
    if let Some(desc) = &plan.description {
        out.push_str(&format!("description = {}\n", toml_string(desc)));
    }
    for op in &plan.ops {
        out.push('\n');
        out.push_str("[[ops]]\n");
        match op {
            Op::CreateDir { path } => {
                out.push_str("type = \"create_dir\"\n");
                out.push_str(&format!("path = {}\n", toml_string(path)));
            }
            Op::Move { src, dst } => {
                out.push_str("type = \"move\"\n");
                out.push_str(&format!("src = {}\n", toml_string(src)));
                out.push_str(&format!("dst = {}\n", toml_string(dst)));
            }
        }
    }
    out
}

/// Write a plan to `path` as human-readable TOML.
pub fn write_plan(path: &Path, plan: &Plan) -> Result<(), io::Error> {
    let content = render_plan_toml(plan);
    std::fs::write(path, content)
}

/// Escape a string as a TOML basic string (double-quoted).
fn toml_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Op::CreateDir parses correctly from `type = "create_dir"` TOML table.
    #[test]
    fn test_parse_create_dir() {
        let toml = r#"
version = "1"

[[ops]]
type = "create_dir"
path = "foundations"
"#;
        let plan: Plan = toml::from_str(toml).unwrap();
        assert_eq!(plan.version, "1");
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(
            plan.ops[0],
            Op::CreateDir {
                path: "foundations".to_string()
            }
        );
    }

    /// Op::Move parses correctly from `type = "move"` TOML table.
    #[test]
    fn test_parse_move() {
        let toml = r#"
version = "1"

[[ops]]
type = "move"
src = "anchor-foundation"
dst = "foundations/anchor-engine"
"#;
        let plan: Plan = toml::from_str(toml).unwrap();
        assert_eq!(plan.ops.len(), 1);
        assert_eq!(
            plan.ops[0],
            Op::Move {
                src: "anchor-foundation".to_string(),
                dst: "foundations/anchor-engine".to_string(),
            }
        );
    }

    /// Mixed ops (create_dir + move) parse in declaration order.
    #[test]
    fn test_parse_mixed_ops() {
        let toml = r#"
version = "1"
description = "scaffold and move"

[[ops]]
type = "create_dir"
path = "foundations"

[[ops]]
type = "move"
src = "anchor-foundation"
dst = "foundations/anchor-engine"
"#;
        let plan: Plan = toml::from_str(toml).unwrap();
        assert_eq!(plan.description, Some("scaffold and move".to_string()));
        assert_eq!(plan.ops.len(), 2);
        assert!(matches!(plan.ops[0], Op::CreateDir { .. }));
        assert!(matches!(plan.ops[1], Op::Move { .. }));
    }

    /// load_plan returns PlanError::UnsupportedVersion when version != "1".
    #[test]
    fn test_unsupported_version() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("plan.toml");
        std::fs::write(
            &path,
            r#"version = "2"

[[ops]]
type = "create_dir"
path = "x"
"#,
        )
        .unwrap();
        let result = load_plan(&path);
        assert!(
            matches!(result, Err(PlanError::UnsupportedVersion(ref v)) if v == "2"),
            "expected UnsupportedVersion(\"2\"), got {:?}",
            result
        );
    }

    /// render_plan_toml → toml::from_str roundtrip preserves version, description, all ops.
    #[test]
    fn test_roundtrip() {
        let plan = Plan {
            version: "1".to_string(),
            description: Some("test plan".to_string()),
            ops: vec![
                Op::CreateDir {
                    path: "foundations".to_string(),
                },
                Op::Move {
                    src: "anchor-foundation".to_string(),
                    dst: "foundations/anchor-engine".to_string(),
                },
            ],
        };
        let rendered = render_plan_toml(&plan);
        let parsed: Plan = toml::from_str(&rendered).expect("rendered TOML must parse");
        assert_eq!(parsed.version, plan.version);
        assert_eq!(parsed.description, plan.description);
        assert_eq!(parsed.ops.len(), plan.ops.len());
        assert_eq!(parsed.ops[0], plan.ops[0]);
        assert_eq!(parsed.ops[1], plan.ops[1]);
    }
}
