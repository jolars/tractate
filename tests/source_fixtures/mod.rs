use panache_parser::syntax::{
    AstNode, CodeBlock, DisplayMath, Heading, InlineMath, List, ListKind, SyntaxKind, SyntaxNode,
    YamlBlockMap, YamlMetadata, YamlNode,
};
use panache_parser::{Flavor, ParserOptions, parse_document};
use tractate::summarize_document;

use crate::common::{TestProject, fixture};

fn parse_fixture(name: &str) -> SyntaxNode {
    let source = fixture(name);
    let parsed = parse_document(&source, Some(ParserOptions::for_flavor(Flavor::Quarto)));
    assert!(parsed.errors().is_empty(), "{name}: {:?}", parsed.errors());
    let root = parsed.document().syntax().clone();
    assert_eq!(root.text().to_string(), source, "{name}: lossless source");
    root
}

fn metadata(root: &SyntaxNode) -> YamlBlockMap {
    root.children()
        .find_map(YamlMetadata::cast)
        .and_then(|metadata| metadata.document())
        .and_then(|document| document.block_map())
        .expect("front matter mapping")
}

fn scalar(map: &YamlBlockMap, key: &str) -> String {
    map.value_of(key)
        .and_then(|value| value.as_scalar())
        .unwrap_or_else(|| panic!("scalar value for {key}"))
        .value()
}

#[test]
fn source_fixtures_cover_inspection_without_execution() {
    for (name, headings, blocks, cells, errors) in [
        ("source-content.qmd", 5, 2, 1, 0),
        ("title-only.qmd", 0, 0, 0, 0),
        ("empty-title.qmd", 1, 0, 0, 0),
        ("no-title.qmd", 3, 0, 0, 0),
        ("leading-body.qmd", 2, 0, 0, 0),
        ("cell-options.qmd", 1, 2, 2, 0),
        ("document-defaults.qmd", 1, 2, 2, 0),
        ("invalid-option-types.qmd", 1, 1, 1, 0),
        ("unknown-options.qmd", 1, 1, 1, 0),
        ("duplicate-labels.qmd", 2, 2, 2, 0),
        ("malformed-options.qmd", 1, 1, 1, 1),
        ("malformed-defaults.qmd", 1, 1, 1, 1),
    ] {
        let source = fixture(name);
        let summary = summarize_document(&source);
        assert_eq!(summary.headings, headings, "{name}");
        assert_eq!(summary.code_blocks, blocks, "{name}");
        assert_eq!(summary.executable_cells, cells, "{name}");
        assert_eq!(summary.parse_errors, errors, "{name}");
        let languages = if cells == 0 { vec![] } else { vec!["r"] };
        assert_eq!(summary.executable_languages, languages, "{name}");

        let project = TestProject::new(&source);
        #[cfg(target_os = "linux")]
        let output = {
            let (output, launches) = crate::common::process::trace(
                &project,
                env!("CARGO_BIN_EXE_tractate"),
                &["inspect", "slides.qmd"],
            );
            assert!(launches.is_empty(), "{name}: {launches:?}");
            output
        };
        #[cfg(not(target_os = "linux"))]
        let output = project.inspect();

        assert_eq!(output.status.success(), errors == 0, "{name}: {output:?}");
        assert!(
            output.stdout.contains(&format!("parse errors: {errors}\n")),
            "{name}: {output:?}"
        );
    }
}

