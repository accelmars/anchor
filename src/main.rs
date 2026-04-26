use accelmars_anchor::cli;
use accelmars_anchor::cli::file::refs::OutputFormat;

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
    },
    /// Preview what a plan file will do — no changes made
    Diff {
        /// Path to the plan file (.toml)
        plan: String,
    },
    /// Recover from a crashed operation
    Recover,
    /// Print the workspace root path
    Root,
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
}

#[derive(Subcommand)]
enum PlanCommands {
    /// Interactive wizard — generates a plan file from a template
    New {
        /// Output path for the generated plan (default: anchor-plan.toml)
        #[arg(long, short)]
        output: Option<String>,
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
        Commands::Apply { plan } => process::exit(cli::apply::run(&plan)),
        Commands::Diff { plan } => process::exit(cli::diff::run(&plan)),
        Commands::Recover => process::exit(cli::recover::run()),
        Commands::Root => cli::root::run(),
        Commands::Plan { subcommand } => match subcommand {
            PlanCommands::New { output } => process::exit(cli::plan::run_new(output.as_deref())),
            PlanCommands::List => process::exit(cli::plan::run_list()),
            PlanCommands::Validate { plan } => process::exit(cli::plan::run_validate(&plan)),
        },
        Commands::Serve { port } => cli::serve::run(port),
        Commands::File { subcommand } => match subcommand {
            FileCommands::Mv {
                src,
                dst,
                verbose,
                format,
            } => match cli::file::mv::run(&src, &dst, verbose, format) {
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
    };

    process::exit(exit_code);
}
