//! Source snapshots and provenance shared by semantic and generated nodes.

#![allow(
    dead_code,
    reason = "Later compiler passes and diagnostics consume the retained origins."
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A half-open UTF-8 byte range, validated when attached to a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceRange {
    pub start: usize,
    pub end: usize,
}

/// An immutable snapshot. Clones share both identity and source bytes.
///
/// A new snapshot has its own identity even at the same path. Origins can thus
/// outlive an edit without resolving old offsets against new text. This is not
/// a semantic node identity or a persistent cache key.
#[derive(Debug, Clone)]
pub(crate) struct SourceFile(Arc<SourceSnapshot>);

#[derive(Debug)]
struct SourceSnapshot {
    path: Option<PathBuf>,
    text: String,
}

impl PartialEq for SourceFile {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SourceFile {}

impl SourceFile {
    /// Keep the supplied path without canonicalization or filesystem access.
    pub fn new(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        Self(Arc::new(SourceSnapshot {
            path: Some(path.into()),
            text: text.into(),
        }))
    }

    pub fn anonymous(text: impl Into<String>) -> Self {
        Self(Arc::new(SourceSnapshot {
            path: None,
            text: text.into(),
        }))
    }

    pub fn path(&self) -> Option<&Path> {
        self.0.path.as_deref()
    }

    pub fn text(&self) -> &str {
        &self.0.text
    }

    pub fn span(&self, range: SourceRange) -> Option<SourceSpan> {
        self.text().get(range.start..range.end)?;
        Some(SourceSpan {
            file: self.clone(),
            range,
        })
    }
}

/// A validated location that retains the exact file snapshot it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceSpan {
    file: SourceFile,
    range: SourceRange,
}

impl SourceSpan {
    pub fn file(&self) -> &SourceFile {
        &self.file
    }

    pub fn range(&self) -> SourceRange {
        self.range
    }

    pub fn text(&self) -> &str {
        &self.file.text()[self.range.start..self.range.end]
    }
}

/// Provenance always ends at source, even when an intermediate node has no
/// physical location. Generated locations supplement the initiating QMD span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Origin {
    Source(SourceSpan),
    Derived(Arc<Derivation>),
}

/// Share transformation steps without enlarging every direct source origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Derivation {
    pub operation: &'static str,
    pub span: Option<SourceSpan>,
    pub parent: Origin,
}

impl Origin {
    pub fn derived(&self, operation: &'static str) -> Self {
        Self::Derived(Arc::new(Derivation {
            operation,
            span: None,
            parent: self.clone(),
        }))
    }

    pub fn generated(&self, operation: &'static str, span: SourceSpan) -> Self {
        Self::Derived(Arc::new(Derivation {
            operation,
            span: Some(span),
            parent: self.clone(),
        }))
    }

    /// Walk from the most recent transformation back to the source.
    pub fn chain(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |origin| match origin {
            Self::Source(_) => None,
            Self::Derived(derivation) => Some(&derivation.parent),
        })
    }

    /// The actionable QMD location, regardless of generated intermediate files.
    pub fn source_span(&self) -> &SourceSpan {
        let mut origin = self;
        loop {
            match origin {
                Self::Source(span) => return span,
                Self::Derived(derivation) => origin = &derivation.parent,
            }
        }
    }

    /// This step's location, if it has physical source or generated text.
    pub fn span(&self) -> Option<&SourceSpan> {
        match self {
            Self::Source(span) => Some(span),
            Self::Derived(derivation) => derivation.span.as_ref(),
        }
    }
}
