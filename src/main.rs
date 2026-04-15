mod cli;
mod core;
mod infra;
mod model;

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
    Init,
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
    Mv { src: String, dst: String },
    /// Detect all broken references in the workspace
    Validate,
    /// List all files referencing a given file
    Refs { file: String },
}

fn main() {
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Commands::Init => {
            cli::init::run();
            0
        }
        Commands::Root => cli::root::run(),
        Commands::File { subcommand } => match subcommand {
            FileCommands::Mv { src, dst } => {
                cli::file::mv::run(&src, &dst);
                0
            }
            FileCommands::Validate => {
                cli::file::validate::run();
                0
            }
            FileCommands::Refs { file } => {
                cli::file::refs::run(&file);
                0
            }
        },
    };

    process::exit(exit_code);
}
