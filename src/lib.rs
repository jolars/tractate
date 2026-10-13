//! Incremental compilation primitives for computational Markdown.

mod build;
mod compiler;
mod document;
mod execution;
mod parser;
mod render;

pub use build::{RenderError, render_html, render_html_with_options};
pub use compiler::{
    Compiler, CompilerSnapshot, RenderOptions, SlideChangeSet, SlideMove, SlidePosition,
    SourceRevision, SourceUpdate, inspect_document, summarize_document,
};
pub use document::{
    CellId, Derivation, Diagnostic, DiagnosticCode, DocumentInspection, DocumentSummary,
    ExecutionKey, Origin, RelatedOrigin, Severity, SlideId, SourceFile, SourceRange, SourceSpan,
};

pub use execution::{
    AttemptId, ExecutionOutcome, ExecutionProvenance, ExecutionRequest, ExecutionStatus,
    RunnerIdentity, SessionId, StateKey,
};
