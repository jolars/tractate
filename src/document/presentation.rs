//! Backend-independent presentations with explicit placement for cell results.

use super::{Block, Cell, CellId, CellOption, Code, Metadata, Origin, SlideId, YamlValue};

/// Slide grouping before executable cells acquire presentation result slots.
pub(crate) type SourcePresentation = Presentation<Cell>;

/// An owned presentation tree. The cell parameter separates source grouping
/// from result placement while sharing the complete Markdown structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Presentation<C = PresentedCell> {
    pub origin: Origin,
    pub metadata: Vec<Metadata>,
    /// Retain leading comments and definitions even when there are no slides.
    pub preamble: Vec<Block<C>>,
    pub slides: Vec<Slide<C>>,
}

impl<C> Presentation<C> {
    /// Visit blocks in presentation order, including definitions and containers.
    pub fn visit_blocks(&self, visitor: &mut impl FnMut(&Block<C>)) {
        for block in &self.preamble {
            block.visit(visitor, &mut |_| {});
        }
        for slide in &self.slides {
            if let SlideKind::Content(blocks) = &slide.kind {
                for block in blocks {
                    block.visit(visitor, &mut |_| {});
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slide<C = PresentedCell> {
    pub id: SlideId,
    /// Title metadata or the first body block initiates this derived node.
    pub origin: Origin,
    pub kind: SlideKind<C>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlideKind<C = PresentedCell> {
    /// Keep the YAML declaration and its origin until metadata is interpreted.
    Title(YamlValue),
    /// A boundary heading belongs to its slide, with its attributes and inlines.
    Content(Vec<Block<C>>),
}

/// Source display and potential result placement, independent of execution.
///
/// Declarations remain available for the later pure option-resolution pass.
/// A slot alone does not authorize execution or require a computation result:
/// `eval`, `include`, and the evaluation plan determine whether it is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PresentedCell {
    pub id: CellId,
    pub origin: Origin,
    pub source: Code,
    pub options: Vec<CellOption>,
    pub preamble: Option<YamlValue>,
    pub result: ResultSlot,
}

/// A position after source display for a cell's ordered output, resolved through
/// an external result store. The planner must validate its execution key before
/// supplying output; semantic identity alone never establishes result validity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResultSlot {
    pub cell: CellId,
    pub origin: Origin,
}
