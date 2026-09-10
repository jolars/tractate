//! Incremental compilation primitives for computational Markdown.

mod compiler;
mod document;
mod parser;
mod render;

pub use compiler::{inspect_document, summarize_document};
pub use document::{
    Derivation, Diagnostic, DiagnosticCode, DocumentInspection, DocumentSummary, Origin,
    RelatedOrigin, Severity, SourceFile, SourceRange, SourceSpan,
};
