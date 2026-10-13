//! Revision ownership and immutable compiler snapshots for a single document.

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::document::{DocumentInspection, Presentation, SemanticIds, SlideKind, SourceFile};
use crate::parser;

use super::changes::{SlideChangeSet, SourceUpdate};
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
/// changed input is parsed and validated anew, while unchanged semantic blocks
/// and HTML fragments reuse their pure compiled values. Only the current snapshot
/// is retained internally.
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
        self.update_source_with_changes(source).revision
    }

    /// Replace the input and describe slide changes from the preceding revision.
    ///
    /// Identical input returns an empty change set at the current revision.
    /// Changes describe semantic slide content, independently of rendering,
    /// execution, diagnostics, and source positions. See [`SlideChangeSet`].
    ///
    /// # Panics
    ///
    /// Panics on revision exhaustion before changing the current state.
    pub fn update_source_with_changes(&mut self, source: SourceFile) -> SourceUpdate {
        let current = self.current.source();
        if source.text() == current.text()
            && source.path().map(Path::as_os_str) == current.path().map(Path::as_os_str)
        {
            return SourceUpdate {
                previous_revision: self.revision(),
                revision: self.revision(),
                changes: SlideChangeSet::unchanged(&self.current),
            };
        }
        let revision = SourceRevision(
            self.revision()
                .get()
                .checked_add(1)
                .expect("source revision space exhausted"),
        );
        let next = CompilerSnapshot::updated(source, revision, Some(&self.current));
        let update = SourceUpdate {
            previous_revision: self.revision(),
            revision,
            changes: SlideChangeSet::between(&self.current, &next),
        };
        self.current = next;
        update
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
    next_id: u64,
    next_slide: u64,
    nodes: Vec<Arc<super::reuse::SemanticNode>>,
    fragments: Arc<Mutex<static_html::FragmentCache>>,
}

impl CompilerSnapshot {
    fn new(source: SourceFile, revision: SourceRevision) -> Self {
        Self::updated(source, revision, None)
    }

    fn updated(source: SourceFile, revision: SourceRevision, previous: Option<&Self>) -> Self {
        let ids = SemanticIds::starting_at(previous.map_or(0, |previous| previous.0.next_id));
        let slide_ids =
            SemanticIds::starting_at(previous.map_or(0, |previous| previous.0.next_slide));
        let mut dependencies = DependencyGraph::default();
        dependencies.query(QueryKey::Source, |_| ());
        let mut lowered = dependencies.query(QueryKey::SourceDocument, |reads| {
            parser::lower(reads.read(QueryKey::Source, source.clone()))
        });
        let nodes = super::reuse::semantic_nodes(
            &mut lowered.document.blocks,
            previous.map_or(&[], |previous| &previous.0.nodes),
            &ids,
        );
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
        let mut grouped = dependencies.query(QueryKey::SlideLayout, |reads| {
            super::build_presentation(reads.read(QueryKey::SourceDocument, lowered.document))
        });
        let mut old_slides: Vec<_> = previous
            .into_iter()
            .flat_map(|previous| &previous.presentation().slides)
            .collect();
        for slide in &mut grouped.slides {
            let matched = old_slides
                .iter()
                .position(|old| match (&old.kind, &slide.kind) {
                    (SlideKind::Title(_), SlideKind::Title(_)) => true,
                    (SlideKind::Content(old), SlideKind::Content(new)) => old
                        .first()
                        .zip(new.first())
                        .is_some_and(|(old, new)| old.id == new.id),
                    _ => false,
                });
            slide.id =
                matched.map_or_else(|| slide_ids.slide(), |index| old_slides.remove(index).id);
        }
        let lowered_block = |block: crate::document::Block| {
            nodes
                .iter()
                .find(|node| node.id() == block.id)
                .expect("every top-level block has a semantic template")
                .presented(&block)
        };
        let presentation = super::presentation::lower_presentation_with(
            grouped,
            |slide| {
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
                    super::presentation::lower_slide_with(slide, lowered_block)
                })
            },
            lowered_block,
        );
        inspection_reads.read(QueryKey::SlideLayout, &presentation);
        for slide in &presentation.slides {
            inspection_reads.read(QueryKey::Slide(slide.id), slide);
        }
        let inspection = DocumentInspection {
            summary: super::summarize_presentation(&presentation, parse_errors),
            diagnostics,
        };
        dependencies.finish(QueryKey::Inspection, inspection_reads);
        let fragments = previous.map_or_else(Default::default, |previous| {
            Arc::clone(&previous.0.fragments)
        });
        fragments
            .lock()
            .expect("fragment cache lock")
            .retain(|id, _| presentation.slides.iter().any(|slide| slide.id == *id));
        Self(Arc::new(SnapshotData {
            revision,
            source,
            presentation,
            inspection,
            dependencies,
            html: Default::default(),
            next_id: ids.next(),
            next_slide: slide_ids.next(),
            nodes,
            fragments,
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

    /// Slide handles in presentation order, scoped to this compiler instance.
    pub fn slide_ids(&self) -> impl ExactSizeIterator<Item = crate::document::SlideId> + '_ {
        self.presentation().slides.iter().map(|slide| slide.id)
    }

    pub(super) fn presentation(&self) -> &Presentation {
        &self.0.presentation
    }

    pub(super) fn dependencies(&self) -> &DependencyGraph {
        &self.0.dependencies
    }

    pub(super) fn nodes(&self) -> &[Arc<super::reuse::SemanticNode>] {
        &self.0.nodes
    }

    pub(super) fn fragments(&self) -> &Mutex<static_html::FragmentCache> {
        &self.0.fragments
    }

    pub(super) fn html(&self, options: RenderOptions) -> &static_html::HtmlCompilation {
        self.0.html[usize::from(options.no_execute)]
            .get_or_init(|| static_html::prepare(self, options))
    }
}
