use tractate::{Compiler, DiagnosticCode, SourceFile, inspect_document};

#[test]
fn compiler_state_advances_revisions_for_changes_and_reverts() {
    let initial = SourceFile::new("slides.qmd", "## First\n");
    let mut compiler = Compiler::new(initial.clone());
    let first = compiler.snapshot();
    assert_eq!(first.revision().get(), 0);
    assert_eq!(compiler.revision(), first.revision());
    assert_eq!(first.source(), &initial);
    assert_eq!(first.inspection().summary.slides, 1);

    let second = compiler.update_source(SourceFile::new("slides.qmd", "## First\n\n## Second\n"));
    assert!(second > first.revision());
    assert_eq!(second.get(), 1);
    assert_eq!(compiler.snapshot().inspection().summary.slides, 2);
    assert_eq!(compiler.update_source(initial.clone()).get(), 2);
    assert_eq!(compiler.snapshot().source(), &initial);
    assert_eq!(compiler.snapshot().inspection().summary.slides, 1);

    drop(compiler);
    assert_eq!(first.revision().get(), 0);
    assert_eq!(first.source().text(), "## First\n");
    assert_eq!(first.inspection().summary.slides, 1);
}

#[test]
fn compiler_state_keeps_identical_input_and_reads_in_the_same_revision() {
    let source = SourceFile::anonymous("## A slide\n");
    let mut compiler = Compiler::new(source.clone());
    let first = compiler.snapshot();
    for input in [source, SourceFile::anonymous("## A slide\n")] {
        assert_eq!(compiler.update_source(input), first.revision());
        let current = compiler.snapshot();
        assert_eq!(current.source(), first.source());
        assert!(std::ptr::eq(current.inspection(), first.inspection()));
    }
    assert_eq!(compiler.revision(), first.revision());
}

#[test]
fn compiler_state_path_changes_advance_without_accessing_the_filesystem() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing/deck.qmd");
    let text = "## A slide\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(text));
    assert_eq!(
        compiler.update_source(SourceFile::new(&path, text)).get(),
        1
    );
    assert_eq!(compiler.snapshot().source().path(), Some(path.as_path()));
    let moved = directory.path().join("missing/other.qmd");
    assert_eq!(
        compiler.update_source(SourceFile::new(moved, text)).get(),
        2
    );
    assert_eq!(compiler.update_source(SourceFile::anonymous(text)).get(), 3);
    assert!(compiler.snapshot().source().path().is_none());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn compiler_state_replaces_diagnostics_on_invalid_edits_and_repair() {
    let duplicate = include_str!("fixtures/duplicate-labels.qmd");
    let malformed = include_str!("fixtures/malformed-frontmatter.qmd");
    let mut compiler = Compiler::new(SourceFile::new("slides.qmd", "## Valid\n"));
    compiler.update_source(SourceFile::new("slides.qmd", duplicate));
    let semantic_error = compiler.snapshot();
    assert_eq!(semantic_error.revision().get(), 1);
    assert_eq!(semantic_error.inspection().semantic_errors(), 1);
    let diagnostic = &semantic_error.inspection().diagnostics[0];
    assert_eq!(diagnostic.code, DiagnosticCode::DuplicateLabel);
    assert_eq!(
        diagnostic.primary.source_span().file(),
        semantic_error.source()
    );
    assert_eq!(
        diagnostic.related[0].origin.source_span().file(),
        semantic_error.source()
    );

    compiler.update_source(SourceFile::new("slides.qmd", malformed));
    let syntax_error = compiler.snapshot();
    assert_eq!(syntax_error.revision().get(), 2);
    assert!(syntax_error.inspection().summary.parse_errors > 0);
    assert_eq!(syntax_error.inspection().semantic_errors(), 0);

    compiler.update_source(SourceFile::new("slides.qmd", "## Repaired\n"));
    let repaired = compiler.snapshot();
    assert_eq!(repaired.revision().get(), 3);
    assert!(!repaired.inspection().has_errors());
    drop(compiler);
    assert_eq!(semantic_error.source().text(), duplicate);
    assert_eq!(diagnostic.primary.source_span().text(), "fig-shared");
    assert_eq!(syntax_error.source().text(), malformed);
    assert!(syntax_error.inspection().has_errors());
}

#[test]
fn compiler_state_inspection_matches_clean_builds_through_edit_sequences() {
    let mut compiler = Compiler::new(SourceFile::anonymous(""));
    for (index, text) in [
        include_str!("fixtures/source-content.qmd"),
        include_str!("fixtures/duplicate-labels.qmd"),
        include_str!("fixtures/malformed-options.qmd"),
        include_str!("fixtures/unknown-options.qmd"),
        include_str!("fixtures/source-content.qmd"),
        "",
    ]
    .into_iter()
    .enumerate()
    {
        compiler.update_source(SourceFile::anonymous(text));
        let current = compiler.snapshot();
        let fresh = inspect_document(text);
        assert_eq!(current.revision().get(), index as u64 + 1);
        assert_eq!(current.inspection().summary, fresh.summary);
        assert_eq!(
            current.inspection().diagnostics.len(),
            fresh.diagnostics.len()
        );
        for (actual, expected) in current
            .inspection()
            .diagnostics
            .iter()
            .zip(&fresh.diagnostics)
        {
            assert_eq!(actual.code, expected.code);
            assert_eq!(actual.severity, expected.severity);
            assert_eq!(actual.message, expected.message);
            assert_eq!(
                actual.primary.source_span().range(),
                expected.primary.source_span().range()
            );
            assert_eq!(actual.primary.source_span().file(), current.source());
        }
    }
}
