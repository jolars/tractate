use super::*;
use crate::document::{BlockKind, CellId, NodeId};
use std::collections::BTreeSet;

fn fragments(snapshot: &CompilerSnapshot) -> Vec<crate::render::html::RenderedSlide> {
    let compilation = snapshot.html(RenderOptions::default());
    compilation.dependencies.assert_complete();
    compilation.result.as_ref().unwrap().slides.clone()
}

fn handles(snapshot: &CompilerSnapshot) -> (Vec<NodeId>, Vec<CellId>) {
    let nodes = std::cell::RefCell::new(Vec::new());
    let mut cells = Vec::new();
    snapshot.presentation().visit_blocks(&mut |block| {
        // Visit each subtree once; visit_blocks already visits nested blocks.
        nodes.borrow_mut().push(block.id);
        if let BlockKind::Cell(cell) = &block.kind {
            cells.push(cell.id);
            assert_eq!(cell.id, cell.result.cell);
        }
        assert_eq!(block.origin.source_span().file(), snapshot.source());
    });
    (nodes.into_inner(), cells)
}

#[test]
fn reuse_changes_only_the_edited_slide_and_shares_semantic_templates() {
    let initial = "## First\n\nOriginal.\n\n## Second\n\n> **café**\n>\n> ```{r}\n> #| eval: false\n> 1\n> ```\n";
    let mut compiler = Compiler::new(SourceFile::new("deck.qmd", initial));
    let original = compiler.snapshot();
    let before = fragments(&original);
    compiler.update_source(SourceFile::new(
        "deck.qmd",
        initial.replace("Original.", "Updated prose with α."),
    ));
    let current = compiler.snapshot();
    let after = fragments(&current);
    assert_eq!(
        before.iter().map(|s| s.id).collect::<Vec<_>>(),
        after.iter().map(|s| s.id).collect::<Vec<_>>()
    );
    assert!(!Arc::ptr_eq(&before[0].html, &after[0].html));
    assert!(Arc::ptr_eq(&before[1].html, &after[1].html));
    for (index, (before, after)) in original.nodes().iter().zip(current.nodes()).enumerate() {
        assert_eq!(Arc::ptr_eq(before, after), index != 1);
    }
    assert_eq!(handles(&original).1, handles(&current).1);
    assert_eq!(
        handles(&current).0.iter().collect::<BTreeSet<_>>().len(),
        handles(&current).0.len()
    );
    assert_eq!(original.source().text(), initial);
    assert_eq!(
        html(&current, RenderOptions::default()),
        html(
            &Compiler::new(current.source().clone()).snapshot(),
            RenderOptions::default()
        )
    );
}

#[test]
fn reuse_preserves_moved_fragments_and_allocates_new_handles_after_removal() {
    let mut compiler = Compiler::new(SourceFile::anonymous("## A\n\na\n\n## B\n\nb\n"));
    let before = fragments(&compiler.snapshot());
    compiler.update_source(SourceFile::anonymous(
        "## Added\n\nnew\n\n## B\n\nb\n\n## A\n\na\n",
    ));
    let moved = fragments(&compiler.snapshot());
    assert!(Arc::ptr_eq(&before[1].html, &moved[1].html));
    assert!(Arc::ptr_eq(&before[0].html, &moved[2].html));
    assert_ne!(moved[0].id, before[0].id);
    assert_ne!(moved[0].id, before[1].id);
    compiler.update_source(SourceFile::anonymous("## B\n\nb\n"));
    let remaining = fragments(&compiler.snapshot());
    assert!(Arc::ptr_eq(&moved[1].html, &remaining[0].html));
    compiler.update_source(SourceFile::anonymous("## A\n\na\n\n## B\n\nb\n"));
    let restored = fragments(&compiler.snapshot());
    assert_ne!(restored[0].id, before[0].id);
    assert!(Arc::ptr_eq(&before[1].html, &restored[1].html));
    assert_eq!(compiler.snapshot().fragments().lock().unwrap().len(), 2);
}

