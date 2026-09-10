use std::path::Path;

use super::*;

#[test]
fn origins_distinguish_files_and_retain_the_source_snapshot() {
    let text = "## Café\n\nA **bold** sentence.\n";
    let first = lower(SourceFile::new("first.qmd", text)).document;
    let second = lower(SourceFile::new("second.qmd", text)).document;
    let revised = lower(SourceFile::new("first.qmd", "## Changed\n")).document;
    let heading = first.blocks[0].clone();
    assert_ne!(heading.origin, second.blocks[0].origin);
    assert_ne!(heading.origin, revised.blocks[0].origin);
    drop(first);

    let span = heading.origin.source_span();
    assert_eq!(span.file().path(), Some(Path::new("first.qmd")));
    assert_eq!(span.text(), "## Café\n");
    assert_eq!(span.range(), SourceRange { start: 0, end: 9 });
    assert_eq!(span.file().text(), text);
}

#[test]
fn origins_trace_generated_ranges_through_semantic_nodes_to_qmd() {
    let source = "---\ntitle: Café\n---\n";
    let document = lower(SourceFile::new("slides.qmd", source)).document;
    let title = &document.metadata[0].value.mapping().unwrap()[0].value;
    let slide = title.origin.derived("title slide");
    let typst = SourceFile::new("build/slide.typ", "#text(\"Café\")");
    let generated = slide.generated(
        "Typst text",
        typst.span(SourceRange { start: 6, end: 13 }).unwrap(),
    );
    let diagnostic = generated.derived("backend diagnostic");

    assert_eq!(diagnostic.source_span(), title.origin.source_span());
    assert_eq!(diagnostic.source_span().text(), "Café");
    assert_eq!(
        diagnostic.source_span().file().path(),
        Some(Path::new("slides.qmd"))
    );
    let chain: Vec<_> = diagnostic.chain().collect();
    assert_eq!(chain.len(), 4);
    assert_eq!(chain[1].span().unwrap().text(), "\"Café\"");
    assert_eq!(chain[1].span().unwrap().file().path(), typst.path());
    assert_eq!(chain[2], &slide);
    assert_eq!(chain[3], &title.origin);
}

#[test]
fn origins_validate_utf8_ranges_and_allow_empty_spans() {
    let file = SourceFile::anonymous("é\r\n");
    assert!(file.span(SourceRange { start: 0, end: 1 }).is_none());
    assert!(file.span(SourceRange { start: 1, end: 2 }).is_none());
    assert!(file.span(SourceRange { start: 3, end: 2 }).is_none());
    assert!(file.span(SourceRange { start: 0, end: 5 }).is_none());
    assert_eq!(
        file.span(SourceRange { start: 0, end: 2 }).unwrap().text(),
        "é"
    );
    assert_eq!(
        file.span(SourceRange { start: 4, end: 4 }).unwrap().text(),
        ""
    );
    assert_eq!(file.path(), None);
}

#[test]
fn origins_follow_synthesized_autolink_labels() {
    let doc = lower(SourceFile::new("links.qmd", "<https://example.com>\n")).document;
    let inline = &paragraph(&doc.blocks[0])[0];
    let InlineKind::Link(link) = &inline.kind else {
        panic!("autolink")
    };
    assert_eq!(
        link.content[0].origin.source_span().text(),
        "https://example.com"
    );
    assert_eq!(link.content[0].origin.chain().count(), 2);
    assert_eq!(link.origin, inline.origin);
}

#[test]
fn origins_cover_nested_semantics_and_rejected_syntax_in_all_fixtures() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut count = 0;
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "qmd") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for text in [text.clone(), text.replace('\n', "\r\n")] {
            let file = SourceFile::new(&path, text);
            let doc = lower(file.clone()).document;
            assert_eq!(doc.origin.source_span().text(), file.text());
            let mut audit = OriginAudit { file, count: 0 };
            audit.origin(&doc.origin);
            for metadata in &doc.metadata {
                audit.metadata(metadata);
            }
            for block in &doc.blocks {
                audit.block(block);
            }
            assert!(audit.count > 1, "{}", path.display());
        }
        count += 1;
    }
    assert!(count > 0);
}

#[test]
fn origins_preserve_precise_option_and_code_locations_in_containers() {
    let source = "> ```{r, echo=FALSE}\r\n> #| label: café\r\n> #| fig-cap: |\r\n> #|   A **caption**.\r\n> print(\"é\")\r\n> ```\r\n";
    let doc = lower(SourceFile::new("nested.qmd", source)).document;
    let BlockKind::Quote(blocks) = &doc.blocks[0].kind else {
        panic!("quote")
    };
    let BlockKind::Cell(cell) = &blocks[0].kind else {
        panic!("cell")
    };
    assert_eq!(cell.origin, blocks[0].origin);
    assert_eq!(cell.code.origin, cell.origin);
    assert_eq!(cell.options[0].origin.source_span().text(), "echo=FALSE");
    assert_eq!(cell.options[1].key_span.as_ref().unwrap().text(), "label");
    assert_eq!(cell.options[1].value_span.as_ref().unwrap().text(), "café");
    assert!(
        cell.options[2]
            .origin
            .source_span()
            .text()
            .contains("> #|   A **caption**.")
    );
    assert_eq!(
        cell.code
            .segments
            .iter()
            .map(SourceSpan::text)
            .collect::<String>(),
        cell.code.source
    );
    for segment in &cell.code.segments {
        assert_eq!(segment.file(), &doc.source);
        assert!(!segment.text().contains('>'));
    }
}

