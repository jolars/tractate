use super::*;
use crate::document::{BlockKind, CellId, NodeId, Transform, TransformTree};
use std::collections::{BTreeMap, BTreeSet};

fn cell_ids(snapshot: &CompilerSnapshot) -> Vec<CellId> {
    let mut ids = Vec::new();
    snapshot.presentation().visit_blocks(&mut |block| {
        if let BlockKind::Cell(cell) = &block.kind {
            assert_eq!(cell.id, cell.result.cell);
            ids.push(cell.id);
        }
    });
    ids
}

fn labeled_nodes(snapshot: &CompilerSnapshot) -> BTreeMap<String, NodeId> {
    let mut nodes = BTreeMap::new();
    snapshot.presentation().visit_blocks(&mut |block| {
        for attribute in &block.attributes {
            if let crate::document::AttributeKind::Identifier(label) = &attribute.kind {
                nodes.insert(label.clone(), block.id);
            }
        }
        if let BlockKind::Cell(cell) = &block.kind {
            for entry in cell.preamble.as_ref().and_then(|value| value.mapping()).unwrap_or_default() {
                if matches!(&entry.key.kind, crate::document::YamlKind::Scalar { text, .. } if text == "label") {
                    nodes.insert(crate::compiler::validation::name(&entry.value).unwrap(), block.id);
                }
            }
        }
        // Inline visitation from each block would visit nested blocks twice.
        if matches!(
            block.kind,
            BlockKind::Paragraph(_) | BlockKind::Heading { .. }
        ) {
            block.visit(&mut |_| {}, &mut |inline| {
                for attribute in &inline.attributes {
                    if let crate::document::AttributeKind::Identifier(label) = &attribute.kind {
                        nodes.insert(label.clone(), inline.id);
                    }
                }
            });
        }
    });
    nodes
}

#[test]
fn semantic_identities_labels_survive_content_edits_moves_and_clean_builds() {
    let first = "## First {#first}\n\n::: {#container}\n\n[Original]{#span}\n\n```{r}\n#| label: calculation\n#| eval: false\n1\n```\n\n:::\n\n## Second {#second}\n";
    let edited = "## Second changed {#second}\n\n## First changed {#first}\n\n::: {.new #container}\n\n[Changed *text*]{#span}\n\n```{r}\n#| label: calculation\n#| eval: false\n2\n```\n\n:::\n";
    let mut compiler = Compiler::new(SourceFile::new("old.qmd", first));
    let old = compiler.snapshot();
    let old_slides: Vec<_> = old.slide_ids().collect();
    let old_nodes = labeled_nodes(&old);
    assert_eq!(old_nodes.len(), 5);
    let old_cells = cell_ids(&old);
    let old_html = old.html(RenderOptions::default()).result.as_ref().unwrap();
    let update = compiler.update_source_with_changes(SourceFile::new("new.qmd", edited));
    let current = compiler.snapshot();
    assert_eq!(update.changes.order, [old_slides[1], old_slides[0]]);
    assert!(update.changes.inserted.is_empty());
    assert!(update.changes.removed.is_empty());
    assert_eq!(update.changes.modified.len(), 2);
    assert_eq!(update.changes.reordered.len(), 2);
    assert_eq!(labeled_nodes(&current), old_nodes);
    assert_eq!(cell_ids(&current), old_cells);
    let current_html = current
        .html(RenderOptions::default())
        .result
        .as_ref()
        .unwrap();
    assert!(!Arc::ptr_eq(
        &old_html.slides[0].html,
        &current_html.slides[1].html
    ));
    assert!(current_html.slides[1].html.contains("Changed"));
    let fresh = Compiler::new(current.source().clone()).snapshot();
    assert_eq!(fresh.slide_ids().collect::<Vec<_>>(), update.changes.order);
    assert_eq!(labeled_nodes(&fresh), old_nodes);
    assert_eq!(cell_ids(&fresh), old_cells);
    super::oracle::assert_full_build(&current, "labeled edits");
    super::oracle::assert_full_build(&old, "retained labeled snapshot");
    compiler.update_source(SourceFile::anonymous(""));
    super::oracle::assert_full_build(&compiler.snapshot(), "removed labeled entities");
    compiler.update_source(SourceFile::new("restored.qmd", first));
    assert_eq!(
        compiler.snapshot().slide_ids().collect::<Vec<_>>(),
        old_slides
    );
    assert_eq!(cell_ids(&compiler.snapshot()), old_cells);
    super::oracle::assert_full_build(&compiler.snapshot(), "restored labeled entities");
}

