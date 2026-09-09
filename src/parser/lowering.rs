use panache_parser::syntax::{self as cst, AstNode, BlockNode, InlineNode, SyntaxKind};

use crate::document::*;

pub(crate) struct LoweredSource {
    pub document: SourceDocument,
    pub parse_errors: usize,
}

pub(crate) fn lower(source: &str) -> LoweredSource {
    let parsed = super::parse(source);
    let mut document = SourceDocument {
        source: source.to_owned(),
        metadata: Vec::new(),
        blocks: Vec::new(),
    };
    for block in blocks(parsed.document().block_nodes()) {
        match block.kind {
            BlockKind::Metadata(metadata) => document.metadata.push(metadata),
            _ => document.blocks.push(block),
        }
    }
    LoweredSource {
        document,
        parse_errors: parsed.errors().len(),
    }
}

pub(super) fn range(range: cst::TextRange) -> SourceRange {
    SourceRange {
        start: range.start().into(),
        end: range.end().into(),
    }
}

fn blocks(nodes: impl IntoIterator<Item = BlockNode>) -> Vec<Block> {
    nodes.into_iter().filter_map(block).collect()
}

fn inlines(nodes: impl IntoIterator<Item = InlineNode>) -> Vec<Inline> {
    nodes.into_iter().filter_map(inline).collect()
}

fn block(node: BlockNode) -> Option<Block> {
    let source_range = range(node.text_range());
    let mut attributes = Vec::new();
    let kind = match node {
        BlockNode::Trivia(_) => return None,
        BlockNode::Heading(node) => {
            attributes = attrs(node.attributes());
            BlockKind::Heading {
                level: node.level(),
                content: inlines(node.inline_nodes()),
            }
        }
        BlockNode::Paragraph(node) => BlockKind::Paragraph(inlines(node.inline_nodes())),
        BlockNode::Plain(node) => BlockKind::Plain(inlines(node.inline_nodes())),
        BlockNode::BlockQuote(node) => BlockKind::Quote(blocks(node.block_nodes())),
        BlockNode::FencedDiv(node) => {
            attributes = attrs(node.attributes());
            BlockKind::Div(blocks(node.block_nodes()))
        }
        BlockNode::List(node) => {
            let items: Vec<_> = node
                .items()
                .map(|item| ListItem {
                    range: range(item.syntax().text_range()),
                    marker: item.marker(),
                    checked: item.task_checked(),
                    blocks: blocks(item.block_nodes()),
                })
                .collect();
            let ordered = items
                .first()
                .and_then(|item| item.marker.as_deref())
                .is_some_and(|marker| !matches!(marker, "-" | "+" | "*"));
            BlockKind::List(List {
                kind: if ordered {
                    ListKind::Ordered
                } else {
                    ListKind::Bullet
                },
                loose: node.is_loose()
                    || node.items().any(|item| {
                        item.is_loose()
                            || item
                                .syntax()
                                .children()
                                .any(|child| child.kind() == SyntaxKind::BLANK_LINE)
                    }),
                items,
            })
        }
        BlockNode::CodeBlock(node) => {
            attributes = node
                .info()
                .map(|info| syntax_attrs(info.syntax()))
                .unwrap_or_default();
            code_block(node)
        }
        BlockNode::DisplayMath(node) => {
            attributes = node
                .syntax()
                .children()
                .filter_map(cst::AttributeNode::cast)
                .flat_map(|node| attrs(Some(node)))
                .collect();
            BlockKind::Math(node.content())
        }
        BlockNode::ThematicBreak(_) => BlockKind::ThematicBreak,
        BlockNode::YamlMetadata(node) => BlockKind::Metadata(Metadata {
            range: source_range,
            value: super::yaml::lower(node.syntax()),
        }),
        BlockNode::ReferenceDefinition(node) => {
            BlockKind::ReferenceDefinition(preserve(node.syntax().clone().into()))
        }
        BlockNode::FootnoteDefinition(node) => {
            BlockKind::FootnoteDefinition(preserve(node.syntax().clone().into()))
        }
        BlockNode::TexBlock(node) => BlockKind::Raw(RawContent {
            format: "tex".into(),
            text: node.text(),
        }),
        BlockNode::Unknown(node) => match node.syntax_kind() {
            SyntaxKind::COMMENT => BlockKind::Comment,
            SyntaxKind::HTML_BLOCK_RAW | SyntaxKind::INLINE_HTML => {
                let text = node.source_text();
                if text.trim_start().starts_with("<!--") {
                    BlockKind::Comment
                } else {
                    BlockKind::Raw(RawContent {
                        format: "html".into(),
                        text,
                    })
                }
            }
            _ => BlockKind::Unsupported(preserve(node.syntax_element().clone())),
        },
        BlockNode::Alert(node) => unsupported_block(node.syntax()),
        BlockNode::DefinitionList(node) => unsupported_block(node.syntax()),
        BlockNode::LineBlock(node) => unsupported_block(node.syntax()),
        BlockNode::Figure(node) => unsupported_block(node.syntax()),
        BlockNode::Table(node) => unsupported_block(node.syntax()),
        BlockNode::MystDirective(node) => unsupported_block(node.syntax()),
        BlockNode::PandocTitleBlock(node) => unsupported_block(node.syntax()),
        BlockNode::MmdTitleBlock(node) => unsupported_block(node.syntax()),
        // Panache's consumer enum is nonexhaustive. A new variant remains
        // recoverable from its source until its traversal is integrated here.
        node => BlockKind::Unsupported(PreservedSyntax {
            range: source_range,
            name: format!("{:?}", node.syntax_kind()),
            children: Vec::new(),
        }),
    };
    Some(Block {
        range: source_range,
        attributes,
        kind,
    })
}

