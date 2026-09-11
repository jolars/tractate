//! The owned source model, independent of Panache and presentation backends.

#![allow(
    dead_code,
    reason = "Inspection uses only part of the source IR; later compiler passes consume the retained content."
)]

use super::{CellId, NodeId, Origin, SourceFile, SourceSpan};

mod mapping;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceDocument {
    pub source: SourceFile,
    pub origin: Origin,
    pub metadata: Vec<Metadata>,
    pub blocks: Vec<Block>,
}

impl SourceDocument {
    /// Visit semantic blocks in source order, including preserved containers.
    pub fn visit_blocks(&self, visitor: &mut impl FnMut(&Block)) {
        self.visit_nodes(visitor, &mut |_| {});
    }

    /// Visit blocks and inlines without flattening preserved syntax.
    pub fn visit_nodes(&self, blocks: &mut impl FnMut(&Block), inlines: &mut impl FnMut(&Inline)) {
        for block in &self.blocks {
            block.visit(blocks, inlines);
        }
    }
}

/// Sharing structure across layers prevents result placement from flattening
/// Markdown containers or maintaining a second, divergent Markdown vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block<C = Cell> {
    pub id: NodeId,
    pub origin: Origin,
    pub attributes: Vec<Attribute>,
    pub kind: BlockKind<C>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BlockKind<C = Cell> {
    Heading {
        level: usize,
        content: Vec<Inline<C>>,
    },
    Paragraph(Vec<Inline<C>>),
    Plain(Vec<Inline<C>>),
    List(List<C>),
    Quote(Vec<Block<C>>),
    Div(Vec<Block<C>>),
    Code(Code),
    Cell(C),
    Math(String),
    ThematicBreak,
    Raw(RawContent),
    Comment,
    ReferenceDefinition(PreservedSyntax<C>),
    FootnoteDefinition(PreservedSyntax<C>),
    Metadata(Metadata),
    Unsupported(PreservedSyntax<C>),
}

