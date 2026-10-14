use std::collections::BTreeMap;

use tractate::{
    Compiler, CompilerSnapshot, EvaluationKeyError, EvaluationPlan, EvaluationScheduleError,
    ExecutionInputs, ExecutionKey, KeyedEvaluationPlan, SessionPreparation, SourceFile, StateKey,
};

fn snapshot(source: &str) -> CompilerSnapshot {
    Compiler::new(SourceFile::new("slides.qmd", source)).snapshot()
}

fn cell(label: &str, language: &str, session: &str) -> String {
    format!("```{{{language}}}\n#| label: {label}\n#| session: {session}\n{label}()\n```\n\n")
}

fn chain() -> String {
    ["a", "b", "c", "d"]
        .map(|label| cell(label, "r", "default"))
        .concat()
}

fn inputs(plan: &EvaluationPlan) -> BTreeMap<tractate::CellId, ExecutionInputs> {
    plan.cells
        .iter()
        .map(|cell| {
            (
                cell.cell_id,
                ExecutionInputs {
                    environment_digest: [1; 32],
                    computation_digest: [2; 32],
                    volatile_nonce: None,
                },
            )
        })
        .collect()
}

fn keys(plan: &KeyedEvaluationPlan<'_>) -> Vec<(ExecutionKey, Option<StateKey>, Option<StateKey>)> {
    plan.cells()
        .iter()
        .map(|cell| {
            (
                cell.execution_key,
                cell.required_predecessor_state_key,
                cell.resulting_state_key,
            )
        })
        .collect()
}

fn assert_render_edits_preserve_keys(initial: &str, edits: &[String], volatile: bool) {
    let observe = |snapshot: &CompilerSnapshot| {
        let plan = snapshot.evaluation_plan().unwrap();
        let mut resolved = inputs(plan);
        if volatile {
            // Rendering edits retain the authorized attempts rather than
            // supplying fresh tokens that would invalidate their suffixes.
            for (index, cell) in plan.cells.iter().enumerate() {
                resolved.get_mut(&cell.cell_id).unwrap().volatile_nonce =
                    Some([u8::try_from(index + 3).unwrap(); 32]);
            }
        }
        keys(&plan.keyed(&resolved).unwrap())
    };
    let mut compiler = Compiler::new(SourceFile::new("slides.qmd", initial));
    let retained = compiler.snapshot();
    let original = observe(&retained);
    assert_eq!(original.len(), 4);
    for edit in edits {
        assert_ne!(edit.as_str(), compiler.snapshot().source().text());
        compiler.update_source(SourceFile::new("slides.qmd", edit));
        let current = observe(&compiler.snapshot());
        assert_eq!(current, observe(&snapshot(edit)), "full build: {edit}");
        assert_eq!(current, original, "render-only edit: {edit}");
    }
    assert_eq!(observe(&retained), original);
}

#[test]
fn evaluation_keys_ignore_cell_render_options_including_nested_cells() {
    let options = "#| echo: true\n#| include: true\n#| results: markup\n#| fig-cap: Caption\n";
    for session in ["default", "analysis", "isolated"] {
        let nested = cell("c", "r", session).replace("c()", &format!("{options}c()"));
        let nested: String = nested.lines().map(|line| format!("> {line}\n")).collect();
        let body = format!(
            "{}{}{nested}\n{}",
            cell("a", "r", session),
            cell("b", "r", session),
            cell("d", "r", session),
        );
        for volatile in [false, true] {
            let initial = format!("---\nexecute:\n  cache: {}\n---\n\n{body}", !volatile);
            let edits = [
                initial.replace("> #| echo: true", "> #| echo: false"),
                initial.replace("> #| include: true", "> #| include: false"),
                initial.replace("> #| results: markup", "> #| results: hide"),
                initial.replace(
                    "> #| fig-cap: Caption",
                    "> #| fig-cap: Updated **caption**.",
                ),
                initial.replace("> #| fig-cap: Caption", "> #| fig-cap: ''"),
                initial.replace("> #| fig-cap: Caption", "> #| fig-cap: '   '"),
                initial.replace(
                    "> #| fig-cap: Caption",
                    "> #| fig-cap: |-\n> #|   A café figure.\n> #|   Another line.",
                ),
                initial.replace(
                    "> #| fig-cap: Caption",
                    "> #| fig-cap: >-\n> #|   A folded\n> #|   caption.",
                ),
                initial
                    .replace("> #| echo: true", "> #| echo: FALSE")
                    .replace("> #| include: true", "> #| include: False")
                    .replace("> #| results: markup", "> #| results: 'hide'")
                    .replace("> #| fig-cap: Caption", "> #| fig-cap: \"New caption\""),
                initial.replace(&options.replace("#|", "> #|"), ""),
                initial.clone(),
            ];
            for line_ending in ["\n", "\r\n"] {
                assert_render_edits_preserve_keys(
                    &initial.replace('\n', line_ending),
                    &edits.each_ref().map(|edit| edit.replace('\n', line_ending)),
                    volatile,
                );
            }
        }
    }
}

