use std::collections::HashSet;

use crate::compiler::{build_presentation, presentation::lower_presentation};
use crate::document::*;

fn source(source: &str) -> SourcePresentation {
    let lowered = crate::parser::lower(SourceFile::new("slides.qmd", source));
    assert_eq!(lowered.error_count(), 0, "{:?}", lowered.diagnostics);
    build_presentation(lowered.document)
}

fn cells<C>(presentation: &Presentation<C>, mut visit: impl FnMut(&C)) {
    presentation.visit_blocks(&mut |block| {
        if let BlockKind::Cell(cell) = &block.kind {
            visit(cell);
        }
    });
}

#[test]
fn presentation_ir_uses_distinct_cell_leaves_and_result_slots() {
    let source = source(
        "## Results\n\n```r\n#| label: literal\n1\n```\n\n```{r}\n1\n```\n\n```{r}\n1\n```\n",
    );
    let mut expected = Vec::new();
    cells(&source, |cell| expected.push(cell.clone()));
    let presentation: Presentation = lower_presentation(source);
    let mut actual = Vec::new();
    cells(&presentation, |cell| actual.push(cell.clone()));
    assert_eq!(actual.len(), 2);
    let slots: HashSet<CellId> = actual.iter().map(|cell| cell.result.cell).collect();
    assert_eq!(slots.len(), 2);
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.result.cell, expected.id);
        assert_eq!(actual.source, expected.code);
        assert_eq!(actual.options, expected.options);
        assert_eq!(actual.preamble, expected.preamble);
        assert_eq!(
            actual.result.origin.source_span(),
            expected.origin.source_span()
        );
        assert_eq!(
            actual.result.origin.chain().count(),
            expected.origin.chain().count() + 1
        );
    }
    let SlideKind::Content(blocks) = &presentation.slides[0].kind else {
        panic!("content slide")
    };
    assert!(matches!(blocks[1].kind, BlockKind::Code(_)));
}

fn recover(presentation: Presentation) -> SourcePresentation {
    Presentation {
        origin: presentation.origin.chain().nth(1).unwrap().clone(),
        metadata: presentation.metadata,
        preamble: presentation
            .preamble
            .into_iter()
            .map(|block| block.map_cells(&mut recover_cell))
            .collect(),
        slides: presentation
            .slides
            .into_iter()
            .map(|slide| Slide {
                id: slide.id,
                origin: slide.origin.chain().nth(1).unwrap().clone(),
                kind: match slide.kind {
                    SlideKind::Title(title) => SlideKind::Title(title),
                    SlideKind::Content(blocks) => SlideKind::Content(
                        blocks
                            .into_iter()
                            .map(|block| block.map_cells(&mut recover_cell))
                            .collect(),
                    ),
                },
            })
            .collect(),
    }
}

fn recover_cell(cell: PresentedCell) -> Cell {
    Cell {
        id: cell.id,
        origin: cell.origin.chain().nth(1).unwrap().clone(),
        code: cell.source,
        options: cell.options,
        preamble: cell.preamble,
    }
}

#[test]
fn presentation_ir_preserves_the_complete_tree_and_source_identities() {
    for text in [
        include_str!("../../../tests/fixtures/source-content.qmd"),
        "<!-- Preamble. -->\n\n[^before]: Before the slides.\n\nLeading.\n\n## Slide {#slide}\n\n::: {#container}\n\n> - **Nested** [link](https://example.com).\n>\n>   ```{r}\n>   #| label: nested\n>   1\n>   ```\n\n:::\n\n| Table |\n| ----- |\n| [Span]{#table-span} |\n\n[^after]: A note.\n\n    ```{r}\n    2\n    ```\n",
    ] {
        for text in [text.to_owned(), text.replace('\n', "\r\n")] {
            let source = source(&text);
            let presentation = lower_presentation(source.clone());
            assert_eq!(recover(presentation), source);
        }
    }
}

#[test]
fn presentation_ir_reaches_cells_in_nested_and_preserved_containers() {
    let source = source(
        "[^before]: Preamble note.\n\n    ```{r}\n    0\n    ```\n\n## Slide\n\n::: {.callout-note}\n\n> ```{r}\n> 1\n> ```\n\n- ```{r}\n  2\n  ```\n\n:::\n\n[^after]: Later note.\n\n    ```{r}\n    3\n    ```\n",
    );
    let mut expected = Vec::new();
    cells(&source, |cell| expected.push(cell.id));
    assert_eq!(expected.len(), 4);
    assert!(!source.preamble.is_empty());
    let presentation = lower_presentation(source);
    let mut actual = Vec::new();
    cells(&presentation, |cell| actual.push(cell.result.cell));
    assert_eq!(actual, expected);
}

#[test]
fn presentation_ir_slots_preserve_declarations_without_implying_required_results() {
    for text in [
        include_str!("../../../tests/fixtures/cell-options.qmd"),
        include_str!("../../../tests/fixtures/document-defaults.qmd"),
    ] {
        let source = source(text);
        let mut expected = Vec::new();
        cells(&source, |cell| expected.push(cell.id));
        let presentation = lower_presentation(source.clone());
        let mut slots = Vec::new();
        cells(&presentation, |cell| slots.push(cell.result.cell));
        // Disabled and hidden cells retain potential placement until the pure
        // option-resolution pass decides whether their output is needed.
        assert_eq!(slots, expected);
        assert_eq!(recover(presentation), source);
    }
}

#[test]
fn presentation_ir_source_only_documents_have_no_result_slots() {
    for text in [
        "",
        "<!-- A comment. -->\n",
        "---\ntitle: Deck\n---\n",
        "Leading.\n\n## Slide\n\n```r\n1\n```\n\n$x$\n",
    ] {
        let source = source(text);
        let presentation = lower_presentation(source.clone());
        cells(&presentation, |_| panic!("source-only document has a slot"));
        assert_eq!(recover(presentation), source);
    }
}

#[test]
fn presentation_ir_owns_its_origins_after_source_is_dropped() {
    let text = "---\ntitle: Café\n---\n\n## α\n\n```{r}\nstop('must not execute')\n```\n";
    let presentation = lower_presentation(source(text));
    let _edited = source("## A different revision\n");
    assert_eq!(presentation.origin.source_span().file().text(), text);
    cells(&presentation, |cell| {
        assert_eq!(cell.result.origin.source_span().file().text(), text);
        assert!(
            cell.result
                .origin
                .source_span()
                .text()
                .contains("stop('must not execute')")
        );
    });
}
