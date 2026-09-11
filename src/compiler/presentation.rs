//! Pure placement of result slots in a source presentation.

use crate::document::*;

pub(super) fn lower_presentation(source: SourcePresentation) -> Presentation {
    Presentation {
        origin: source.origin.derived("presentation-ir"),
        metadata: source.metadata,
        preamble: source.preamble.into_iter().map(lower_block).collect(),
        slides: source
            .slides
            .into_iter()
            .map(|slide| Slide {
                id: slide.id,
                origin: slide.origin.derived("presentation-slide"),
                kind: match slide.kind {
                    SlideKind::Title(title) => SlideKind::Title(title),
                    SlideKind::Content(blocks) => {
                        SlideKind::Content(blocks.into_iter().map(lower_block).collect())
                    }
                },
            })
            .collect(),
    }
}

fn lower_block(block: Block) -> Block<PresentedCell> {
    block.map_cells(&mut |cell| PresentedCell {
        id: cell.id,
        origin: cell.origin.derived("cell-presentation"),
        source: cell.code,
        options: cell.options,
        preamble: cell.preamble,
        result: ResultSlot {
            cell: cell.id,
            origin: cell.origin.derived("result-slot"),
        },
    })
}
