use accelmars_mind::cli;
use accelmars_mind::cli::file::refs::OutputFormat;

use clap::{Parser, Subcommand};
use std::process;

#[derive(Parser)]
#[command(
    name = "mind",
    about = "Reference-safe file operations for Markdown workspaces"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new mind workspace
    Init {
        /// Accept detected workspace root without prompting
        #[arg(long)]
        yes: bool,
        /// Specify workspace path explicitly, skipping detection
        #[arg(long)]
        path: Option<String>,
    },
    /// Print the workspace root path
    Root,
    /// File operations
    File {
        #[command(subcommand)]
        subcommand: FileCommands,
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
        Commands::Root => cli::root::run(),
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
