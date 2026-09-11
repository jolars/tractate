use crate::compiler::build_presentation;
use crate::document::*;
use crate::parser;

fn document(source: &str) -> SourceDocument {
    let lowered = parser::lower(SourceFile::new("slides.qmd", source));
    assert_eq!(lowered.error_count(), 0, "{:#?}", lowered.diagnostics);
    lowered.document
}

fn content(slide: &Slide<Cell>) -> &[Block] {
    let SlideKind::Content(blocks) = &slide.kind else {
        panic!("expected a content slide");
    };
    blocks
}

fn title(slide: &Slide<Cell>) -> &YamlValue {
    let SlideKind::Title(title) = &slide.kind else {
        panic!("expected a title slide");
    };
    title
}

#[test]
fn presentations_assign_distinct_slide_identities_and_retain_source_identities() {
    let doc = document(
        "---\ntitle: Same\n---\n\nLeading.\n\n## Same\n\n```{r}\n1\n```\n\n## Same\n\n```{r}\n1\n```\n",
    );
    let presentation = build_presentation(doc.clone());
    let ids: std::collections::HashSet<SlideId> =
        presentation.slides.iter().map(|slide| slide.id).collect();

    assert_eq!(ids.len(), 4);
    let blocks: Vec<_> = presentation.slides[1..]
        .iter()
        .flat_map(content)
        .cloned()
        .collect();
    // Whole-tree equality checks that regrouping retains node and cell IDs.
    assert_eq!(blocks, doc.blocks);
    assert_eq!(presentation, presentation.clone());
}

#[test]
fn presentations_partition_source_content_without_flattening() {
    let doc = document(include_str!("../../../tests/fixtures/source-content.qmd"));
    let presentation = build_presentation(doc.clone());

    assert_eq!(presentation.metadata, doc.metadata);
    assert!(presentation.preamble.is_empty());
    assert_eq!(presentation.slides.len(), 5);
    assert_eq!(
        title(&presentation.slides[0]),
        &doc.metadata[0].value.mapping().unwrap()[0].value
    );
    // Whole-tree equality also checks nested lists, inline math, cell options,
    // and source origins, which a flat list of slide headings would lose.
    for (slide, expected) in presentation.slides[1..].iter().zip([
        &doc.blocks[0..3],
        &doc.blocks[3..8],
        &doc.blocks[8..11],
        &doc.blocks[11..14],
    ]) {
        assert_eq!(content(slide), expected);
    }
}

#[test]
fn presentations_keep_nested_headings_and_attributes_on_their_slide() {
    let source = "## *First* {#first .wide}\n\n> ## Quoted\n>\n> A **bold** paragraph.\n\n- ## Listed\n\n::: {.callout-note #note}\n\n## In a div\n\n:::\n\n```{r}\n#| label: example\n## In code\nstop('must not run')\n```\n\n***\n\n### Detail\n\n## Second\n";
    let doc = document(source);
    let presentation = build_presentation(doc.clone());

    assert_eq!(presentation.slides.len(), 2);
    assert_eq!(content(&presentation.slides[0]), &doc.blocks[..7]);
    assert_eq!(content(&presentation.slides[1]), &doc.blocks[7..]);
    assert!(matches!(doc.blocks[1].kind, BlockKind::Quote(_)));
    assert!(matches!(doc.blocks[2].kind, BlockKind::List(_)));
    assert!(matches!(doc.blocks[3].kind, BlockKind::Div(_)));
    assert!(matches!(doc.blocks[4].kind, BlockKind::Cell(_)));
    assert_eq!(
        content(&presentation.slides[0])[0].attributes[0].kind,
        AttributeKind::Identifier("first".into())
    );
}

#[test]
fn presentations_retain_empty_and_consecutive_heading_slides() {
    let doc = document("##\n\nSecond\n------\n\n## Third\n");
    let presentation = build_presentation(doc.clone());

    assert_eq!(presentation.slides.len(), 3);
    for (slide, block) in presentation.slides.iter().zip(&doc.blocks) {
        assert_eq!(content(slide), std::slice::from_ref(block));
    }
}

