use super::*;
use crate::compiler::{RenderOptions, static_html};
use crate::document::{DiagnosticCode, SourceRange};

mod dependencies;

#[test]
fn compiler_state_revision_exhaustion_preserves_the_current_snapshot() {
    let mut compiler = Compiler {
        current: CompilerSnapshot::new(
            SourceFile::anonymous("## Before\n"),
            SourceRevision(u64::MAX - 1),
        ),
    };
    assert_eq!(
        compiler
            .update_source(SourceFile::anonymous("## Last\n"))
            .get(),
        u64::MAX
    );
    let last = compiler.snapshot();
    assert_eq!(
        compiler.update_source(SourceFile::anonymous("## Last\n")),
        last.revision()
    );
    let overflow = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compiler.update_source(SourceFile::anonymous("## Overflow\n"));
    }));
    assert!(overflow.is_err());
    assert_eq!(compiler.revision(), last.revision());
    assert!(Arc::ptr_eq(&compiler.current.0, &last.0));
}

#[test]
fn compiler_state_shares_reads_and_releases_unretained_revisions() {
    let mut compiler = Compiler::new(SourceFile::anonymous("## Before\n"));
    let previous = Arc::downgrade(&compiler.current.0);
    let retained = compiler.snapshot();
    assert!(std::ptr::eq(
        retained.presentation(),
        compiler.snapshot().presentation()
    ));
    compiler.update_source(SourceFile::anonymous("## After\n"));
    assert!(previous.upgrade().is_some());
    drop(retained);
    assert!(previous.upgrade().is_none());
}

#[derive(Debug, PartialEq, Eq)]
struct HtmlObservation {
    title: String,
    slides: Vec<String>,
    resources: Vec<(String, String)>,
}

fn html(
    snapshot: &CompilerSnapshot,
    options: RenderOptions,
) -> Result<HtmlObservation, Vec<(DiagnosticCode, SourceRange, String)>> {
    let compiled = static_html::compile(snapshot, options).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| {
                assert_eq!(diagnostic.primary.source_span().file(), snapshot.source());
                (
                    diagnostic.code,
                    diagnostic.primary.source_span().range(),
                    diagnostic.message,
                )
            })
            .collect::<Vec<_>>()
    })?;
    for slide in &compiled.slides {
        assert_eq!(slide.origin.source_span().file(), snapshot.source());
    }
    for resource in &compiled.resources {
        assert_eq!(resource.origin.source_span().file(), snapshot.source());
    }
    Ok(HtmlObservation {
        title: compiled.title,
        slides: compiled
            .slides
            .into_iter()
            .map(|slide| slide.html)
            .collect(),
        resources: compiled
            .resources
            .into_iter()
            .map(|resource| (resource.source, resource.destination))
            .collect(),
    })
}

#[test]
fn compiler_state_html_matches_clean_builds_after_edits_and_errors() {
    let initial = "## Original\n\n![Plot](missing.svg)\n";
    let mut compiler = Compiler::new(SourceFile::new("slides.qmd", initial));
    let retained = compiler.snapshot();
    let original_html = html(&retained, RenderOptions::default()).unwrap();
    assert_eq!(original_html.resources.len(), 1);
    for text in [
        "---\ntitle: Updated\n---\n\n## First\n\ncafé and α\n\n## Second\n",
        include_str!("../../../tests/fixtures/malformed-options.qmd"),
        include_str!("../../../tests/fixtures/duplicate-labels.qmd"),
        "---\nexecute:\n  eval: false\n---\n\n> ```{r}\n> 1 + 1\n> ```\n",
        "```{r}\nstop('must not run')\n```\n",
        "```{r}\n#| eval: 'false'\n1\n```\n",
        "## Second\n\n## First\n",
        "",
        initial,
    ] {
        compiler.update_source(SourceFile::new("slides.qmd", text));
        let snapshot = compiler.snapshot();
        let fresh = Compiler::new(SourceFile::new("slides.qmd", text)).snapshot();
        for no_execute in [false, true] {
            let options = RenderOptions { no_execute };
            assert_eq!(html(&snapshot, options), html(&fresh, options));
        }
        assert_eq!(
            html(&retained, RenderOptions::default()).unwrap(),
            original_html
        );
    }
    assert_eq!(compiler.revision().get(), 9);
    assert_eq!(retained.revision().get(), 0);
}

#[test]
fn compiler_state_path_edits_update_html_title_and_diagnostic_origins() {
    let mut compiler = Compiler::new(SourceFile::new("first.qmd", "## Slide\n"));
    let first = compiler.snapshot();
    compiler.update_source(SourceFile::new("second.qmd", first.source().text()));
    assert_eq!(
        html(&first, RenderOptions::default()).unwrap().title,
        "first"
    );
    assert_eq!(
        html(&compiler.snapshot(), RenderOptions::default())
            .unwrap()
            .title,
        "second"
    );

    compiler.update_source(SourceFile::new("invalid.qmd", "```{r}\n1\n```\n"));
    let current = compiler.snapshot();
    let diagnostics = static_html::compile(&current, RenderOptions { no_execute: true })
        .err()
        .unwrap();
    assert_eq!(
        diagnostics[0].primary.source_span().file(),
        current.source()
    );
    assert_eq!(
        diagnostics[0].primary.source_span().file().path(),
        Some(Path::new("invalid.qmd"))
    );
    assert_eq!(current.revision().get(), 2);
}
