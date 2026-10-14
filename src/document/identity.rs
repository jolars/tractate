//! Semantic identities, computation reuse keys, and immutable artifact addresses.

use std::cell::Cell;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Tagged domains keep deterministic labels separate from allocated handles.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(untagged)]
enum SemanticId {
    Anonymous(u64),
    Label([u8; 32]),
}

impl SemanticId {
    fn label(label: &str) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"tractate-semantic-label-v1\0");
        hash.update(label.as_bytes());
        Self::Label(hash.finalize().into())
    }

    fn is_labeled(self) -> bool {
        matches!(self, Self::Label(_))
    }
}

/// A block or inline entity within a compilation's semantic identity scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NodeId(SemanticId);

impl NodeId {
    pub fn labeled(label: &str) -> Self {
        Self(SemanticId::label(label))
    }

    pub fn is_labeled(self) -> bool {
        self.0.is_labeled()
    }
}

/// A slide entity, independent of its heading, position, or rendered bytes.
///
/// Labeled handles are deterministic across builds of the same document.
/// Compare anonymous handles only within one compiler instance; removed
/// anonymous handles are never reassigned. Labels are document scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlideId(SemanticId);

impl SlideId {
    pub(crate) fn labeled(label: &str) -> Self {
        Self(SemanticId::label(label))
    }
}

impl fmt::Display for SlideId {
    /// Format the semantic handle within its compilation scope.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            SemanticId::Anonymous(id) => id.fmt(formatter),
            SemanticId::Label(digest) => {
                formatter.write_str("label-")?;
                for byte in digest {
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    }
}

/// An executable cell entity, independent of its computation's reuse key.
///
/// Labels give deterministic, document-scoped identities across full builds.
/// Anonymous identities are matched only against the preceding revision of
/// their owning compiler. Neither kind establishes computation validity.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellId(SemanticId);

impl CellId {
    pub(crate) fn labeled(label: &str) -> Self {
        Self(SemanticId::label(label))
    }

    pub(crate) fn is_labeled(self) -> bool {
        self.0.is_labeled()
    }
}

/// Allocates anonymous semantic handles independently of source or output bytes.
///
/// Handles are unique within this allocator's scope. Fresh full builds start a
/// new scope; values from separate scopes must not be compared. Retaining IDs
/// across revisions uses the compiler's previous-revision matcher.
#[derive(Debug, Default)]
pub(crate) struct SemanticIds {
    next: Cell<u64>,
}

impl SemanticIds {
    pub fn starting_at(next: u64) -> Self {
        Self {
            next: Cell::new(next),
        }
    }

    pub fn next(&self) -> u64 {
        self.next.get()
    }

    pub fn node(&self) -> NodeId {
        NodeId(SemanticId::Anonymous(self.allocate()))
    }

    pub fn slide(&self) -> SlideId {
        SlideId(SemanticId::Anonymous(self.allocate()))
    }

    pub fn cell(&self) -> CellId {
        CellId(SemanticId::Anonymous(self.allocate()))
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
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutionKey([u8; 32]);

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

    #[test]
    fn identity_labels_are_deterministic_and_separate_from_anonymous_handles() {
        let ids = SemanticIds::default();
        let node = NodeId::labeled("name");
        let cell = CellId::labeled("name");
        assert_eq!(node, NodeId::labeled("name"));
        assert_eq!(cell, CellId::labeled("name"));
        assert_ne!(node, NodeId::labeled("Name"));
        assert_ne!(node, ids.node());
        assert_ne!(cell, ids.cell());
        assert_ne!(SlideId::labeled("name"), ids.slide());
        assert_eq!(
            SlideId::labeled("name").to_string(),
            "label-f46909ebb351d04d9814f437e2f19b10320f9b1d3f12d389dc81d7e2eb480682"
        );
    }

    #[test]
    fn identity_cell_handles_round_trip_in_execution_records() {
        for cell in [SemanticIds::default().cell(), CellId::labeled("café")] {
            let encoded = serde_json::to_string(&cell).unwrap();
            assert_eq!(serde_json::from_str::<CellId>(&encoded).unwrap(), cell);
        }
        assert_eq!(
            serde_json::from_str::<CellId>("0").unwrap(),
            SemanticIds::default().cell()
        );
    }
}
