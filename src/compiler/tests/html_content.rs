use crate::compiler::{build_presentation, presentation::lower_presentation};
use crate::document::*;
use crate::render::html::{CellContent, render_presentation};

fn presentation(source: &str) -> Presentation {
    let lowered = crate::parser::lower(SourceFile::new("slides.qmd", source));
    assert_eq!(lowered.error_count(), 0, "{:?}", lowered.diagnostics);
    lower_presentation(build_presentation(lowered.document))
}

fn html(source: &str) -> String {
    render_presentation(&presentation(source), |_| Ok(CellContent::source_only()))
        .unwrap()
        .into_iter()
        .map(|slide| slide.html)
        .collect()
}

#[test]
fn html_content_renders_the_mvp_source_fixture() {
    let rendered = html(include_str!("../../../tests/fixtures/source-content.qmd"));
    for expected in [
        "<h1>Source semantics</h1>",
        "<p>Leading prose with café and α, before the first slide heading.</p>",
        "<h2>Markdown</h2>",
        "<em>emphasis</em>",
        "<strong>strong text</strong>",
        "<code>inline code</code>",
        "<a href=\"https://example.com/\">link</a>",
        "<h3>Details</h3>",
        "<ul>\n<li>First item.\n<ul>",
        "<ol>\n<li>First step.</li>",
        "<span class=\"math inline\">\\(x + y\\)</span>",
        "class=\"math display\">\\[",
        "\\bar{x} = \\frac{1}{n} \\sum_{i=1}^{n} x_i",
        "<pre><code class=\"language-r\">#| echo: false\n## This is code, not a slide\nx &lt;- 1\n</code></pre>",
        "<pre><code class=\"language-r\">\nmean(c(1, 2, 3))\n</code></pre>",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?} in {rendered}"
        );
    }
    assert_eq!(rendered.matches("<section ").count(), 5);
    assert!(!rendered.contains("#| eval"));
}

#[test]
fn html_content_preserves_nested_lists_quotes_breaks_and_attributes() {
    let rendered = html(
        "## Heading {#intro .accent}\n\n::: {.panel}\n\n> Quoted *text*.\n\n3. Third\n4. Fourth\n\n- Loose first.\n\n- Loose second.\n\n- [x] Done\n- [ ] Open\n\n::: \n\n[Small **span**]{#span .small title='a & b'}.\n\nHard\\\nbreak and\nsoft break.\n\n---\n",
    );
    for expected in [
        "<h2 id=\"intro\" class=\"accent\">Heading</h2>",
        "<div class=\"panel\">\n<blockquote>\n<p>Quoted <em>text</em>.</p>",
        "<ol start=\"3\">\n<li>Third</li>\n<li>Fourth</li>\n</ol>",
        "<li><p>Loose first.</p>",
        "<input type=\"checkbox\" disabled checked>",
        "<input type=\"checkbox\" disabled>",
        "<span id=\"span\" class=\"small\" title=\"a &amp; b\">Small <strong>span</strong></span>",
        "Hard<br>\nbreak and\nsoft break.",
        "<hr>",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?} in {rendered}"
        );
    }
}

#[test]
fn html_content_escapes_text_code_links_images_and_math() {
    let rendered = html(
        "## Escaping\n\nA &amp; B \\<tag> and `</code> & \"x\"`.\n\n[link](<https://example.com/?a=1&b=2> 'a \"quote\"')\n\n![A *small* &amp; `plot`](plot.svg 'a \"title\"')\n\n<user@example.com>\n\n$x < y & z$\n\n$$\na < b & c\n$$\n\n```html\n</code><script>bad()</script>\n```\n",
    );
    for expected in [
        "A &amp; B &lt;tag&gt;",
        "<code>&lt;/code&gt; &amp; &quot;x&quot;</code>",
        "href=\"https://example.com/?a=1&amp;b=2\" title=\"a &quot;quote&quot;\"",
        "<img src=\"plot.svg\" alt=\"A small &amp; plot\" title=\"a &quot;title&quot;\">",
        "<a href=\"mailto:user@example.com\">user@example.com</a>",
        "\\(x &lt; y &amp; z\\)",
        "a &lt; b &amp; c",
        "&lt;/code&gt;&lt;script&gt;bad()&lt;/script&gt;",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?} in {rendered}"
        );
    }
    assert!(!rendered.contains("<script>"));
    assert!(!rendered.contains("&amp;amp;"));
}

#[test]
fn html_content_resolves_document_wide_references_and_preserves_missing_ones() {
    let rendered = html(
        "[Site Name]: /first \"First\"\n\n## Links\n\n[go][SITE   NAME], [Site Name][], [Site Name], ![plot][pic], [missing][unknown].\n\n## Definitions\n\n[pic]: plot.svg\n[Site Name]: /ignored\n",
    );
    assert_eq!(
        rendered.matches("href=\"/first\" title=\"First\"").count(),
        3,
        "{rendered}"
    );
    assert!(
        rendered.contains("<img src=\"plot.svg\" alt=\"plot\">"),
        "{rendered}"
    );
    assert!(rendered.contains("[missing][unknown]"), "{rendered}");
    assert!(!rendered.contains("/ignored"));
}