#[test]
fn evaluation_keys_ignore_document_render_defaults_and_local_overrides() {
    let defaults = "  echo: true\n  include: true\n  results: markup\n";
    let body = chain().replace("#| session: default\n", "").replace(
        "c()",
        "#| echo: true\n#| include: true\n#| results: markup\nc()",
    );
    for session in ["default", "analysis", "isolated"] {
        for volatile in [false, true] {
            let initial = format!(
                "---\nexecute:\n  session: {session}\n  cache: {}\n{defaults}---\n\n{body}",
                !volatile,
            );
            let edits = [
                initial.replace("  echo: true", "  echo: false"),
                initial.replace("  include: true", "  include: false"),
                initial.replace("  results: markup", "  results: hide"),
                initial.replace(
                    defaults,
                    "  echo: FALSE\n  include: False\n  results: 'hide'\n",
                ),
                initial.replace(
                    defaults,
                    "  results: \"markup\"\n  include: TRUE\n  echo: True\n",
                ),
                initial.replace(defaults, ""),
                initial.clone(),
            ];
            for line_ending in ["\n", "\r\n"] {
                assert_render_edits_preserve_keys(
                    &initial.replace('\n', line_ending),
                    &edits.each_ref().map(|edit| edit.replace('\n', line_ending)),
                    volatile,
                );
            }
        }
    }
}

#[test]
fn evaluation_keys_preserve_cached_prefix_but_require_replay_after_a_suffix_edit() {
    let source = chain();
    let mut compiler = Compiler::new(SourceFile::new("slides.qmd", &source));
    let original = compiler.snapshot();
    let plan = original.evaluation_plan().unwrap();
    let original = plan.keyed(&inputs(plan)).unwrap();
    let cached: Vec<_> = original
        .cells()
        .iter()
        .map(|cell| cell.execution_key)
        .collect();
    let old_materialized_state = original.cells()[3].resulting_state_key;

    compiler.update_source(SourceFile::new(
        "slides.qmd",
        source.replace("c()", "new_c()"),
    ));
    let edited = compiler.snapshot();
    let plan = edited.evaluation_plan().unwrap();
    let current = plan.keyed(&inputs(plan)).unwrap();
    assert_eq!(current.source_revision(), edited.revision());
    assert_ne!(current.source_revision(), original.source_revision());
    assert_eq!(&keys(&current)[..2], &keys(&original)[..2]);
    for (index, cell) in current.cells().iter().enumerate() {
        assert_eq!(cell.has_cached_result(Some(cached[index])), index < 2);
        assert!(!cell.has_cached_result(None));
        if index > 0 {
            assert_eq!(
                cell.required_predecessor_state_key,
                current.cells()[index - 1].resulting_state_key
            );
        }
    }
    for (index, cached_key) in cached.iter().enumerate().skip(2) {
        assert_ne!(current.cells()[index].execution_key, *cached_key);
        assert_ne!(
            current.cells()[index].resulting_state_key,
            original.cells()[index].resulting_state_key
        );
    }
    let c = current.cells()[2].cell.cell_id;
    for state in [
        None,
        old_materialized_state,
        original.cells()[0].resulting_state_key,
    ] {
        assert_eq!(
            current.preparation(c, state),
            Some(SessionPreparation::Replay {
                initial_state_key: current.cells()[0].required_predecessor_state_key.unwrap(),
                prefix: current.cells()[..2].iter().collect(),
            })
        );
    }
    assert_eq!(
        current.preparation(c, current.cells()[1].resulting_state_key),
        Some(SessionPreparation::Ready)
    );
    assert!(current.cells()[1].has_cached_result(Some(cached[1])));
    assert_eq!(
        current.preparation(
            current.cells()[1].cell.cell_id,
            current.cells()[0].resulting_state_key,
        ),
        Some(SessionPreparation::Ready)
    );
    // Cached A and B still need physical execution when reconstructing C's state.
    assert!(current.cells()[0].has_cached_result(Some(cached[0])));
    assert_eq!(
        current.preparation(current.cells()[0].cell.cell_id, None),
        Some(SessionPreparation::Replay {
            initial_state_key: current.cells()[0].required_predecessor_state_key.unwrap(),
            prefix: vec![],
        })
    );
    assert_eq!(
        current.preparation(
            current.cells()[0].cell.cell_id,
            current.cells()[0].required_predecessor_state_key
        ),
        Some(SessionPreparation::Ready)
    );
}