#[test]
fn semantic_identities_labels_use_decoded_case_sensitive_values() {
    let cell = |label| format!("```{{r}}\n#| label: {label}\n1\n```\n");
    let ids = |label| cell_ids(&Compiler::new(SourceFile::anonymous(cell(label))).snapshot())[0];
    assert_eq!(ids("'café'"), ids("\"caf\\u00e9\""));
    assert_eq!(ids("name"), ids("'name'"));
    assert_eq!(ids("name"), ids("|-\n#|   name"));
    assert_ne!(ids("Name"), ids("name"));
    assert_ne!(ids("'1'"), ids("name"));
}

#[derive(Default)]
struct UniqueIds {
    nodes: BTreeSet<NodeId>,
}

impl Transform for UniqueIds {
    fn origin(&mut self, _: &mut crate::document::Origin) {}
    fn span(&mut self, _: &mut crate::document::SourceSpan) {}
    fn node(&mut self, id: &mut NodeId) {
        assert!(self.nodes.insert(*id), "aliased node {id:?}");
    }
    fn cell(&mut self, _: &mut CellId) {}
}

#[test]
fn semantic_identities_duplicate_labels_never_alias_and_repair_restores_ids() {
    let valid = "## Slide {#shared}\n\n```{r}\n#| label: cell\n1\n```\n";
    let duplicate = "## Slide {#shared}\n\n[Span]{#shared}\n\n```{r}\n#| label: cell\n1\n```\n\n> ```{r}\n> #| label: cell\n> 2\n> ```\n\n## Other {#shared}\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(valid));
    let old = compiler.snapshot();
    for source in [duplicate, valid, duplicate, valid] {
        compiler.update_source(SourceFile::anonymous(source));
        let snapshot = compiler.snapshot();
        let mut unique = UniqueIds::default();
        // Transform each top-level subtree once to include inline identities.
        snapshot
            .presentation()
            .preamble
            .clone()
            .transform(&mut unique);
        for slide in &snapshot.presentation().slides {
            if let SlideKind::Content(blocks) = &slide.kind {
                blocks.clone().transform(&mut unique);
            }
        }
        let cells = cell_ids(&snapshot);
        assert_eq!(cells.iter().collect::<BTreeSet<_>>().len(), cells.len());
        let slides: Vec<_> = snapshot.slide_ids().collect();
        assert_eq!(slides.iter().collect::<BTreeSet<_>>().len(), slides.len());
        if source == valid {
            assert_eq!(cells, cell_ids(&old));
            assert_eq!(slides, old.slide_ids().collect::<Vec<_>>());
        } else {
            assert!(snapshot.inspection().has_errors());
            assert!(snapshot.evaluation_plan().is_err());
        }
        super::oracle::assert_full_build(&snapshot, "duplicate and repair");
    }
}

