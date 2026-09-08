//! Pure document analysis and compilation.
//!
//! Inspection currently reads the CST directly. Semantic lowering will supply
//! the source IR as the document model grows.

use crate::document::DocumentSummary;
use crate::parser;
use panache_parser::syntax::{
    AstNode, CodeBlock, Heading, SyntaxKind, SyntaxNode, YamlMetadata, YamlNode, YamlScalarStyle,
};

/// Parse Quarto-flavored Markdown and report its computational structure.
///
/// This operation is safe for editor and inspection use: it does not execute
/// code cells.
#[must_use]
pub fn summarize_document(source: &str) -> DocumentSummary {
    let parsed = parser::parse(source);
    let root = parsed.document().syntax();
    let headings = root.descendants().filter_map(Heading::cast).count();
    let mut code_blocks = 0;
    let mut executable_cells = 0;
    let mut executable_languages = Vec::new();

    for block in root.descendants().filter_map(CodeBlock::cast) {
        code_blocks += 1;
        if let Some(cell) = block.executable_cell() {
            executable_cells += 1;
            if let Some(language) = cell.language() {
                executable_languages.push(language);
            }
        }
    }

    executable_languages.sort_unstable();
    executable_languages.dedup();

    DocumentSummary {
        slides: count_slides(root),
        headings,
        code_blocks,
        executable_cells,
        executable_languages,
        parse_errors: parsed.errors().len(),
    }
}

fn count_slides(root: &SyntaxNode) -> usize {
    let mut content_slides = 0;

    // Only document-level blocks define boundaries; nested headings belong to
    // their containing block and must not split a slide.
    for node in root.children().filter(has_body_content) {
        if Heading::cast(node).is_some_and(|heading| heading.level() == 2) || content_slides == 0 {
            content_slides += 1;
        }
    }

    usize::from(has_title(root)) + content_slides
}

fn has_body_content(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::YAML_METADATA
        | SyntaxKind::BLANK_LINE
        | SyntaxKind::COMMENT
        | SyntaxKind::REFERENCE_DEFINITION
        | SyntaxKind::FOOTNOTE_DEFINITION => false,
        SyntaxKind::HTML_BLOCK_RAW | SyntaxKind::INLINE_HTML => {
            !node.text().to_string().trim_start().starts_with("<!--")
        }
        SyntaxKind::PARAGRAPH => {
            node.children().any(|child| has_body_content(&child))
                || node
                    .children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .any(|token| !token.text().trim().is_empty())
        }
        _ => true,
    }
}

fn has_title(root: &SyntaxNode) -> bool {
    root.children()
        .find_map(YamlMetadata::cast)
        .and_then(|metadata| metadata.document())
        .and_then(|document| document.as_node())
        .and_then(|node| match node {
            YamlNode::BlockMap(map) => map.value_of("title").and_then(|value| value.as_scalar()),
            YamlNode::FlowMap(map) => map.value_of("title").and_then(|value| value.as_scalar()),
            _ => None,
        })
        .is_some_and(|title| {
            let value = title.value();
            match title.style() {
                YamlScalarStyle::Plain => {
                    !matches!(value.trim(), "" | "null" | "Null" | "NULL" | "~")
                }
                YamlScalarStyle::SingleQuoted | YamlScalarStyle::DoubleQuoted => {
                    !value.trim().is_empty()
                }
                // Panache retains the block scalar header in the value. Only
                // its body can supply title content.
                YamlScalarStyle::Literal | YamlScalarStyle::Folded => {
                    value.lines().skip(1).any(|line| !line.trim().is_empty())
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_display_code_from_executable_cells() {
        let source = r#"---
title: Example
---

## Simulation

```r
x <- 1
```

```{r}
#| echo: false
plot(x)
```
"#;

        let summary = summarize_document(source);

        assert_eq!(summary.headings, 1);
        assert_eq!(summary.code_blocks, 2);
        assert_eq!(summary.executable_cells, 1);
        assert_eq!(summary.executable_languages, ["r"]);
        assert_eq!(summary.parse_errors, 0);
    }
}