#[test]
fn origins_keep_anonymous_snapshots_distinct_and_recover_empty_documents() {
    let first = SourceFile::anonymous("");
    let second = SourceFile::anonymous("");
    assert_ne!(first, second);
    let doc = lower(first.clone()).document;
    assert_eq!(doc.origin.source_span().file(), &first);
    assert_eq!(
        doc.origin.source_span().range(),
        SourceRange { start: 0, end: 0 }
    );
    assert_eq!(doc.origin.source_span().text(), "");
}

/// Exhaustive matches keep new semantic variants in this provenance audit.
struct OriginAudit {
    file: SourceFile,
    count: usize,
}

impl OriginAudit {
    fn span(&self, span: &SourceSpan) {
        assert_eq!(span.file(), &self.file);
        let range = span.range();
        assert_eq!(
            self.file.text().get(range.start..range.end),
            Some(span.text())
        );
    }

    fn origin(&mut self, origin: &Origin) {
        self.span(origin.source_span());
        self.count += 1;
    }

    fn block(&mut self, block: &Block) {
        self.origin(&block.origin);
        for attribute in &block.attributes {
            self.origin(&attribute.origin);
        }
        match &block.kind {
            BlockKind::Heading { content, .. }
            | BlockKind::Paragraph(content)
            | BlockKind::Plain(content) => {
                for inline in content {
                    self.inline(inline);
                }
            }
            BlockKind::List(list) => {
                self.origin(&list.origin);
                for item in &list.items {
                    self.origin(&item.origin);
                    for block in &item.blocks {
                        self.block(block);
                    }
                }
            }
            BlockKind::Quote(blocks) | BlockKind::Div(blocks) => {
                for block in blocks {
                    self.block(block);
                }
            }
            BlockKind::Code(code) => self.code(code),
            BlockKind::Cell(cell) => {
                self.origin(&cell.origin);
                self.code(&cell.code);
                for option in &cell.options {
                    self.origin(&option.origin);
                    for span in option.key_span.iter().chain(&option.value_span) {
                        self.span(span);
                    }
                    if let Some(value) = &option.value {
                        self.yaml(value);
                    }
                }
                if let Some(value) = &cell.preamble {
                    self.yaml(value);
                }
            }
            BlockKind::Raw(raw) => self.origin(&raw.origin),
            BlockKind::Metadata(metadata) => self.metadata(metadata),
            BlockKind::ReferenceDefinition(syntax)
            | BlockKind::FootnoteDefinition(syntax)
            | BlockKind::Unsupported(syntax) => self.syntax(syntax),
            BlockKind::Math(_) | BlockKind::ThematicBreak | BlockKind::Comment => {}
        }
    }

    fn inline(&mut self, inline: &Inline) {
        self.origin(&inline.origin);
        for attribute in &inline.attributes {
            self.origin(&attribute.origin);
        }
        match &inline.kind {
            InlineKind::Emphasis(content)
            | InlineKind::Strong(content)
            | InlineKind::Span(content) => {
                for inline in content {
                    self.inline(inline);
                }
            }
            InlineKind::Link(link) | InlineKind::Image(link) => {
                self.origin(&link.origin);
                for inline in &link.content {
                    self.inline(inline);
                }
            }
            InlineKind::Raw(raw) => self.origin(&raw.origin),
            InlineKind::Unsupported(syntax) => self.syntax(syntax),
            InlineKind::Text(_)
            | InlineKind::Space
            | InlineKind::NonbreakingSpace
            | InlineKind::SoftBreak
            | InlineKind::HardBreak
            | InlineKind::Code(_)
            | InlineKind::Math(_)
            | InlineKind::DisplayMath(_)
            | InlineKind::Comment => {}
        }
    }

    fn code(&mut self, code: &Code) {
        self.origin(&code.origin);
        for segment in &code.segments {
            self.span(segment);
        }
        assert_eq!(
            code.segments
                .iter()
                .map(SourceSpan::text)
                .collect::<String>(),
            code.source
        );
        if let Some(info) = &code.info {
            self.syntax(info);
        }
    }

    fn metadata(&mut self, metadata: &Metadata) {
        self.origin(&metadata.origin);
        self.yaml(&metadata.value);
    }

    fn yaml(&mut self, value: &YamlValue) {
        self.origin(&value.origin);
        for property in &value.properties {
            self.origin(&property.origin);
        }
        match &value.kind {
            YamlKind::Mapping { entries, .. } => {
                for entry in entries {
                    self.origin(&entry.origin);
                    self.yaml(&entry.key);
                    self.yaml(&entry.value);
                }
            }
            YamlKind::Sequence { items, .. } => {
                for item in items {
                    self.yaml(item);
                }
            }
            YamlKind::Unsupported(syntax) => self.syntax(syntax),
            YamlKind::Scalar { .. } | YamlKind::Empty => {}
        }
    }

    fn syntax(&mut self, syntax: &PreservedSyntax) {
        self.origin(&syntax.origin);
        for child in &syntax.children {
            match child {
                PreservedChild::Block(block) => self.block(block),
                PreservedChild::Inline(inline) => self.inline(inline),
                PreservedChild::Syntax(syntax) => self.syntax(syntax),
            }
        }
    }
}