#[test]
fn reuse_identical_blocks_and_slides_never_alias_handles() {
    let mut compiler = Compiler::new(SourceFile::anonymous(
        "## Same\n\nSame.\n\n## Same\n\nSame.\n",
    ));
    let before = fragments(&compiler.snapshot());
    compiler.update_source(SourceFile::anonymous(
        "\n\n## Same\n\nSame.\n\n## Same\n\nSame.\n\n## Same\n\nSame.\n",
    ));
    let current = compiler.snapshot();
    let after = fragments(&current);
    assert!(Arc::ptr_eq(&before[0].html, &after[0].html));
    assert!(Arc::ptr_eq(&before[1].html, &after[1].html));
    assert_eq!(after.iter().map(|s| s.id).collect::<BTreeSet<_>>().len(), 3);
    let (nodes, _) = handles(&current);
    assert_eq!(nodes.iter().collect::<BTreeSet<_>>().len(), nodes.len());
}

#[test]
fn reuse_invalidates_only_used_reference_targets_including_missing_targets() {
    let body = "## Links\n\n[go][target]\n\n## Plain\n\nPlain.\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(format!("[other]: old\n\n{body}")));
    let first = fragments(&compiler.snapshot());
    for (definitions, used_changes) in [
        ("[other]: new\n", false),
        ("[other]: new\n[target]: first\n", true),
        ("[target]: first\n[other]: third\n", false),
        ("[target]: second\n", true),
        ("", true),
    ] {
        let previous = fragments(&compiler.snapshot());
        compiler.update_source(SourceFile::anonymous(format!("{definitions}\n{body}")));
        let snapshot = compiler.snapshot();
        let current = fragments(&snapshot);
        assert_eq!(
            !Arc::ptr_eq(&previous[0].html, &current[0].html),
            used_changes
        );
        assert!(Arc::ptr_eq(&first[1].html, &current[1].html));
        assert_eq!(
            html(&snapshot, RenderOptions::default()),
            html(
                &Compiler::new(snapshot.source().clone()).snapshot(),
                RenderOptions::default()
            )
        );
    }
}

#[test]
fn reuse_compares_effective_cell_policies_and_rechecks_execution_gates() {
    let body = "## Defaults\n\n```{r}\n1\n```\n\n## Override\n\n```{r}\n#| echo: true\n2\n```\n\n## Plain\n\nPlain.\n";
    let source = |eval, echo| {
        SourceFile::anonymous(format!(
            "---\nexecute:\n  eval: {eval}\n  echo: {echo}\n---\n\n{body}"
        ))
    };
    let mut compiler = Compiler::new(source(false, true));
    let first = fragments(&compiler.snapshot());
    compiler.update_source(source(false, false));
    let current = fragments(&compiler.snapshot());
    assert!(!Arc::ptr_eq(&first[0].html, &current[0].html));
    assert!(Arc::ptr_eq(&first[1].html, &current[1].html));
    assert!(Arc::ptr_eq(&first[2].html, &current[2].html));
    compiler.update_source(source(true, false));
    let enabled = compiler.snapshot();
    for no_execute in [false, true] {
        let options = RenderOptions { no_execute };
        assert!(enabled.html(options).result.is_err());
        enabled.html(options).dependencies.assert_complete();
    }
    compiler.update_source(source(false, false));
    let repaired = fragments(&compiler.snapshot());
    for (before, after) in current.iter().zip(repaired) {
        assert!(Arc::ptr_eq(&before.html, &after.html));
    }
}

