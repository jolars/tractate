use super::lower;
use crate::document::*;

mod diagnostics;
mod identities;
mod origins;

fn document(source: &str) -> SourceDocument {
    let lowered = lower(SourceFile::anonymous(source));
    assert_eq!(lowered.error_count(), 0);
    lowered.document
}

fn paragraph(block: &Block) -> &[Inline] {
    match &block.kind {
        BlockKind::Paragraph(inlines) | BlockKind::Plain(inlines) => inlines,
        other => panic!("expected paragraph, got {other:?}"),
    }
}

fn scalar(value: &YamlValue) -> (&str, ScalarStyle) {
    match &value.kind {
        YamlKind::Scalar { text, style } => (text, *style),
        other => panic!("expected scalar, got {other:?}"),
    }
}

#[test]
fn lowering_preserves_block_and_inline_nesting() {
    let source = "::: {.callout-note #note}\n\n> ## A *nested* heading\n>\n> - First **bold** item.\n>   - A [*nested* link](https://example.com).\n\nAfter the quote.\n\n:::\n";
    let doc = document(source);
    assert_eq!(doc.blocks.len(), 1);
    let div = &doc.blocks[0];
    assert_eq!(
        div.attributes[0].kind,
        AttributeKind::Class("callout-note".into())
    );
    assert_eq!(
        div.attributes[1].kind,
        AttributeKind::Identifier("note".into())
    );
    let BlockKind::Div(blocks) = &div.kind else {
        panic!("div")
    };
    assert_eq!(blocks.len(), 2);
    let BlockKind::Quote(quoted) = &blocks[0].kind else {
        panic!("quote")
    };
    assert_eq!(quoted.len(), 2);
    let BlockKind::Heading { level, content } = &quoted[0].kind else {
        panic!("heading")
    };
    assert_eq!(*level, 2);
    assert!(content.iter().any(
        |inline| matches!(&inline.kind, InlineKind::Emphasis(children) if children.len() == 1)
    ));
    let BlockKind::List(list) = &quoted[1].kind else {
        panic!("list")
    };
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].blocks.len(), 2);
    assert!(
        paragraph(&list.items[0].blocks[0])
            .iter()
            .any(|inline| matches!(inline.kind, InlineKind::Strong(_)))
    );
    let BlockKind::List(nested) = &list.items[0].blocks[1].kind else {
        panic!("nested list")
    };
    let link = paragraph(&nested.items[0].blocks[0])
        .iter()
        .find_map(|inline| match &inline.kind {
            InlineKind::Link(link) => Some(link),
            _ => None,
        })
        .unwrap();
    assert!(matches!(link.content[0].kind, InlineKind::Emphasis(_)));
    assert_eq!(link.destination.as_deref(), Some("https://example.com"));
    assert!(matches!(blocks[1].kind, BlockKind::Paragraph(_)));
}

#[test]
fn lowering_preserves_list_markers_and_task_states() {
    let doc = document("3. First.\n\n4. Second.\n\n- [ ] Open\n- [x] Done\n- Plain\n");
    let BlockKind::List(ordered) = &doc.blocks[0].kind else {
        panic!("ordered list")
    };
    assert_eq!(ordered.kind, ListKind::Ordered);
    assert!(ordered.loose);
    assert_eq!(ordered.items[0].marker.as_deref(), Some("3."));
    assert!(matches!(
        ordered.items[0].blocks[0].kind,
        BlockKind::Plain(_)
    ));
    let BlockKind::List(tasks) = &doc.blocks[1].kind else {
        panic!("task list")
    };
    assert_eq!(tasks.kind, ListKind::Bullet);
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|item| item.checked)
            .collect::<Vec<_>>(),
        [Some(false), Some(true), None]
    );
}

#[test]
fn lowering_keeps_decoded_text_breaks_and_source_ranges() {
    let source = "> Café \\* &amp; α\n> next\\\n> last\n";
    let doc = document(source);
    let BlockKind::Quote(blocks) = &doc.blocks[0].kind else {
        panic!("quote")
    };
    let inlines = paragraph(&blocks[0]);
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(inline.kind, InlineKind::SoftBreak))
    );
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(inline.kind, InlineKind::HardBreak))
    );
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(&inline.kind, InlineKind::Text(text) if text == "*"))
    );
    let text: String = inlines
        .iter()
        .filter_map(|inline| match &inline.kind {
            InlineKind::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains('&'));
    assert!(!text.contains("&amp;"));
    assert!(!text.contains('>'));
    for inline in inlines {
        assert!(
            source
                .get(
                    inline.origin.source_span().range().start
                        ..inline.origin.source_span().range().end
                )
                .is_some()
        );
        assert!(
            inline.origin.source_span().range().start
                >= blocks[0].origin.source_span().range().start
        );
        assert!(
            inline.origin.source_span().range().end <= blocks[0].origin.source_span().range().end
        );
    }
    let escaped = inlines
        .iter()
        .find(|inline| matches!(&inline.kind, InlineKind::Text(text) if text == "*"))
        .unwrap();
    assert_eq!(escaped.origin.source_span().text(), "\\*");
}