#[test]
fn evaluation_keys_scope_state_and_replay_to_language_and_session() {
    let source = [
        cell("a", "r", "default"),
        cell("named_a", "r", "analysis"),
        cell("python_a", "python", "default"),
        cell("isolated_a", "r", "isolated"),
        cell("b", "r", "default"),
        cell("named_b", "r", "analysis"),
        cell("python_b", "python", "default"),
        cell("isolated_b", "r", "isolated"),
    ]
    .concat();
    let original = snapshot(&source);
    let plan = original.evaluation_plan().unwrap();
    let original = plan.keyed(&inputs(plan)).unwrap();
    let edited = snapshot(&source.replacen("a()", "changed()", 1));
    let plan = edited.evaluation_plan().unwrap();
    let current = plan.keyed(&inputs(plan)).unwrap();
    for index in 0..8 {
        assert_eq!(
            current.cells()[index].has_cached_result(Some(original.cells()[index].execution_key)),
            ![0, 4].contains(&index)
        );
    }
    for (head, suffix) in [(0, 4), (1, 5), (2, 6)] {
        assert_eq!(
            current.preparation(current.cells()[suffix].cell.cell_id, None),
            Some(SessionPreparation::Replay {
                initial_state_key: current.cells()[head]
                    .required_predecessor_state_key
                    .unwrap(),
                prefix: vec![&current.cells()[head]],
            })
        );
        for other_head in [0, 1, 2].into_iter().filter(|other| *other != head) {
            assert_ne!(
                current.cells()[head].required_predecessor_state_key,
                current.cells()[other_head].required_predecessor_state_key
            );
            assert!(matches!(
                current.preparation(
                    current.cells()[suffix].cell.cell_id,
                    current.cells()[other_head].resulting_state_key
                ),
                Some(SessionPreparation::Replay { .. })
            ));
        }
    }
    for index in [3, 7] {
        let cell = &current.cells()[index];
        assert_eq!(cell.required_predecessor_state_key, None);
        assert_eq!(cell.resulting_state_key, None);
        assert_eq!(
            current.preparation(cell.cell.cell_id, original.cells()[0].resulting_state_key),
            Some(SessionPreparation::Isolated)
        );
    }
}

