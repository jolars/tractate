//! Revision-independent semantic templates and revision-local provenance.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::document::*;

/// Cached lowering uses an isolated block source, so templates retain neither
/// a whole previous document nor offsets that belong to another revision.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SemanticNode {
    source: Block,
    presented: Block<PresentedCell>,
    nodes: BTreeMap<NodeId, NodeId>,
    cells: BTreeMap<CellId, CellId>,
}

impl SemanticNode {
    pub fn id(&self) -> NodeId {
        self.nodes[&self.source.id]
    }

    pub fn presented(&self, block: &Block) -> Block<PresentedCell> {
        let mut presented = self.presented.clone();
        presented.transform(&mut self.binding(block));
        presented
    }

    fn binding(&self, block: &Block) -> Binding<'_> {
        Binding {
            file: block.origin.source_span().file().clone(),
            offset: block.origin.source_span().range().start as i128,
            nodes: &self.nodes,
            cells: &self.cells,
        }
    }
}

pub(super) fn semantic_nodes(
    blocks: &mut [Block],
    previous: &[Arc<SemanticNode>],
    ids: &SemanticIds,
) -> Vec<Arc<SemanticNode>> {
    let mut available: BTreeMap<String, Vec<&Arc<SemanticNode>>> = BTreeMap::new();
    for node in previous.iter().rev() {
        available
            .entry(node.source.origin.source_span().file().text().to_owned())
            .or_default()
            .push(node);
    }
    blocks
        .iter_mut()
        .map(|block| {
            let text = block.origin.source_span().text().to_owned();
            let candidates = available.entry(text.clone()).or_default();
            let mut found = None;
            for (index, candidate) in candidates.iter().enumerate().rev() {
                let normalized =
                    normalize(block, candidate.source.origin.source_span().file().clone());
                if normalized == candidate.source {
                    found = Some(index);
                    break;
                }
            }
            let node = if let Some(index) = found {
                Arc::clone(candidates.remove(index))
            } else {
                let source = normalize(block, SourceFile::anonymous(text));
                let mut nodes = BTreeMap::new();
                let mut cells = BTreeMap::new();
                source.clone().transform(&mut Allocate {
                    ids,
                    nodes: &mut nodes,
                    cells: &mut cells,
                });
                let presented = super::presentation::lower_block(source.clone());
                Arc::new(SemanticNode {
                    source,
                    presented,
                    nodes,
                    cells,
                })
            };
            // Materialize current origins while retaining the template's handles.
            let mut source = node.source.clone();
            source.transform(&mut node.binding(block));
            *block = source;
            node
        })
        .collect()
}

fn normalize(block: &Block, file: SourceFile) -> Block {
    let mut normalized = block.clone();
    normalized.transform(&mut Normalize {
        file,
        offset: -(block.origin.source_span().range().start as i128),
        ids: SemanticIds::default(),
    });
    normalized
}

struct Allocate<'a> {
    ids: &'a SemanticIds,
    nodes: &'a mut BTreeMap<NodeId, NodeId>,
    cells: &'a mut BTreeMap<CellId, CellId>,
}

impl Transform for Allocate<'_> {
    fn origin(&mut self, _: &mut Origin) {}
    fn span(&mut self, _: &mut SourceSpan) {}
    fn node(&mut self, id: &mut NodeId) {
        self.nodes.entry(*id).or_insert_with(|| self.ids.node());
    }
    fn cell(&mut self, id: &mut CellId) {
        self.cells.entry(*id).or_insert_with(|| self.ids.cell());
    }
}

struct Normalize {
    file: SourceFile,
    offset: i128,
    ids: SemanticIds,
}

impl Transform for Normalize {
    fn origin(&mut self, origin: &mut Origin) {
        *origin = rebind_origin(origin, &self.file, self.offset);
    }
    fn span(&mut self, span: &mut SourceSpan) {
        *span = rebind_span(span, &self.file, self.offset);
    }
    fn node(&mut self, id: &mut NodeId) {
        *id = self.ids.node();
    }
    fn cell(&mut self, id: &mut CellId) {
        *id = self.ids.cell();
    }
}

struct Binding<'a> {
    file: SourceFile,
    offset: i128,
    nodes: &'a BTreeMap<NodeId, NodeId>,
    cells: &'a BTreeMap<CellId, CellId>,
}

impl Transform for Binding<'_> {
    fn origin(&mut self, origin: &mut Origin) {
        *origin = rebind_origin(origin, &self.file, self.offset);
    }
    fn span(&mut self, span: &mut SourceSpan) {
        *span = rebind_span(span, &self.file, self.offset);
    }
    fn node(&mut self, id: &mut NodeId) {
        *id = self.nodes[id];
    }
    fn cell(&mut self, id: &mut CellId) {
        *id = self.cells[id];
    }
}

pub(super) fn rebind_origin(origin: &Origin, file: &SourceFile, offset: i128) -> Origin {
    match origin {
        Origin::Source(span) => Origin::Source(rebind_span(span, file, offset)),
        Origin::Derived(derivation) => Origin::Derived(Arc::new(Derivation {
            operation: derivation.operation,
            span: derivation
                .span
                .as_ref()
                .map(|span| rebind_span(span, file, offset)),
            parent: rebind_origin(&derivation.parent, file, offset),
        })),
    }
}

fn rebind_span(span: &SourceSpan, file: &SourceFile, offset: i128) -> SourceSpan {
    let range = span.range();
    file.span(SourceRange {
        start: usize::try_from(range.start as i128 + offset).expect("rebased span start"),
        end: usize::try_from(range.end as i128 + offset).expect("rebased span end"),
    })
    .expect("rebased span must lie on source UTF-8 boundaries")
}
