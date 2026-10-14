//! Backend-independent slide differences between adjacent source revisions.

use std::collections::{BTreeMap, BTreeSet};

use crate::document::SlideId;

use super::{CompilerSnapshot, SourceRevision, reuse};

/// A slide and its zero-based position in the indicated revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlidePosition {
    pub id: SlideId,
    pub index: usize,
}

/// A retained slide whose order relative to other retained slides changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlideMove {
    pub id: SlideId,
    /// Position in the previous revision, including slides subsequently removed.
    pub from: usize,
    /// Position in the new revision, including newly inserted slides.
    pub to: usize,
}

/// Semantic slide changes from one source revision to the next.
///
/// Insertions and removals use positions in the new and previous revision,
/// respectively. Reordering compares ranks among retained slides, so an
/// insertion or removal alone does not report shifted slides as reordered.
/// Every retained slide whose rank changes is reported, in new slide order.
/// A slide may be both reordered and modified.
///
/// Modification compares title scalars or the slide's ordered semantic block
/// templates, excluding source paths, offsets, and gaps between blocks. It does
/// not describe backend output invalidation from document defaults, reference
/// definitions elsewhere, themes, or computation results. Labeled boundaries
/// retain deterministic identities through edits. The matcher may assign a new
/// handle to an edited anonymous boundary; such a slide is removed and inserted
/// rather than modified. Repeated identical anonymous boundaries match in
/// occurrence order, so their identities are best effort.
///
/// These lists describe differences, not sequential patch instructions. Use
/// `order` to reconstruct the complete new order. Anonymous handles belong to
/// the owning compiler; labeled handles are deterministic within the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideChangeSet {
    pub inserted: Vec<SlidePosition>,
    pub removed: Vec<SlidePosition>,
    pub reordered: Vec<SlideMove>,
    /// Retained slides with changed content, at their new positions.
    pub modified: Vec<SlidePosition>,
    /// All slide handles in the new revision's presentation order.
    pub order: Vec<SlideId>,
}

impl SlideChangeSet {
    /// Whether the slides have identical identities, order, and content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty()
            && self.removed.is_empty()
            && self.reordered.is_empty()
            && self.modified.is_empty()
    }

    pub(super) fn unchanged(snapshot: &CompilerSnapshot) -> Self {
        Self {
            inserted: Vec::new(),
            removed: Vec::new(),
            reordered: Vec::new(),
            modified: Vec::new(),
            order: snapshot.slide_ids().collect(),
        }
    }

    pub(super) fn between(previous: &CompilerSnapshot, next: &CompilerSnapshot) -> Self {
        let mut changes = Self::unchanged(next);
        let old: BTreeMap<_, _> = previous
            .slide_ids()
            .enumerate()
            .map(|(i, id)| (id, i))
            .collect();
        let new: BTreeSet<_> = next.slide_ids().collect();
        let ranks: BTreeMap<_, _> = previous
            .slide_ids()
            .filter(|id| new.contains(id))
            .enumerate()
            .map(|(rank, id)| (id, rank))
            .collect();
        for (index, id) in previous.slide_ids().enumerate() {
            if !new.contains(&id) {
                changes.removed.push(SlidePosition { id, index });
            }
        }
        let mut rank = 0;
        for (index, slide) in next.presentation().slides.iter().enumerate() {
            let id = slide.id;
            let Some(&from) = old.get(&id) else {
                changes.inserted.push(SlidePosition { id, index });
                continue;
            };
            if ranks[&id] != rank {
                changes.reordered.push(SlideMove {
                    id,
                    from,
                    to: index,
                });
            }
            rank += 1;
            if reuse::slide_input(previous, &previous.presentation().slides[from])
                != reuse::slide_input(next, slide)
            {
                changes.modified.push(SlidePosition { id, index });
            }
        }
        changes
    }
}

/// The revision and slide differences returned by a source update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUpdate {
    pub previous_revision: SourceRevision,
    pub revision: SourceRevision,
    pub changes: SlideChangeSet,
}