#[test]
fn source_fixtures_preserve_metadata_and_boundary_inputs() {
    for (name, title, levels, leading_body) in [
        (
            "source-content.qmd",
            Some("Source semantics"),
            vec![1, 2, 3, 2, 2],
            true,
        ),
        ("title-only.qmd", Some("Title only"), vec![], false),
        ("empty-title.qmd", Some(""), vec![2], false),
        ("no-title.qmd", None, vec![2, 3, 2], false),
        ("leading-body.qmd", None, vec![2, 2], true),
    ] {
        let root = parse_fixture(name);
        if let Some(title) = title {
            assert_eq!(scalar(&metadata(&root), "title"), title, "{name}");
        } else {
            assert!(root.children().find_map(YamlMetadata::cast).is_none());
        }
        let headings: Vec<_> = root.children().filter_map(Heading::cast).collect();
        assert_eq!(
            headings.iter().map(Heading::level).collect::<Vec<_>>(),
            levels,
            "{name}"
        );
        let leading: Vec<_> = root
            .children()
            .take_while(|node| Heading::cast(node.clone()).is_none_or(|h| h.level() != 2))
            .filter(|node| {
                !matches!(
                    node.kind(),
                    SyntaxKind::YAML_METADATA | SyntaxKind::BLANK_LINE
                )
            })
            .collect();
        assert_eq!(!leading.is_empty(), leading_body, "{name}");
    }

    let frontmatter = metadata(&parse_fixture("source-content.qmd"));
    assert_eq!(scalar(&frontmatter, "subtitle"), "Markdown and computation");
    assert_eq!(scalar(&frontmatter, "author"), "Alex Example");
    assert_eq!(scalar(&frontmatter, "date"), "2026-01-15");
}

#[test]
fn source_fixtures_preserve_nested_markdown_math_and_code() {
    let root = parse_fixture("source-content.qmd");
    for kind in [
        SyntaxKind::PARAGRAPH,
        SyntaxKind::EMPHASIS,
        SyntaxKind::STRONG,
        SyntaxKind::INLINE_CODE,
        SyntaxKind::LINK,
    ] {
        assert!(
            root.descendants().any(|node| node.kind() == kind),
            "{kind:?}"
        );
    }
    let lists: Vec<_> = root.descendants().filter_map(List::cast).collect();
    assert_eq!(
        lists.iter().map(List::kind).collect::<Vec<_>>(),
        [
            Some(ListKind::Bullet),
            Some(ListKind::Bullet),
            Some(ListKind::Ordered)
        ]
    );
    assert!(lists[0].items().any(|item| {
        item.syntax()
            .children()
            .any(|node| List::can_cast(node.kind()))
    }));
    assert_eq!(
        root.descendants()
            .filter_map(InlineMath::cast)
            .map(|m| m.content())
            .collect::<Vec<_>>(),
        ["x + y"]
    );
    assert_eq!(
        root.descendants()
            .find_map(DisplayMath::cast)
            .expect("display math")
            .content()
            .trim(),
        r"\bar{x} = \frac{1}{n} \sum_{i=1}^{n} x_i"
    );

    let blocks: Vec<_> = root.descendants().filter_map(CodeBlock::cast).collect();
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].executable_cell().is_none());
    assert!(blocks[0].code_source().contains("#| echo: false"));
    assert!(
        blocks[0]
            .code_source()
            .contains("## This is code, not a slide")
    );
    let cell = blocks[1].executable_cell().expect("executable R cell");
    assert_eq!(cell.language().as_deref(), Some("r"));
    assert_eq!(cell.code_source().trim(), "mean(c(1, 2, 3))");
    assert!(!cell.code_source().contains("#|"));
}

