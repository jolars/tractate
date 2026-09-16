//! Incremental compilation primitives for computational Markdown.

mod build;
mod compiler;
mod document;
mod parser;
mod render;

pub use build::{RenderError, render_html, render_html_with_options};
pub use compiler::{
    Compiler, CompilerSnapshot, RenderOptions, SourceRevision, inspect_document, summarize_document,
};
pub use document::{
    Derivation, Diagnostic, DiagnosticCode, DocumentInspection, DocumentSummary, Origin,
    RelatedOrigin, Severity, SourceFile, SourceRange, SourceSpan,
};
