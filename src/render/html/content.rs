//! Pure Markdown rendering. Cell policy and validated output are caller inputs.

use std::collections::HashMap;
use std::fmt::Write;

use crate::document::*;

use super::{RenderedSlide, render_sections};

/// Already resolved presentation policy and HTML from validated result lookup.
/// Suppressed cells use `show_source: false` and an empty result body. Supplying
/// this value never authorizes execution or establishes result validity.
pub(crate) struct CellContent {
    pub show_source: bool,
    pub results: String,
}

impl CellContent {
    /// Display source with no output, for example for an `eval: false` cell.
    pub fn source_only() -> Self {
        Self {
            show_source: true,
            results: String::new(),
        }
    }
}

/// Render semantic Markdown to Reveal sections without starting any processes.
///
/// The caller resolves cell options and required results before returning each
/// cell's display data. A missing required result must be returned as an error,
/// not a source-only cell. Unsupported Markdown also fails with a QMD origin.
/// Math uses escaped TeX inside explicit delimiters for Reveal's math plugin;
/// loading and configuring that plugin belongs to the HTML directory writer.
#[cfg(test)]
pub(crate) fn render_presentation(
    presentation: &Presentation,
    cell_content: impl FnMut(&PresentedCell) -> Result<CellContent, Diagnostic>,
) -> Result<Vec<RenderedSlide>, Diagnostic> {
    render_with_resources(presentation, cell_content, |destination, _| {
        Ok(destination.to_owned())
    })
}

/// Resolve only destinations actually emitted, including document-wide references.
pub(crate) fn render_with_resources(
    presentation: &Presentation,
    mut cell_content: impl FnMut(&PresentedCell) -> Result<CellContent, Diagnostic>,
    mut resource: impl FnMut(&str, &Origin) -> Result<String, Diagnostic>,
) -> Result<Vec<RenderedSlide>, Diagnostic> {
    let mut references = HashMap::new();
    presentation.visit_blocks(&mut |block| {
        if let BlockKind::ReferenceDefinition(reference) = &block.kind {
            references
                .entry(reference.label.clone())
                .or_insert_with(|| (reference.destination.clone(), reference.title.clone()));
        }
    });
    let mut renderer = Renderer {
        references,
        cell_content: &mut cell_content,
        resource: &mut resource,
    };
    render_sections(presentation, |slide| {
        let mut html = String::new();
        match &slide.kind {
            SlideKind::Title(title) => {
                html.push_str("<h1>");
                escape(&title_text(title)?, &mut html);
                html.push_str("</h1>\n");
            }
            SlideKind::Content(blocks) => renderer.blocks(blocks, &mut html)?,
        }
        Ok(html)
    })
}

struct Renderer<'a, F, R> {
    references: HashMap<String, (Option<String>, Option<String>)>,
    cell_content: &'a mut F,
    resource: &'a mut R,
}