#[test]
fn html_content_keeps_raw_html_but_omits_comments_and_other_raw_formats() {
    let rendered = html(
        "<!-- before -->\n\n## Raw\n\nText <b>trusted</b><!-- inside -->.\n\n<div>trusted block</div>\n\n`<i>raw</i>`{=html}\n\n`\\LaTeX`{=tex}\n",
    );
    assert!(rendered.contains("Text <b>trusted</b>."), "{rendered}");
    assert!(rendered.contains("<div>trusted block</div>"));
    assert!(rendered.contains("<i>raw</i>"));
    assert!(!rendered.contains("<!--"));
    assert!(!rendered.contains("LaTeX"));
}

#[test]
fn html_content_uses_supplied_cell_visibility_and_results_without_executing() {
    let source = "## Cells\n\n```{r}\n#| echo: false\nstop('never execute')\n```\n\n> ```{r}\n> #| include: false\n> hidden()\n> ```\n\n```{r}\n#| eval: false\nsource_only()\n```\n";
    let deck = presentation(source);
    let mut cells = Vec::new();
    let rendered = render_presentation(&deck, |cell| {
        cells.push(cell.id);
        Ok(match cells.len() {
            1 => CellContent {
                show_source: false,
                results: "<p>Cached result</p>\n".into(),
            },
            2 => CellContent {
                show_source: false,
                results: String::new(),
            },
            _ => CellContent::source_only(),
        })
    })
    .unwrap();
    let rendered = &rendered[0].html;
    assert_eq!(cells.len(), 3);
    assert!(rendered.contains("<p>Cached result</p>"));
    assert!(rendered.contains("<pre><code class=\"language-r\">source_only()\n</code></pre>"));
    assert!(!rendered.contains("never execute"));
    assert!(!rendered.contains("hidden()"));
    assert!(!rendered.contains("#|"));
}

#[test]
fn html_content_reports_unsupported_syntax_with_qmd_origins() {
    let source = "## Table\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
    let error =
        render_presentation(&presentation(source), |_| Ok(CellContent::source_only())).unwrap_err();
    assert_eq!(error.code.as_str(), "render.unsupported-html");
    assert_eq!(error.primary.source_span().file().text(), source);
    assert!(error.primary.source_span().text().starts_with("| A | B |"));
}

#[test]
fn html_content_is_equivalent_for_lf_and_crlf_source() {
    let source = include_str!("../../../tests/fixtures/source-content.qmd");
    assert_eq!(html(source), html(&source.replace('\n', "\r\n")));
}

#[test]
fn html_content_decodes_and_escapes_title_metadata() {
    for (title, expected) in [
        (
            "'Café <slides> & \"quotes\"'",
            "Café &lt;slides&gt; &amp; &quot;quotes&quot;",
        ),
        ("|-\n  First\n  second", "First\nsecond"),
        (">-\n  First\n  second", "First second"),
        (">-\n  First\n\n  second", "First\nsecond"),
        ("|2-\n    Indented", "  Indented"),
    ] {
        let rendered = html(&format!("---\ntitle: {title}\n---\n"));
        assert!(
            rendered.contains(&format!("<h1>{expected}</h1>")),
            "{rendered}"
        );
    }
}

#[test]
fn html_content_preserves_cells_and_markdown_in_html_containers() {
    let source = "## HTML\n\n<div class=\"panel\">\n\n<div class=\"inner\">\n\n**Bold**\n\n```{r}\n#| eval: false\n1 + 1\n```\n\n</div>\n\n</div>\n";
    let deck = presentation(source);
    let mut cells = 0;
    let rendered = render_presentation(&deck, |_| {
        cells += 1;
        Ok(CellContent::source_only())
    })
    .unwrap();
    assert_eq!(cells, 1);
    assert!(rendered[0].html.contains("<strong>Bold</strong>"));
    assert!(
        rendered[0]
            .html
            .contains("<pre><code class=\"language-r\">1 + 1\n</code></pre>")
    );
}

#[test]
fn html_content_propagates_cell_errors_with_the_original_origin() {
    let deck = presentation("## First\n\n```{r}\n1\n```\n\n## Second\n");
    let error = render_presentation(&deck, |cell| {
        Err(Diagnostic {
            severity: Severity::Error,
            code: DiagnosticCode::UnsupportedHtml,
            message: "Required result unavailable".into(),
            primary: cell.result.origin.clone(),
            related: Vec::new(),
        })
    })
    .unwrap_err();
    assert_eq!(error.message, "Required result unavailable");
    assert!(error.primary.source_span().text().contains("```{r}"));
}

#[test]
fn html_content_decodes_link_entities_and_escapes_once() {
    let rendered = html(
        "## Links\n\n[direct](<a?x=1&amp;y=2> 'A &quot;title&quot;') and [reference][STRASSE].\n\n[Straße]: <b?x=1&amp;y=2> 'Other &quot;title&quot;'\n",
    );
    assert!(
        rendered.contains("href=\"a?x=1&amp;y=2\" title=\"A &quot;title&quot;\""),
        "{rendered}"
    );
    assert!(
        rendered.contains("href=\"b?x=1&amp;y=2\" title=\"Other &quot;title&quot;\""),
        "{rendered}"
    );
    assert!(!rendered.contains("&amp;amp;"));
}

#[test]
fn html_content_retains_line_breaks_at_the_end_of_inline_containers() {
    let rendered = html("## Breaks\n\n[*soft\nbreak*](target) and *trailing\n*text.\n");
    assert!(rendered.contains("<em>soft\nbreak</em>"), "{rendered}");
    assert!(rendered.contains("<em>trailing\n</em>text."), "{rendered}");
}
