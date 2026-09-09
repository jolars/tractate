use tractate::summarize_document;

use crate::common::{TestProject, fixture};

fn assert_slides(source: &str, expected: usize) {
    let summary = summarize_document(source);
    assert_eq!(summary.parse_errors, 0, "source: {source:?}");
    assert_eq!(summary.slides, expected, "source: {source:?}");
}

#[test]
fn slide_rules_apply_to_source_fixtures_and_inspection() {
    for (name, expected) in [
        ("source-content.qmd", 5),
        ("title-only.qmd", 1),
        ("empty-title.qmd", 1),
        ("no-title.qmd", 2),
        ("leading-body.qmd", 3),
        ("slide-boundaries.qmd", 3),
    ] {
        let source = fixture(name);
        assert_slides(&source, expected);

        let output = TestProject::new(&source).inspect();
        assert!(output.status.success(), "{name}: {output:?}");
        assert!(
            output.stdout.contains(&format!("slides: {expected}\n")),
            "{name}: {output:?}"
        );
    }
}

#[test]
fn slide_rules_require_nonempty_title_metadata() {
    for (metadata, title_slides) in [
        ("author: Alex Example", 0),
        ("title:", 0),
        ("title: null", 0),
        ("title: Null", 0),
        ("title: NULL", 0),
        ("title: ~", 0),
        ("title: ''", 0),
        ("title: \"\"", 0),
        ("title: '   '", 0),
        ("title: \"\\t\\n\"", 0),
        ("title: |\n  ", 0),
        ("title: >- # Empty folded title\n  ", 0),
        ("title: []", 0),
        ("title: {}", 0),
        ("format:\n  revealjs:\n    title: Nested", 0),
        ("title: A title", 1),
        ("title: '  A title  '", 1),
        ("title: \"Café and α\"", 1),
        ("title: 'null'", 1),
        ("title: \"~\"", 1),
        ("title: |\n  null", 1),
        ("title: |\n  A multiline\n  title", 1),
        ("title: >-\n  A folded\n  title", 1),
        ("{title: Flow mapping}", 1),
    ] {
        let source = format!("---\n{metadata}\n---\n");
        assert_slides(&source, title_slides);
        assert_slides(&format!("{source}\n## Content\n"), title_slides + 1);
        assert_slides(
            &format!("{source}\nLeading prose.\n\n## Content\n"),
            title_slides + 2,
        );
    }
}

#[test]
fn slide_rules_do_not_create_slides_for_trivia_or_definitions() {
    for source in [
        "",
        "\n \t\n\n",
        "<!-- A comment. -->\n",
        "<!-- First. --> <!-- Second. -->\n",
        "<!--\n## A commented heading\n-->\n",
        "[reference]: https://example.com\n",
        "[^note]: A footnote definition.\n",
        "---\nauthor: Alex Example\n---\n\n<!-- A comment. -->\n",
    ] {
        assert_slides(source, 0);
        assert_slides(&format!("{source}\n\n## Content\n"), 1);
    }
}

#[test]
fn slide_rules_keep_leading_blocks_on_one_content_slide() {
    for body in [
        "A paragraph.\n\nAnother paragraph.\n",
        "<!-- A comment. --> Visible text.\n",
        "<div>HTML content.</div>\n",
        "# Context\n\n### Detail\n",
        "- A list item.\n- Another item.\n",
        "> ## A quoted heading\n",
        "::: {.callout-note}\n\n## A nested heading\n\n:::\n",
        "```r\n## A code comment\n```\n",
        "```{r}\n#| eval: false\n## A code comment\n```\n",
        "$$\nx + y\n$$\n",
        "***\n",
    ] {
        assert_slides(body, 1);
        assert_slides(&format!("{body}\n## Content\n"), 2);
        assert_slides(&format!("---\ntitle: Example\n---\n\n{body}"), 2);
    }
}

#[test]
fn slide_rules_retain_empty_and_consecutive_level_two_headings() {
    assert_slides("##\n\n## Named\n\n##\n", 3);
    assert_slides("First\n-----\n\nSecond\n------\n", 2);
    assert_slides("## First\n\n***\n\n# Context\n\n### Detail\n", 1);
}

#[test]
fn slide_rules_count_written_entities_as_leading_source_content() {
    for source in ["&nbsp;\n", "&#32;\n", "&#x09;\n"] {
        assert_slides(source, 1);
        assert_slides(&format!("{source}\n## Content\n"), 2);
    }
}
