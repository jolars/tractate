use crate::document::*;

fn diagnostics(source: &str) -> Vec<Diagnostic> {
    let lowered = crate::parser::lower(SourceFile::new("slides.qmd", source));
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    super::super::validation::validate(&lowered.document)
}

fn codes(source: &str) -> Vec<&'static str> {
    diagnostics(source)
        .iter()
        .map(|d| d.code.as_str())
        .collect()
}

#[test]
fn validation_rejects_labels_across_slides_and_sessions() {
    let source = include_str!("../../../tests/fixtures/duplicate-labels.qmd");
    let errors = diagnostics(source);
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.code.as_str(), "semantic.duplicate-label");
    assert_eq!(error.severity, Severity::Error);
    assert!(error.message.contains("fig-shared"));
    assert_eq!(error.primary.source_span().text().trim(), "fig-shared");
    assert_eq!(error.related.len(), 1);
    assert_eq!(
        error.related[0].origin.source_span().text().trim(),
        "fig-shared"
    );
    assert!(
        error.primary.source_span().range().start
            > error.related[0].origin.source_span().range().start
    );
}

#[test]
fn validation_reports_unknown_keys_in_both_scopes() {
    let errors = diagnostics(include_str!("../../../tests/fixtures/unknown-options.qmd"));
    assert_eq!(errors.len(), 2);
    for (error, key) in errors
        .iter()
        .zip(["unknown-default", "unknown-cell-option"])
    {
        assert_eq!(error.code.as_str(), "option.unknown");
        assert_eq!(error.primary.source_span().text().trim(), key);
        assert!(error.message.contains(key));
    }
}

#[test]
fn validation_keeps_one_label_namespace_for_blocks_inlines_and_cells() {
    let source = "## Slide {#shared}\n\n::: {#shared}\n\n[Span]{#shared}\n\n::: \n\n```{r}\n#| label: shared\n#| eval: false\n#| include: false\n1\n```\n";
    let errors = diagnostics(source);
    assert_eq!(errors.len(), 3, "{errors:?}");
    for error in errors {
        assert_eq!(error.code.as_str(), "semantic.duplicate-label");
        assert_eq!(
            error.related[0].origin.source_span().range().start,
            source.find("shared").unwrap()
        );
    }
}

#[test]
fn validation_checks_scopes_spelling_duplicates_and_mapping_shapes() {
    for (source, expected) in [
        (
            "---\nexecute: {label: forbidden, fig-cap: caption}\n---\n",
            vec!["option.wrong-scope", "option.wrong-scope"],
        ),
        ("---\nexecute: false\n---\n", vec!["option.invalid-mapping"]),
        (
            "```{r}\n#| [echo, false]\n1\n```\n",
            vec!["option.invalid-mapping"],
        ),
        (
            "```{r}\n#| Echo: false\n#| fig.width: 7\n1\n```\n",
            vec!["option.unknown", "option.unknown"],
        ),
    ] {
        assert_eq!(codes(source), expected, "{source}");
    }
}

#[test]
fn validation_rejects_inline_options_and_yaml_extensions_without_evaluation() {
    for declaration in ["r old-label", "r, echo=FALSE", "r old-label, echo=FALSE"] {
        let errors = diagnostics(&format!("```{{{declaration}}}\n1\n```\n"));
        assert!(!errors.is_empty(), "{declaration}");
        assert!(
            errors
                .iter()
                .all(|d| d.code.as_str() == "option.unsupported-syntax")
        );
    }
    for option in [
        "echo: !expr system('touch sentinel')",
        "echo: !!bool false",
        "inputs: [&paths data.csv, *paths]",
        "<<: {echo: false}",
        "echo: &flag false",
    ] {
        let errors = diagnostics(&format!("```{{r}}\n#| {option}\n1\n```\n"));
        assert!(!errors.is_empty(), "{option}");
        assert!(
            errors
                .iter()
                .all(|d| d.code.as_str() == "option.unsupported-syntax"),
            "{errors:?}"
        );
    }
}

