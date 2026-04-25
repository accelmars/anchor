// src/cli/plan/templates.rs — Static template registry for the plan wizard (AN-018)

const BATCH_MOVE_TOML: &str = include_str!("../../../templates/batch-move.toml");
const CATEGORIZE_TOML: &str = include_str!("../../../templates/categorize.toml");
const ARCHIVE_TOML: &str = include_str!("../../../templates/archive.toml");
const RENAME_TOML: &str = include_str!("../../../templates/rename.toml");
const SCAFFOLD_TOML: &str = include_str!("../../../templates/scaffold.toml");

/// A wizard template: id, display name, description, and raw TOML content.
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub content: &'static str,
}

/// The five Pareto templates shipped with the wizard.
///
/// All templates are domain-agnostic — no AccelMars-specific terminology.
pub const TEMPLATES: &[Template] = &[
    Template {
        id: "batch-move",
        name: "Batch Move",
        description: "Explicit list of src→dst moves",
        content: BATCH_MOVE_TOML,
    },
    Template {
        id: "categorize",
        name: "Categorize",
        description: "Group flat items under a parent folder",
        content: CATEGORIZE_TOML,
    },
    Template {
        id: "archive",
        name: "Archive",
        description: "Move completed items to an archive location",
        content: ARCHIVE_TOML,
    },
    Template {
        id: "rename",
        name: "Rename",
        description: "Rename items by specifying new names",
        content: RENAME_TOML,
    },
    Template {
        id: "scaffold",
        name: "Scaffold",
        description: "Create a directory structure",
        content: SCAFFOLD_TOML,
    },
];
