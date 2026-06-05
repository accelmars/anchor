use accelmars_anchor::cli;
use accelmars_anchor::cli::file::refs::OutputFormat;
use accelmars_anchor::cli::frontmatter::audit::AuditFormat;
use accelmars_anchor::cli::frontmatter::{
    run_add_required, run_audit, run_check_schema, run_migrate, run_migrate_plan, run_normalize,
    FmOutputFormat,
};
use accelmars_anchor::cli::text::find::{FindArgs, FindFormat};
use accelmars_anchor::cli::text::verify::VerifyArgs;

use clap::{Parser, Subcommand};
use std::process;

#[derive(Parser)]
#[command(
    name = "anchor",
    about = "Reference-safe file operations for Markdown workspaces",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new anchor workspace
    Init {
        /// Accept detected workspace root without prompting
        #[arg(long)]
        yes: bool,
        /// Specify workspace path explicitly, skipping detection
        #[arg(long)]
        path: Option<String>,
    },
    /// Execute a plan file — applies all operations sequentially
    Apply {
        /// Path to the plan file (.toml)
        plan: String,
        /// Acknowledge a known broken ref: <file>:<line> (repeatable)
        #[arg(long = "allow-broken", value_name = "FILE:LINE")]
        allow_broken: Vec<String>,
        /// Acknowledge broken refs from a file containing one FILE:LINE per line
        #[arg(long = "allow-broken-from", value_name = "PATH")]
        allow_broken_from: Option<String>,
        /// Rewrite all backtick path refs, including prose mentions (disables AENG-010 heuristic)
        #[arg(long = "allow-prose-rewrites")]
        allow_prose_rewrites: bool,
    },
    /// Preview what a plan file will do — no changes made
    Diff {
        /// Path to the plan file (.toml)
        plan: String,
        /// Print per-ref details for each move operation
        #[arg(long)]
        verbose: bool,
    },
    /// Recover from a crashed operation
    Recover,
    /// Print the workspace root path
    Root {
        /// Override the starting directory for workspace discovery
        #[arg(long)]
        cwd: Option<String>,
        /// Select a specific tenant slug (integrated mode only)
        #[arg(long)]
        tenant: Option<String>,
    },
    /// Print the workspace mode: standalone, integrated, or none
    Mode {
        /// Override the starting directory for workspace discovery
        #[arg(long)]
        cwd: Option<String>,
    },
    /// List all tenant slugs in .accelmars/
    Tenants {
        /// Override the starting directory for workspace discovery
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Print the root path for a specific tenant
    Tenant {
        /// The tenant slug to look up
        slug: String,
        /// Override the starting directory for workspace discovery
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Detect all broken references in the workspace (alias for 'anchor file validate')
    Validate {
        /// Output format (default: human-readable)
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
    },
    /// File operations
    File {
        #[command(subcommand)]
        subcommand: FileCommands,
    },
    /// Plan authoring
    Plan {
        #[command(subcommand)]
        subcommand: PlanCommands,
    },
    /// Start HTTP server
    Serve {
        /// Port to listen on
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Frontmatter management (audit, migrate, normalize, add-required, check-schema)
    Frontmatter {
        #[command(subcommand)]
        subcommand: FrontmatterCommands,
    },
    /// Text inspection primitives (find, ...)
    Text {
        #[command(subcommand)]
        subcommand: TextCommands,
    },
}

#[derive(Subcommand)]
enum TextCommands {
    /// Enumerate occurrences of a string or regex across markdown files
    Find {
        /// Pattern to search for (literal by default)
        pattern: String,
        /// Treat pattern as a regex instead of a literal string
        #[arg(long)]
        regex: bool,
        /// Glob pattern of files to include (repeatable)
        #[arg(long = "include", value_name = "GLOB")]
        include: Vec<String>,
        /// Glob pattern of files to exclude (repeatable)
        #[arg(long = "exclude", value_name = "GLOB")]
        exclude: Vec<String>,
        /// File extensions to search (default: md)
        #[arg(long = "file-type", value_name = "EXT")]
        file_type: Vec<String>,
        /// Lines of context to capture around each match
        #[arg(long, default_value_t = 2)]
        context: usize,
        /// Output format
        #[arg(long, value_enum, default_value_t = FindFormat::Text)]
        format: FindFormat,
        /// Include matches inside fenced code blocks (default: skip)
        #[arg(long = "include-code-blocks")]
        include_code_blocks: bool,
        /// Skip matches inside YAML frontmatter (default: include)
        #[arg(long = "exclude-frontmatter")]
        exclude_frontmatter: bool,
    },
    /// Verify that no unallow-listed occurrences remain in scope
    Verify {
        /// Pattern to search for (literal by default)
        pattern: String,
        /// Treat pattern as a regex instead of a literal string
        #[arg(long)]
        regex: bool,
        /// Glob pattern of files to include (repeatable)
        #[arg(long = "include", value_name = "GLOB")]
        include: Vec<String>,
        /// Glob pattern of files to exclude (repeatable)
        #[arg(long = "exclude", value_name = "GLOB")]
        exclude: Vec<String>,
        /// File extensions to search (default: md)
        #[arg(long = "file-type", value_name = "EXT")]
        file_type: Vec<String>,
        /// Include matches inside fenced code blocks (default: skip)
        #[arg(long = "include-code-blocks")]
        include_code_blocks: bool,
        /// Skip matches inside YAML frontmatter (default: include)
        #[arg(long = "exclude-frontmatter")]
        exclude_frontmatter: bool,
        /// Acknowledge an allowed occurrence: <file>:<line> (repeatable)
        #[arg(long = "allow", value_name = "FILE:LINE")]
        allow: Vec<String>,
        /// Read allowed occurrences from a file containing one FILE:LINE per line
        #[arg(long = "allow-from", value_name = "PATH")]
        allow_from: Option<String>,
        /// Output format
        #[arg(long, value_enum, default_value_t = FindFormat::Text)]
        format: FindFormat,
    },
}

#[derive(Subcommand)]
enum FrontmatterCommands {
    /// Report schema compliance for .md files under PATH
    Audit {
        /// Path to scan (default: current directory)
        path: Option<String>,
        /// Output format
        #[arg(long, value_enum, default_value = "human")]
        format: FmOutputFormat,
        /// Enable strict checks (e.g. _INDEX.md TOC presence)
        #[arg(long)]
        strict: bool,
        /// Path to JSON Schema file (overrides workspace default)
        #[arg(long)]
        schema: Option<String>,
    },
    /// Apply schema_version migrations or a TOML plan file
    Migrate {
        /// Path to migrate or TOML plan file (default: current directory)
        path: Option<String>,
        /// Target schema version (omit to use a plan file)
        #[arg(long = "to")]
        to: Option<u32>,
        /// Apply changes (default: dry-run)
        #[arg(long)]
        apply: bool,
    },
    /// Normalize frontmatter (status synonyms, optional key reordering)
    Normalize {
        /// Path to normalize (default: current directory)
        path: Option<String>,
        /// Apply changes (default: dry-run)
        #[arg(long)]
        apply: bool,
        /// Reorder keys to canonical order
        #[arg(long)]
        reorder: bool,
        /// Path to JSON Schema file (overrides workspace default)
        #[arg(long)]
        schema: Option<String>,
    },
    /// Fill deterministic missing required fields
    #[command(name = "add-required")]
    AddRequired {
        /// File or directory path
        path: String,
        /// Auto-fill safe defaults without prompting
        #[arg(long)]
        auto: bool,
        /// Scan directory recursively (default: single file)
        #[arg(long)]
        batch: bool,
        /// Path to JSON Schema file (overrides workspace default)
        #[arg(long)]
        schema: Option<String>,
    },
    /// CI guard: verify FRONTMATTER.md and FRONTMATTER.schema.json are in sync
    #[command(name = "check-schema")]
    CheckSchema {
        /// Path to FRONTMATTER.md (default: workspace-relative)
        spec: Option<String>,
        /// Path to FRONTMATTER.schema.json (default: workspace-relative)
        schema: Option<String>,
    },
}

#[derive(Subcommand)]
enum PlanCommands {
    /// Interactive wizard — generates a plan file from a template
    New {
        /// Output path for the generated plan (default: anchor-plan.toml)
        #[arg(long, short)]
        output: Option<String>,
        /// Skip wizard — write this template directly (batch-move, categorize, archive, rename, scaffold)
        #[arg(long, short = 't')]
        template: Option<String>,
    },
    /// List available plan templates
    List,
    /// Validate a plan file without executing
    Validate {
        /// Path to the plan file (.toml)
        plan: String,
    },
}

#[derive(Subcommand)]
enum FileCommands {
    /// Move a file or directory, rewriting all references
    Mv {
        src: String,
        dst: String,
        /// Print a human-readable confirmation on success
        #[arg(long)]
        verbose: bool,
        /// Output format for machine consumers (mutually exclusive with --verbose)
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
        /// Rewrite all backtick path refs, including prose mentions (disables AENG-010 heuristic)
        #[arg(long)]
        allow_prose_rewrites: bool,
    },
    /// Detect all broken references in the workspace
    Validate {
        /// Output format (default: human-readable)
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
    },
    /// List all files referencing a given file
    Refs {
        file: String,
        /// Output format (default: human-readable)
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
    },
}

fn main() {
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Commands::Init { yes, path } => match cli::init::run(yes, path.as_deref()) {
            Ok(()) => 0,
            Err(cli::init::InitError::Aborted) => 0,
            Err(e) => {
                eprintln!("error: {}", e);
                1
            }
        },
        Commands::Apply {
            plan,
            allow_broken,
            allow_broken_from,
            allow_prose_rewrites,
        } => process::exit(cli::apply::run(
            &plan,
            &allow_broken,
            allow_broken_from.as_deref(),
            allow_prose_rewrites,
        )),
        Commands::Diff { plan, verbose } => process::exit(cli::diff::run(&plan, verbose)),
        Commands::Recover => process::exit(cli::recover::run()),
        Commands::Root { cwd, tenant } => cli::root::run(cwd.as_deref(), tenant.as_deref()),
        Commands::Mode { cwd } => cli::mode::run(cwd.as_deref()),
        Commands::Tenants { cwd } => cli::tenants::run(cwd.as_deref()),
        Commands::Tenant { slug, cwd } => cli::tenant::run(&slug, cwd.as_deref()),
        Commands::Validate { format } => cli::file::validate::run(format),
        Commands::Plan { subcommand } => match subcommand {
            PlanCommands::New { output, template } => {
                process::exit(cli::plan::run_new(output.as_deref(), template.as_deref()))
            }
            PlanCommands::List => process::exit(cli::plan::run_list()),
            PlanCommands::Validate { plan } => process::exit(cli::plan::run_validate(&plan)),
        },
        Commands::Serve { port } => cli::serve::run(port),
        Commands::Frontmatter { subcommand } => match subcommand {
            FrontmatterCommands::Audit {
                path,
                format,
                strict,
                schema,
            } => {
                let fmt: AuditFormat = format.into();
                run_audit(path.as_deref(), fmt, schema.as_deref(), strict)
            }
            FrontmatterCommands::Migrate { path, to, apply } => match (path.as_deref(), to) {
                (Some(p), None) if p.ends_with(".toml") => run_migrate_plan(p, apply),
                (_, Some(n)) => run_migrate(path.as_deref(), n, apply),
                (Some(_), None) => {
                    eprintln!("error: --to <VERSION> required for non-plan paths");
                    1
                }
                (None, None) => {
                    eprintln!("error: --to <VERSION> required");
                    1
                }
            },
            FrontmatterCommands::Normalize {
                path,
                apply,
                reorder,
                schema,
            } => run_normalize(path.as_deref(), apply, reorder, schema.as_deref()),
            FrontmatterCommands::AddRequired {
                path,
                auto,
                batch,
                schema,
            } => run_add_required(&path, auto, batch, schema.as_deref()),
            FrontmatterCommands::CheckSchema { spec, schema } => {
                run_check_schema(spec.as_deref(), schema.as_deref())
            }
        },
        Commands::File { subcommand } => match subcommand {
            FileCommands::Mv {
                src,
                dst,
                verbose,
                format,
                allow_prose_rewrites,
            } => match cli::file::mv::run(&src, &dst, verbose, format, allow_prose_rewrites) {
                Ok(()) => 0,
                Err(cli::file::mv::MvError::ConflictingFlags(_)) => {
                    eprintln!("error: --verbose and --format are mutually exclusive");
                    1
                }
                Err(cli::file::mv::MvError::SrcNotFound) => {
                    use accelmars_anchor::core::scanner;
                    use accelmars_anchor::core::suggest::{format_suggestions, suggest_similar};
                    use accelmars_anchor::infra::workspace;
                    let root = workspace::find_workspace_root().ok();
                    let workspace_files: Vec<String> = root
                        .as_ref()
                        .and_then(|r| scanner::scan_workspace(r).ok())
                        .unwrap_or_default();
                    let suggestions = suggest_similar(&src, &workspace_files);
                    let corrected_command = suggestions
                        .first()
                        .map(|s| format!("anchor file mv \"{}\" {}", s, dst));
                    eprintln!(
                        "{}",
                        format_suggestions(&src, &suggestions, corrected_command.as_deref())
                    );
                    if let Some(root_path) = &root {
                        eprintln!(
                            "{}",
                            cli::file::mv::format_src_not_found_hint(&src, root_path)
                        );
                    }
                    1
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            },
            FileCommands::Validate { format } => cli::file::validate::run(format),
            FileCommands::Refs { file, format } => cli::file::refs::run(&file, format),
        },
        Commands::Text { subcommand } => match subcommand {
            TextCommands::Find {
                pattern,
                regex,
                include,
                exclude,
                file_type,
                context,
                format,
                include_code_blocks,
                exclude_frontmatter,
            } => {
                let file_types = if file_type.is_empty() {
                    vec!["md".to_string()]
                } else {
                    file_type
                };
                cli::text::find::run(FindArgs {
                    pattern,
                    regex,
                    include,
                    exclude,
                    file_types,
                    context,
                    format,
                    include_code_blocks,
                    include_frontmatter: !exclude_frontmatter,
                })
            }
            TextCommands::Verify {
                pattern,
                regex,
                include,
                exclude,
                file_type,
                include_code_blocks,
                exclude_frontmatter,
                allow,
                allow_from,
                format,
            } => {
                let file_types = if file_type.is_empty() {
                    vec!["md".to_string()]
                } else {
                    file_type
                };
                cli::text::verify::run(VerifyArgs {
                    pattern,
                    regex,
                    include,
                    exclude,
                    file_types,
                    include_code_blocks,
                    include_frontmatter: !exclude_frontmatter,
                    allow,
                    allow_from,
                    format,
                })
            }
        },
    };

    process::exit(exit_code);
}