impl<F, R> Renderer<'_, F, R>
where
    F: FnMut(&PresentedCell) -> Result<CellContent, Diagnostic>,
    R: FnMut(&str, &Origin) -> Result<String, Diagnostic>,
{
    fn blocks(
        &mut self,
        blocks: &[Block<PresentedCell>],
        html: &mut String,
    ) -> Result<(), Diagnostic> {
        for block in blocks {
            self.block(block, html)?;
        }
        Ok(())
    }

    fn block(&mut self, block: &Block<PresentedCell>, html: &mut String) -> Result<(), Diagnostic> {
        let attrs = &block.attributes;
        match &block.kind {
            BlockKind::Heading { level, content } => {
                let tag = format!("h{level}");
                open(&tag, attrs, &[], html)?;
                self.block_inlines(content, html)?;
                writeln!(html, "</{tag}>").unwrap();
            }
            BlockKind::Paragraph(content) => {
                open("p", attrs, &[], html)?;
                self.block_inlines(content, html)?;
                html.push_str("</p>\n");
            }
            BlockKind::Plain(content) => self.block_inlines(content, html)?,
            BlockKind::Figure(content) => {
                open("figure", attrs, &[], html)?;
                self.inlines(content, html)?;
                if let Some(Inline {
                    kind: InlineKind::Image(link),
                    ..
                }) = content.first()
                    && !link.content.is_empty()
                {
                    html.push_str("<figcaption>");
                    self.inlines(&link.content, html)?;
                    html.push_str("</figcaption>");
                }
                html.push_str("</figure>\n");
            }
            BlockKind::Quote(blocks) | BlockKind::Div(blocks) => {
                let tag = if matches!(block.kind, BlockKind::Quote(_)) {
                    "blockquote"
                } else {
                    "div"
                };
                open(tag, attrs, &[], html)?;
                html.push('\n');
                self.blocks(blocks, html)?;
                writeln!(html, "</{tag}>").unwrap();
            }
            BlockKind::List(list) => {
                let tag = if list.kind == ListKind::Ordered {
                    "ol"
                } else {
                    "ul"
                };
                let start = if list.kind == ListKind::Ordered {
                    let marker = list
                        .items
                        .first()
                        .and_then(|item| item.marker.as_deref())
                        .unwrap_or("1.");
                    let number = marker
                        .trim_matches(['(', ')', '.'])
                        .parse::<u64>()
                        .map_err(|_| unsupported(&list.origin, "Nondecimal ordered lists"))?;
                    (number != 1).then(|| number.to_string())
                } else {
                    None
                };
                let extra: Vec<_> = start
                    .as_deref()
                    .map(|start| ("start", start))
                    .into_iter()
                    .collect();
                open(tag, attrs, &extra, html)?;
                html.push('\n');
                for item in &list.items {
                    html.push_str("<li>");
                    if let Some(checked) = item.checked {
                        html.push_str(if checked {
                            "<input type=\"checkbox\" disabled checked> "
                        } else {
                            "<input type=\"checkbox\" disabled> "
                        });
                    }
                    for (index, child) in item.blocks.iter().enumerate() {
                        if index > 0 && matches!(item.blocks[index - 1].kind, BlockKind::Plain(_)) {
                            html.push('\n');
                        }
                        if list.loose
                            && let BlockKind::Plain(content) = &child.kind
                        {
                            html.push_str("<p>");
                            self.block_inlines(content, html)?;
                            html.push_str("</p>\n");
                        } else {
                            self.block(child, html)?;
                        }
                    }
                    html.push_str("</li>\n");
                }
                writeln!(html, "</{tag}>").unwrap();
            }
            BlockKind::Code(code) => code_html(code, attrs, html)?,
            BlockKind::Cell(cell) => {
                let content = (self.cell_content)(cell)?;
                if content.show_source {
                    code_html(&cell.source, attrs, html)?;
                }
                html.push_str(&content.results);
            }
            BlockKind::Math(text) => {
                math(text, true, "div", attrs, html)?;
                html.push('\n');
            }
            BlockKind::ThematicBreak => {
                open("hr", attrs, &[], html)?;
                html.push('\n');
            }
            BlockKind::Raw(raw) => raw_html(raw, html),
            BlockKind::Html(blocks) => self.blocks(blocks, html)?,
            BlockKind::Comment | BlockKind::ReferenceDefinition(_) | BlockKind::Metadata(_) => {}
            BlockKind::FootnoteDefinition(node) | BlockKind::Unsupported(node) => {
                return Err(unsupported(&node.origin, &node.name));
            }
        }
        Ok(())
    }

    fn block_inlines(
        &mut self,
        inlines: &[Inline<PresentedCell>],
        html: &mut String,
    ) -> Result<(), Diagnostic> {
        // Panache includes the line ending that terminates a block as a soft
        // break. It is framing, unlike breaks between lines in the same block.
        let inlines = if inlines
            .last()
            .is_some_and(|inline| matches!(inline.kind, InlineKind::SoftBreak))
        {
            &inlines[..inlines.len() - 1]
        } else {
            inlines
        };
        self.inlines(inlines, html)
    }

    fn inlines(
        &mut self,
        inlines: &[Inline<PresentedCell>],
        html: &mut String,
    ) -> Result<(), Diagnostic> {
        for inline in inlines {
            let attrs = &inline.attributes;
            match &inline.kind {
                InlineKind::Text(text) => escape(text, html),
                InlineKind::Space => html.push(' '),
                InlineKind::NonbreakingSpace => html.push_str("&nbsp;"),
                InlineKind::SoftBreak => html.push('\n'),
                InlineKind::HardBreak => html.push_str("<br>\n"),
                InlineKind::Emphasis(content)
                | InlineKind::Strong(content)
                | InlineKind::Span(content) => {
                    let tag = match inline.kind {
                        InlineKind::Emphasis(_) => "em",
                        InlineKind::Strong(_) => "strong",
                        _ => "span",
                    };
                    open(tag, attrs, &[], html)?;
                    self.inlines(content, html)?;
                    write!(html, "</{tag}>").unwrap();
                }
                InlineKind::Code(text) => {
                    open("code", attrs, &[], html)?;
                    escape(text, html);
                    html.push_str("</code>");
                }
                InlineKind::Math(text) => math(text, false, "span", attrs, html)?,
                InlineKind::DisplayMath(text) => math(text, true, "span", attrs, html)?,
                InlineKind::Link(link) | InlineKind::Image(link) => {
                    let reference = link
                        .reference
                        .as_ref()
                        .and_then(|label| self.references.get(label));
                    let destination = link
                        .destination
                        .as_ref()
                        .or_else(|| reference.and_then(|r| r.0.as_ref()));
                    let title = link
                        .title
                        .as_ref()
                        .or_else(|| reference.and_then(|r| r.1.as_ref()));
                    let Some(destination) = destination else {
                        escape(link.origin.source_span().text(), html);
                        continue;
                    };
                    let image = matches!(inline.kind, InlineKind::Image(_));
                    let destination = (self.resource)(destination, &link.origin)?;
                    let alt = plain_text(&link.content);
                    let mut extra =
                        vec![(if image { "src" } else { "href" }, destination.as_str())];
                    if image {
                        extra.push(("alt", &alt));
                    }
                    if let Some(title) = title {
                        extra.push(("title", title));
                    }
                    open(if image { "img" } else { "a" }, attrs, &extra, html)?;
                    if !image {
                        self.inlines(&link.content, html)?;
                        html.push_str("</a>");
                    }
                }
                InlineKind::Raw(raw) => raw_html(raw, html),
                InlineKind::Comment => {}
                InlineKind::Unsupported(node) => return Err(unsupported(&node.origin, &node.name)),
            }
        }
        Ok(())
    }
}

