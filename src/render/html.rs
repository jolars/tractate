//! Reveal sections and Markdown content, independent of cell execution.

use crate::document::{Origin, Presentation, Slide, SlideId};

mod content;
mod directory;
#[cfg(test)]
pub(crate) use content::render_presentation;
pub(crate) use content::{CellContent, render_with_resources, title_text};
pub(crate) use directory::{HtmlAsset, HtmlDirectory};

/// One independently replaceable Reveal section and its semantic provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedSlide {
    pub id: SlideId,
    pub origin: Origin,
    pub html: String,
}

/// Render one section per slide, in presentation order, including empty bodies.
///
/// The body pass receives the complete slide, including title declarations or
/// nested content and result slots. It supplies HTML, with escaping and option
/// resolution already applied. A body error aborts assembly instead of yielding
/// an apparently complete deck. Preamble definitions remain document-scoped
/// inputs for the body pass and do not create sections of their own.
///
/// `data-slide-id` is an injective encoding of the semantic `SlideId`, independent
/// of position, heading text, source offsets, and body bytes. The compiler must
/// retain semantic IDs across revisions; this backend does not match slides.
/// These identifiers are local to a compilation scope, not persistent cache keys.
pub(crate) fn render_sections<E>(
    presentation: &Presentation,
    mut render_body: impl FnMut(&Slide) -> Result<String, E>,
) -> Result<Vec<RenderedSlide>, E> {
    presentation
        .slides
        .iter()
        .map(|slide| {
            let body = render_body(slide)?;
            Ok(RenderedSlide {
                id: slide.id,
                origin: slide.origin.derived("html-section"),
                html: format!(
                    "<section data-slide-id=\"slide-{}\">\n{body}</section>\n",
                    slide.id
                ),
            })
        })
        .collect()
}
