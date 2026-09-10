use tractate::{DocumentSummary, summarize_document};

#[test]
fn public_inspection_reports_semantics_and_retains_source_snapshots() {
    let source = include_str!("fixtures/duplicate-labels.qmd").to_owned();
    let inspection = tractate::inspect_document(&source);
    assert_eq!(inspection.summary, summarize_document(&source));
    assert_eq!(inspection.summary.parse_errors, 0);
    assert!(inspection.has_errors());
    assert_eq!(inspection.semantic_errors(), 1);
    let revised = source.replacen("fig-shared", "fig-renamed", 1);
    assert!(!tractate::inspect_document(&revised).has_errors());
    drop(source);

    let diagnostic = &inspection.diagnostics[0];
    assert_eq!(diagnostic.code, tractate::DiagnosticCode::DuplicateLabel);
    assert_eq!(diagnostic.primary.source_span().text(), "fig-shared");
    assert_eq!(
        diagnostic.related[0].origin.source_span().text(),
        "fig-shared"
    );
    assert_eq!(
        diagnostic.primary.source_span().file().text(),
        include_str!("fixtures/duplicate-labels.qmd")
    );
}

#[test]
fn public_facade_reports_document_structure() {
    let source = include_str!("fixtures/minimal.qmd");

    assert_eq!(
        summarize_document(source),
        DocumentSummary {
            slides: 2,
            headings: 1,
            code_blocks: 2,
            executable_cells: 1,
            executable_languages: vec!["r".to_owned()],
            parse_errors: 0,
        }
    );
}

#[test]
fn public_facade_reports_sorted_unique_executable_languages() {
    let source = "```{r}\n1\n```\n\n```python\n1\n```\n\n```{julia}\n1\n```\n\n```{r}\n2\n```\n";

    assert_eq!(
        summarize_document(source),
        DocumentSummary {
            slides: 1,
            headings: 0,
            code_blocks: 4,
            executable_cells: 3,
            executable_languages: vec!["julia".to_owned(), "r".to_owned()],
            parse_errors: 0,
        }
    );
}
