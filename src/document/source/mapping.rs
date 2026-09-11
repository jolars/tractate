//! Change cell leaves while preserving all surrounding Markdown structure.

use super::*;

impl<C> Block<C> {
    pub fn map_cells<D>(self, map: &mut impl FnMut(C) -> D) -> Block<D> {
        let kind = match self.kind {
            BlockKind::Heading { level, content } => BlockKind::Heading {
                level,
                content: map_inlines(content, map),
            },
            BlockKind::Paragraph(content) => BlockKind::Paragraph(map_inlines(content, map)),
            BlockKind::Plain(content) => BlockKind::Plain(map_inlines(content, map)),
            BlockKind::List(list) => BlockKind::List(List {
                origin: list.origin,
                kind: list.kind,
                loose: list.loose,
                items: list
                    .items
                    .into_iter()
                    .map(|item| ListItem {
                        origin: item.origin,
                        marker: item.marker,
                        checked: item.checked,
                        blocks: map_blocks(item.blocks, map),
                    })
                    .collect(),
            }),
            BlockKind::Quote(blocks) => BlockKind::Quote(map_blocks(blocks, map)),
            BlockKind::Div(blocks) => BlockKind::Div(map_blocks(blocks, map)),
            BlockKind::Code(code) => BlockKind::Code(code),
            BlockKind::Cell(cell) => BlockKind::Cell(map(cell)),
            BlockKind::Math(text) => BlockKind::Math(text),
            BlockKind::ThematicBreak => BlockKind::ThematicBreak,
            BlockKind::Raw(raw) => BlockKind::Raw(raw),
            BlockKind::Comment => BlockKind::Comment,
            BlockKind::ReferenceDefinition(node) => {
                BlockKind::ReferenceDefinition(node.map_cells(map))
            }
            BlockKind::FootnoteDefinition(node) => {
                BlockKind::FootnoteDefinition(node.map_cells(map))
            }
            BlockKind::Metadata(metadata) => BlockKind::Metadata(metadata),
            BlockKind::Unsupported(node) => BlockKind::Unsupported(node.map_cells(map)),
        };
        Block {
            id: self.id,
            origin: self.origin,
            attributes: self.attributes,
            kind,
        }
    }
}

fn map_blocks<C, D>(blocks: Vec<Block<C>>, map: &mut impl FnMut(C) -> D) -> Vec<Block<D>> {
    blocks
        .into_iter()
        .map(|block| block.map_cells(map))
        .collect()
}

fn map_inlines<C, D>(inlines: Vec<Inline<C>>, map: &mut impl FnMut(C) -> D) -> Vec<Inline<D>> {
    inlines
        .into_iter()
        .map(|inline| inline.map_cells(map))
        .collect()
}

impl<C> Inline<C> {
    fn map_cells<D>(self, map: &mut impl FnMut(C) -> D) -> Inline<D> {
        let kind = match self.kind {
            InlineKind::Text(text) => InlineKind::Text(text),
            InlineKind::Space => InlineKind::Space,
            InlineKind::NonbreakingSpace => InlineKind::NonbreakingSpace,
            InlineKind::SoftBreak => InlineKind::SoftBreak,
            InlineKind::HardBreak => InlineKind::HardBreak,
            InlineKind::Emphasis(content) => InlineKind::Emphasis(map_inlines(content, map)),
            InlineKind::Strong(content) => InlineKind::Strong(map_inlines(content, map)),
            InlineKind::Code(code) => InlineKind::Code(code),
            InlineKind::Link(link) => InlineKind::Link(link.map_cells(map)),
            InlineKind::Image(link) => InlineKind::Image(link.map_cells(map)),
            InlineKind::Math(text) => InlineKind::Math(text),
            InlineKind::Span(content) => InlineKind::Span(map_inlines(content, map)),
            InlineKind::DisplayMath(text) => InlineKind::DisplayMath(text),
            InlineKind::Raw(raw) => InlineKind::Raw(raw),
            InlineKind::Comment => InlineKind::Comment,
            InlineKind::Unsupported(node) => InlineKind::Unsupported(node.map_cells(map)),
        };
        Inline {
            id: self.id,
            origin: self.origin,
            attributes: self.attributes,
            kind,
        }
    }
}

impl<C> Link<C> {
    fn map_cells<D>(self, map: &mut impl FnMut(C) -> D) -> Link<D> {
        Link {
            origin: self.origin,
            content: map_inlines(self.content, map),
            destination: self.destination,
            title: self.title,
            reference: self.reference,
        }
    }
}

impl<C> PreservedSyntax<C> {
    fn map_cells<D>(self, map: &mut impl FnMut(C) -> D) -> PreservedSyntax<D> {
        PreservedSyntax {
            origin: self.origin,
            name: self.name,
            children: self
                .children
                .into_iter()
                .map(|child| match child {
                    PreservedChild::Block(block) => {
                        PreservedChild::Block(Box::new(block.map_cells(map)))
                    }
                    PreservedChild::Inline(inline) => {
                        PreservedChild::Inline(Box::new(inline.map_cells(map)))
                    }
                    PreservedChild::Syntax(node) => PreservedChild::Syntax(node.map_cells(map)),
                })
                .collect(),
        }
    }
}
