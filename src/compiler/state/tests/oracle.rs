use super::*;
use crate::document::{
    CellId, NodeId, Presentation, SemanticIds, SourceSpan, Transform, TransformTree,
};
use std::collections::BTreeMap;

// Full builds own a different identity scope. Canonicalize handles in traversal
// order while preserving repeated uses of a handle, including result slots.
#[derive(Default)]
struct CanonicalIds {
    ids: SemanticIds,
    nodes: BTreeMap<NodeId, NodeId>,
    cells: BTreeMap<CellId, CellId>,
}

impl Transform for CanonicalIds {
    fn origin(&mut self, _: &mut crate::document::Origin) {}
    fn span(&mut self, _: &mut SourceSpan) {}
    fn node(&mut self, id: &mut NodeId) {
        *id = *self.nodes.entry(*id).or_insert_with(|| self.ids.node());
    }
    fn cell(&mut self, id: &mut CellId) {
        *id = *self.cells.entry(*id).or_insert_with(|| self.ids.cell());
    }
}

fn canonical(mut presentation: Presentation) -> Presentation {
    let mut map = CanonicalIds::default();
    presentation.metadata.transform(&mut map);
    for block in &mut presentation.preamble {
        block.transform(&mut CanonicalIds::default());
    }
    for slide in &mut presentation.slides {
        slide.id = map.ids.slide();
        match &mut slide.kind {
            SlideKind::Title(title) => title.transform(&mut CanonicalIds::default()),
            SlideKind::Content(blocks) => {
                for block in blocks {
                    block.transform(&mut CanonicalIds::default());
                }
            }
        }
    }
    presentation
}

fn canonical_plan(plan: &crate::EvaluationPlan) -> Vec<crate::PlannedCell> {
    let mut map = CanonicalIds::default();
    let mut cells = plan.cells.clone();
    for cell in &mut cells {
        map.cell(&mut cell.cell_id);
        if let Some(predecessor) = &mut cell.predecessor {
            map.cell(predecessor);
        }
    }
    cells
}

pub(super) fn assert_full_build(snapshot: &CompilerSnapshot, context: &str) {
    // Bypass semantic-template reuse entirely for the semantic oracle.
    let lowered = parser::lower(snapshot.source().clone());
    let full = crate::compiler::presentation::lower_presentation(
        crate::compiler::build_presentation(lowered.document),
    );
    assert_eq!(
        canonical(snapshot.presentation().clone()),
        canonical(full),
        "{context}: presentation"
    );
    let fresh = Compiler::new(snapshot.source().clone()).snapshot();
    assert_eq!(
        snapshot.inspection(),
        fresh.inspection(),
        "{context}: inspection"
    );
    match (snapshot.evaluation_plan(), fresh.evaluation_plan()) {
        (Ok(actual), Ok(expected)) => {
            assert_eq!(actual.source_revision, snapshot.revision());
            assert_eq!(expected.source_revision, fresh.revision());
            assert_eq!(
                canonical_plan(actual),
                canonical_plan(expected),
                "{context}: evaluation plan"
            );
        }
        (Err(actual), Err(expected)) => assert_eq!(actual, expected, "{context}: plan diagnostics"),
        results => panic!("{context}: inconsistent planning outcome: {results:?}"),
    }
    for no_execute in [false, true] {
        let options = RenderOptions { no_execute };
        assert_eq!(
            html(snapshot, options),
            html(&fresh, options),
            "{context}: HTML ({options:?})"
        );
        // Compare complete diagnostics and resource origins, not just messages.
        match (
            static_html::compile(snapshot, options),
            static_html::compile(&fresh, options),
        ) {
            (Err(actual), Err(expected)) => assert_eq!(actual, expected, "{context}: diagnostics"),
            (Ok(actual), Ok(expected)) => {
                assert_eq!(actual.resources.len(), expected.resources.len());
                for (actual, expected) in actual.resources.iter().zip(&expected.resources) {
                    assert_eq!(actual.origin, expected.origin, "{context}: resource origin");
                }
                for (actual, expected) in actual.slides.iter().zip(&expected.slides) {
                    assert_eq!(actual.origin, expected.origin, "{context}: slide origin");
                }
            }
            _ => panic!("{context}: inconsistent render outcome"),
        }
    }
}

#[test]
fn full_build_oracle_matches_every_fixture_edit_sequence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "qmd"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    for path in paths {
        let fixture = std::fs::read_to_string(&path).unwrap();
        for crlf in [false, true] {
            let text = if crlf {
                fixture.replace('\n', "\r\n")
            } else {
                fixture.clone()
            };
            let added = format!("{text}\n\n## Oracle added\n\nOriginal café.\n");
            let edited = added.replace("Original café.", "Edited α with **emphasis**.");
            let moved = format!("## Oracle added\n\nEdited α with **emphasis**.\n\n{text}");
            let revisions = [
                text.clone(),
                added,
                edited,
                moved,
                text.clone(),
                String::new(),
                text,
            ];
            let mut compiler = Compiler::new(SourceFile::new(&path, &revisions[0]));
            let retained = compiler.snapshot();
            for (index, text) in revisions.iter().enumerate() {
                compiler.update_source(SourceFile::new(&path, text));
                assert_full_build(
                    &compiler.snapshot(),
                    &format!("{} CRLF={crlf} revision={index}", path.display()),
                );
                assert_full_build(&retained, "retained initial revision");
            }
        }
    }
}

#[test]
#[should_panic(expected = "presentation")]
fn full_build_oracle_rejects_stale_semantic_output() {
    let mut stale = CompilerSnapshot::new(SourceFile::anonymous("## Old\n"), SourceRevision(0));
    Arc::get_mut(&mut stale.0).unwrap().source = SourceFile::anonymous("## New\n");
    assert_full_build(&stale, "deliberately stale");
}

#[test]
fn full_build_oracle_matches_reference_default_and_error_edits() {
    let initial = "---\ntitle: Original\nexecute:\n  eval: false\n  echo: true\n---\n\n## First\n\n[Link][target]\n\n```{r}\n#| label: one\n1\n```\n\n## Second\n\n![Plot][target]\n\n```{r}\n#| echo: false\n#| label: two\n2\n```\n\n[target]: figures/plot.svg\n";
    let mut text = initial.to_owned();
    let mut compiler = Compiler::new(SourceFile::new("deck.qmd", &text));
    assert_full_build(&compiler.snapshot(), "dependency initial");
    for (name, from, to) in [
        ("reference target", "figures/plot.svg", "figures/other.svg"),
        ("inherited visibility", "echo: true", "echo: false"),
        ("local override", "#| echo: false", "#| echo: true"),
        ("title", "title: Original", "title: Changed"),
        ("duplicate label", "label: two", "label: one"),
        ("repair label", "label: one\n2", "label: two\n2"),
        ("enable execution", "eval: false", "eval: true"),
        ("disable execution", "eval: true", "eval: false"),
        ("invalid option", "#| echo: true", "#| echo: 'true'"),
        ("repair option", "#| echo: 'true'", "#| echo: true"),
        ("remove definition", "[target]: figures/other.svg", ""),
    ] {
        assert_eq!(text.matches(from).count(), 1, "{name}: edit target");
        text = text.replacen(from, to, 1);
        compiler.update_source(SourceFile::new("deck.qmd", &text));
        assert_full_build(&compiler.snapshot(), name);
    }
    compiler.update_source(SourceFile::new("renamed.qmd", &text));
    assert_full_build(&compiler.snapshot(), "path change");
    compiler.update_source(SourceFile::new("deck.qmd", initial));
    assert_full_build(&compiler.snapshot(), "revert");
}