#[test]
fn reuse_rebinds_resources_and_errors_after_offset_and_path_changes() {
    let body = "## Resources\n\n![α](plot.svg)\n\n![β](other.svg)\n";
    let mut compiler = Compiler::new(SourceFile::new("first.qmd", body));
    let old = compiler.snapshot();
    let before = fragments(&old);
    let edited = format!(
        "<!-- café -->\n\n{}",
        body.replace("\n\n![β]", "\n\n\n\n![β]")
    );
    compiler.update_source(SourceFile::new("second.qmd", edited));
    let current = compiler.snapshot();
    let after = fragments(&current);
    assert!(Arc::ptr_eq(&before[0].html, &after[0].html));
    assert_eq!(
        html(&current, RenderOptions::default()),
        html(
            &Compiler::new(current.source().clone()).snapshot(),
            RenderOptions::default()
        )
    );
    assert_eq!(
        html(&current, RenderOptions::default()).unwrap().title,
        "second"
    );
    assert_eq!(html(&old, RenderOptions::default()).unwrap().title, "first");

    let invalid = "## Error\n\n![α](bad%ZZ.svg)\n";
    compiler.update_source(SourceFile::new("error.qmd", invalid));
    let error = compiler.snapshot();
    assert!(html(&error, RenderOptions::default()).is_err());
    compiler.update_source(SourceFile::new(
        "moved.qmd",
        format!("<!-- shifted -->\n\n{invalid}"),
    ));
    let shifted = compiler.snapshot();
    assert_eq!(
        html(&shifted, RenderOptions::default()),
        html(
            &Compiler::new(shifted.source().clone()).snapshot(),
            RenderOptions::default()
        )
    );
    for no_execute in [false, true] {
        shifted
            .html(RenderOptions { no_execute })
            .dependencies
            .assert_complete();
    }
}

#[test]
fn reuse_works_when_html_is_first_requested_after_an_update() {
    let mut compiler = Compiler::new(SourceFile::anonymous("## Unchanged\n\nText.\n"));
    let previous = compiler.snapshot();
    compiler.update_source(SourceFile::anonymous(
        "<!-- Added -->\n\n## Unchanged\n\nText.\n",
    ));
    let before = fragments(&previous);
    let current = compiler.snapshot();
    let after = current
        .html(RenderOptions { no_execute: true })
        .result
        .as_ref()
        .unwrap();
    assert!(Arc::ptr_eq(&before[0].html, &after.slides[0].html));
    current
        .html(RenderOptions { no_execute: true })
        .dependencies
        .assert_complete();
}

#[test]
fn reuse_rebinds_every_retained_field_in_the_qmd_fixtures() {
    use crate::document::{Origin, SourceSpan, Transform, TransformTree};

    struct Audit<'a>(&'a SourceFile);
    impl Transform for Audit<'_> {
        fn origin(&mut self, origin: &mut Origin) {
            for step in origin.chain() {
                if let Some(span) = step.span() {
                    assert_eq!(span.file(), self.0);
                    assert!(
                        self.0
                            .text()
                            .get(span.range().start..span.range().end)
                            .is_some()
                    );
                }
            }
        }
        fn span(&mut self, span: &mut SourceSpan) {
            assert_eq!(span.file(), self.0);
        }
        fn node(&mut self, _: &mut NodeId) {}
        fn cell(&mut self, _: &mut CellId) {}
    }

    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "qmd") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for text in [text.clone(), text.replace('\n', "\r\n")] {
            let mut compiler = Compiler::new(SourceFile::new(&path, &text));
            let previous = compiler.snapshot();
            for no_execute in [false, true] {
                let _ = html(&previous, RenderOptions { no_execute });
            }
            compiler.update_source(SourceFile::new("moved.qmd", &text));
            let current = compiler.snapshot();
            for (old, new) in previous.nodes().iter().zip(current.nodes()) {
                assert!(Arc::ptr_eq(old, new), "{}", path.display());
            }
            current.presentation().visit_blocks(&mut |block| {
                block.clone().transform(&mut Audit(current.source()));
            });
            for no_execute in [false, true] {
                let options = RenderOptions { no_execute };
                assert_eq!(
                    html(&current, options),
                    html(&Compiler::new(current.source().clone()).snapshot(), options),
                    "{}",
                    path.display()
                );
                current.html(options).dependencies.assert_complete();
            }
        }
    }
}
