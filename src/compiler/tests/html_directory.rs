use crate::compiler::{build_presentation, presentation::lower_presentation, validation};
use crate::document::SourceFile;
use crate::render::html::{HtmlAsset, HtmlDirectory, render_presentation};

#[test]
fn html_directory_compiles_source_only_qmd_without_cell_results() {
    let lowered = crate::parser::lower(SourceFile::new(
        "slides.qmd",
        include_str!("../../../tests/fixtures/static-deck.qmd"),
    ));
    assert_eq!(lowered.error_count(), 0, "{:?}", lowered.diagnostics);
    assert!(validation::validate(&lowered.document).is_empty());
    let presentation = lower_presentation(build_presentation(lowered.document));
    let slides = render_presentation(&presentation, |_| {
        panic!("Source-only output must not request cell results")
    })
    .unwrap();
    let output = tempfile::tempdir().unwrap();
    let destination = output.path().join("deck");
    let directory = HtmlDirectory::new(
        "A static deck",
        &slides,
        &[HtmlAsset {
            path: "figures/plot.svg",
            bytes: include_bytes!("../../../tests/fixtures/figures/plot.svg"),
        }],
    )
    .unwrap();
    directory.write(&destination).unwrap();
    let html = std::fs::read_to_string(destination.join("index.html")).unwrap();
    assert_eq!(html.matches("<section data-slide-id=").count(), 4);
    assert!(html.contains("<h1>A static deck</h1>"));
    assert!(html.contains("<strong>leading content</strong>"));
    assert!(html.contains("\\(x^2\\)"));
    assert!(html.contains("class=\"math display\">\\["));
    assert!(html.contains("Ordinary prose costs $5, then $10."));
    assert!(html.contains("<code class=\"language-r\">"));
    assert!(html.contains("src=\"figures/plot.svg\""));
    assert!(destination.join("figures/plot.svg").is_file());
}
