// src/cli/frontmatter/mod.rs — anchor frontmatter subcommand family
//
// Subcommand registration and dispatch for `anchor frontmatter`.
// Five subcommands:
//   audit        — schema compliance report
//   migrate      — apply schema_version transitions
//   normalize    — canonical status values and key ordering
//   add-required — fill deterministic missing type-conditional fields
//   check-schema — CI guard: FRONTMATTER.md vs FRONTMATTER.schema.json diff
//
// AENG-006 — anchor v0.6.0

pub mod add_required;
pub mod audit;
pub mod check_schema;
pub mod migrate;
pub mod normalize;
pub(crate) mod parser;
pub mod schema;

use audit::AuditFormat;

pub use add_required::run_from_env as run_add_required;
/// CLI argument types for `anchor frontmatter` subcommands.
/// These are re-exported from main.rs FrontmatterCommands dispatch.
pub use audit::run_from_env as run_audit;
pub use check_schema::run_from_env as run_check_schema;
pub use migrate::run_from_env as run_migrate;
pub use migrate::run_plan_from_env as run_migrate_plan;
pub use normalize::run_from_env as run_normalize;

/// Output format shared across subcommands that support JSON output.
#[derive(Debug, Clone, PartialEq, clap::ValueEnum)]
pub enum FmOutputFormat {
    Human,
    Json,
}

impl From<FmOutputFormat> for AuditFormat {
    fn from(f: FmOutputFormat) -> Self {
        match f {
            FmOutputFormat::Human => AuditFormat::Human,
            FmOutputFormat::Json => AuditFormat::Json,
        }
    }
}
