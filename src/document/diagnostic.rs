//! Diagnostics shared by parsing, compilation, execution, and rendering.

use super::Origin;

/// Severity is independent of the diagnostic's code and producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    #[allow(dead_code, reason = "Subsequent compiler passes can emit warnings.")]
    Warning,
    #[allow(dead_code, reason = "Subsequent compiler passes can emit notes.")]
    Note,
}

/// Tractate-owned categories; codes must not depend on upstream message wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    InvalidYaml,
    UnknownOption,
    WrongOptionScope,
    DuplicateOption,
    InvalidOptionMapping,
    UnsupportedOptionSyntax,
    InvalidLabel,
    DuplicateLabel,
}

impl DiagnosticCode {
    /// Stable machine-readable spelling. Never reuse a code for a different meaning.
    #[allow(dead_code, reason = "Diagnostic frontends will display these codes.")]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidYaml => "syntax.invalid-yaml",
            Self::UnknownOption => "option.unknown",
            Self::WrongOptionScope => "option.wrong-scope",
            Self::DuplicateOption => "option.duplicate",
            Self::InvalidOptionMapping => "option.invalid-mapping",
            Self::UnsupportedOptionSyntax => "option.unsupported-syntax",
            Self::InvalidLabel => "option.invalid-label",
            Self::DuplicateLabel => "semantic.duplicate-label",
        }
    }
}

/// A diagnostic owns its message and retains the snapshots behind every origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
    /// Resolve `source_span()` for the actionable QMD location, even for generated code.
    pub primary: Origin,
    /// Ordered supporting locations, each with an explanation of its relationship.
    pub related: Vec<RelatedOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedOrigin {
    pub message: String,
    pub origin: Origin,
}
