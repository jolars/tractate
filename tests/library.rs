use tractate::{DocumentSummary, summarize_document};

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
