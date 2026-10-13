use tractate::{Compiler, SlideMove, SlidePosition, SourceFile};

fn update(compiler: &mut Compiler, text: &str) -> tractate::SourceUpdate {
    compiler.update_source_with_changes(SourceFile::anonymous(text))
}

#[test]
fn slide_changes_distinguish_structural_edits_from_position_shifts() {
    let mut compiler = Compiler::new(SourceFile::anonymous(
        "## A\n\nAlpha\n\n## B\n\nBeta\n\n## C\n\nGamma\n",
    ));
    let original = compiler.snapshot();
    let ids: Vec<_> = original.slide_ids().collect();
    let inserted = update(
        &mut compiler,
        "## X\n\n## A\n\nAlpha\n\n## B\n\nBeta\n\n## C\n\nGamma\n",
    );
    assert_eq!(inserted.previous_revision, original.revision());
    assert_eq!(inserted.revision.get(), 1);
    let x = inserted.changes.order[0];
    assert_eq!(
        inserted.changes.inserted,
        [SlidePosition { id: x, index: 0 }]
    );
    assert!(inserted.changes.removed.is_empty());
    assert!(inserted.changes.reordered.is_empty());
    assert!(inserted.changes.modified.is_empty());
    assert_eq!(&inserted.changes.order[1..], ids);

    let moved = update(
        &mut compiler,
        "## C\n\nChanged gamma\n\n## A\n\nAlpha\n\n## B\n\nBeta\n",
    );
    assert_eq!(moved.changes.removed, [SlidePosition { id: x, index: 0 }]);
    assert!(moved.changes.inserted.is_empty());
    assert_eq!(moved.changes.order, [ids[2], ids[0], ids[1]]);
    assert_eq!(
        moved.changes.reordered,
        [
            SlideMove {
                id: ids[2],
                from: 3,
                to: 0
            },
            SlideMove {
                id: ids[0],
                from: 1,
                to: 1
            },
            SlideMove {
                id: ids[1],
                from: 2,
                to: 2
            },
        ]
    );
    assert_eq!(
        moved.changes.modified,
        [SlidePosition {
            id: ids[2],
            index: 0
        }]
    );

    let removed = update(&mut compiler, "## C\n\nChanged gamma\n\n## B\n\nBeta\n");
    assert_eq!(
        removed.changes.removed,
        [SlidePosition {
            id: ids[0],
            index: 1
        }]
    );
    assert!(removed.changes.reordered.is_empty());
    assert!(removed.changes.modified.is_empty());
    assert_eq!(original.slide_ids().collect::<Vec<_>>(), ids);
}

#[test]
fn slide_changes_handle_titles_noops_paths_and_boundary_replacements() {
    let text = "---\ntitle: First\n---\n\n## A\n\nBody\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(text));
    let ids: Vec<_> = compiler.snapshot().slide_ids().collect();
    let noop = update(&mut compiler, text);
    assert_eq!(noop.previous_revision, noop.revision);
    assert!(noop.changes.is_empty());
    assert_eq!(noop.changes.order, ids);
    let path = compiler.update_source_with_changes(SourceFile::new("other.qmd", text));
    assert!(path.changes.is_empty());
    assert_eq!(path.revision.get(), 1);
    let title = update(&mut compiler, "---\ntitle: Second\n---\n\n## A\n\nBody\n");
    assert_eq!(
        title.changes.modified,
        [SlidePosition {
            id: ids[0],
            index: 0
        }]
    );
    assert!(title.changes.inserted.is_empty());
    assert!(title.changes.removed.is_empty());
    let heading = update(
        &mut compiler,
        "---\ntitle: Second\n---\n\n## Renamed\n\nBody\n",
    );
    assert_eq!(
        heading.changes.removed,
        [SlidePosition {
            id: ids[1],
            index: 1
        }]
    );
    assert_eq!(heading.changes.inserted.len(), 1);
    assert_eq!(heading.changes.inserted[0].index, 1);
    assert_ne!(heading.changes.inserted[0].id, ids[1]);
    assert!(heading.changes.modified.is_empty());
    let empty = update(&mut compiler, "");
    assert_eq!(empty.changes.removed.len(), 2);
    assert!(empty.changes.order.is_empty());
    let restored = update(&mut compiler, text);
    assert_eq!(restored.changes.inserted.len(), 2);
    assert!(restored.changes.order.iter().all(|id| !ids.contains(id)));
}

#[test]
fn slide_changes_preserve_duplicate_occurrences_and_ignore_source_gaps() {
    let mut compiler = Compiler::new(SourceFile::anonymous("## Same\n\nOne\n\n## Same\n\nTwo\n"));
    let ids: Vec<_> = compiler.snapshot().slide_ids().collect();
    assert_ne!(ids[0], ids[1]);
    let changed = update(
        &mut compiler,
        "\n\n## Same\n\nOne\n\n\n## Same\n\nChanged\n",
    );
    assert_eq!(changed.changes.order, ids);
    assert_eq!(
        changed.changes.modified,
        [SlidePosition {
            id: ids[1],
            index: 1
        }]
    );
    assert!(changed.changes.reordered.is_empty());
    assert!(changed.changes.inserted.is_empty());
    assert!(changed.changes.removed.is_empty());
}

#[test]
fn slide_changes_remain_available_for_invalid_nonexecuting_updates() {
    let mut compiler = Compiler::new(SourceFile::anonymous("## A\n"));
    let changed = update(
        &mut compiler,
        "## A\n\n```{r}\n#| eval: [\nstop('must not execute')\n```\n",
    );
    assert!(compiler.snapshot().inspection().has_errors());
    assert_eq!(changed.changes.modified.len(), 1);
    assert!(changed.changes.inserted.is_empty());
    let repaired = update(&mut compiler, "## A\n");
    assert!(!compiler.snapshot().inspection().has_errors());
    assert_eq!(repaired.changes.modified, changed.changes.modified);
}
