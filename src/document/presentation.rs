//! Source presentations with slide boundaries resolved independently of a backend.

use super::{Block, Metadata, Origin, YamlValue};

/// An owned grouping of source content, before result placement or rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Presentation {
    pub origin: Origin,
    pub metadata: Vec<Metadata>,
    /// Retain leading comments and definitions even when there are no slides.
    pub preamble: Vec<Block>,
    pub slides: Vec<Slide>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slide {
    /// Title metadata or the first body block initiates this derived node.
    pub origin: Origin,
    pub kind: SlideKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlideKind {
    /// Keep the YAML declaration and its origin until metadata is interpreted.
    Title(YamlValue),
    /// A boundary heading belongs to its slide, with its attributes and inlines.
    Content(Vec<Block>),
}
