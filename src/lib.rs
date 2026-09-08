//! Incremental compilation primitives for computational Markdown.

mod compiler;
mod document;
mod parser;
mod render;

pub use compiler::summarize_document;
pub use document::DocumentSummary;
