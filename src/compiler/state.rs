//! Revision ownership and immutable compiler snapshots for a single document.

use std::fmt;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use crate::document::{DocumentInspection, Presentation, SlideKind, SourceFile};
use crate::parser;

use super::queries::{DependencyGraph, QueryKey};
use super::{RenderOptions, static_html};

#[cfg(test)]
mod tests;

/// A source version within one [`Compiler`] instance, starting at zero.
///
/// Compare revisions only within their owning compiler. This is neither a
/// semantic identity nor a persistent cache key. Equal numbers from independent
/// compilers do not identify the same input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRevision(u64);

impl SourceRevision {
    /// Return the monotonically increasing revision number.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SourceRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Long-lived, in-memory compilation state for one QMD or Markdown document.
///
/// Construction and updates parse, lower, and validate the supplied source
/// without filesystem access or execution. Snapshots track pure dependencies
/// through semantic blocks, slides, and lazily prepared HTML fragments. Each
/// changed input currently rebuilds the complete presentation; reuse across edits
/// is separate compiler work. Only the current snapshot is retained internally.
///
/// ```
/// use tractate::{Compiler, SourceFile};
///
/// let mut compiler = Compiler::new(SourceFile::anonymous("## First\n"));
/// let original = compiler.snapshot();
/// compiler.update_source(SourceFile::anonymous("## First\n\n## Second\n"));
/// assert!(compiler.revision() > original.revision());
/// assert_eq!(original.inspection().summary.slides, 1);
/// assert_eq!(compiler.snapshot().inspection().summary.slides, 2);
/// ```
#[derive(Debug)]
pub struct Compiler {
    current: CompilerSnapshot,
}

impl Compiler {
    /// Compile the supplied source at revision zero, including invalid input.
    #[must_use]
    pub fn new(source: SourceFile) -> Self {
        Self {
            current: CompilerSnapshot::new(source, SourceRevision(0)),
        }
    }

    #[must_use]
    pub fn revision(&self) -> SourceRevision {
        self.current.revision()
    }

    /// Share the current immutable state without recompiling or copying its tree.
    #[must_use]
    pub fn snapshot(&self) -> CompilerSnapshot {
        self.current.clone()
    }

    /// Replace the input and return its revision.
    ///
    /// Identical source bytes and supplied path leave the current snapshot and
    /// revision unchanged. Any change advances by one, including malformed input,
    /// path changes, and reversions. Paths are compared as supplied without I/O.
    /// Previously returned snapshots remain valid after updates or compiler drop.
    ///
    /// # Panics
    ///
    /// Panics on revision exhaustion before changing the current state, rather
    /// than wrapping or reusing a revision number.
    pub fn update_source(&mut self, source: SourceFile) -> SourceRevision {
        let current = self.current.source();
        if source.text() == current.text()
            && source.path().map(Path::as_os_str) == current.path().map(Path::as_os_str)
        {
            return self.revision();
        }
        let revision = SourceRevision(
            self.revision()
                .get()
                .checked_add(1)
                .expect("source revision space exhausted"),
        );
        self.current = CompilerSnapshot::new(source, revision);
        revision
    }
}

/// An immutable compilation at a specific source revision.
///
/// Clones share source, presentation, and diagnostics. Origins always resolve
/// against this snapshot's source, even after the owning compiler advances.
#[derive(Debug, Clone)]
pub struct CompilerSnapshot(Arc<SnapshotData>);

#[derive(Debug)]
struct SnapshotData {
    revision: SourceRevision,
    source: SourceFile,
    presentation: Presentation,
    inspection: DocumentInspection,
    dependencies: DependencyGraph,
    html: [OnceLock<static_html::HtmlCompilation>; 2],
}

impl CompilerSnapshot {
    fn new(source: SourceFile, revision: SourceRevision) -> Self {
        let mut dependencies = DependencyGraph::default();
        dependencies.query(QueryKey::Source, |_| ());
        let lowered = dependencies.query(QueryKey::SourceDocument, |reads| {
            parser::lower(reads.read(QueryKey::Source, source.clone()))
        });
        dependencies.query(QueryKey::Metadata, |reads| {
            reads.read(QueryKey::SourceDocument, &lowered.document.metadata)
        });
        lowered.document.visit_blocks(&mut |block| {
            dependencies.query(QueryKey::SemanticBlock(block.id), |reads| {
                let block = reads.read(QueryKey::SourceDocument, block);
                block.visit(
                    &mut |child| {
                        if child.id != block.id {
                            reads.read(QueryKey::SemanticBlock(child.id), child);
                        }
                    },
                    &mut |_| {},
                );
                block
            });
        });
        let parse_errors = lowered.error_count();
        let inspection_reads = super::queries::Reads::default();
        let mut diagnostics = inspection_reads.read(QueryKey::SourceDocument, lowered.diagnostics);
        if parse_errors == 0 {
            diagnostics.extend(super::validation::validate(
                inspection_reads.read(QueryKey::SourceDocument, &lowered.document),
            ));
        }
        let grouped = dependencies.query(QueryKey::SlideLayout, |reads| {
            super::build_presentation(reads.read(QueryKey::SourceDocument, lowered.document))
        });
        let presentation = super::presentation::lower_presentation_with(grouped, |slide| {
            dependencies.query(QueryKey::Slide(slide.id), |reads| {
                let slide = reads.read(QueryKey::SlideLayout, slide);
                match &slide.kind {
                    SlideKind::Title(title) => {
                        reads.read(QueryKey::Metadata, title);
                    }
                    SlideKind::Content(blocks) => {
                        for block in blocks {
                            reads.read(QueryKey::SemanticBlock(block.id), block);
                        }
                    }
                }
                super::presentation::lower_slide(slide)
            })
        });
        inspection_reads.read(QueryKey::SlideLayout, &presentation);
        for slide in &presentation.slides {
            inspection_reads.read(QueryKey::Slide(slide.id), slide);
        }
        let inspection = DocumentInspection {
            summary: super::summarize_presentation(&presentation, parse_errors),
            diagnostics,
        };
        dependencies.finish(QueryKey::Inspection, inspection_reads);
        Self(Arc::new(SnapshotData {
            revision,
            source,
            presentation,
            inspection,
            dependencies,
            html: Default::default(),
        }))
    }

    #[must_use]
    pub fn revision(&self) -> SourceRevision {
        self.0.revision
    }

    #[must_use]
    pub fn source(&self) -> &SourceFile {
        &self.0.source
    }

    /// Inspect this revision without reparsing or executing cells.
    #[must_use]
    pub fn inspection(&self) -> &DocumentInspection {
        &self.0.inspection
    }

    pub(super) fn presentation(&self) -> &Presentation {
        &self.0.presentation
    }

    pub(super) fn dependencies(&self) -> &DependencyGraph {
        &self.0.dependencies
    }

    pub(super) fn html(&self, options: RenderOptions) -> &static_html::HtmlCompilation {
        self.0.html[usize::from(options.no_execute)]
            .get_or_init(|| static_html::prepare(self, options))
    }
}