fn unsupported_block(node: &cst::SyntaxNode) -> BlockKind {
    BlockKind::Unsupported(preserve(node.clone().into()))
}

fn inline(node: InlineNode) -> Option<Inline> {
    let source_range = range(node.text_range());
    let mut attributes = Vec::new();
    let kind = match node {
        InlineNode::Text(text) => InlineKind::Text(text.decoded()),
        InlineNode::Space(_) => InlineKind::Space,
        InlineNode::NonbreakingSpace(_) => InlineKind::NonbreakingSpace,
        InlineNode::SoftBreak(_) => InlineKind::SoftBreak,
        InlineNode::HardBreak(_) => InlineKind::HardBreak,
        InlineNode::Emphasis(node) => InlineKind::Emphasis(inlines(node.inline_nodes())),
        InlineNode::Strong(node) => InlineKind::Strong(inlines(node.inline_nodes())),
        InlineNode::Code(node) => InlineKind::Code(node.content()),
        InlineNode::Math(node) => InlineKind::Math(node.content()),
        InlineNode::Link(node) => {
            attributes = attrs(node.attributes());
            InlineKind::Link(link(
                inlines(node.inline_nodes()),
                node.dest(),
                node.reference(),
            ))
        }
        InlineNode::Image(node) => {
            attributes = attrs(node.attributes());
            InlineKind::Image(link(
                inlines(node.inline_nodes()),
                node.dest(),
                node.reference(),
            ))
        }
        InlineNode::AutoLink(node) => {
            let target = node.target();
            let destination = if target.contains('@') && !target.contains(':') {
                format!("mailto:{target}")
            } else {
                target.clone()
            };
            let content_range = SourceRange {
                start: source_range.start + 1,
                end: source_range.end - 1,
            };
            InlineKind::Link(Link {
                content: vec![Inline {
                    range: content_range,
                    attributes: Vec::new(),
                    kind: InlineKind::Text(target),
                }],
                destination: Some(destination),
                title: None,
                reference: None,
            })
        }
        InlineNode::UnresolvedReference(node) => {
            let link = Link {
                content: inlines(node.inline_nodes()),
                destination: None,
                title: None,
                reference: node.label(),
            };
            if node.is_image() {
                InlineKind::Image(link)
            } else {
                InlineKind::Link(link)
            }
        }
        InlineNode::Unknown(node) => {
            match node.syntax_kind() {
                // Host prefixes are framing, not text inside a paragraph.
                SyntaxKind::LINE_PREFIX | SyntaxKind::BLOCK_QUOTE_MARKER => return None,
                SyntaxKind::COMMENT => InlineKind::Comment,
                SyntaxKind::INLINE_HTML => {
                    let text = node.source_text();
                    if text.starts_with("<!--") {
                        InlineKind::Comment
                    } else {
                        InlineKind::Raw(RawContent {
                            format: "html".into(),
                            text,
                        })
                    }
                }
                SyntaxKind::BRACKETED_SPAN => {
                    let syntax = node.syntax_element().as_node().expect("span node");
                    attributes = syntax
                        .children()
                        .filter(|child| child.kind() == SyntaxKind::SPAN_ATTRIBUTES)
                        .flat_map(|node| syntax_attrs(&node))
                        .collect();
                    let content = syntax
                        .children()
                        .find(|child| child.kind() == SyntaxKind::SPAN_CONTENT);
                    InlineKind::Span(
                        content
                            .map(|content| {
                                inlines(content.children_with_tokens().map(InlineNode::cast))
                            })
                            .unwrap_or_default(),
                    )
                }
                SyntaxKind::DISPLAY_MATH => {
                    let syntax = node.syntax_element().as_node().expect("display math node");
                    attributes = syntax
                        .children()
                        .filter_map(cst::AttributeNode::cast)
                        .flat_map(|node| attrs(Some(node)))
                        .collect();
                    InlineKind::DisplayMath(
                        cst::DisplayMath::cast(syntax.clone())
                            .expect("display math")
                            .content(),
                    )
                }
                SyntaxKind::RAW_INLINE => {
                    let syntax = node.syntax_element().as_node().expect("raw inline node");
                    let text = token_text(syntax, SyntaxKind::RAW_INLINE_CONTENT);
                    let format = syntax
                        .children()
                        .filter_map(cst::AttributeNode::cast)
                        .flat_map(|node| node.classes())
                        .find_map(|class| class.strip_prefix('=').map(str::to_owned))
                        .unwrap_or_default();
                    InlineKind::Raw(RawContent { format, text })
                }
                _ => InlineKind::Unsupported(preserve(node.syntax_element().clone())),
            }
        }
        InlineNode::Strikeout(node)
        | InlineNode::Mark(node)
        | InlineNode::Superscript(node)
        | InlineNode::Subscript(node) => {
            InlineKind::Unsupported(preserve(node.syntax().clone().into()))
        }
        node => InlineKind::Unsupported(PreservedSyntax {
            range: source_range,
            name: format!("{:?}", node.syntax_kind()),
            children: Vec::new(),
        }),
    };
    Some(Inline {
        range: source_range,
        attributes,
        kind,
    })
}

