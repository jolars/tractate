//! Pure document analysis and compilation.
//!
//! Inspection and subsequent compiler passes consume the source semantic IR.

use crate::document::{
    Block, BlockKind, DocumentSummary, InlineKind, ScalarStyle, SourceDocument, SourceFile,
    YamlKind,
};
use crate::parser;

/// Parse Quarto-flavored Markdown and report its computational structure.
///
/// This operation is safe for editor and inspection use: it does not execute
/// code cells.
#[must_use]
pub fn summarize_document(source: &str) -> DocumentSummary {
    let lowered = parser::lower(SourceFile::anonymous(source));
    let mut headings = 0;
    let mut code_blocks = 0;
    let mut executable_cells = 0;
    let mut executable_languages = Vec::new();

    lowered
        .document
        .visit_blocks(&mut |block| match &block.kind {
            BlockKind::Heading { .. } => headings += 1,
            BlockKind::Code(_) => code_blocks += 1,
            BlockKind::Cell(cell) => {
                code_blocks += 1;
                executable_cells += 1;
                if let Some(language) = &cell.code.language {
                    executable_languages.push(language.clone());
                }
            }
            _ => {}
        });

    executable_languages.sort_unstable();
    executable_languages.dedup();

    DocumentSummary {
        slides: count_slides(&lowered.document),
        headings,
        code_blocks,
        executable_cells,
        executable_languages,
        parse_errors: lowered.parse_errors,
    }
}

fn count_slides(document: &SourceDocument) -> usize {
    let mut content_slides = 0;

    // Only document-level blocks define boundaries; nested headings belong to
    // their containing block and must not split a slide.
    for block in document
        .blocks
        .iter()
        .filter(|block| has_body_content(block))
    {
        if matches!(block.kind, BlockKind::Heading { level: 2, .. }) || content_slides == 0 {
            content_slides += 1;
        }
    }

    usize::from(has_title(document)) + content_slides
}

fn has_body_content(block: &Block) -> bool {
    match &block.kind {
        BlockKind::Metadata(_)
        | BlockKind::Comment
        | BlockKind::ReferenceDefinition(_)
        | BlockKind::FootnoteDefinition(_) => false,
        BlockKind::Paragraph(inlines) => inlines.iter().any(|inline| match &inline.kind {
            // Written entities are source content even when they decode to whitespace.
            InlineKind::Text(_) => !inline.origin.source_span().text().trim().is_empty(),
            InlineKind::Space | InlineKind::SoftBreak | InlineKind::Comment => false,
            _ => true,
        }),
        _ => true,
    }
}

fn has_title(document: &SourceDocument) -> bool {
    document
        .metadata
        .first()
        .and_then(|metadata| metadata.value.mapping())
        .and_then(|entries| {
            entries.iter().find(
                |entry| matches!(&entry.key.kind, YamlKind::Scalar { text, .. } if text == "title"),
            )
        })
        .is_some_and(|entry| {
            let YamlKind::Scalar { text: value, style } = &entry.value.kind else {
                return false;
            };
            match style {
                ScalarStyle::Plain => !matches!(value.trim(), "" | "null" | "Null" | "NULL" | "~"),
                ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => !value.trim().is_empty(),
                // Panache retains the block scalar header in the value. Only
                // its body can supply title content.
                ScalarStyle::Literal | ScalarStyle::Folded => {
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
