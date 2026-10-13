//! Transform provenance and handles without changing semantic content.

use super::*;
use crate::document::{PresentedCell, ResultSlot};

pub(crate) trait Transform {
    fn origin(&mut self, origin: &mut Origin);
    fn span(&mut self, span: &mut SourceSpan);
    fn node(&mut self, id: &mut NodeId);
    fn cell(&mut self, id: &mut CellId);
}

pub(crate) trait TransformTree {
    fn transform(&mut self, map: &mut impl Transform);
}

impl<T: TransformTree> TransformTree for Vec<T> {
    fn transform(&mut self, map: &mut impl Transform) {
        for value in self {
            value.transform(map);
        }
    }
}

impl<T: TransformTree> TransformTree for Option<T> {
    fn transform(&mut self, map: &mut impl Transform) {
        if let Some(value) = self {
            value.transform(map);
        }
    }
}

impl TransformTree for Origin {
    fn transform(&mut self, map: &mut impl Transform) {
        map.origin(self);
    }
}

impl TransformTree for SourceSpan {
    fn transform(&mut self, map: &mut impl Transform) {
        map.span(self);
    }
}

macro_rules! tree {
    ($name:ident $(<$cell:ident>)? { $($field:ident),* $(,)? }) => {
        impl $(<$cell: TransformTree>)? TransformTree for $name $(<$cell>)? {
            fn transform(&mut self, map: &mut impl Transform) {
                $(self.$field.transform(map);)*
            }
        }
    };
}

tree!(Attribute { origin });
tree!(RawContent { origin });
tree!(Code {
    origin,
    segments,
    info
});
tree!(CellOption {
    origin,
    key_span,
    value_span,
    value
});
tree!(Metadata { origin, value });
tree!(YamlValue {
    origin,
    properties,
    kind
});
tree!(YamlEntry { origin, key, value });
tree!(YamlProperty { origin });
tree!(List<C> { origin, items });
tree!(ListItem<C> { origin, blocks });
tree!(Link<C> { origin, content });
tree!(ReferenceDefinition<C> { syntax });
tree!(PreservedSyntax<C> { origin, children });

impl<C: TransformTree> TransformTree for Block<C> {
    fn transform(&mut self, map: &mut impl Transform) {
        map.node(&mut self.id);
        self.origin.transform(map);
        self.attributes.transform(map);
        self.kind.transform(map);
    }
}

impl<C: TransformTree> TransformTree for Inline<C> {
    fn transform(&mut self, map: &mut impl Transform) {
        map.node(&mut self.id);
        self.origin.transform(map);
        self.attributes.transform(map);
        match &mut self.kind {
            InlineKind::Emphasis(v) | InlineKind::Strong(v) | InlineKind::Span(v) => {
                v.transform(map)
            }
            InlineKind::Link(v) | InlineKind::Image(v) => v.transform(map),
            InlineKind::Raw(v) => v.transform(map),
            InlineKind::Unsupported(v) => v.transform(map),
            _ => {}
        }
    }
}

impl<C: TransformTree> TransformTree for BlockKind<C> {
    fn transform(&mut self, map: &mut impl Transform) {
        match self {
            Self::Heading { content, .. }
            | Self::Paragraph(content)
            | Self::Plain(content)
            | Self::Figure(content) => content.transform(map),
            Self::List(v) => v.transform(map),
            Self::Quote(v) | Self::Div(v) | Self::Html(v) => v.transform(map),
            Self::Code(v) => v.transform(map),
            Self::Cell(v) => v.transform(map),
            Self::Raw(v) => v.transform(map),
            Self::ReferenceDefinition(v) => v.transform(map),
            Self::FootnoteDefinition(v) | Self::Unsupported(v) => v.transform(map),
            Self::Metadata(v) => v.transform(map),
            _ => {}
        }
    }
}

impl<C: TransformTree> TransformTree for PreservedChild<C> {
    fn transform(&mut self, map: &mut impl Transform) {
        match self {
            Self::Block(v) => v.transform(map),
            Self::Inline(v) => v.transform(map),
            Self::Syntax(v) => v.transform(map),
        }
    }
}

impl TransformTree for YamlKind {
    fn transform(&mut self, map: &mut impl Transform) {
        match self {
            Self::Mapping { entries, .. } => entries.transform(map),
            Self::Sequence { items, .. } => items.transform(map),
            Self::Unsupported(v) => v.transform(map),
            _ => {}
        }
    }
}

impl TransformTree for Cell {
    fn transform(&mut self, map: &mut impl Transform) {
        map.cell(&mut self.id);
        self.origin.transform(map);
        self.code.transform(map);
        self.options.transform(map);
        self.preamble.transform(map);
    }
}

impl TransformTree for PresentedCell {
    fn transform(&mut self, map: &mut impl Transform) {
        map.cell(&mut self.id);
        self.origin.transform(map);
        self.source.transform(map);
        self.options.transform(map);
        self.preamble.transform(map);
        self.result.transform(map);
    }
}

impl TransformTree for ResultSlot {
    fn transform(&mut self, map: &mut impl Transform) {
        map.cell(&mut self.cell);
        self.origin.transform(map);
    }
}