fn token_text(node: &cst::SyntaxNode, kind: SyntaxKind) -> String {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == kind)
        .map(|token| token.text().to_owned())
        .collect()
}

fn link(
    content: Vec<Inline>,
    destination: Option<cst::LinkDest>,
    reference: Option<cst::LinkRef>,
) -> Link {
    Link {
        content,
        destination: destination.as_ref().map(cst::LinkDest::url_content),
        title: destination.as_ref().and_then(cst::LinkDest::title),
        reference: reference.map(|reference| reference.label()),
    }
}

fn attrs(node: Option<cst::AttributeNode>) -> Vec<Attribute> {
    node.into_iter()
        .flat_map(|node| node.entries())
        .map(|entry| match entry {
            cst::AttributeEntry::Identifier(value) => Attribute {
                range: range(value.text_range()),
                kind: AttributeKind::Identifier(value.value().into()),
            },
            cst::AttributeEntry::Class(value) => Attribute {
                range: range(value.text_range()),
                kind: AttributeKind::Class(value.value().into()),
            },
            cst::AttributeEntry::KeyValue { key, value } => Attribute {
                range: SourceRange {
                    start: key.text_range().start().into(),
                    end: value.text_range().end().into(),
                },
                kind: AttributeKind::KeyValue {
                    key: key.value().into(),
                    value: value.value().into(),
                },
            },
        })
        .collect()
}