#[test]
fn semantic_identities_unlabeled_cells_match_once_inside_changed_containers() {
    let initial = "## First\n\n> Before.\n>\n> ```{r}\n> 1\n> ```\n>\n> ```{r}\n> 1\n> ```\n\n## Second\n\n```{r}\n2\n```\n";
    let edited = "## Second\n\n```{r}\n2\n```\n\n## First\n\n> Changed prose.\n>\n> ```{r}\n> 1\n> ```\n>\n> ```{r}\n> 1\n> ```\n>\n> ```{r}\n> 3\n> ```\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(initial));
    let old = compiler.snapshot();
    let old_cells = cell_ids(&old);
    let old_slides: Vec<_> = old.slide_ids().collect();
    super::oracle::assert_full_build(&old, "initial nested cells");
    compiler.update_source(SourceFile::anonymous(edited));
    let current = compiler.snapshot();
    let cells = cell_ids(&current);
    assert_eq!(&cells[..3], [old_cells[2], old_cells[0], old_cells[1]]);
    assert!(!old_cells.contains(&cells[3]));
    assert_eq!(
        current.slide_ids().collect::<Vec<_>>(),
        [old_slides[1], old_slides[0]]
    );
    super::oracle::assert_full_build(&current, "nested cell matching");
    compiler.update_source(SourceFile::anonymous(edited.replace("> 1\n", "> 9\n")));
    let changed = cell_ids(&compiler.snapshot());
    assert_eq!(changed[0], cells[0]);
    assert_eq!(changed[3], cells[3]);
    assert!(!cells.contains(&changed[1]));
    assert!(!cells.contains(&changed[2]));
    super::oracle::assert_full_build(&compiler.snapshot(), "edited nested cells");
    compiler.update_source(SourceFile::anonymous(""));
    super::oracle::assert_full_build(&compiler.snapshot(), "removed anonymous cells");
    compiler.update_source(SourceFile::anonymous(initial));
    assert!(
        cell_ids(&compiler.snapshot())
            .iter()
            .all(|id| !old_cells.contains(id))
    );
    super::oracle::assert_full_build(&compiler.snapshot(), "restored anonymous cells");
}

#[test]
fn semantic_identities_valid_labels_survive_unrelated_parse_errors() {
    let initial = "## Slide {#slide}\n\n```{r}\n#| label: cell\n1\n```\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(initial));
    let old = compiler.snapshot();
    compiler.update_source(SourceFile::anonymous(format!(
        "---\ntitle: [\n---\n\n{}",
        initial
            .replace("Slide", "Changed")
            .replace("\n1\n", "\n2\n")
    )));
    let invalid = compiler.snapshot();
    assert!(invalid.inspection().has_errors());
    assert!(invalid.inspection().summary.parse_errors > 0);
    assert_eq!(
        invalid.slide_ids().collect::<Vec<_>>(),
        old.slide_ids().collect::<Vec<_>>()
    );
    assert_eq!(cell_ids(&invalid)[0], cell_ids(&old)[0]);
    super::oracle::assert_full_build(&invalid, "unrelated parse errors");
}

#[test]
fn semantic_identities_label_rename_changes_handles_without_changing_execution_keys() {
    use crate::{ExecutionInputs, ExecutionKey};

    let key = |snapshot: &CompilerSnapshot| -> ExecutionKey {
        let plan = snapshot.evaluation_plan().unwrap();
        let inputs = BTreeMap::from([(
            plan.cells[0].cell_id,
            ExecutionInputs {
                environment_digest: [1; 32],
                computation_digest: [2; 32],
                volatile_nonce: None,
            },
        )]);
        plan.keyed(&inputs).unwrap().cells()[0].execution_key
    };
    let initial = "```{r}\n#| label: original\n1\n```\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(initial));
    let old = compiler.snapshot();
    compiler.update_source(SourceFile::anonymous(
        initial.replace("original", "renamed"),
    ));
    let renamed = compiler.snapshot();
    assert_ne!(cell_ids(&old), cell_ids(&renamed));
    assert_ne!(
        old.slide_ids().collect::<Vec<_>>(),
        renamed.slide_ids().collect::<Vec<_>>()
    );
    assert_eq!(key(&old), key(&renamed));
    super::oracle::assert_full_build(&old, "original cell label");
    super::oracle::assert_full_build(&renamed, "renamed cell label");
    compiler.update_source(SourceFile::anonymous(
        renamed.source().text().replace("\n1\n", "\n2\n"),
    ));
    let changed = compiler.snapshot();
    assert_eq!(cell_ids(&renamed), cell_ids(&changed));
    assert_ne!(key(&renamed), key(&changed));
    assert_eq!(
        cell_ids(&changed),
        cell_ids(&Compiler::new(changed.source().clone()).snapshot())
    );
    super::oracle::assert_full_build(&changed, "label and execution identities");
}