impl<C> Block<C> {
    pub fn visit(&self, visitor: &mut impl FnMut(&Block<C>), inlines: &mut impl FnMut(&Inline<C>)) {
        visitor(self);
        match &self.kind {
            BlockKind::Heading { content, .. }
            | BlockKind::Paragraph(content)
            | BlockKind::Plain(content) => {
                for inline in content {
                    inline.visit_blocks(visitor, inlines);
                }
            }
            BlockKind::List(list) => {
                for item in &list.items {
                    for block in &item.blocks {
                        block.visit(visitor, inlines);
                    }
                }
            }
            BlockKind::Quote(blocks) | BlockKind::Div(blocks) => {
                for block in blocks {
                    block.visit(visitor, inlines);
                }
            }
            BlockKind::ReferenceDefinition(node)
            | BlockKind::FootnoteDefinition(node)
            | BlockKind::Unsupported(node) => node.visit_blocks(visitor, inlines),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct List<C = Cell> {
    pub origin: Origin,
    pub kind: ListKind,
    pub loose: bool,
    pub items: Vec<ListItem<C>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListKind {
    Bullet,
    Ordered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListItem<C = Cell> {
    pub origin: Origin,
    /// Keeping each original marker also preserves nondecimal ordered lists.
    pub marker: Option<String>,
    pub checked: Option<bool>,
    pub blocks: Vec<Block<C>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Inline<C = Cell> {
    pub id: NodeId,
    pub origin: Origin,
    pub attributes: Vec<Attribute>,
    pub kind: InlineKind<C>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InlineKind<C = Cell> {
    Text(String),
    Space,
    NonbreakingSpace,
    SoftBreak,
    HardBreak,
    Emphasis(Vec<Inline<C>>),
    Strong(Vec<Inline<C>>),
    Code(String),
    Link(Link<C>),
    Image(Link<C>),
    Math(String),
    Span(Vec<Inline<C>>),
    /// Panache can embed display math within a paragraph, including prose.
    DisplayMath(String),
    Raw(RawContent),
    Comment,
    Unsupported(PreservedSyntax<C>),
}

impl<C> Inline<C> {
    fn visit_blocks(
        &self,
        visitor: &mut impl FnMut(&Block<C>),
        inlines: &mut impl FnMut(&Inline<C>),
    ) {
        inlines(self);
        match &self.kind {
            InlineKind::Emphasis(content)
            | InlineKind::Strong(content)
            | InlineKind::Span(content) => {
                for inline in content {
                    inline.visit_blocks(visitor, inlines);
                }
            }
            InlineKind::Link(link) | InlineKind::Image(link) => {
                for inline in &link.content {
                    inline.visit_blocks(visitor, inlines);
                }
            }
            InlineKind::Unsupported(node) => node.visit_blocks(visitor, inlines),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link<C = Cell> {
    pub origin: Origin,
    pub content: Vec<Inline<C>>,
    pub destination: Option<String>,
    pub title: Option<String>,
    /// Reference labels remain unresolved in the source model.
    pub reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attribute {
    pub origin: Origin,
    pub kind: AttributeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttributeKind {
    Identifier(String),
    Class(String),
    KeyValue { key: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawContent {
    pub origin: Origin,
    pub format: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Code {
    pub origin: Origin,
    pub language: Option<String>,
    pub source: String,
    /// Code can occupy discontiguous ranges inside quotes and lists.
    pub segments: Vec<SourceSpan>,
    pub info: Option<PreservedSyntax>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    pub id: CellId,
    pub origin: Origin,
    pub code: Code,
    pub options: Vec<CellOption>,
    /// Retain even malformed or nonmapping preambles for later diagnostics.
    pub preamble: Option<YamlValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CellOption {
    pub origin: Origin,
    pub source: OptionSource,
    pub key: Option<String>,
    pub key_span: Option<SourceSpan>,
    pub value_span: Option<SourceSpan>,
    pub value: Option<YamlValue>,
    pub inline_value: Option<String>,
    pub inline_quoted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptionSource {
    Inline,
    Hashpipe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub origin: Origin,
    pub value: YamlValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct YamlValue {
    pub origin: Origin,
    pub properties: Vec<YamlProperty>,
    pub kind: YamlKind,
}

impl YamlValue {
    pub fn mapping(&self) -> Option<&[YamlEntry]> {
        match &self.kind {
            YamlKind::Mapping { entries, .. } => Some(entries),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum YamlKind {
    Scalar { text: String, style: ScalarStyle },
    Mapping { flow: bool, entries: Vec<YamlEntry> },
    Sequence { flow: bool, items: Vec<YamlValue> },
    Empty,
    Unsupported(PreservedSyntax),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScalarStyle {
    Plain,
    SingleQuoted,
    DoubleQuoted,
    Literal,
    Folded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct YamlEntry {
    pub origin: Origin,
    pub key: YamlValue,
    pub value: YamlValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct YamlProperty {
    pub origin: Origin,
    pub name: String,
}

/// An unsupported construct keeps its original tree, with known descendants
/// lowered in place. Its origin retains the source for verbatim recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreservedSyntax<C = Cell> {
    pub origin: Origin,
    pub name: String,
    pub children: Vec<PreservedChild<C>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreservedChild<C = Cell> {
    Block(Box<Block<C>>),
    Inline(Box<Inline<C>>),
    Syntax(PreservedSyntax<C>),
}

impl<C> PreservedSyntax<C> {
    fn visit_blocks(
        &self,
        visitor: &mut impl FnMut(&Block<C>),
        inlines: &mut impl FnMut(&Inline<C>),
    ) {
        for child in &self.children {
            match child {
                PreservedChild::Block(block) => block.visit(visitor, inlines),
                PreservedChild::Inline(inline) => inline.visit_blocks(visitor, inlines),
                PreservedChild::Syntax(node) => node.visit_blocks(visitor, inlines),
            }
        }
    }
}
