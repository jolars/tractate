use super::*;
use crate::compiler::queries::QueryKey;
use crate::document::{BlockKind, SlideKind};

#[test]
fn dependencies_connect_nested_semantics_to_their_rendered_slide() {
    let snapshot = Compiler::new(SourceFile::anonymous(
        "## First\n\n> Nested prose.\n\n## Second\n\nOther prose.\n",
    ))
    .snapshot();
    let compilation = snapshot.html(RenderOptions::default());
    assert!(compilation.result.is_ok());
    let slides = &snapshot.presentation().slides;
    let SlideKind::Content(blocks) = &slides[0].kind else {
        panic!("expected content");
    };
    let quote = blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::Quote(_)))
        .unwrap();
    let BlockKind::Quote(nested) = &quote.kind else {
        unreachable!()
    };
    let graph = &compilation.dependencies;
    let affected = graph.dependents(&QueryKey::SemanticBlock(nested[0].id));
    assert!(affected.contains(&QueryKey::SemanticBlock(quote.id)));
    assert!(affected.contains(&QueryKey::Slide(slides[0].id)));
    assert!(affected.contains(&QueryKey::HtmlSlide(slides[0].id)));
    assert!(!affected.contains(&QueryKey::HtmlSlide(slides[1].id)));
    let from_source = graph.dependents(&QueryKey::Source);
    for slide in slides {
        assert!(from_source.contains(&QueryKey::HtmlSlide(slide.id)));
    }
    graph.assert_complete();
}

#[test]
fn dependencies_record_only_reference_lookups_used_by_each_fragment() {
    let snapshot = Compiler::new(SourceFile::anonymous(
        "[shared]: shared.svg\n\n[other]: other.svg\n\n## First\n\n![Plot][shared]\n\n## Second\n\n[File][other]\n\n## Third\n\nPlain prose.\n",
    ))
    .snapshot();
    let compilation = snapshot.html(RenderOptions::default());
    assert!(compilation.result.is_ok());
    let slides = &snapshot.presentation().slides;
    let affected = compilation
        .dependencies
        .dependents(&QueryKey::Reference("shared".into()));
    assert!(affected.contains(&QueryKey::HtmlSlide(slides[0].id)));
    assert!(!affected.contains(&QueryKey::HtmlSlide(slides[1].id)));
    assert!(!affected.contains(&QueryKey::HtmlSlide(slides[2].id)));
    let definition = &snapshot.presentation().preamble[0];
    let affected = compilation
        .dependencies
        .dependents(&QueryKey::SemanticBlock(definition.id));
    assert!(affected.contains(&QueryKey::HtmlSlide(slides[0].id)));
    assert!(!affected.contains(&QueryKey::HtmlSlide(slides[1].id)));
    assert!(!affected.contains(&QueryKey::HtmlSlide(slides[2].id)));
    compilation.dependencies.assert_complete();
}

#[test]
fn dependencies_connect_title_metadata_to_the_title_and_deck() {
    let snapshot = Compiler::new(SourceFile::anonymous(
        "---\ntitle: A title\n---\n\n## Prose\n\nBody.\n",
    ))
    .snapshot();
    let compilation = snapshot.html(RenderOptions::default());
    let compiled = compilation.result.as_ref().unwrap();
    assert_eq!(compiled.title, "A title");
    assert!(compiled.slides[0].html.contains("<h1>A title</h1>"));
    let affected = compilation.dependencies.dependents(&QueryKey::Metadata);
    assert!(affected.contains(&QueryKey::HtmlTitle));
    assert!(affected.contains(&QueryKey::HtmlSlide(compiled.slides[0].id)));
    assert!(!affected.contains(&QueryKey::HtmlSlide(compiled.slides[1].id)));
    assert!(affected.contains(&QueryKey::HtmlDeck(RenderOptions::default())));
    compilation.dependencies.assert_complete();
}