fn code_html(code: &Code, attrs: &[Attribute], html: &mut String) -> Result<(), Diagnostic> {
    open("pre", attrs, &[], html)?;
    let class = code
        .language
        .as_ref()
        .map(|language| format!("language-{language}"));
    let extra: Vec<_> = class
        .as_deref()
        .map(|class| ("class", class))
        .into_iter()
        .collect();
    open("code", &[], &extra, html)?;
    escape(&code.source.replace("\r\n", "\n"), html);
    html.push_str("</code></pre>\n");
    Ok(())
}

fn math(
    text: &str,
    display: bool,
    tag: &str,
    attrs: &[Attribute],
    html: &mut String,
) -> Result<(), Diagnostic> {
    open(
        tag,
        attrs,
        &[(
            "class",
            if display {
                "math display"
            } else {
                "math inline"
            },
        )],
        html,
    )?;
    html.push_str(if display { "\\[" } else { "\\(" });
    escape(&text.replace("\r\n", "\n"), html);
    html.push_str(if display { "\\]" } else { "\\)" });
    write!(html, "</{tag}>").unwrap();
    Ok(())
}

fn open(
    tag: &str,
    attrs: &[Attribute],
    extra: &[(&str, &str)],
    html: &mut String,
) -> Result<(), Diagnostic> {
    write!(html, "<{tag}").unwrap();
    let mut values: Vec<(&str, String)> = extra
        .iter()
        .map(|&(key, value)| (key, value.to_owned()))
        .collect();
    for attr in attrs {
        let (key, value) = match &attr.kind {
            AttributeKind::Identifier(value) => ("id", value.as_str()),
            AttributeKind::Class(value) => ("class", value.as_str()),
            AttributeKind::KeyValue { key, value } => (key.as_str(), value.as_str()),
        };
        if key.is_empty()
            || !key.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            })
        {
            return Err(unsupported(&attr.origin, "Invalid HTML attribute name"));
        }
        if let Some((_, previous)) = values.iter_mut().find(|(name, _)| *name == key) {
            if key == "class" {
                previous.push(' ');
                previous.push_str(value);
            }
        } else {
            values.push((key, value.to_owned()));
        }
    }
    for (key, value) in values {
        write!(html, " {key}=\"").unwrap();
        escape(&value, html);
        html.push('"');
    }
    html.push('>');
    Ok(())
}