#[test]
fn evaluation_keys_are_independent_of_revisions_positions_and_semantic_handles() {
    let source = chain();
    let mut compiler = Compiler::new(SourceFile::new("slides.qmd", &source));
    let original = compiler.snapshot();
    let plan = original.evaluation_plan().unwrap();
    let original = plan.keyed(&inputs(plan)).unwrap();
    for edit in [
        format!("Leading prose.\n\n{source}"),
        source.replace("label: c", "label: renamed"),
        source.replace("c()", "#| echo: false\n#| fig-cap: Caption\nc()"),
        String::new(),
        source.clone(),
    ] {
        compiler.update_source(SourceFile::new("slides.qmd", &edit));
        let current = compiler.snapshot();
        let plan = current.evaluation_plan().unwrap();
        let current = plan.keyed(&inputs(plan)).unwrap();
        let fresh = snapshot(&edit);
        let plan = fresh.evaluation_plan().unwrap();
        let fresh = plan.keyed(&inputs(plan)).unwrap();
        assert_eq!(keys(&current), keys(&fresh));
        if !edit.is_empty() {
            assert_eq!(keys(&current), keys(&original));
        }
        if edit.contains("label: renamed") {
            assert_ne!(
                current.cells()[2].cell.cell_id,
                original.cells()[2].cell.cell_id
            );
        }
    }
    let current = compiler.snapshot();
    assert_eq!(
        current.evaluation_plan().unwrap().cells[0].cell_id,
        original.cells()[0].cell.cell_id
    );

    let anonymous = "```{r}\n1\n```\n";
    let mut compiler = Compiler::new(SourceFile::anonymous(anonymous));
    let old = compiler.snapshot();
    compiler.update_source(SourceFile::anonymous(""));
    compiler.update_source(SourceFile::anonymous(anonymous));
    let restored = compiler.snapshot();
    let old = old.evaluation_plan().unwrap();
    let restored = restored.evaluation_plan().unwrap();
    assert_ne!(old.cells[0].cell_id, restored.cells[0].cell_id);
    assert_eq!(
        keys(&old.keyed(&inputs(old)).unwrap()),
        keys(&restored.keyed(&inputs(restored)).unwrap())
    );
}

#[test]
fn evaluation_keys_exclude_disabled_cells_and_rebuild_structural_suffixes() {
    let source = chain().replace("b()", "#| eval: false\nb()");
    let original = snapshot(&source);
    let plan = original.evaluation_plan().unwrap();
    let original = plan.keyed(&inputs(plan)).unwrap();
    let edited = snapshot(&source.replace("b()", "disabled_changed()"));
    let plan = edited.evaluation_plan().unwrap();
    assert_eq!(keys(&original), keys(&plan.keyed(&inputs(plan)).unwrap()));
    let enabled = snapshot(&source.replace("eval: false", "eval: true"));
    let plan = enabled.evaluation_plan().unwrap();
    let enabled = plan.keyed(&inputs(plan)).unwrap();
    assert_eq!(keys(&original)[0], keys(&enabled)[0]);
    for (old, new) in [(1, 2), (2, 3)] {
        assert_ne!(
            original.cells()[old].execution_key,
            enabled.cells()[new].execution_key
        );
    }
    let moved = snapshot(
        &[
            cell("b", "r", "default"),
            cell("a", "r", "default"),
            cell("c", "r", "default"),
            cell("d", "r", "default"),
        ]
        .concat(),
    );
    let plan = moved.evaluation_plan().unwrap();
    let moved = plan.keyed(&inputs(plan)).unwrap();
    assert_ne!(
        moved.cells()[2].execution_key,
        enabled.cells()[2].execution_key
    );
    assert_ne!(
        moved.cells()[3].execution_key,
        enabled.cells()[3].execution_key
    );
}

#[test]
fn evaluation_keys_invalidate_for_resolved_inputs_environment_and_volatile_state() {
    let snapshot = snapshot(&chain());
    let plan = snapshot.evaluation_plan().unwrap();
    let base_inputs = inputs(plan);
    let original = plan.keyed(&base_inputs).unwrap();
    let mut changed_inputs = base_inputs.clone();
    changed_inputs
        .get_mut(&plan.cells[2].cell_id)
        .unwrap()
        .computation_digest = [3; 32];
    let changed = plan.keyed(&changed_inputs).unwrap();
    assert_eq!(&keys(&changed)[..2], &keys(&original)[..2]);
    for index in 2..4 {
        assert_ne!(
            changed.cells()[index].execution_key,
            original.cells()[index].execution_key
        );
    }
    for input in changed_inputs.values_mut() {
        input.environment_digest = [4; 32];
    }
    let changed = plan.keyed(&changed_inputs).unwrap();
    for index in 0..4 {
        assert_ne!(keys(&changed)[index], keys(&original)[index]);
    }
    let mut volatile_inputs = base_inputs.clone();
    volatile_inputs
        .get_mut(&plan.cells[1].cell_id)
        .unwrap()
        .volatile_nonce = Some([5; 32]);
    let first_attempt = plan.keyed(&volatile_inputs).unwrap();
    assert_eq!(keys(&first_attempt)[0], keys(&original)[0]);
    assert_eq!(
        first_attempt.cells()[1].execution_key,
        original.cells()[1].execution_key
    );
    assert_ne!(
        first_attempt.cells()[1].resulting_state_key,
        original.cells()[1].resulting_state_key
    );
    volatile_inputs
        .get_mut(&plan.cells[1].cell_id)
        .unwrap()
        .volatile_nonce = Some([6; 32]);
    let next_attempt = plan.keyed(&volatile_inputs).unwrap();
    assert_eq!(
        next_attempt.cells()[1].execution_key,
        first_attempt.cells()[1].execution_key
    );
    for index in 2..4 {
        assert_ne!(
            next_attempt.cells()[index].execution_key,
            first_attempt.cells()[index].execution_key
        );
    }
    assert_eq!(
        keys(&next_attempt),
        keys(&plan.keyed(&volatile_inputs).unwrap())
    );
}

