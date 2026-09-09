use std::path::Path;

use super::*;

#[test]
fn diagnostics_retain_parser_messages_and_precise_qmd_origins() {
    for source in [
        include_str!("../../../tests/fixtures/malformed-frontmatter.qmd"),
        include_str!("../../../tests/fixtures/malformed-defaults.qmd"),
        include_str!("../../../tests/fixtures/malformed-options.qmd"),
        "Café α.\n\n> ```{r}\n> #| fig-cap: café\n> #| echo: [\n> 1\n> ```\n",
        "Café α.\n\n- ```{r}\n  #| fig-cap: café\n  #| echo: [\n  1\n  ```\n",
    ] {
        for source in [source.to_owned(), source.replace('\n', "\r\n")] {
            let file = SourceFile::new("slides.qmd", source.clone());
            let parsed = crate::parser::parse(&source);
            let lowered = lower(file.clone());
            assert_eq!(lowered.diagnostics.len(), 1, "{source:?}");
            assert_eq!(lowered.error_count(), 1);

            let diagnostic = &lowered.diagnostics[0];
            assert_eq!(diagnostic.severity, Severity::Error);
            assert_eq!(diagnostic.code, DiagnosticCode::InvalidYaml);
            assert_eq!(diagnostic.code.as_str(), "syntax.invalid-yaml");
            assert_eq!(diagnostic.message, parsed.errors()[0].message);
            assert!(!diagnostic.message.is_empty());
            assert!(diagnostic.related.is_empty());

            let span = diagnostic.primary.source_span();
            assert_eq!(span.file(), &file);
            assert_eq!(span.file().path(), Some(Path::new("slides.qmd")));
            assert_eq!(span.range().start, source.find('[').unwrap());
            assert_eq!(
                span.range(),
                crate::parser::lowering::range(parsed.errors()[0].range)
            );
            assert!(span.text().starts_with('['));
        }
    }
}

#[test]
fn diagnostics_map_quoted_list_errors_independently_of_cst_ranges() {
    // Panache 0.29 duplicates a quote marker in this CST. Its diagnostic offsets
    // still describe the QMD and must not be remapped through that tree.
    let source = "Café α.\n\n> - ```{r}\n>   #| fig-cap: café\n>   #| echo: [\n>   1\n>   ```\n";
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        let file = SourceFile::new("slides.qmd", source.clone());
        let parsed = crate::parser::parse(&source);
        let diagnostics = crate::parser::lowering::diagnostics(&file, parsed.errors());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].primary.source_span().file(), &file);
        assert_eq!(
            diagnostics[0].primary.source_span().range().start,
            source.find('[').unwrap()
        );
        assert!(diagnostics[0].primary.source_span().text().starts_with('['));
    }
}

#[test]
fn diagnostics_accept_empty_eof_ranges_and_codes_ignore_message_wording() {
    use panache_parser::parser::{SyntaxError, SyntaxErrorSource};
    use panache_parser::syntax::TextRange;

    let file = SourceFile::anonymous("é");
    let errors: Vec<_> = ["Unexpected end of YAML.", "A revised upstream message."]
        .into_iter()
        .map(|message| SyntaxError {
            source: SyntaxErrorSource::Yaml,
            message: message.into(),
            range: TextRange::empty(2.into()),
        })
        .collect();
    let diagnostics = crate::parser::lowering::diagnostics(&file, &errors);
    for (diagnostic, error) in diagnostics.iter().zip(&errors) {
        assert_eq!(diagnostic.code.as_str(), "syntax.invalid-yaml");
        assert_eq!(diagnostic.message, error.message);
        assert_eq!(
            diagnostic.primary.source_span().range(),
            SourceRange { start: 2, end: 2 }
        );
        assert_eq!(diagnostic.primary.source_span().text(), "");
        assert!(diagnostic.primary.source_span().file().path().is_none());
    }
}

#[test]
fn diagnostics_keep_multiple_errors_and_survive_source_revisions() {
    let source = "---\ntitle: [\n---\n\n```{r}\n#| echo: [\n1\n```\n";
    let lowered = lower(SourceFile::new("slides.qmd", source));
    assert_eq!(lowered.error_count(), 2);
    let diagnostics = lowered.diagnostics;
    drop(lowered.document);

    let revised = lower(SourceFile::new("slides.qmd", source.replace('[', "false")));
    assert!(revised.diagnostics.is_empty());
    assert_eq!(revised.error_count(), 0);
    for (diagnostic, (start, _)) in diagnostics.iter().zip(source.match_indices('[')) {
        let span = diagnostic.primary.source_span();
        assert_eq!(span.file().text(), source);
        assert_ne!(span.file(), &revised.document.source);
        assert_eq!(span.range().start, start);
        assert_eq!(diagnostic.code.as_str(), "syntax.invalid-yaml");
    }
}

#[test]
fn diagnostics_preserve_related_origins_and_generated_provenance() {
    let lowered = lower(SourceFile::new("slides.qmd", "---\ntitle: [\n---\n"));
    let mut diagnostic = lowered.diagnostics.into_iter().next().unwrap();
    let source_origin = diagnostic.primary.clone();
    let generated_file = SourceFile::new(".build/metadata.yaml", "title: [\n");
    let generated_span = generated_file
        .span(SourceRange { start: 7, end: 8 })
        .unwrap();
    diagnostic.primary = source_origin
        .derived("metadata lowering")
        .generated("metadata emission", generated_span.clone());

    let related_file = SourceFile::new("defaults.qmd", "title: Default\n");
    let related_origin = Origin::Source(
        related_file
            .span(SourceRange { start: 7, end: 14 })
            .unwrap(),
    );
    diagnostic.related.push(RelatedOrigin {
        message: "The default title was declared here.".into(),
        origin: related_origin.clone(),
    });
    diagnostic.related.push(RelatedOrigin {
        message: "The title overrides this declaration.".into(),
        origin: related_origin.derived("document defaults"),
    });
    drop(lowered.document);
    drop(generated_file);
    drop(related_file);

    assert_eq!(
        diagnostic.primary.source_span(),
        source_origin.source_span()
    );
    assert_eq!(diagnostic.primary.span(), Some(&generated_span));
    assert_eq!(diagnostic.primary.chain().count(), 3);
    assert_eq!(diagnostic.related.len(), 2);
    assert_eq!(
        diagnostic.related[0].message,
        "The default title was declared here."
    );
    assert_eq!(diagnostic.related[0].origin, related_origin);
    assert_eq!(diagnostic.related[1].origin.source_span().text(), "Default");
    assert_eq!(
        diagnostic.related[1].origin.source_span().file().path(),
        Some(Path::new("defaults.qmd"))
    );
    assert_eq!(diagnostic.related[1].origin.chain().count(), 2);
}

#[test]
fn diagnostics_count_only_errors() {
    let mut lowered = lower(SourceFile::anonymous("---\ntitle: [\n---\n"));
    for severity in [Severity::Warning, Severity::Note] {
        let mut diagnostic = lowered.diagnostics[0].clone();
        diagnostic.severity = severity;
        lowered.diagnostics.push(diagnostic);
    }
    assert_eq!(lowered.diagnostics.len(), 3);
    assert_eq!(lowered.error_count(), 1);
    lowered.diagnostics.remove(0);
    assert_eq!(lowered.error_count(), 0);
}