#[test]
fn source_fixtures_preserve_cell_options_and_declared_inputs() {
    let root = parse_fixture("cell-options.qmd");
    let cells: Vec<_> = root
        .descendants()
        .filter_map(CodeBlock::cast)
        .filter_map(|block| block.executable_cell())
        .collect();
    let options = cells[0].option_declarations();
    assert_eq!(
        options
            .iter()
            .map(|option| (option.key(), option.cooked_value()))
            .collect::<Vec<_>>(),
        [
            (Some("label"), Some("fig-options")),
            (Some("eval"), Some("false")),
            (Some("echo"), Some("false")),
            (Some("include"), Some("true")),
            (Some("results"), Some("hide")),
            (Some("session"), Some("isolated")),
            (Some("cache"), Some("false")),
            (Some("inputs"), None),
            (Some("fig-width"), Some("7.5")),
            (Some("fig-height"), Some("4")),
            (Some("fig-cap"), Some("Observed values: a small sample.")),
        ]
    );
    let Some(YamlNode::BlockSequence(inputs)) = options[7].yaml_value() else {
        panic!("inputs must retain their YAML sequence");
    };
    let paths: Vec<_> = inputs
        .items()
        .map(|item| item.as_scalar().expect("input path").value())
        .collect();
    assert_eq!(paths, ["data/observations.csv"]);
    assert_eq!(fixture(&paths[0]), "value\n1\n2\n3\n");
    assert_eq!(
        cells[1]
            .option_declarations()
            .iter()
            .find(|option| option.key() == Some("session"))
            .and_then(|option| option.cooked_value()),
        Some("model")
    );
}

#[test]
fn source_fixtures_keep_document_defaults_separate_from_cell_overrides() {
    let root = parse_fixture("document-defaults.qmd");
    let defaults = metadata(&root)
        .value_of("execute")
        .and_then(|value| value.as_block_map())
        .expect("document execution defaults");
    for (key, expected) in [
        ("eval", "false"),
        ("echo", "false"),
        ("include", "true"),
        ("cache", "true"),
        ("fig-width", "6"),
        ("fig-height", "4"),
    ] {
        assert_eq!(scalar(&defaults, key), expected, "{key}");
    }
    let cells: Vec<_> = root
        .descendants()
        .filter_map(CodeBlock::cast)
        .filter_map(|block| block.executable_cell())
        .collect();
    assert_eq!(cells.len(), 2);
    assert!(cells[0].option_declarations().is_empty());
    assert_eq!(
        cells[1]
            .option_declarations()
            .iter()
            .map(|option| (option.key(), option.cooked_value()))
            .collect::<Vec<_>>(),
        [
            (Some("label"), Some("override")),
            (Some("eval"), Some("true")),
            (Some("echo"), Some("true")),
            (Some("include"), Some("false")),
            (Some("fig-width"), Some("8"))
        ]
    );
}

#[test]
fn source_fixtures_retain_semantic_error_cases_for_lowering() {
    let root = parse_fixture("invalid-option-types.qmd");
    let cell = root
        .descendants()
        .find_map(CodeBlock::cast)
        .unwrap()
        .executable_cell()
        .unwrap();
    let options = cell.option_declarations();
    assert_eq!(options[0].key(), Some("eval"));
    assert_eq!(options[0].cooked_value(), Some("false"));
    assert!(options[0].is_quoted());
    assert_eq!(options[1].key(), Some("inputs"));
    assert_eq!(options[1].cooked_value(), Some("42"));
    assert_eq!(options[2].key(), Some("fig-width"));
    assert_eq!(options[2].cooked_value(), Some("wide"));

    let root = parse_fixture("unknown-options.qmd");
    let defaults = metadata(&root)
        .value_of("execute")
        .and_then(|value| value.as_block_map())
        .unwrap();
    assert_eq!(scalar(&defaults, "unknown-default"), "true");
    let cell = root
        .descendants()
        .find_map(CodeBlock::cast)
        .unwrap()
        .executable_cell()
        .unwrap();
    assert!(
        cell.option_declarations()
            .iter()
            .any(|option| option.key() == Some("unknown-cell-option"))
    );

    let root = parse_fixture("duplicate-labels.qmd");
    let labels: Vec<_> = root
        .descendants()
        .filter_map(CodeBlock::cast)
        .filter_map(|block| block.executable_cell())
        .flat_map(|cell| cell.labels())
        .collect();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].value(), "fig-shared");
    assert_eq!(labels[1].value(), "fig-shared");
    assert_ne!(labels[0].value_range(), labels[1].value_range());
}
