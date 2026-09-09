use std::collections::HashSet;

use super::*;

#[test]
fn lowering_assigns_distinct_node_and_cell_identities_to_identical_content() {
    let doc = document(
        "## Same\n\nSame *text* <https://example.com>.\n\n```{r}\n1\n```\n\n> ```{r}\n> 1\n> ```\n\n## Same\n\nSame *text* <https://example.com>.\n",
    );
    let mut nodes: HashSet<NodeId> = HashSet::new();
    let mut cells: HashSet<CellId> = HashSet::new();
    doc.visit_blocks(&mut |block| {
        assert!(nodes.insert(block.id));
        if let BlockKind::Cell(cell) = &block.kind {
            assert!(cells.insert(cell.id));
        }
        match &block.kind {
            BlockKind::Heading { content, .. }
            | BlockKind::Paragraph(content)
            | BlockKind::Plain(content) => collect_inlines(content, &mut nodes),
            _ => {}
        }
    });
    assert_eq!(cells.len(), 2);
    assert!(nodes.len() > 15);
    assert_eq!(doc, doc.clone());
}

fn collect_inlines(inlines: &[Inline], ids: &mut HashSet<NodeId>) {
    for inline in inlines {
        assert!(ids.insert(inline.id));
        match &inline.kind {
            InlineKind::Emphasis(content)
            | InlineKind::Strong(content)
            | InlineKind::Span(content) => collect_inlines(content, ids),
            InlineKind::Link(link) | InlineKind::Image(link) => collect_inlines(&link.content, ids),
            _ => {}
        }
    }
}
