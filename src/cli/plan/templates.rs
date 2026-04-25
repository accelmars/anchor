// src/cli/plan/templates.rs — Static template registry for the plan wizard (AN-018)

/// A wizard template: id, display name, and one-line description.
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// The five Pareto templates shipped with the wizard.
///
/// All templates are domain-agnostic — no AccelMars-specific terminology.
pub const TEMPLATES: &[Template] = &[
    Template {
        id: "batch-move",
        name: "Batch Move",
        description: "Explicit list of src→dst moves",
    },
    Template {
        id: "categorize",
        name: "Categorize",
        description: "Group flat items under a parent folder",
    },
    Template {
        id: "archive",
        name: "Archive",
        description: "Move completed items to an archive location",
    },
    Template {
        id: "rename",
        name: "Rename",
        description: "Rename items by specifying new names",
    },
    Template {
        id: "scaffold",
        name: "Scaffold",
        description: "Create a directory structure",
    },
];
