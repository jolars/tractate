//! Label identities and one-to-one matching against the preceding revision.

use super::*;
use crate::compiler::validation::LabelDeclaration;

/// `None` requires an anonymous handle for a nonunique label. The distinction
/// prevents a reused template from retaining a formerly unique label identity.
#[derive(Default)]
pub(super) struct Identities {
    pub nodes: BTreeMap<NodeId, Option<NodeId>>,
    pub cells: BTreeMap<CellId, Option<CellId>>,
}

impl Identities {
    pub fn local(&self, block: &Block) -> Self {
        let mut local = Local {
            ids: SemanticIds::default(),
            assigned: self,
            result: Self::default(),
        };
        block.clone().transform(&mut local);
        local.result
    }

    pub fn matches(&self, candidate: &SemanticNode) -> bool {
        self.nodes.iter().all(|(local, desired)| {
            desired.map_or_else(
                || !candidate.nodes[local].is_labeled(),
                |desired| candidate.nodes[local] == desired,
            )
        }) && self.cells.iter().all(|(local, desired)| {
            desired.map_or_else(
                || !candidate.cells[local].is_labeled(),
                |desired| candidate.cells[local] == desired,
            )
        })
    }
}

pub(super) fn match_identities(
    blocks: &[Block],
    previous: &[Arc<SemanticNode>],
    labels: &[LabelDeclaration],
) -> Identities {
    let mut assigned = Identities::default();
    let mut counts = BTreeMap::new();
    for label in labels {
        *counts.entry(&label.label).or_insert(0) += 1;
    }
    for label in labels {
        let unique = counts[&label.label] == 1;
        assigned
            .nodes
            .entry(label.node)
            .or_insert_with(|| unique.then(|| NodeId::labeled(&label.label)));
        if let Some(cell) = label.cell {
            assigned
                .cells
                .entry(cell)
                .or_insert_with(|| unique.then(|| CellId::labeled(&label.label)));
        }
    }

    // Cell matching is independent of its container template, so editing a
    // surrounding quote or list does not discard an unchanged cell's handle.
    let mut available: BTreeMap<String, Vec<CellCandidate>> = BTreeMap::new();
    for template in previous {
        template.source.visit(
            &mut |block| {
                let BlockKind::Cell(cell) = &block.kind else {
                    return;
                };
                let cell_id = template.cells[&cell.id];
                if cell_id.is_labeled() {
                    return;
                }
                let text = block.origin.source_span().text().to_owned();
                let source = normalize(block, SourceFile::anonymous(&text));
                available.entry(text).or_default().push(CellCandidate {
                    source,
                    node: template.nodes[&block.id],
                    cell: cell_id,
                });
            },
            &mut |_| {},
        );
    }
    for block in blocks {
        block.visit(
            &mut |block| {
                let BlockKind::Cell(cell) = &block.kind else {
                    return;
                };
                if assigned.cells.get(&cell.id).is_some_and(Option::is_some) {
                    return;
                }
                let Some(candidates) = available.get_mut(block.origin.source_span().text()) else {
                    return;
                };
                let matched = candidates.iter().position(|candidate| {
                    normalize(block, candidate.source.origin.source_span().file().clone())
                        == candidate.source
                });
                if let Some(index) = matched {
                    let candidate = candidates.remove(index);
                    let node = assigned.nodes.entry(block.id).or_default();
                    if node.is_none() && !candidate.node.is_labeled() {
                        *node = Some(candidate.node);
                    }
                    assigned.cells.insert(cell.id, Some(candidate.cell));
                }
            },
            &mut |_| {},
        );
    }
    assigned
}

struct CellCandidate {
    source: Block,
    node: NodeId,
    cell: CellId,
}

struct Local<'a> {
    ids: SemanticIds,
    assigned: &'a Identities,
    result: Identities,
}

impl Transform for Local<'_> {
    fn origin(&mut self, _: &mut Origin) {}
    fn span(&mut self, _: &mut SourceSpan) {}
    fn node(&mut self, id: &mut NodeId) {
        let local = self.ids.node();
        if let Some(desired) = self.assigned.nodes.get(id) {
            self.result.nodes.insert(local, *desired);
        }
    }
    fn cell(&mut self, id: &mut CellId) {
        let local = self.ids.cell();
        if let Some(desired) = self.assigned.cells.get(id) {
            self.result.cells.insert(local, *desired);
        }
    }
}
