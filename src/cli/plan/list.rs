// src/cli/plan/list.rs — anchor plan list implementation (AN-029)

use crate::cli::plan::templates::TEMPLATES;

/// Format the three-section template list output.
///
/// Sections: BUILT-IN TEMPLATES, WORKSPACE TEMPLATES, USER TEMPLATES.
/// Phase 1 delivers built-in templates only; workspace and user sections show "(none)".
pub fn format_output() -> String {
    let mut out = String::new();

    out.push_str("BUILT-IN TEMPLATES\n");
    for t in TEMPLATES {
        out.push_str(&format!("  {:<12}  {}\n", t.id, t.description));
    }
    out.push('\n');
    out.push_str("WORKSPACE TEMPLATES (.accelmars/templates/)\n");
    out.push_str("  (none)\n");
    out.push('\n');
    out.push_str("USER TEMPLATES (~/.config/anchor/templates/)\n");
    out.push_str("  (none)\n");

    out
}

/// Entry point for `anchor plan list` — prints available templates.
pub fn run() -> i32 {
    print!("{}", format_output());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_output_contains_header() {
        let output = format_output();
        assert!(output.contains("BUILT-IN TEMPLATES"));
    }

    #[test]
    fn test_list_output_all_template_names() {
        let output = format_output();
        assert!(output.contains("batch-move"));
        assert!(output.contains("categorize"));
        assert!(output.contains("archive"));
        assert!(output.contains("rename"));
        assert!(output.contains("scaffold"));
    }

    #[test]
    fn test_list_output_workspace_section() {
        let output = format_output();
        assert!(output.contains("WORKSPACE TEMPLATES (.accelmars/templates/)"));
        assert!(output.contains("  (none)"));
    }

    #[test]
    fn test_list_output_user_section() {
        let output = format_output();
        assert!(output.contains("USER TEMPLATES (~/.config/anchor/templates/)"));
        assert!(output.contains("  (none)"));
    }
}
