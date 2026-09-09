//! Diagnostics shared by parsing, compilation, execution, and rendering.

use super::Origin;

/// Severity is independent of the diagnostic's code and producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    Error,
    #[allow(dead_code, reason = "Subsequent compiler passes can emit warnings.")]
    Warning,
    #[allow(dead_code, reason = "Subsequent compiler passes can emit notes.")]
    Note,
}

/// Tractate-owned categories; codes must not depend on upstream message wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticCode {
    InvalidYaml,
}

impl DiagnosticCode {
    /// Stable machine-readable spelling. Never reuse a code for a different meaning.
    #[allow(dead_code, reason = "Diagnostic frontends will display these codes.")]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidYaml => "syntax.invalid-yaml",
        }
    }
}

/// A diagnostic owns its message and retains the snapshots behind every origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
    /// Resolve `source_span()` for the actionable QMD location, even for generated code.
    pub primary: Origin,
    /// Ordered supporting locations, each with an explanation of its relationship.
    pub related: Vec<RelatedOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelatedOrigin {
    pub message: String,
    pub origin: Origin,
}
