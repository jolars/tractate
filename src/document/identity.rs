//! Semantic identities, computation reuse keys, and immutable artifact addresses.

use std::cell::Cell;

/// A block or inline entity within a compilation's semantic identity scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NodeId(u64);

/// A slide entity, independent of its heading, position, or rendered bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SlideId(u64);

/// An executable cell entity, independent of its computation's reuse key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CellId(u64);

/// Allocates semantic handles without deriving them from source or output bytes.
///
/// Handles are unique within this allocator's scope. Fresh full builds start a
/// new scope; values from separate scopes must not be compared. Retaining IDs
/// across revisions will require label resolution and previous-revision matching.
#[derive(Debug, Default)]
pub(crate) struct SemanticIds {
    next: Cell<u64>,
}

impl SemanticIds {
    pub fn node(&self) -> NodeId {
        NodeId(self.allocate())
    }

    pub fn slide(&self) -> SlideId {
        SlideId(self.allocate())
    }

    pub fn cell(&self) -> CellId {
        CellId(self.allocate())
    }

    fn allocate(&self) -> u64 {
        let id = self.next.get();
        self.next.set(
            id.checked_add(1)
                .expect("semantic identity space exhausted"),
        );
        id
    }
}

/// A digest of computation inputs, used to decide whether a result is reusable.
///
/// The execution planner supplies the digest after encoding all relevant inputs.
/// There are no conversions from semantic IDs or artifact IDs.
#[allow(dead_code, reason = "Execution planning is a later roadmap stage.")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ExecutionKey([u8; 32]);

#[allow(dead_code, reason = "Execution planning is a later roadmap stage.")]
impl ExecutionKey {
    pub fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A digest addressing immutable output bytes, independent of their producer.
///
/// The artifact store supplies the digest after hashing the output bytes.
#[allow(dead_code, reason = "Artifact storage is a later roadmap stage.")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ArtifactId([u8; 32]);

#[allow(dead_code, reason = "Artifact storage is a later roadmap stage.")]
impl ArtifactId {
    pub fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::collections::{HashMap, HashSet};

    use super::*;

    #[test]
    fn identity_domains_are_distinct_types() {
        let types = [
            TypeId::of::<NodeId>(),
            TypeId::of::<SlideId>(),
            TypeId::of::<CellId>(),
            TypeId::of::<ExecutionKey>(),
            TypeId::of::<ArtifactId>(),
        ];
        assert_eq!(types.into_iter().collect::<HashSet<_>>().len(), 5);
    }

    #[test]
    fn identity_allocation_is_unique_within_each_semantic_domain() {
        let ids = SemanticIds::default();
        let mut nodes = HashSet::new();
        let mut slides = HashSet::new();
        let mut cells = HashSet::new();
        for _ in 0..100 {
            assert!(nodes.insert(ids.node()));
            assert!(slides.insert(ids.slide()));
            assert!(cells.insert(ids.cell()));
        }
    }

    #[test]
    fn identity_keys_allow_reuse_independently_of_semantic_ids() {
        let ids = SemanticIds::default();
        let first = ids.cell();
        let second = ids.cell();
        let key = ExecutionKey::from_digest([1; 32]);
        let artifact = ArtifactId::from_digest([2; 32]);
        let desired = HashMap::from([(first, key), (second, key)]);
        let results = HashMap::from([(key, artifact)]);

        assert_ne!(first, second);
        assert_eq!(results[&desired[&first]], results[&desired[&second]]);
        assert_eq!(key.digest(), &[1; 32]);
        assert_eq!(artifact.digest(), &[2; 32]);
        assert_eq!(key, ExecutionKey::from_digest([1; 32]));
        assert_eq!(artifact, ArtifactId::from_digest([2; 32]));
        assert_ne!(key, ExecutionKey::from_digest([3; 32]));
        assert_ne!(artifact, ArtifactId::from_digest([3; 32]));
    }
}
