// src/cli/plan/templates.rs — Static template registry for the plan wizard (AN-018)

const BATCH_MOVE_TOML: &str = include_str!("../../../templates/batch-move.toml");
const CATEGORIZE_TOML: &str = include_str!("../../../templates/categorize.toml");
const ARCHIVE_TOML: &str = include_str!("../../../templates/archive.toml");
const RENAME_TOML: &str = include_str!("../../../templates/rename.toml");
const SCAFFOLD_TOML: &str = include_str!("../../../templates/scaffold.toml");

const BATCH_MOVE_SKELETON: &str = r#"version = "1"

# Batch Move — replace FILL_ME paths, add or remove [[ops]] blocks as needed
# then: anchor apply plan.toml

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME"

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME"

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME"
"#;

const CATEGORIZE_SKELETON: &str = r#"version = "1"

# Categorize — create a parent folder and move items into it
# replace FILL_ME paths, add or remove [[ops]] blocks as needed
# then: anchor apply plan.toml

[[ops]]
type = "create_dir"
path = "FILL_ME"

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME/FILL_ME"

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME/FILL_ME"
"#;

const ARCHIVE_SKELETON: &str = r#"version = "1"

# Archive — create an archive folder and move items into it
# replace FILL_ME paths, add or remove [[ops]] blocks as needed
# then: anchor apply plan.toml

[[ops]]
type = "create_dir"
path = "FILL_ME"

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME/FILL_ME"

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME/FILL_ME"
"#;

const RENAME_SKELETON: &str = r#"version = "1"

# Rename — replace FILL_ME paths, add or remove [[ops]] blocks as needed
# then: anchor apply plan.toml

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME"

[[ops]]
type = "move"
src = "FILL_ME"
dst = "FILL_ME"
"#;

const SCAFFOLD_SKELETON: &str = r#"version = "1"

# Scaffold — create directory structure, add or remove [[ops]] blocks as needed
# then: anchor apply plan.toml

[[ops]]
type = "create_dir"
path = "FILL_ME"

[[ops]]
type = "create_dir"
path = "FILL_ME"
"#;

/// A wizard template: id, display name, description, raw DSL content, and executable skeleton.
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub content: &'static str,
    pub skeleton: &'static str,
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
        skeleton: BATCH_MOVE_SKELETON,
    },
    Template {
        id: "categorize",
        name: "Categorize",
        description: "Group flat items under a parent folder",
        content: CATEGORIZE_TOML,
        skeleton: CATEGORIZE_SKELETON,
    },
    Template {
        id: "archive",
        name: "Archive",
        description: "Move completed items to an archive location",
        content: ARCHIVE_TOML,
        skeleton: ARCHIVE_SKELETON,
    },
    Template {
        id: "rename",
        name: "Rename",
        description: "Rename items by specifying new names",
        content: RENAME_TOML,
        skeleton: RENAME_SKELETON,
    },
    Template {
        id: "scaffold",
        name: "Scaffold",
        description: "Create a directory structure",
        content: SCAFFOLD_TOML,
        skeleton: SCAFFOLD_SKELETON,
    },
];
