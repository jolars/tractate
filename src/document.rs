use panache_parser::syntax::{AstNode, CodeBlock, Heading};
use panache_parser::{Flavor, ParserOptions, parse_document};

/// A structural summary of a computational Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSummary {
    /// Number of headings in the document.
    pub headings: usize,
    /// Number of fenced code blocks, including executable cells.
    pub code_blocks: usize,
    /// Number of executable fenced code blocks.
    pub executable_cells: usize,
    /// Languages used by executable cells, in lexical order.
    pub executable_languages: Vec<String>,
    /// Number of syntax errors reported by the parser.
    pub parse_errors: usize,
}

/// Parse Quarto-flavored Markdown and report its computational structure.
///
/// This operation is safe for editor and inspection use: it does not execute
/// code cells.
#[must_use]
pub fn summarize_document(source: &str) -> DocumentSummary {
    let parsed = parse_document(source, Some(ParserOptions::for_flavor(Flavor::Quarto)));
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
        headings,
        code_blocks,
        executable_cells,
        executable_languages,
        parse_errors: parsed.errors().len(),
    }
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
