use std::collections::HashSet;
use std::convert::Infallible;

use crate::compiler::{build_presentation, presentation::lower_presentation};
use crate::document::*;
use crate::render::html::render_sections;

fn presentation(source: &str) -> Presentation {
    let lowered = crate::parser::lower(SourceFile::new("slides.qmd", source));
    assert_eq!(lowered.error_count(), 0, "{:?}", lowered.diagnostics);
    lower_presentation(build_presentation(lowered.document))
}

#[test]
fn html_sections_wrap_every_slide_in_presentation_order() {
    let presentation = presentation(
        "---\ntitle: Café\n---\n\n<!-- Preamble. -->\n\nLeading.\n\n## Same\n\n## Same\n\n##\n",
    );
    assert_eq!(presentation.slides.len(), 5);
    assert!(!presentation.preamble.is_empty());
    let bodies = [
        "<h1>Café</h1>\n",
        "<p>Leading.</p>\n",
        "<h2>Same</h2>\n",
        "<h2>Same</h2>\n",
        "",
    ];
    let mut visited = Vec::new();
    let rendered = render_sections(&presentation, |slide| {
        assert_eq!(slide, &presentation.slides[visited.len()]);
        let body = bodies[visited.len()];
        visited.push(slide.id);
        Ok::<_, Infallible>(body.to_owned())
    })
    .unwrap();

    assert_eq!(visited.len(), bodies.len());
    assert_eq!(rendered.len(), bodies.len());
    assert_eq!(visited.iter().collect::<HashSet<_>>().len(), bodies.len());
    for (index, (section, body)) in rendered.iter().zip(bodies).enumerate() {
        assert_eq!(section.id, visited[index]);
        assert_eq!(
            section.html,
            format!("<section data-slide-id=\"slide-{index}\">\n{body}</section>\n")
        );
    }
}

#[test]
fn html_sections_do_not_create_slides_for_an_empty_deck_or_preamble() {
    for source in ["", "<!-- Comment. -->\n\n[link]: https://example.com\n"] {
        let presentation = presentation(source);
        let rendered = render_sections(&presentation, |_| -> Result<String, Infallible> {
            panic!("a preamble must not reach the slide body renderer")
        })
        .unwrap();
        assert!(rendered.is_empty());
    }
}

#[test]
fn html_sections_preserve_body_markup_and_whitespace_exactly() {
    let presentation = presentation("## Body\n");
    let body = "<pre><code>  x &lt;- 1\n\n</code></pre>\n<p>α &amp; β</p>";
    let rendered =
        render_sections(&presentation, |_| Ok::<_, Infallible>(body.to_owned())).unwrap();
    assert_eq!(
        rendered[0].html,
        format!("<section data-slide-id=\"slide-0\">\n{body}</section>\n")
    );
}

#[test]
fn html_sections_keep_semantic_ids_through_edits_and_structural_changes() {
    // Identity matching belongs to the compiler. These edits retain the IDs
    // that a matcher would supply, while changing order, origins, and content.
    let mut deck = presentation("## First\n\n## Second\n\n## Third\n\n## Added\n");
    let added = deck.slides.pop().unwrap();
    let original =
        render_sections(&deck, |_| Ok::<_, Infallible>("<p>Original</p>".into())).unwrap();
    assert_eq!(
        original,
        render_sections(&deck.clone(), |_| Ok::<_, Infallible>(
            "<p>Original</p>".into()
        ))
        .unwrap()
    );

    let mut edited = presentation("\n\n## Renamed\n\nChanged prose.\n");
    let replacement = edited.slides.pop().unwrap();
    deck.slides[1].kind = replacement.kind;
    deck.slides[1].origin = replacement.origin;
    deck.slides.remove(0);
    deck.slides.reverse();
    deck.slides.insert(0, added);
    let rendered =
        render_sections(&deck, |_| Ok::<_, Infallible>("<p>Changed</p>".into())).unwrap();
    for (section, expected) in rendered.iter().zip([3, 2, 1]) {
        assert_eq!(
            section.html,
            format!("<section data-slide-id=\"slide-{expected}\">\n<p>Changed</p></section>\n")
        );
    }
    assert_eq!(rendered[1].id, original[2].id);
    assert_eq!(rendered[2].id, original[1].id);
}

#[test]
fn html_sections_retain_qmd_origins_after_the_presentation_is_dropped() {
    let source = "---\ntitle: Café\n---\n\n## α\n";
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        let deck = presentation(&source);
        let origins: Vec<_> = deck
            .slides
            .iter()
            .map(|slide| slide.origin.clone())
            .collect();
        let rendered = render_sections(&deck, |_| Ok::<_, Infallible>(String::new())).unwrap();
        drop(deck);
        for (section, origin) in rendered.iter().zip(origins) {
            assert_eq!(section.origin.source_span(), origin.source_span());
            assert_eq!(section.origin.chain().nth(1), Some(&origin));
            assert_eq!(section.origin.source_span().file().text(), source);
            let Origin::Derived(derivation) = &section.origin else {
                panic!("expected a generated HTML origin");
            };
            assert_eq!(derivation.operation, "html-section");
        }
    }
}

#[test]
fn html_sections_propagate_body_errors_without_returning_a_partial_deck() {
    let deck = presentation("## First\n\n## Second\n\n## Third\n");
    let mut visited = Vec::new();
    let result = render_sections(&deck, |slide| {
        visited.push(slide.id);
        if slide.id == deck.slides[1].id {
            Err("body unavailable")
        } else {
            Ok("<p>Ready</p>".into())
        }
    });
    assert_eq!(result, Err("body unavailable"));
    assert_eq!(visited, [deck.slides[0].id, deck.slides[1].id]);
}