/// Panache structures span attributes and code info without exposing them
/// through AttributeNode. Keep this small adapter at the parser boundary.
fn syntax_attrs(node: &cst::SyntaxNode) -> Vec<Attribute> {
    if let Some(node) = cst::AttributeNode::cast(node.clone()) {
        return attrs(Some(node));
    }
    node.children_with_tokens()
        .filter_map(|element| {
            let mut source_range = range(element.text_range());
            let kind = match element.kind() {
                SyntaxKind::ATTR_ID | SyntaxKind::ATTR_CLASS => {
                    let token = element.as_token()?;
                    source_range.start += 1;
                    let value = token.text()[1..].to_owned();
                    if element.kind() == SyntaxKind::ATTR_ID {
                        AttributeKind::Identifier(value)
                    } else {
                        AttributeKind::Class(value)
                    }
                }
                SyntaxKind::ATTR_UNNUMBERED => AttributeKind::Class("unnumbered".into()),
                SyntaxKind::ATTR_KEY_VALUE => {
                    let node = element.as_node()?;
                    let key = token_text(node, SyntaxKind::ATTR_KEY);
                    let raw = token_text(node, SyntaxKind::ATTR_VALUE);
                    let value = if raw.len() >= 2
                        && matches!(raw.as_bytes()[0], b'\'' | b'"')
                        && raw.as_bytes().first() == raw.as_bytes().last()
                    {
                        raw[1..raw.len() - 1].to_owned()
                    } else {
                        raw
                    };
                    AttributeKind::KeyValue { key, value }
                }
                _ => return None,
            };
            Some(Attribute {
                range: source_range,
                kind,
            })
        })
        .collect()
}

fn code_block(node: cst::CodeBlock) -> BlockKind {
    let code = Code {
        language: node.language(),
        source: node.code_source(),
        segments: node
            .code_source_segments()
            .iter()
            .map(|segment| range(segment.text_range()))
            .collect(),
        info: node
            .info()
            .map(|info| preserve(info.syntax().clone().into())),
    };
    let Some(cell) = node.executable_cell() else {
        return BlockKind::Code(code);
    };
    let preamble = node
        .hashpipe_yaml_preamble()
        .map(|preamble| super::yaml::lower(preamble.syntax()));
    let options = cell
        .option_declarations()
        .into_iter()
        .map(|option| {
            let source = match option.source() {
                cst::ChunkOptionSource::InlineInfo => OptionSource::Inline,
                cst::ChunkOptionSource::HashpipeYaml => OptionSource::Hashpipe,
            };
            // The declaration's value wrapper includes tags and anchors that
            // Panache's scalar projection intentionally omits.
            let value = preamble
                .as_ref()
                .and_then(YamlValue::mapping)
                .and_then(|entries| {
                    entries
                        .iter()
                        .find(|entry| entry.range == range(option.declaration_range()))
                })
                .map(|entry| entry.value.clone());
            CellOption {
                range: range(option.declaration_range()),
                source,
                key: option.key().map(str::to_owned),
                key_range: option.key_range().map(range),
                value_range: option.value_range().map(range),
                value,
                inline_value: (source == OptionSource::Inline)
                    .then(|| option.raw_value().map(str::to_owned))
                    .flatten(),
                inline_quoted: source == OptionSource::Inline && option.is_quoted(),
            }
        })
        .collect();
    BlockKind::Cell(Cell {
        code,
        options,
        preamble,
    })
}

pub(super) fn preserve(element: cst::SyntaxElement) -> PreservedSyntax {
    let children = element
        .as_node()
        .map(|node| node.children_with_tokens().map(preserve_child).collect())
        .unwrap_or_default();
    PreservedSyntax {
        range: range(element.text_range()),
        name: format!("{:?}", element.kind()),
        children,
    }
}

fn preserve_child(element: cst::SyntaxElement) -> PreservedChild {
    let block_node = BlockNode::cast(element.clone());
    if !matches!(block_node, BlockNode::Unknown(_) | BlockNode::Trivia(_))
        && let Some(block) = block(block_node)
    {
        return PreservedChild::Block(Box::new(block));
    }
    let inline_node = InlineNode::cast(element.clone());
    if (!matches!(inline_node, InlineNode::Unknown(_))
        || matches!(
            element.kind(),
            SyntaxKind::BRACKETED_SPAN | SyntaxKind::INLINE_HTML | SyntaxKind::RAW_INLINE
        ))
        && let Some(inline) = inline(inline_node)
    {
        return PreservedChild::Inline(Box::new(inline));
    }
    PreservedChild::Syntax(preserve(element))
}