#[test]
fn presentations_retain_noncontent_without_creating_slides() {
    let preamble = "<!-- First. --> <!-- Second. -->\n\n[reference]: https://example.com\n\n[^note]: A footnote definition.\n";
    for source in ["", "\n \t\n", preamble] {
        let doc = document(source);
        let presentation = build_presentation(doc.clone());
        assert!(presentation.slides.is_empty());
        assert_eq!(presentation.preamble, doc.blocks);
    }

    let expected_preamble: Vec<_> = document(preamble)
        .blocks
        .iter()
        .map(|block| block.origin.source_span().text().to_owned())
        .collect();
    for (body, expected_slides) in [
        ("", 1),
        ("## Content\n", 2),
        ("Leading prose.\n\n## Content\n", 3),
    ] {
        let doc = document(&format!("---\ntitle: Deck\n---\n\n{preamble}\n{body}"));
        let presentation = build_presentation(doc.clone());
        assert_eq!(
            title(&presentation.slides[0])
                .origin
                .source_span()
                .text()
                .trim(),
            "Deck"
        );
        let actual_preamble: Vec<_> = presentation
            .preamble
            .iter()
            .map(|block| block.origin.source_span().text())
            .collect();
        assert_eq!(actual_preamble, expected_preamble);
        let blocks: Vec<_> = presentation
            .preamble
            .iter()
            .chain(presentation.slides[1..].iter().flat_map(content))
            .cloned()
            .collect();
        assert_eq!(blocks, doc.blocks);
        assert_eq!(presentation.slides.len(), expected_slides);
    }
}

#[test]
fn presentations_keep_comments_and_definitions_with_existing_body_content() {
    let doc = document(
        "Leading prose.\n\n<!-- In the leading slide. -->\n\n[leading]: https://example.com\n\n## Content\n\nA [link][later].\n\n[later]: https://example.org\n\n[^note]: A footnote.\n\n<!-- At the end. -->\n",
    );
    let presentation = build_presentation(doc.clone());

    assert!(presentation.preamble.is_empty());
    assert_eq!(presentation.slides.len(), 2);
    assert_eq!(content(&presentation.slides[0]), &doc.blocks[..3]);
    assert_eq!(content(&presentation.slides[1]), &doc.blocks[3..]);
}

#[test]
fn presentations_retain_title_declarations_and_use_their_origins() {
    for metadata in [
        "title: A title",
        "title: 'null'",
        "title: \"Café and α\"",
        "title: |\n  A multiline\n  title",
        "title: >-\n  A folded\n  title",
        "{title: Flow mapping}",
    ] {
        let source = format!("---\n{metadata}\n---\n\n## Content\n");
        for source in [source.clone(), source.replace('\n', "\r\n")] {
            let doc = document(&source);
            let expected = doc.metadata[0].value.mapping().unwrap()[0].value.clone();
            let presentation = build_presentation(doc);

            assert_eq!(presentation.slides.len(), 2);
            let slide = &presentation.slides[0];
            assert_eq!(title(slide), &expected);
            assert_eq!(slide.origin.source_span(), expected.origin.source_span());
            assert_eq!(
                slide.origin.chain().count(),
                expected.origin.chain().count() + 1
            );
        }
    }
}

#[test]
fn presentations_do_not_synthesize_titles_from_empty_or_nonscalar_metadata() {
    for metadata in [
        "author: Alex",
        "title:",
        "title: null",
        "title: NULL",
        "title: Null",
        "title: ~",
        "title: ''",
        "title: \"\\t\\n\"",
        "title: '   '",
        "title: |\n  ",
        "title: >- # Empty\n  ",
        "title: []",
        "title: {}",
        "format:\n  revealjs:\n    title: Nested",
    ] {
        let source = format!("---\n{metadata}\n---\n");
        assert!(
            build_presentation(document(&source)).slides.is_empty(),
            "{metadata}"
        );
        let doc = document(&format!("{source}\n## Content\n"));
        let presentation = build_presentation(doc.clone());
        assert_eq!(presentation.slides.len(), 1, "{metadata}");
        assert_eq!(content(&presentation.slides[0]), doc.blocks);
    }
}

#[test]
fn presentations_own_source_origins_after_the_source_document_is_dropped() {
    let source = "---\ntitle: Café\n---\n\nLeading α.\n\n## *Second*\n\nBody.\n";
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        let doc = document(&source);
        let original = doc.origin.clone();
        let body_origins = [0, 1].map(|index| doc.blocks[index].origin.clone());
        let presentation = build_presentation(doc);

        assert_eq!(presentation.origin.source_span(), original.source_span());
        assert_eq!(
            presentation.origin.chain().count(),
            original.chain().count() + 1
        );
        for (slide, expected) in presentation.slides[1..].iter().zip(body_origins) {
            assert_eq!(slide.origin.source_span(), expected.source_span());
            assert_eq!(slide.origin.chain().count(), expected.chain().count() + 1);
        }
        let _edited_document = document("## A new revision\n");
        assert_eq!(presentation.origin.source_span().text(), source);
        assert_eq!(
            title(&presentation.slides[0])
                .origin
                .source_span()
                .text()
                .trim(),
            "Café"
        );
        assert!(
            content(&presentation.slides[2])[0]
                .origin
                .source_span()
                .text()
                .contains("Second")
        );
    }
}
