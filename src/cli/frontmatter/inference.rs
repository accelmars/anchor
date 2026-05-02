// src/cli/frontmatter/inference.rs — path-based field inference for add-required
//
// Loads inference-rules.toml from the active template and applies fill-if-absent
// inferences based on a file's immediate parent folder name.

use serde::Deserialize;
use serde_yaml::Value;
use std::path::Path;

#[derive(Deserialize)]
struct InferRulesFile {
    #[serde(default)]
    infer: Vec<InferRuleEntry>,
}

#[derive(Deserialize)]
struct InferRuleEntry {
    folder_prefix: String,
    field: String,
    strategy: String,
    value: Option<String>,
}

pub(crate) struct InferenceRule {
    pub folder_prefix: String,
    pub field: String,
    pub strategy: String,
    pub value: Option<String>,
}

pub(crate) struct InferenceRules {
    pub rules: Vec<InferenceRule>,
}

impl InferenceRules {
    /// Load from `<workspace_root>/.accelmars/canon/templates/accelmars-standard/inference-rules.toml`.
    /// Returns empty rules if the file is absent or unparseable — inference is optional enrichment.
    pub fn load(workspace_root: &Path) -> Self {
        let path = workspace_root
            .join(".accelmars")
            .join("canon")
            .join("templates")
            .join("accelmars-standard")
            .join("inference-rules.toml");

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self { rules: Vec::new() },
        };

        let file: InferRulesFile = match toml::from_str(&content) {
            Ok(f) => f,
            Err(_) => return Self { rules: Vec::new() },
        };

        Self {
            rules: file
                .infer
                .into_iter()
                .map(|e| InferenceRule {
                    folder_prefix: e.folder_prefix,
                    field: e.field,
                    strategy: e.strategy,
                    value: e.value,
                })
                .collect(),
        }
    }

    /// Apply matching rules to `fm` for `file_path`. Fill-if-absent only — never overwrites.
    /// Folder matching checks the immediate parent directory name by prefix.
    pub fn apply(&self, mut fm: Value, file_path: &Path) -> Value {
        let parent_name = file_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        for rule in &self.rules {
            if !parent_name.starts_with(rule.folder_prefix.as_str()) {
                continue;
            }

            if let Value::Mapping(ref map) = fm {
                if map.contains_key(Value::String(rule.field.clone())) {
                    continue;
                }
            }

            let inferred = match rule.strategy.as_str() {
                "stem" => file_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string()),
                "constant" => rule.value.clone(),
                _ => None,
            };

            if let Some(v) = inferred {
                if let Value::Mapping(ref mut map) = fm {
                    map.insert(Value::String(rule.field.clone()), Value::String(v));
                }
            }
        }

        fm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn rule(
        folder_prefix: &str,
        field: &str,
        strategy: &str,
        value: Option<&str>,
    ) -> InferenceRule {
        InferenceRule {
            folder_prefix: folder_prefix.to_string(),
            field: field.to_string(),
            strategy: strategy.to_string(),
            value: value.map(str::to_string),
        }
    }

    fn rules(vec: Vec<InferenceRule>) -> InferenceRules {
        InferenceRules { rules: vec }
    }

    fn empty_fm() -> Value {
        serde_yaml::from_str("{}").unwrap()
    }

    fn fm_with(field: &str, val: &str) -> Value {
        serde_yaml::from_str(&format!("{field}: {val}")).unwrap()
    }

    #[test]
    fn stem_rule_fills_absent_field() {
        let r = rules(vec![rule("15-providers", "provider", "stem", None)]);
        let path = PathBuf::from("15-providers/claude.md");
        let result = r.apply(empty_fm(), &path);
        assert_eq!(result["provider"].as_str(), Some("claude"));
    }

    #[test]
    fn stem_rule_skips_present_field() {
        let r = rules(vec![rule("15-providers", "provider", "stem", None)]);
        let path = PathBuf::from("15-providers/claude.md");
        let fm = fm_with("provider", "openai");
        let result = r.apply(fm, &path);
        assert_eq!(result["provider"].as_str(), Some("openai"));
    }

    #[test]
    fn constant_rule_fills_absent_field() {
        let r = rules(vec![rule(
            "31-evals",
            "pass_status",
            "constant",
            Some("NOT_RUN"),
        )]);
        let path = PathBuf::from("31-evals/eval-001.md");
        let result = r.apply(empty_fm(), &path);
        assert_eq!(result["pass_status"].as_str(), Some("NOT_RUN"));
    }

    #[test]
    fn no_match_folder_leaves_fm_unchanged() {
        let r = rules(vec![rule("15-providers", "provider", "stem", None)]);
        let path = PathBuf::from("01-identity/overview.md");
        let result = r.apply(empty_fm(), &path);
        assert!(result["provider"].is_null());
    }

    #[test]
    fn missing_rules_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let loaded = InferenceRules::load(tmp.path());
        assert!(loaded.rules.is_empty());
    }
}
