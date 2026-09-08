use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tractate::summarize_document;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse a document without executing its cells.
    Inspect {
        /// Computational Markdown source to inspect.
        source: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tractate: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Inspect { source } => inspect(&source),
    }
}

fn inspect(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("could not read `{}`: {error}", path.display()))?;
    let summary = summarize_document(&source);
    let languages = if summary.executable_languages.is_empty() {
        "none".to_owned()
    } else {
        summary.executable_languages.join(", ")
    };

    println!("source: {}", path.display());
    println!("headings: {}", summary.headings);
    println!("code blocks: {}", summary.code_blocks);
    println!("executable cells: {}", summary.executable_cells);
    println!("executable languages: {languages}");
    println!("parse errors: {}", summary.parse_errors);

    if summary.parse_errors == 0 {
        Ok(())
    } else {
        Err(format!(
            "parser reported {} error(s) in `{}`",
            summary.parse_errors,
            path.display()
        ))
    }
}
