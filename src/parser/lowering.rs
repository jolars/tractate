use panache_parser::parser::{SyntaxError, SyntaxErrorSource};
use panache_parser::syntax::{self as cst, AstNode, BlockNode, InlineNode, SyntaxKind};

use crate::document::*;

pub(crate) struct LoweredSource {
    pub document: SourceDocument,
    pub diagnostics: Vec<Diagnostic>,
}

impl LoweredSource {
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }
}

pub(crate) fn lower(source: SourceFile) -> LoweredSource {
    let parsed = super::parse(source.text());
    let lowering = Lowerer {
        source,
        ids: SemanticIds::default(),
    };
    let mut document = SourceDocument {
        origin: lowering.origin(SourceRange {
            start: 0,
            end: lowering.source.text().len(),
        }),
        source: lowering.source.clone(),
        metadata: Vec::new(),
        blocks: Vec::new(),
    };
    for block in lowering.blocks(parsed.document().block_nodes()) {
        match block.kind {
            BlockKind::Metadata(metadata) => document.metadata.push(metadata),
            _ => document.blocks.push(block),
        }
    }
    LoweredSource {
        document,
        diagnostics: diagnostics(&lowering.source, parsed.errors()),
    }
}

pub(super) fn diagnostics(source: &SourceFile, errors: &[SyntaxError]) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|error| Diagnostic {
            severity: Severity::Error,
            code: match error.source {
                SyntaxErrorSource::Yaml => DiagnosticCode::InvalidYaml,
            },
            message: error.message.clone(),
            // Panache already reports offsets in the original QMD, including
            // container prefixes and hashpipe markers.
            primary: Origin::Source(
                source
                    .span(range(error.range))
                    .expect("parser diagnostic range must lie on source UTF-8 boundaries"),
            ),
            related: Vec::new(),
        })
        .collect()
}

pub(super) fn range(range: cst::TextRange) -> SourceRange {
    SourceRange {
        start: range.start().into(),
        end: range.end().into(),
    }
}

pub(super) struct Lowerer {
    source: SourceFile,
    ids: SemanticIds,
}

impl Lowerer {
    pub(super) fn span(&self, range: SourceRange) -> SourceSpan {
        self.source
            .span(range)
            .expect("CST range must lie on source UTF-8 boundaries")
    }

    pub(super) fn origin(&self, range: SourceRange) -> Origin {
        Origin::Source(self.span(range))
    }

    fn blocks(&self, nodes: impl IntoIterator<Item = BlockNode>) -> Vec<Block> {
        nodes
            .into_iter()
            .filter_map(|node| self.block(node))
            .collect()
    }

    fn inlines(&self, nodes: impl IntoIterator<Item = InlineNode>) -> Vec<Inline> {
        nodes
            .into_iter()
            .filter_map(|node| self.inline(node))
            .collect()
    }