#[test]
fn dependencies_retain_reads_before_a_fragment_error() {
    let snapshot = Compiler::new(SourceFile::anonymous(
        "[broken]: image%00.svg\n\n## First\n\n![Image][broken]\n\n## Later\n\nProse.\n",
    ))
    .snapshot();
    let compilation = snapshot.html(RenderOptions::default());
    let diagnostics = compilation.result.as_ref().err().unwrap();
    assert_eq!(diagnostics[0].code, DiagnosticCode::ResourceUnavailable);
    let slides = &snapshot.presentation().slides;
    let affected = compilation
        .dependencies
        .dependents(&QueryKey::Reference("broken".into()));
    assert!(affected.contains(&QueryKey::HtmlSlide(slides[0].id)));
    assert!(affected.contains(&QueryKey::HtmlDeck(RenderOptions::default())));
    assert!(
        !compilation
            .dependencies
            .dependents(&QueryKey::Slide(slides[1].id))
            .contains(&QueryKey::HtmlSlide(slides[1].id))
    );
    compilation.dependencies.assert_complete();
}

#[test]
fn dependencies_prepare_resources_without_reading_or_writing_files() {
    let directory = tempfile::tempdir().unwrap();
    let snapshot = Compiler::new(SourceFile::new(
        directory.path().join("missing/deck.qmd"),
        "## Image\n\n![Missing](missing.svg)\n",
    ))
    .snapshot();
    for no_execute in [false, true] {
        let compilation = snapshot.html(RenderOptions { no_execute });
        let compiled = compilation.result.as_ref().unwrap();
        assert_eq!(compiled.resources.len(), 1);
        assert_eq!(compiled.resources[0].source, "missing.svg");
        assert!(
            compiled.slides[0]
                .html
                .contains(&compiled.resources[0].destination)
        );
        compilation.dependencies.assert_complete();
    }
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn dependencies_connect_display_defaults_only_to_fragments_with_cells() {
    let snapshot = Compiler::new(SourceFile::anonymous(
        "---\nexecute:\n  eval: false\n  echo: false\n---\n\n## Code\n\n> ```{r}\n> hidden()\n> ```\n\n## Prose\n\nVisible.\n",
    ))
    .snapshot();
    let compilation = snapshot.html(RenderOptions::default());
    let compiled = compilation.result.as_ref().unwrap();
    assert!(!compiled.slides[0].html.contains("hidden()"));
    let slides = &snapshot.presentation().slides;
    let affected = compilation
        .dependencies
        .dependents(&QueryKey::DisplayDefaults);
    assert!(affected.contains(&QueryKey::HtmlSlide(slides[0].id)));
    assert!(!affected.contains(&QueryKey::HtmlSlide(slides[1].id)));
    compilation.dependencies.assert_complete();
}

#[test]
fn dependencies_memoize_success_and_failure_without_mixing_render_policies() {
    let mut compiler = Compiler::new(SourceFile::anonymous("```{r}\n1\n```\n"));
    let snapshot = compiler.snapshot();
    let normal = snapshot.html(RenderOptions::default());
    let forbidden = snapshot.html(RenderOptions { no_execute: true });
    assert_eq!(
        normal.result.as_ref().err().unwrap()[0].code,
        DiagnosticCode::ExecutionUnavailable
    );
    assert_eq!(
        forbidden.result.as_ref().err().unwrap()[0].code,
        DiagnosticCode::ResultUnavailable
    );
    assert!(std::ptr::eq(
        normal,
        snapshot.html(RenderOptions::default())
    ));
    assert!(std::ptr::eq(
        forbidden,
        snapshot.html(RenderOptions { no_execute: true })
    ));
    normal.dependencies.assert_complete();
    forbidden.dependencies.assert_complete();

    compiler.update_source(SourceFile::anonymous("## Repaired\n"));
    let repaired = compiler.snapshot();
    let success = repaired.html(RenderOptions::default());
    assert!(success.result.is_ok());
    assert!(std::ptr::eq(
        success,
        repaired.html(RenderOptions::default())
    ));
    assert!(normal.result.is_err());
    success.dependencies.assert_complete();
}
