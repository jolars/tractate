//! Preserve declarations without applying Tractate's option contract yet.

use panache_parser::syntax::{self as cst, SyntaxKind};

use super::lowering::{Lowerer, range};
use crate::document::*;

impl Lowerer {
    pub(super) fn yaml(&self, node: &cst::SyntaxNode) -> YamlValue {
        let source_range = range(node.text_range());
        let mut origin = self.origin(source_range);
        let mut properties: Vec<_> = node
            .children_with_tokens()
            .filter(|element| {
                matches!(
                    element.kind(),
                    SyntaxKind::YAML_TAG | SyntaxKind::YAML_ANCHOR | SyntaxKind::YAML_ALIAS
                )
            })
            .map(|element| YamlProperty {
                origin: self.origin(range(element.text_range())),
                name: format!("{:?}", element.kind()),
            })
            .collect();

        let kind = match node.kind() {
            SyntaxKind::YAML_SCALAR => {
                let scalar = cst::YamlScalar::cast(node.clone()).expect("scalar node");
                let style = match scalar.style() {
                    cst::YamlScalarStyle::Plain => ScalarStyle::Plain,
                    cst::YamlScalarStyle::SingleQuoted => ScalarStyle::SingleQuoted,
                    cst::YamlScalarStyle::DoubleQuoted => ScalarStyle::DoubleQuoted,
                    cst::YamlScalarStyle::Literal => ScalarStyle::Literal,
                    cst::YamlScalarStyle::Folded => ScalarStyle::Folded,
                };
                YamlKind::Scalar {
                    text: scalar.value(),
                    style,
                }
            }
            SyntaxKind::YAML_BLOCK_MAP | SyntaxKind::YAML_FLOW_MAP => {
                let entries = node
                    .children()
                    .filter(|child| {
                        matches!(
                            child.kind(),
                            SyntaxKind::YAML_BLOCK_MAP_ENTRY | SyntaxKind::YAML_FLOW_MAP_ENTRY
                        )
                    })
                    .map(|entry| {
                        let key = entry.children().find(|child| {
                            matches!(
                                child.kind(),
                                SyntaxKind::YAML_BLOCK_MAP_KEY | SyntaxKind::YAML_FLOW_MAP_KEY
                            )
                        });
                        let value = entry.children().find(|child| {
                            matches!(
                                child.kind(),
                                SyntaxKind::YAML_BLOCK_MAP_VALUE | SyntaxKind::YAML_FLOW_MAP_VALUE
                            )
                        });
                        YamlEntry {
                            origin: self.origin(range(entry.text_range())),
                            key: key
                                .as_ref()
                                .map(|node| self.yaml(node))
                                .unwrap_or_else(|| self.empty(range(entry.text_range()))),
                            value: value.as_ref().map(|node| self.yaml(node)).unwrap_or_else(
                                || {
                                    self.empty(SourceRange {
                                        start: usize::from(entry.text_range().end()),
                                        end: usize::from(entry.text_range().end()),
                                    })
                                },
                            ),
                        }
                    })
                    .collect();
                YamlKind::Mapping {
                    flow: node.kind() == SyntaxKind::YAML_FLOW_MAP,
                    entries,
                }
            }
            SyntaxKind::YAML_BLOCK_SEQUENCE | SyntaxKind::YAML_FLOW_SEQUENCE => {
                let items = node
                    .children()
                    .filter(|child| {
                        matches!(
                            child.kind(),
                            SyntaxKind::YAML_BLOCK_SEQUENCE_ITEM
                                | SyntaxKind::YAML_FLOW_SEQUENCE_ITEM
                        )
                    })
                    .map(|item| self.yaml(&item))
                    .collect();
                YamlKind::Sequence {
                    flow: node.kind() == SyntaxKind::YAML_FLOW_SEQUENCE,
                    items,
                }
            }
            _ => {
                let content = node.children().find(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::YAML_METADATA_CONTENT
                            | SyntaxKind::HASHPIPE_YAML_CONTENT
                            | SyntaxKind::YAML_DOCUMENT
                            | SyntaxKind::YAML_STREAM
                            | SyntaxKind::YAML_BLOCK_MAP
                            | SyntaxKind::YAML_FLOW_MAP
                            | SyntaxKind::YAML_BLOCK_SEQUENCE
                            | SyntaxKind::YAML_FLOW_SEQUENCE
                            | SyntaxKind::YAML_SCALAR
                    )
                });
                if let Some(content) = content {
                    let value = self.yaml(&content);
                    origin = value.origin;
                    properties.extend(value.properties);
                    value.kind
                } else if node.children_with_tokens().all(|element| {
                    matches!(
                        element.kind(),
                        SyntaxKind::WHITESPACE
                            | SyntaxKind::NEWLINE
                            | SyntaxKind::YAML_COMMENT
                            | SyntaxKind::YAML_LINE_PREFIX
                            | SyntaxKind::LINE_PREFIX
                            | SyntaxKind::YAML_COLON
                    )
                }) {
                    YamlKind::Empty
                } else {
                    YamlKind::Unsupported(self.preserve(node.clone().into()))
                }
            }
        };
        YamlValue {
            origin,
            properties,
            kind,
        }
    }

    fn empty(&self, range: SourceRange) -> YamlValue {
        YamlValue {
            origin: self.origin(range),
            properties: Vec::new(),
            kind: YamlKind::Empty,
        }
    }
}
