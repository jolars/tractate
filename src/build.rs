//! One-shot filesystem orchestration around the pure compiler and HTML backend.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::compiler::{RenderOptions, static_html};
use crate::document::{Diagnostic, DiagnosticCode, SourceFile};
use crate::render::html::{HtmlAsset, HtmlDirectory};

/// A rejected document or an unsuccessful filesystem operation.
#[derive(Debug)]
pub enum RenderError {
    Diagnostics(Vec<Diagnostic>),
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Diagnostics(diagnostics) => {
                write!(f, "render reported {} error(s)", diagnostics.len())
            }
            Self::Io { path, source } => {
                write!(f, "could not access `{}`: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Diagnostics(_) => None,
        }
    }
}

/// Render a source-only QMD or Markdown file to a complete HTML directory.
///
/// Local Markdown images and linked files resolve from the source's parent.
/// The destination is used as supplied, and its parent must exist. The complete
/// deck replaces a prior Tractate output atomically; errors preserve it. Viewing
/// requires network access for pinned Reveal.js and KaTeX assets.
///
/// This implementation never starts processes. Cells with effective `eval: true`
/// fail because execution is not implemented. Disabled cells honor `echo` and
/// `include`. Resource URLs inside raw HTML or copied files are not rewritten.
pub fn render_html(source: &Path, destination: &Path) -> Result<(), RenderError> {
    render_html_with_options(source, destination, RenderOptions::default())
}

/// Render HTML with an explicit execution policy.
///
/// Uses the same paths, resource handling, and atomic publication as [`render_html`].
/// With [`RenderOptions::no_execute`], unavailable required results fail before
/// publication and leave any previous deck intact. `eval: false` cells require
/// no result. Enabled cells currently have no available results because runners
/// and result caching are not implemented yet.
pub fn render_html_with_options(
    source: &Path,
    destination: &Path,
    options: RenderOptions,
) -> Result<(), RenderError> {
    let text = fs::read_to_string(source).map_err(|error| io_error(source, error))?;
    let compiled = static_html::compile(SourceFile::new(source, text), options)
        .map_err(RenderError::Diagnostics)?;
    let root = source.parent().unwrap_or(Path::new("."));
    let mut bytes = Vec::new();
    for resource in &compiled.resources {
        let path = root.join(&resource.source);
        bytes.push(fs::read(&path).map_err(|error| {
            RenderError::Diagnostics(vec![static_html::error(
                DiagnosticCode::ResourceUnavailable,
                &resource.origin,
                format!(
                    "could not read local resource `{}`: {error}",
                    path.display()
                ),
            )])
        })?);
    }
    protect_inputs(
        destination,
        std::iter::once(source.to_owned())
            .chain(compiled.resources.iter().map(|r| root.join(&r.source))),
    )?;
    let assets: Vec<_> = compiled
        .resources
        .iter()
        .zip(&bytes)
        .map(|(resource, bytes)| HtmlAsset {
            path: &resource.destination,
            bytes,
        })
        .collect();
    let directory = HtmlDirectory::new(&compiled.title, &compiled.slides, &assets)
        .map_err(|error| io_error(destination, error))?;
    directory
        .write(destination)
        .map_err(|error| io_error(destination, error))
}

fn protect_inputs(
    destination: &Path,
    inputs: impl Iterator<Item = PathBuf>,
) -> Result<(), RenderError> {
    let output = match destination.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(destination, error)),
    };
    for input in inputs {
        let resolved = input
            .canonicalize()
            .map_err(|error| io_error(&input, error))?;
        if resolved.starts_with(&output) {
            return Err(io_error(
                destination,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("output directory contains an input: `{}`", input.display()),
                ),
            ));
        }
    }
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> RenderError {
    RenderError::Io {
        path: path.to_owned(),
        source,
    }
}