#[test]
fn evaluation_keys_reject_missing_inputs_inconsistent_environments_and_invalid_plans() {
    let snapshot = snapshot(&chain());
    let plan = snapshot.evaluation_plan().unwrap();
    let mut incomplete = inputs(plan);
    incomplete.remove(&plan.cells[1].cell_id);
    assert_eq!(
        plan.keyed(&incomplete).unwrap_err(),
        EvaluationKeyError::MissingInputs(plan.cells[1].cell_id)
    );
    let mut inconsistent = inputs(plan);
    inconsistent
        .get_mut(&plan.cells[1].cell_id)
        .unwrap()
        .environment_digest = [7; 32];
    assert_eq!(
        plan.keyed(&inconsistent).unwrap_err(),
        EvaluationKeyError::InconsistentEnvironment {
            cell_id: plan.cells[1].cell_id,
            predecessor: plan.cells[0].cell_id,
        }
    );
    let mut duplicate = plan.clone();
    duplicate.cells.push(plan.cells[0].clone());
    assert_eq!(
        duplicate.keyed(&inputs(plan)).unwrap_err(),
        EvaluationKeyError::InvalidPlan(EvaluationScheduleError::DuplicateCell(
            plan.cells[0].cell_id
        ))
    );
    let mut invalid = plan.clone();
    invalid.cells[2].predecessor = Some(plan.cells[0].cell_id);
    assert_eq!(
        invalid.keyed(&inputs(plan)).unwrap_err(),
        EvaluationKeyError::InvalidPlan(EvaluationScheduleError::InvalidPredecessor {
            cell_id: plan.cells[2].cell_id,
            expected: Some(plan.cells[1].cell_id),
            actual: Some(plan.cells[0].cell_id),
        })
    );
    let mut empty = plan.clone();
    empty.cells.clear();
    let keyed = empty.keyed(&inputs(plan)).unwrap();
    assert!(keyed.cells().is_empty());
    assert_eq!(keyed.cell(plan.cells[0].cell_id), None);
    assert_eq!(keyed.preparation(plan.cells[0].cell_id, None), None);
}

#[test]
fn evaluation_keys_preserve_isolated_computations_across_moves_and_distinguish_context() {
    let source = [cell("a", "r", "isolated"), cell("b", "r", "isolated")].concat();
    let original = snapshot(&source);
    let plan = original.evaluation_plan().unwrap();
    let original = plan.keyed(&inputs(plan)).unwrap();
    let moved = snapshot(&[cell("b", "r", "isolated"), cell("a", "r", "isolated")].concat());
    let plan = moved.evaluation_plan().unwrap();
    let moved = plan.keyed(&inputs(plan)).unwrap();
    assert_eq!(keys(&original)[0], keys(&moved)[1]);
    assert_eq!(keys(&original)[1], keys(&moved)[0]);
    let base = cell("a", "r", "default");
    let original = snapshot(&base);
    let plan = original.evaluation_plan().unwrap();
    let original = plan.keyed(&inputs(plan)).unwrap();
    for source in [
        cell("a", "python", "default"),
        cell("a", "r", "analysis"),
        cell("a", "r", "isolated"),
    ] {
        let changed = snapshot(&source);
        let plan = changed.evaluation_plan().unwrap();
        assert_ne!(keys(&original), keys(&plan.keyed(&inputs(plan)).unwrap()));
    }
}