    fn block(&self, node: BlockNode) -> Option<Block> {
        let source_range = range(node.text_range());
        let mut attributes = Vec::new();
        let kind = match node {
            BlockNode::Trivia(_) => return None,
            BlockNode::Heading(node) => {
                attributes = self.attrs(node.attributes());
                BlockKind::Heading {
                    level: node.level(),
                    content: self.inlines(node.inline_nodes()),
                }
            }
            BlockNode::Paragraph(node) => BlockKind::Paragraph(self.inlines(node.inline_nodes())),
            BlockNode::Plain(node) => BlockKind::Plain(self.inlines(node.inline_nodes())),
            BlockNode::BlockQuote(node) => BlockKind::Quote(self.blocks(node.block_nodes())),
            BlockNode::FencedDiv(node) => {
                attributes = self.attrs(node.attributes());
                BlockKind::Div(self.blocks(node.block_nodes()))
            }
            BlockNode::List(node) => {
                let items: Vec<_> = node
                    .items()
                    .map(|item| ListItem {
                        origin: self.origin(range(item.syntax().text_range())),
                        marker: item.marker(),
                        checked: item.task_checked(),
                        blocks: self.blocks(item.block_nodes()),
                    })
                    .collect();
                let ordered = items
                    .first()
                    .and_then(|item| item.marker.as_deref())
                    .is_some_and(|marker| !matches!(marker, "-" | "+" | "*"));
                BlockKind::List(List {
                    origin: self.origin(source_range),
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
                    .map(|info| self.syntax_attrs(info.syntax()))
                    .unwrap_or_default();
                self.code_block(node)
            }
            BlockNode::DisplayMath(node) => {
                attributes = node
                    .syntax()
                    .children()
                    .filter_map(cst::AttributeNode::cast)
                    .flat_map(|node| self.attrs(Some(node)))
                    .collect();
                BlockKind::Math(node.content())
            }
            BlockNode::ThematicBreak(_) => BlockKind::ThematicBreak,
            BlockNode::YamlMetadata(node) => BlockKind::Metadata(Metadata {
                origin: self.origin(source_range),
                value: self.yaml(node.syntax()),
            }),
            BlockNode::ReferenceDefinition(node) => {
                BlockKind::ReferenceDefinition(self.preserve(node.syntax().clone().into()))
            }
            BlockNode::FootnoteDefinition(node) => {
                BlockKind::FootnoteDefinition(self.preserve(node.syntax().clone().into()))
            }
            BlockNode::TexBlock(node) => BlockKind::Raw(RawContent {
                origin: self.origin(source_range),
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
                            origin: self.origin(source_range),
                            format: "html".into(),
                            text,
                        })
                    }
                }
                _ => BlockKind::Unsupported(self.preserve(node.syntax_element().clone())),
            },
            BlockNode::Alert(node) => self.unsupported_block(node.syntax()),
            BlockNode::DefinitionList(node) => self.unsupported_block(node.syntax()),
            BlockNode::LineBlock(node) => self.unsupported_block(node.syntax()),
            BlockNode::Figure(node) => self.unsupported_block(node.syntax()),
            BlockNode::Table(node) => self.unsupported_block(node.syntax()),
            BlockNode::MystDirective(node) => self.unsupported_block(node.syntax()),
            BlockNode::PandocTitleBlock(node) => self.unsupported_block(node.syntax()),
            BlockNode::MmdTitleBlock(node) => self.unsupported_block(node.syntax()),
            // Panache's consumer enum is nonexhaustive. A new variant remains
            // recoverable from its source until its traversal is integrated here.
            node => BlockKind::Unsupported(PreservedSyntax {
                origin: self.origin(source_range),
                name: format!("{:?}", node.syntax_kind()),
                children: Vec::new(),
            }),
        };
        Some(Block {
            id: self.ids.node(),
            origin: self.origin(source_range),
            attributes,
            kind,
        })
    }

    fn unsupported_block(&self, node: &cst::SyntaxNode) -> BlockKind {
        BlockKind::Unsupported(self.preserve(node.clone().into()))
    }

    fn inline(&self, node: InlineNode) -> Option<Inline> {
        let source_range = range(node.text_range());
        let mut attributes = Vec::new();
        let kind = match node {
            InlineNode::Text(text) => InlineKind::Text(text.decoded()),
            InlineNode::Space(_) => InlineKind::Space,
            InlineNode::NonbreakingSpace(_) => InlineKind::NonbreakingSpace,
            InlineNode::SoftBreak(_) => InlineKind::SoftBreak,
            InlineNode::HardBreak(_) => InlineKind::HardBreak,
            InlineNode::Emphasis(node) => InlineKind::Emphasis(self.inlines(node.inline_nodes())),
            InlineNode::Strong(node) => InlineKind::Strong(self.inlines(node.inline_nodes())),
            InlineNode::Code(node) => InlineKind::Code(node.content()),
            InlineNode::Math(node) => InlineKind::Math(node.content()),
            InlineNode::Link(node) => {
                attributes = self.attrs(node.attributes());
                InlineKind::Link(self.link(
                    self.origin(source_range),
                    self.inlines(node.inline_nodes()),
                    node.dest(),
                    node.reference(),
                ))
            }
            InlineNode::Image(node) => {
                attributes = self.attrs(node.attributes());
                InlineKind::Image(self.link(
                    self.origin(source_range),
                    self.inlines(node.inline_nodes()),
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
                    origin: self.origin(source_range),
                    content: vec![Inline {
                        id: self.ids.node(),
                        origin: self.origin(content_range).derived("autolink label"),
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
                    origin: self.origin(source_range),
                    content: self.inlines(node.inline_nodes()),
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
                                origin: self.origin(source_range),
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
                            .flat_map(|node| self.syntax_attrs(&node))
                            .collect();
                        let content = syntax
                            .children()
                            .find(|child| child.kind() == SyntaxKind::SPAN_CONTENT);
                        InlineKind::Span(
                            content
                                .map(|content| {
                                    self.inlines(
                                        content.children_with_tokens().map(InlineNode::cast),
                                    )
                                })
                                .unwrap_or_default(),
                        )
                    }
                    SyntaxKind::DISPLAY_MATH => {
                        let syntax = node.syntax_element().as_node().expect("display math node");
                        attributes = syntax
                            .children()
                            .filter_map(cst::AttributeNode::cast)
                            .flat_map(|node| self.attrs(Some(node)))
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
                        InlineKind::Raw(RawContent {
                            origin: self.origin(source_range),
                            format,
                            text,
                        })
                    }
                    _ => InlineKind::Unsupported(self.preserve(node.syntax_element().clone())),
                }
            }
            InlineNode::Strikeout(node)
            | InlineNode::Mark(node)
            | InlineNode::Superscript(node)
            | InlineNode::Subscript(node) => {
                InlineKind::Unsupported(self.preserve(node.syntax().clone().into()))
            }
            node => InlineKind::Unsupported(PreservedSyntax {
                origin: self.origin(source_range),
                name: format!("{:?}", node.syntax_kind()),
                children: Vec::new(),
            }),
        };
        Some(Inline {
            id: self.ids.node(),
            origin: self.origin(source_range),
            attributes,
            kind,
        })
    }

    fn link(
        &self,
        origin: Origin,
        content: Vec<Inline>,
        destination: Option<cst::LinkDest>,
        reference: Option<cst::LinkRef>,
    ) -> Link {
        Link {
            origin,
            content,
            destination: destination.as_ref().map(cst::LinkDest::url_content),
            title: destination.as_ref().and_then(cst::LinkDest::title),
            reference: reference.map(|reference| reference.label()),
        }
    }

    fn attrs(&self, node: Option<cst::AttributeNode>) -> Vec<Attribute> {
        node.into_iter()
            .flat_map(|node| node.entries())
            .map(|entry| match entry {
                cst::AttributeEntry::Identifier(value) => Attribute {
                    origin: self.origin(range(value.text_range())),
                    kind: AttributeKind::Identifier(value.value().into()),
                },
                cst::AttributeEntry::Class(value) => Attribute {
                    origin: self.origin(range(value.text_range())),
                    kind: AttributeKind::Class(value.value().into()),
                },
                cst::AttributeEntry::KeyValue { key, value } => Attribute {
                    origin: self.origin(SourceRange {
                        start: key.text_range().start().into(),
                        end: value.text_range().end().into(),
                    }),
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
    fn syntax_attrs(&self, node: &cst::SyntaxNode) -> Vec<Attribute> {
        if let Some(node) = cst::AttributeNode::cast(node.clone()) {
            return self.attrs(Some(node));
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
                    origin: self.origin(source_range),
                    kind,
                })
            })
            .collect()
    }

    fn code_block(&self, node: cst::CodeBlock) -> BlockKind {
        let origin = self.origin(range(node.syntax().text_range()));
        let code = Code {
            origin: origin.clone(),
            language: node.language(),
            source: node.code_source(),
            segments: node
                .code_source_segments()
                .iter()
                .map(|segment| self.span(range(segment.text_range())))
                .collect(),
            info: node
                .info()
                .map(|info| self.preserve(info.syntax().clone().into())),
        };
        let Some(cell) = node.executable_cell() else {
            return BlockKind::Code(code);
        };
        let preamble = node
            .hashpipe_yaml_preamble()
            .map(|preamble| self.yaml(preamble.syntax()));
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
                        entries.iter().find(|entry| {
                            entry.origin.source_span().range() == range(option.declaration_range())
                        })
                    })
                    .map(|entry| entry.value.clone());
                CellOption {
                    origin: self.origin(range(option.declaration_range())),
                    source,
                    key: option.key().map(str::to_owned),
                    key_span: option.key_range().map(|range_| self.span(range(range_))),
                    value_span: option.value_range().map(|range_| self.span(range(range_))),
                    value,
                    inline_value: (source == OptionSource::Inline)
                        .then(|| option.raw_value().map(str::to_owned))
                        .flatten(),
                    inline_quoted: source == OptionSource::Inline && option.is_quoted(),
                }
            })
            .collect();
        BlockKind::Cell(Cell {
            id: self.ids.cell(),
            origin,
            code,
            options,
            preamble,
        })
    }

    pub(super) fn preserve(&self, element: cst::SyntaxElement) -> PreservedSyntax {
        let children = element
            .as_node()
            .map(|node| {
                node.children_with_tokens()
                    .map(|node| self.preserve_child(node))
                    .collect()
            })
            .unwrap_or_default();
        PreservedSyntax {
            origin: self.origin(range(element.text_range())),
            name: format!("{:?}", element.kind()),
            children,
        }
    }

    fn preserve_child(&self, element: cst::SyntaxElement) -> PreservedChild {
        let block_node = BlockNode::cast(element.clone());
        if !matches!(block_node, BlockNode::Unknown(_) | BlockNode::Trivia(_))
            && let Some(block) = self.block(block_node)
        {
            return PreservedChild::Block(Box::new(block));
        }
        let inline_node = InlineNode::cast(element.clone());
        if (!matches!(inline_node, InlineNode::Unknown(_))
            || matches!(
                element.kind(),
                SyntaxKind::BRACKETED_SPAN | SyntaxKind::INLINE_HTML | SyntaxKind::RAW_INLINE
            ))
            && let Some(inline) = self.inline(inline_node)
        {
            return PreservedChild::Inline(Box::new(inline));
        }
        PreservedChild::Syntax(self.preserve(element))
    }
}

fn token_text(node: &cst::SyntaxNode, kind: SyntaxKind) -> String {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == kind)
        .map(|token| token.text().to_owned())
        .collect()
}
