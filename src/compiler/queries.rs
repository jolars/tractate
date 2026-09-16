//! Dependencies of pure queries within one immutable source revision.
//!
//! Keys use semantic handles, never source ranges. The graph records reads,
//! including reads made before a query fails. It does not perform effects or
//! establish identity or reuse between different revisions.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::document::{CellId, NodeId, SlideId};

use super::RenderOptions;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum QueryKey {
    Source,
    SourceDocument,
    Metadata,
    SemanticBlock(NodeId),
    SlideLayout,
    Slide(SlideId),
    Inspection,
    ReferenceIndex,
    Reference(String),
    DisplayDefaults,
    CellPolicy(CellId),
    HtmlTitle,
    HtmlSlide(SlideId),
    HtmlDeck(RenderOptions),
}

#[derive(Debug, Clone, Default)]
pub(super) struct DependencyGraph {
    edges: BTreeMap<QueryKey, BTreeSet<QueryKey>>,
}

impl DependencyGraph {
    pub fn query<T>(&mut self, key: QueryKey, evaluate: impl FnOnce(&Reads) -> T) -> T {
        let reads = Reads::default();
        let value = evaluate(&reads);
        self.finish(key, reads);
        value
    }

    pub fn finish(&mut self, key: QueryKey, reads: Reads) {
        self.edges.insert(key, reads.0.into_inner());
    }

    pub fn is_closed(&self) -> bool {
        self.edges
            .values()
            .flatten()
            .all(|input| self.edges.contains_key(input))
    }

    #[cfg(test)]
    pub fn dependents(&self, input: &QueryKey) -> BTreeSet<QueryKey> {
        let mut pending = vec![input.clone()];
        let mut affected = BTreeSet::new();
        while let Some(input) = pending.pop() {
            for (key, dependencies) in &self.edges {
                if dependencies.contains(&input) && affected.insert(key.clone()) {
                    pending.push(key.clone());
                }
            }
        }
        affected
    }

    #[cfg(test)]
    pub fn assert_complete(&self) {
        for (key, inputs) in &self.edges {
            for input in inputs {
                assert!(
                    self.edges.contains_key(input),
                    "{key:?} reads missing {input:?}"
                );
            }
            assert!(!self.dependents(key).contains(key), "cycle through {key:?}");
        }
    }
}

/// Read values through their producing query so the graph follows actual use.
/// Interior mutability lets independent renderer callbacks share one read set.
#[derive(Default)]
pub(super) struct Reads(RefCell<BTreeSet<QueryKey>>);

impl Reads {
    pub fn read<T>(&self, key: QueryKey, value: T) -> T {
        self.0.borrow_mut().insert(key);
        value
    }
}