#[test]
fn lowering_separates_code_from_options_and_container_prefixes() {
    let source = "> ```{r}\n> #| label: café\n> #| eval: false\n>\n> x <- 1\n> print(x)\n> ```\n\n```r\n#| echo: false\nx <- 2\n```\n";
    let doc = document(source);
    let BlockKind::Quote(blocks) = &doc.blocks[0].kind else {
        panic!("quote")
    };
    let BlockKind::Cell(cell) = &blocks[0].kind else {
        panic!("cell")
    };
    assert_eq!(cell.code.language.as_deref(), Some("r"));
    assert_eq!(cell.code.source.trim(), "x <- 1\nprint(x)");
    assert_eq!(
        cell.code
            .segments
            .iter()
            .map(|range| range.text())
            .collect::<String>(),
        cell.code.source
    );
    assert_eq!(cell.options.len(), 2);
    assert_eq!(cell.options[0].key.as_deref(), Some("label"));
    assert_eq!(scalar(cell.options[0].value.as_ref().unwrap()).0, "café");
    assert_eq!(cell.options[0].key_span.as_ref().unwrap().text(), "label");
    assert_eq!(cell.options[0].value_span.as_ref().unwrap().text(), "café");
    let BlockKind::Code(code) = &doc.blocks[1].kind else {
        panic!("ordinary code")
    };
    assert_eq!(code.source, "#| echo: false\nx <- 2\n");
}

#[test]
fn lowering_preserves_metadata_and_unresolved_option_declarations() {
    let source = "---\ntitle: 'null'\nexecute:\n  echo: false\n  eval: true\n  inputs: [a.csv, a.csv]\n---\n\n```{r, echo=TRUE}\n#| echo: \"false\"\n#| eval: false\n#| custom: !expr dangerous()\n#| alias: &data [1, 2]\n#| inputs: *data\n#| fig-cap: |-\n#|   Café and **values**.\nplot(1)\n```\n";
    let doc = document(source);
    let entries = doc.metadata[0].value.mapping().unwrap();
    assert_eq!(
        scalar(&entries[0].value),
        ("null", ScalarStyle::SingleQuoted)
    );
    let defaults = entries[1].value.mapping().unwrap();
    assert_eq!(
        defaults
            .iter()
            .map(|entry| scalar(&entry.key).0)
            .collect::<Vec<_>>(),
        ["echo", "eval", "inputs"]
    );
    let BlockKind::Cell(cell) = &doc.blocks[0].kind else {
        panic!("cell")
    };
    assert_eq!(
        cell.options
            .iter()
            .map(|option| option.key.as_deref().unwrap())
            .collect::<Vec<_>>(),
        [
            "echo", "echo", "eval", "custom", "alias", "inputs", "fig-cap"
        ]
    );
    assert_eq!(cell.options[0].source, OptionSource::Inline);
    assert_eq!(cell.options[1].source, OptionSource::Hashpipe);
    assert_eq!(
        scalar(cell.options[1].value.as_ref().unwrap()),
        ("false", ScalarStyle::DoubleQuoted)
    );
    assert_eq!(
        scalar(cell.options[2].value.as_ref().unwrap()),
        ("false", ScalarStyle::Plain)
    );
    assert!(
        cell.options[3]
            .origin
            .source_span()
            .text()
            .contains("!expr")
    );
    assert_eq!(
        cell.options[3].value.as_ref().unwrap().properties[0]
            .origin
            .source_span()
            .text(),
        "!expr"
    );
    assert_eq!(
        cell.options[4].value.as_ref().unwrap().properties[0]
            .origin
            .source_span()
            .text(),
        "&data"
    );
    assert!(
        cell.options[5]
            .value
            .as_ref()
            .unwrap()
            .origin
            .source_span()
            .text()
            .contains("*data")
    );
    assert_eq!(
        scalar(cell.options[6].value.as_ref().unwrap()).1,
        ScalarStyle::Literal
    );
}