pub(super) fn escape(text: &str, html: &mut String) {
    for c in text.chars() {
        match c {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            _ => html.push(c),
        }
    }
}

fn raw_html(raw: &RawContent, html: &mut String) {
    if raw.format == "html" {
        html.push_str(&raw.text.replace("\r\n", "\n"));
    }
}

fn plain_text(inlines: &[Inline<PresentedCell>]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match &inline.kind {
            InlineKind::Text(value)
            | InlineKind::Code(value)
            | InlineKind::Math(value)
            | InlineKind::DisplayMath(value) => text.push_str(value),
            InlineKind::Space | InlineKind::SoftBreak | InlineKind::HardBreak => text.push(' '),
            InlineKind::NonbreakingSpace => text.push('\u{a0}'),
            InlineKind::Emphasis(content)
            | InlineKind::Strong(content)
            | InlineKind::Span(content) => text.push_str(&plain_text(content)),
            InlineKind::Link(link) | InlineKind::Image(link) => {
                text.push_str(&plain_text(&link.content))
            }
            InlineKind::Unsupported(node) => text.push_str(node.origin.source_span().text()),
            InlineKind::Raw(_) | InlineKind::Comment => {}
        }
    }
    text
}

pub(crate) fn title_text(title: &YamlValue) -> Result<String, Diagnostic> {
    match &title.kind {
        YamlKind::Scalar {
            text,
            style: style @ (ScalarStyle::Literal | ScalarStyle::Folded),
        } => {
            // The title is a top-level YAML scalar. Panache retains block headers.
            let mut source_lines = text.lines();
            let header = source_lines
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or("");
            let lines: Vec<_> = source_lines.collect();
            let indent = header
                .bytes()
                .find(|b| matches!(b, b'1'..=b'9'))
                .map(|b| usize::from(b - b'0'))
                .unwrap_or_else(|| {
                    lines
                        .iter()
                        .find(|line| !line.trim().is_empty())
                        .map(|line| line.bytes().take_while(|&b| b == b' ').count())
                        .unwrap_or(0)
                });
            let lines: Vec<_> = lines
                .iter()
                .map(|line| line.get(indent..).unwrap_or(""))
                .collect();
            let mut cooked = String::new();
            let mut previous_nonempty = None;
            for (index, line) in lines.iter().enumerate() {
                cooked.push_str(line);
                if *style == ScalarStyle::Folded && index + 1 < lines.len() {
                    let next = lines[index + 1];
                    if !line.is_empty()
                        && !next.is_empty()
                        && !line.starts_with([' ', '\t'])
                        && !next.starts_with([' ', '\t'])
                    {
                        cooked.push(' ');
                    } else if !(line.is_empty()
                        && !next.is_empty()
                        && previous_nonempty
                            .is_some_and(|previous: &str| !previous.starts_with([' ', '\t']))
                        && !next.starts_with([' ', '\t']))
                    {
                        cooked.push('\n');
                    }
                } else if index + 1 < lines.len() || text.ends_with('\n') {
                    cooked.push('\n');
                }
                if !line.is_empty() {
                    previous_nonempty = Some(*line);
                }
            }
            if !header.contains('+') {
                let end = cooked.trim_end_matches('\n').len();
                let had_break = end < cooked.len();
                cooked.truncate(end);
                if !header.contains('-') && had_break && !cooked.is_empty() {
                    cooked.push('\n');
                }
            }
            Ok(cooked)
        }
        YamlKind::Scalar { text, .. } => Ok(text.clone()),
        _ => Err(unsupported(&title.origin, "Nonscalar titles")),
    }
}

fn unsupported(origin: &Origin, name: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code: DiagnosticCode::UnsupportedHtml,
        message: format!("{name} is not supported by the HTML backend"),
        primary: origin.clone(),
        related: Vec::new(),
    }
}
