use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tractate::{Origin, Severity, inspect_document};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect syntax, labels, and option declarations without executing cells.
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
    let inspection = inspect_document(&source);
    let summary = &inspection.summary;
    let languages = if summary.executable_languages.is_empty() {
        "none".to_owned()
    } else {
        summary.executable_languages.join(", ")
    };

    println!("source: {}", path.display());
    println!("slides: {}", summary.slides);
    println!("headings: {}", summary.headings);
    println!("code blocks: {}", summary.code_blocks);
    println!("executable cells: {}", summary.executable_cells);
    println!("executable languages: {languages}");
    println!("parse errors: {}", summary.parse_errors);
    println!("semantic errors: {}", inspection.semantic_errors());

    for diagnostic in &inspection.diagnostics {
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        eprintln!(
            "{}: {severity}[{}]: {}",
            location(path, &diagnostic.primary),
            diagnostic.code.as_str(),
            diagnostic.message
        );
        for related in &diagnostic.related {
            eprintln!(
                "{}: note: {}",
                location(path, &related.origin),
                related.message
            );
        }
    }

    if !inspection.has_errors() {
        Ok(())
    } else if summary.parse_errors > 0 {
        Err(format!(
            "parser reported {} error(s) in `{}`",
            summary.parse_errors,
            path.display()
        ))
    } else {
        Err(format!(
            "inspection reported {} semantic error(s) in `{}`",
            inspection.semantic_errors(),
            path.display()
        ))
    }
}

fn location(path: &Path, origin: &Origin) -> String {
    let span = origin.source_span();
    let prefix = &span.file().text()[..span.range().start];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("{}:{line}:{column}", path.display())
}