#[test]
fn lowering_preserves_unsupported_structure_and_known_descendants() {
    let source = "Term\n:   ## Nested heading\n\n    ```{r}\n    #| eval: false\n    1\n    ```\n\nText with @citation and ^[a **nested** note].\n";
    let doc = document(source);
    let BlockKind::Unsupported(syntax) = &doc.blocks[0].kind else {
        panic!("unsupported definition list")
    };
    assert_eq!(syntax.name, "DEFINITION_LIST");
    assert!(syntax.origin.source_span().text().starts_with("Term\n:"));
    assert!(!syntax.children.is_empty());
    let mut headings = 0;
    let mut cells = 0;
    doc.visit_blocks(&mut |block| match block.kind {
        BlockKind::Heading { .. } => headings += 1,
        BlockKind::Cell(_) => cells += 1,
        _ => {}
    });
    assert_eq!((headings, cells), (1, 1));
    let inlines = paragraph(doc.blocks.last().unwrap());
    assert!(inlines.iter().any(
        |inline| matches!(&inline.kind, InlineKind::Unsupported(node) if node.name == "CITATION")
    ));
    assert!(inlines.iter().any(|inline| matches!(&inline.kind, InlineKind::Unsupported(node) if node.name == "INLINE_FOOTNOTE" && !node.children.is_empty())));
}

#[test]
fn lowering_retains_partial_documents_for_malformed_yaml() {
    let source = include_str!("../../tests/fixtures/malformed-options.qmd");
    let lowered = lower(SourceFile::anonymous(source));
    assert_eq!(lowered.error_count(), 1);
    assert_eq!(lowered.document.source.text(), source);
    let cell = lowered
        .document
        .blocks
        .iter()
        .find_map(|block| match &block.kind {
            BlockKind::Cell(cell) => Some(cell),
            _ => None,
        })
        .unwrap();
    assert!(
        cell.preamble
            .as_ref()
            .unwrap()
            .origin
            .source_span()
            .text()
            .contains("#|")
    );
}

#[test]
fn lowering_preserves_inline_attributes_math_and_raw_content() {
    let source = "## A **strong** heading {#slide .a .a key=one key=two}\n\nA [*marked* span]{#span .mark key=\"two words\"}, `code`, $x + y$, and ![**alt** text](image.png \"Title\"){.image}.\n\n> $$\n> x^2 + y^2\n> $$\n\nText <b>HTML</b> and `raw <i>`{=html}. <!-- Comment. -->\n";
    let doc = document(source);
    assert_eq!(
        doc.blocks[0]
            .attributes
            .iter()
            .map(|attribute| &attribute.kind)
            .collect::<Vec<_>>(),
        [
            &AttributeKind::Identifier("slide".into()),
            &AttributeKind::Class("a".into()),
            &AttributeKind::Class("a".into()),
            &AttributeKind::KeyValue {
                key: "key".into(),
                value: "one".into()
            },
            &AttributeKind::KeyValue {
                key: "key".into(),
                value: "two".into()
            },
        ]
    );
    let content = paragraph(&doc.blocks[1]);
    let span = content
        .iter()
        .find(|inline| matches!(inline.kind, InlineKind::Span(_)))
        .unwrap();
    assert_eq!(span.attributes.len(), 3);
    assert_eq!(
        span.attributes[0].kind,
        AttributeKind::Identifier("span".into())
    );
    let InlineKind::Span(children) = &span.kind else {
        unreachable!()
    };
    assert!(matches!(children[0].kind, InlineKind::Emphasis(_)));
    assert!(
        content
            .iter()
            .any(|inline| matches!(&inline.kind, InlineKind::Code(text) if text == "code"))
    );
    assert!(
        content
            .iter()
            .any(|inline| matches!(&inline.kind, InlineKind::Math(text) if text == "x + y"))
    );
    let image = content
        .iter()
        .find(|inline| matches!(inline.kind, InlineKind::Image(_)))
        .unwrap();
    assert_eq!(
        image.attributes[0].kind,
        AttributeKind::Class("image".into())
    );
    let InlineKind::Image(link) = &image.kind else {
        unreachable!()
    };
    assert!(matches!(link.content[0].kind, InlineKind::Strong(_)));
    assert_eq!(link.destination.as_deref(), Some("image.png"));
    assert_eq!(link.title.as_deref(), Some("Title"));
    let BlockKind::Quote(blocks) = &doc.blocks[2].kind else {
        panic!("quote")
    };
    let math = paragraph(&blocks[0])
        .iter()
        .find_map(|inline| match &inline.kind {
            InlineKind::DisplayMath(math) => Some(math),
            _ => None,
        })
        .expect("display math within the quoted paragraph");
    assert_eq!(math.trim(), "x^2 + y^2");
    let content = paragraph(&doc.blocks[3]);
    assert!(content.iter().any(|inline| matches!(&inline.kind, InlineKind::Raw(raw) if raw.format == "html" && raw.text == "raw <i>")), "{content:?}");
    assert!(
        content
            .iter()
            .any(|inline| matches!(inline.kind, InlineKind::Comment))
    );
}

