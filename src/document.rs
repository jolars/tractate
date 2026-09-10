//! Backend-independent document types shared by the compiler and its callers.

mod diagnostic;
mod identity;
mod origin;
mod presentation;
mod source;

pub use diagnostic::*;
pub(crate) use identity::*;
pub use origin::*;
pub(crate) use presentation::*;
pub(crate) use source::*;

/// A structural summary of a computational Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSummary {
    /// Number of slides under the MVP title and level-two heading rules.
    pub slides: usize,
    /// Number of headings in the document.
    pub headings: usize,
    /// Number of fenced code blocks, including executable cells.
    pub code_blocks: usize,
    /// Number of executable fenced code blocks.
    pub executable_cells: usize,
    /// Languages used by executable cells, in lexical order.
    pub executable_languages: Vec<String>,
    /// Number of syntax errors reported by the parser.
    pub parse_errors: usize,
}

/// Structural information and diagnostics from execution-free inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInspection {
    pub summary: DocumentSummary,
    pub diagnostics: Vec<Diagnostic>,
}

impl DocumentInspection {
    /// Return whether inspection found any error-severity diagnostics.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Count semantic errors independently of syntax errors in the summary.
    #[must_use]
    pub fn semantic_errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error && d.code != DiagnosticCode::InvalidYaml)
            .count()
    }
}