#[test]
fn validation_ignores_ordinary_code_and_unrelated_metadata() {
    let source = "---\ntitle: Test\ncustom: {unknown: true}\nexecute: {echo: false}\n---\n\n```r\n#| unknown: true\n#| label: shared\n```\n\n```{r}\n#| echo: true\n#| label: shared\n1\n```\n";
    assert!(diagnostics(source).is_empty());
    for source in [
        include_str!("../../../tests/fixtures/cell-options.qmd"),
        include_str!("../../../tests/fixtures/document-defaults.qmd"),
        include_str!("../../../tests/fixtures/option-type-boundaries.qmd"),
    ] {
        assert!(diagnostics(source).is_empty());
    }
}

#[test]
fn validation_compares_decoded_labels_and_rejects_invalid_names() {
    assert_eq!(
        codes("```{r}\n#| label: 'café'\n1\n```\n\n```{r}\n#| label: \"caf\\u00e9\"\n2\n```\n"),
        ["semantic.duplicate-label"]
    );
    for label in [
        "''",
        "null",
        "true",
        "42",
        "0x2A",
        "7e0",
        ".nan",
        "[]",
        "{}",
        "'two words'",
        "\"a\\0b\"",
        "\" a\"",
    ] {
        assert_eq!(
            codes(&format!("```{{r}}\n#| label: {label}\n1\n```\n")),
            ["option.invalid-label"],
            "{label}"
        );
    }
    assert!(diagnostics("```{r}\n#| label: '42'\n1\n```\n\n```{r}\n#| label: Name\n1\n```\n\n```{r}\n#| label: name\n1\n```\n").is_empty());
}

#[test]
fn validation_preserves_nested_unicode_and_crlf_origins() {
    let source = "Café.\n\n> ```{r}\n> #| unknown: true\n> 1\n> ```\n\n- ```{r}\n  #| unknown: true\n  1\n  ```\n";
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        let errors = diagnostics(&source);
        assert_eq!(errors.len(), 2);
        for (error, (offset, _)) in errors.iter().zip(source.match_indices("unknown")) {
            let span = error.primary.source_span();
            assert_eq!(span.text().trim(), "unknown");
            assert_eq!(span.range().start, offset);
            assert_eq!(span.file().text(), source);
        }
    }
}

#[test]
fn validation_rejects_duplicate_keys_even_when_the_parser_rejects_the_mapping() {
    for source in [
        "```{r}\n#| echo: true\n#| echo: false\n1\n```\n",
        "---\nexecute: {echo: true, echo: false}\n---\n",
        "---\nexecute: {}\nexecute: {}\n---\n",
    ] {
        let inspection = crate::inspect_document(source);
        assert!(inspection.has_errors(), "{source}");
        assert_eq!(inspection.diagnostics.len(), 1);
        assert_eq!(
            inspection.diagnostics[0].code.as_str(),
            "syntax.invalid-yaml"
        );
        assert!(inspection.diagnostics[0].message.contains("duplicate"));
    }
}

#[test]
fn validation_checks_block_label_chomping_and_explicit_indentation() {
    for (declaration, expected) in [
        ("|-\n#|   shared", "semantic.duplicate-label"),
        (">-\n#|   shared", "semantic.duplicate-label"),
        ("|2-\n#|   shared", "semantic.duplicate-label"),
        ("|2-\n#|     shared", "option.invalid-label"),
        (
            "|-\n#|   shared\n#|     \n#| echo: false",
            "option.invalid-label",
        ),
        ("|\n#|   shared", "option.invalid-label"),
        ("|-\n#|\n#|   shared", "option.invalid-label"),
        (">-\n#|   two\n#|   words", "option.invalid-label"),
    ] {
        let source = format!("## First {{#shared}}\n\n```{{r}}\n#| label: {declaration}\n1\n```\n");
        assert_eq!(codes(&source), [expected], "{source}");
    }
    let source = "## First {#shared}\n\n> ```{r}\n> #| label: |2-\n> #|   shared\n> 1\n> ```\n";
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        assert_eq!(codes(&source), ["semantic.duplicate-label"]);
    }
}

#[test]
fn validation_checks_defaults_without_cells_and_nested_yaml_extensions() {
    for source in [
        "---\nexecute: {echo: !expr 1 + 1}\n---\n",
        "---\nexecute:\n  inputs:\n    - &path data.csv\n---\n",
        "---\nexecute: {unknown: false}\n---\n\n```{r}\n#| unknown: true\n1\n```\n",
    ] {
        assert!(!diagnostics(source).is_empty(), "{source}");
    }
}