#[test]
fn lowering_keeps_reference_link_labels_and_definitions() {
    let source = "A [**named** link][target], [missing], and <https://example.com>.\n\n[target]: https://example.org \"Reference title\"\n\n[^note]: A **note**.\n";
    let doc = document(source);
    let links: Vec<_> = paragraph(&doc.blocks[0])
        .iter()
        .filter_map(|inline| match &inline.kind {
            InlineKind::Link(link) => Some(link),
            _ => None,
        })
        .collect();
    assert_eq!(links[0].reference.as_deref(), Some("target"));
    assert!(matches!(links[0].content[0].kind, InlineKind::Strong(_)));
    assert!(
        !links[1].content.is_empty(),
        "unresolved label must remain available"
    );
    assert_eq!(links[2].destination.as_deref(), Some("https://example.com"));
    assert!(matches!(
        doc.blocks[1].kind,
        BlockKind::ReferenceDefinition(_)
    ));
    assert!(matches!(
        doc.blocks[2].kind,
        BlockKind::FootnoteDefinition(_)
    ));
    assert_eq!(crate::summarize_document(source).slides, 1);
}

#[test]
fn lowering_preserves_looseness_within_a_single_list_item() {
    let doc = document("- First paragraph.\n\n  Second paragraph.\n");
    let BlockKind::List(list) = &doc.blocks[0].kind else {
        panic!("list")
    };
    assert!(list.loose);
    assert_eq!(list.items[0].blocks.len(), 2);
    assert!(matches!(list.items[0].blocks[1].kind, BlockKind::Plain(_)));
}

#[test]
fn lowering_preserves_rejected_duplicate_yaml_verbatim() {
    let source = "---\nexecute:\n  echo: true\n  echo: false\n---\n\n```{r}\n#| label: first\n#| label: second\n1\n```\n";
    let lowered = lower(SourceFile::anonymous(source));
    assert_eq!(lowered.error_count(), 2);
    let metadata = &lowered.document.metadata[0];
    assert!(matches!(metadata.value.kind, YamlKind::Unsupported(_)));
    assert!(
        metadata
            .origin
            .source_span()
            .text()
            .contains("  echo: true\n  echo: false")
    );
    let BlockKind::Cell(cell) = &lowered.document.blocks[0].kind else {
        panic!("cell")
    };
    let preamble = cell.preamble.as_ref().unwrap();
    assert!(matches!(preamble.kind, YamlKind::Unsupported(_)));
    assert_eq!(
        preamble.origin.source_span().text(),
        "#| label: first\n#| label: second\n"
    );
}

#[test]
fn lowering_handles_all_existing_source_fixtures() {
    for (name, source) in [
        (
            "source-content",
            include_str!("../../tests/fixtures/source-content.qmd"),
        ),
        (
            "cell-options",
            include_str!("../../tests/fixtures/cell-options.qmd"),
        ),
        (
            "document-defaults",
            include_str!("../../tests/fixtures/document-defaults.qmd"),
        ),
        (
            "option-type-boundaries",
            include_str!("../../tests/fixtures/option-type-boundaries.qmd"),
        ),
        (
            "invalid-option-types",
            include_str!("../../tests/fixtures/invalid-option-types.qmd"),
        ),
        (
            "invalid-option-values",
            include_str!("../../tests/fixtures/invalid-option-values.qmd"),
        ),
        (
            "unknown-options",
            include_str!("../../tests/fixtures/unknown-options.qmd"),
        ),
        (
            "duplicate-labels",
            include_str!("../../tests/fixtures/duplicate-labels.qmd"),
        ),
    ] {
        let doc = document(source);
        let parsed = super::parse(source);
        use panache_parser::syntax::{AstNode, CodeBlock};
        let expected: Vec<_> = parsed
            .document()
            .syntax()
            .descendants()
            .filter_map(CodeBlock::cast)
            .filter_map(|code| code.executable_cell())
            .flat_map(|cell| cell.option_declarations())
            .map(|option| {
                (
                    option.key().map(str::to_owned),
                    option.cooked_value().map(str::to_owned),
                )
            })
            .collect();
        let mut actual = Vec::new();
        doc.visit_blocks(&mut |block| {
            if let BlockKind::Cell(cell) = &block.kind {
                for option in &cell.options {
                    let value = option.value.as_ref().and_then(|value| match &value.kind {
                        YamlKind::Scalar { text, .. } => Some(text.clone()),
                        _ => None,
                    });
                    actual.push((option.key.clone(), value));
                    assert!(
                        option
                            .origin
                            .source_span()
                            .text()
                            .contains(option.key.as_deref().unwrap()),
                        "{name}"
                    );
                }
            }
        });
        assert_eq!(actual, expected, "{name}");
    }
}
