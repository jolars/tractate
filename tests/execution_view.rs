use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use tractate::{
    ArtifactId, ArtifactReference, CellExecutionState, CellId, Compiler, CompilerSnapshot,
    Diagnostic, DiagnosticCode, DisplayBundle, DisplayRepresentation, ExecutionCoordinator,
    ExecutionInputs, ExecutionRequest, ExecutionStore, FakeArtifact, FakeCompletion, FakeRunner,
    FakeScript, FakeStep, OutputPayload, RunnerEvent, ScheduledCellState, Severity, SourceFile,
};

type Coordinator = ExecutionCoordinator<FakeRunner>;

fn source() -> &'static str {
    "```{r}\n#| label: a\na()\n```\n\n```{r}\n#| label: b\nb()\n```\n"
}

fn inputs(snapshot: &CompilerSnapshot) -> BTreeMap<CellId, ExecutionInputs> {
    snapshot
        .evaluation_plan()
        .unwrap()
        .cells
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

fn reconcile(coordinator: &mut Coordinator, snapshot: &CompilerSnapshot) -> Vec<CellId> {
    let plan = snapshot.evaluation_plan().unwrap();
    coordinator
        .reconcile(&plan.keyed(&inputs(snapshot)).unwrap())
        .unwrap();
    plan.cells.iter().map(|cell| cell.cell_id).collect()
}

fn coordinator() -> Coordinator {
    Coordinator::new(NonZeroUsize::new(1).unwrap())
}

fn dispatch(coordinator: &mut Coordinator) -> ExecutionRequest {
    coordinator
        .dispatch_ready(FakeRunner::new)
        .unwrap()
        .pop()
        .unwrap()
}

fn finish(coordinator: &mut Coordinator) {
    for _ in 0..20 {
        if coordinator.is_finished() {
            return;
        }
        coordinator.dispatch_ready(FakeRunner::new).unwrap();
        coordinator.poll().unwrap();
    }
    panic!("execution did not finish");
}

fn diagnostic(snapshot: &CompilerSnapshot, message: &str) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code: DiagnosticCode::ExecutionFailed,
        message: message.into(),
        primary: snapshot.evaluation_plan().unwrap().cells[0].origin.clone(),
        related: Vec::new(),
    }
}

#[test]
fn execution_view_reports_progress_without_exposing_provisional_output() {
    let snapshot = Compiler::new(SourceFile::anonymous(source())).snapshot();
    let mut coordinator = coordinator();
    assert_eq!(coordinator.cell_views(false).count(), 0);
    let ids = reconcile(&mut coordinator, &snapshot);
    assert_eq!(
        coordinator
            .cell_views(false)
            .map(|view| (view.cell_id, view.state))
            .collect::<Vec<_>>(),
        vec![
            (ids[0], CellExecutionState::Pending),
            (ids[1], CellExecutionState::Pending)
        ]
    );
    for view in coordinator.cell_views(true) {
        assert_eq!(view.state, CellExecutionState::Unavailable);
        assert!(view.result.is_none());
        assert!(view.diagnostics.is_empty());
        assert!(
            coordinator
                .runner(coordinator.session_id(view.cell_id).unwrap())
                .is_none()
        );
    }
    assert_eq!(coordinator.running_count(), 0);

    let warning = Diagnostic {
        severity: Severity::Warning,
        code: DiagnosticCode::ExecutionWarning,
        ..diagnostic(&snapshot, "warning in current output")
    };
    let request = coordinator
        .dispatch_ready(|session_id| {
            let mut runner = FakeRunner::new(session_id);
            runner.queue_script(FakeScript {
                steps: vec![
                    FakeStep::Output(OutputPayload::Stdout("provisional".into())),
                    FakeStep::Block,
                ],
                diagnostics: vec![warning.clone()],
                ..FakeScript::default()
            });
            runner
        })
        .unwrap()
        .pop()
        .unwrap();
    coordinator.poll().unwrap();
    coordinator.poll().unwrap();
    let view = coordinator.cell_view(ids[0], false).unwrap();
    assert_eq!(view.state, CellExecutionState::Running);
    assert_eq!(view.progress, ScheduledCellState::Running);
    assert!(view.result.is_none());
    assert!(view.diagnostics.is_empty());
    coordinator
        .runner_mut(request.session_id)
        .unwrap()
        .release(request.attempt_id)
        .unwrap();
    coordinator.poll().unwrap();
    let view = coordinator.cell_view(ids[0], true).unwrap();
    assert_eq!(view.state, CellExecutionState::Succeeded);
    assert_eq!(view.source_revision, snapshot.revision());
    assert_eq!(view.execution_key, request.execution_key);
    assert_eq!(
        view.required_predecessor_state_key,
        request.required_predecessor_state_key
    );
    assert_eq!(view.diagnostics, &[warning]);
    assert_eq!(view.result.unwrap().outcome.events.len(), 1);
    assert_eq!(
        coordinator.cell_view(ids[1], true).unwrap().state,
        CellExecutionState::Unavailable
    );
}

#[test]
fn execution_view_retains_current_failure_and_cancellation_diagnostics() {
    for completion in [FakeCompletion::Failure, FakeCompletion::Crash] {
        for cancel in [false, true] {
            let snapshot = Compiler::new(SourceFile::anonymous(source())).snapshot();
            let mut coordinator = coordinator();
            let ids = reconcile(&mut coordinator, &snapshot);
            let failure = diagnostic(&snapshot, "current computation failed");
            let request = coordinator
                .dispatch_ready(|session_id| {
                    let mut runner = FakeRunner::new(session_id);
                    runner.queue_script(FakeScript {
                        completion,
                        diagnostics: vec![failure.clone()],
                        steps: vec![FakeStep::Block],
                        ..FakeScript::default()
                    });
                    runner
                })
                .unwrap()
                .pop()
                .unwrap();
            coordinator.poll().unwrap();
            if cancel {
                coordinator
                    .runner_mut(request.session_id)
                    .unwrap()
                    .interrupt();
            } else {
                coordinator
                    .runner_mut(request.session_id)
                    .unwrap()
                    .release(request.attempt_id)
                    .unwrap();
            }
            coordinator.poll().unwrap();
            let view = coordinator.cell_view(ids[0], false).unwrap();
            assert_eq!(
                view.state,
                if cancel {
                    CellExecutionState::Cancelled
                } else {
                    CellExecutionState::Failed
                }
            );
            if cancel {
                assert_eq!(view.diagnostics[0].code, DiagnosticCode::ExecutionCancelled);
            } else {
                assert_eq!(view.diagnostics, &[failure]);
            }
            assert!(view.result.is_none());
            let blocked = coordinator.cell_view(ids[1], false).unwrap();
            assert_eq!(blocked.state, CellExecutionState::Failed);
            assert_eq!(blocked.progress, ScheduledCellState::Blocked);
            assert_eq!(
                blocked.diagnostics[0].code,
                DiagnosticCode::ExecutionBlocked
            );
            assert_eq!(
                blocked.diagnostics[0].related[0].origin,
                snapshot.evaluation_plan().unwrap().cells[0].origin
            );
            assert!(blocked.result.is_none());
            coordinator.retry(ids[0]).unwrap();
            assert!(
                coordinator
                    .cell_views(false)
                    .all(|view| view.diagnostics.is_empty())
            );
            finish(&mut coordinator);
            assert!(
                coordinator
                    .cell_views(false)
                    .all(|view| view.state == CellExecutionState::Succeeded)
            );
        }
    }
}

#[test]
fn execution_view_keeps_stale_artifacts_separate_from_current_results_and_cache() {
    let snapshot = Compiler::new(SourceFile::anonymous(source())).snapshot();
    let temp = tempfile::tempdir().unwrap();
    let mut coordinator = Coordinator::with_store(
        NonZeroUsize::new(1).unwrap(),
        ExecutionStore::open(temp.path()).unwrap(),
    );
    let ids = reconcile(&mut coordinator, &snapshot);
    let reference = ArtifactReference {
        id: ArtifactId("plot".into()),
        media_type: "image/png".into(),
    };
    coordinator
        .dispatch_ready(|session_id| {
            let mut runner = FakeRunner::new(session_id);
            runner.queue_script(FakeScript {
                steps: vec![
                    FakeStep::Artifact(FakeArtifact {
                        reference: reference.clone(),
                        bytes: vec![1, 2, 3],
                    }),
                    FakeStep::Output(OutputPayload::Display(DisplayBundle {
                        representations: vec![DisplayRepresentation::Png(reference.clone())],
                    })),
                ],
                diagnostics: vec![diagnostic(&snapshot, "old diagnostic")],
                ..FakeScript::default()
            });
            runner
        })
        .unwrap();
    finish(&mut coordinator);
    let last_good = coordinator.result(ids[0]).unwrap().clone();
    let old_key = last_good.outcome.provenance.request.execution_key;
    let plan = snapshot.evaluation_plan().unwrap();
    let mut changed = inputs(&snapshot);
    changed.get_mut(&ids[0]).unwrap().computation_digest = [3; 32];
    coordinator
        .reconcile(&plan.keyed(&changed).unwrap())
        .unwrap();
    for view in coordinator.cell_views(true) {
        assert_eq!(view.state, CellExecutionState::StaleLastKnownGood);
        assert_eq!(view.progress, ScheduledCellState::Pending);
        assert!(coordinator.result(view.cell_id).is_none());
        assert!(view.diagnostics.is_empty());
        assert_ne!(
            view.execution_key,
            view.result
                .unwrap()
                .outcome
                .provenance
                .effective_execution_key
        );
    }
    let session_id = coordinator.session_id(ids[0]).unwrap();
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .queue_script(FakeScript {
            completion: FakeCompletion::Failure,
            ..FakeScript::default()
        });
    dispatch(&mut coordinator);
    assert_eq!(
        coordinator.cell_view(ids[0], false).unwrap().progress,
        ScheduledCellState::Running
    );
    coordinator.poll().unwrap();
    let view = coordinator.cell_view(ids[0], false).unwrap();
    assert_eq!(view.state, CellExecutionState::StaleLastKnownGood);
    assert_eq!(view.progress, ScheduledCellState::Failed);
    assert_eq!(view.result, Some(&last_good));
    assert_eq!(view.diagnostics[0].code, DiagnosticCode::ExecutionFailed);
    assert_eq!(
        view.result
            .unwrap()
            .artifacts
            .values()
            .next()
            .unwrap()
            .bytes,
        vec![1, 2, 3]
    );
    let new_key = view.execution_key;
    assert!(coordinator.store().unwrap().load(new_key).is_none());
    let cached = coordinator.store().unwrap().load(old_key).unwrap();
    assert_eq!(cached.outcome.events, last_good.outcome.events);
    assert_eq!(
        coordinator.cell_view(ids[1], false).unwrap().progress,
        ScheduledCellState::Blocked
    );
    coordinator.retry(ids[0]).unwrap();
    assert_eq!(
        coordinator.cell_view(ids[0], false).unwrap().result,
        Some(&last_good)
    );
    assert!(
        coordinator
            .cell_view(ids[0], false)
            .unwrap()
            .diagnostics
            .is_empty()
    );
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .queue_script(FakeScript {
            steps: vec![FakeStep::Block],
            ..FakeScript::default()
        });
    let request = dispatch(&mut coordinator);
    coordinator.poll().unwrap();
    coordinator
        .runner_mut(request.session_id)
        .unwrap()
        .interrupt();
    coordinator.poll().unwrap();
    let view = coordinator.cell_view(ids[0], false).unwrap();
    assert_eq!(view.state, CellExecutionState::StaleLastKnownGood);
    assert_eq!(view.progress, ScheduledCellState::Cancelled);
    assert_eq!(view.result, Some(&last_good));
    assert_eq!(view.diagnostics[0].code, DiagnosticCode::ExecutionCancelled);
    coordinator.retry(ids[0]).unwrap();
    finish(&mut coordinator);
    assert_eq!(
        coordinator.cell_view(ids[0], false).unwrap().state,
        CellExecutionState::Succeeded
    );
    assert!(
        coordinator
            .cell_view(ids[0], false)
            .unwrap()
            .result
            .unwrap()
            .artifacts
            .is_empty()
    );
}

#[test]
fn execution_view_preserves_valid_prefix_during_replay_and_reports_replay_failure() {
    let snapshot = Compiler::new(SourceFile::anonymous(source())).snapshot();
    let mut coordinator = coordinator();
    let ids = reconcile(&mut coordinator, &snapshot);
    finish(&mut coordinator);
    let prefix = coordinator.result(ids[0]).unwrap().clone();
    let suffix = coordinator.result(ids[1]).unwrap().clone();
    let plan = snapshot.evaluation_plan().unwrap();
    let mut changed = inputs(&snapshot);
    changed.get_mut(&ids[1]).unwrap().computation_digest = [3; 32];
    coordinator
        .reconcile(&plan.keyed(&changed).unwrap())
        .unwrap();
    let session_id = coordinator.session_id(ids[0]).unwrap();
    let failure = diagnostic(&snapshot, "prefix replay failed");
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .queue_script(FakeScript {
            completion: FakeCompletion::Failure,
            diagnostics: vec![failure.clone()],
            ..FakeScript::default()
        });
    let replay = dispatch(&mut coordinator);
    assert_eq!(replay.cell_id, ids[0]);
    assert_eq!(
        coordinator.cell_view(ids[0], false).unwrap().state,
        CellExecutionState::Succeeded
    );
    assert_eq!(
        coordinator.cell_view(ids[1], false).unwrap().progress,
        ScheduledCellState::Running
    );
    coordinator.poll().unwrap();
    let failed = coordinator.cell_view(ids[1], false).unwrap();
    assert_eq!(failed.state, CellExecutionState::StaleLastKnownGood);
    assert_eq!(failed.progress, ScheduledCellState::Failed);
    assert_eq!(failed.result, Some(&suffix));
    assert_eq!(failed.diagnostics, &[failure]);
    assert_eq!(
        coordinator.cell_view(ids[0], false).unwrap().result,
        Some(&prefix)
    );
    assert!(
        coordinator
            .cell_view(ids[0], false)
            .unwrap()
            .diagnostics
            .is_empty()
    );
}

#[test]
fn execution_view_rejects_obsolete_diagnostics_and_preserves_reusable_provenance() {
    let mut compiler = Compiler::new(SourceFile::anonymous(source()));
    let snapshot = compiler.snapshot();
    let mut coordinator = coordinator();
    let ids = reconcile(&mut coordinator, &snapshot);
    finish(&mut coordinator);
    let accepted = coordinator.result(ids[0]).unwrap().clone();
    compiler.update_source(SourceFile::anonymous(format!("Prose.\n\n{}", source())));
    reconcile(&mut coordinator, &compiler.snapshot());
    let view = coordinator.cell_view(ids[0], false).unwrap();
    assert_eq!(view.state, CellExecutionState::Succeeded);
    assert_eq!(view.source_revision, compiler.revision());
    assert_eq!(view.result, Some(&accepted));
    assert_ne!(
        view.source_revision,
        view.result
            .unwrap()
            .outcome
            .provenance
            .request
            .source_revision
    );

    coordinator.retry(ids[0]).unwrap();
    let session_id = coordinator.session_id(ids[0]).unwrap();
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .queue_script(FakeScript {
            steps: vec![FakeStep::Block],
            completion: FakeCompletion::Failure,
            diagnostics: vec![diagnostic(&snapshot, "obsolete failure")],
            ignore_interrupt: true,
            ..FakeScript::default()
        });
    let request = dispatch(&mut coordinator);
    coordinator.poll().unwrap();
    compiler.update_source(SourceFile::anonymous(format!("New prose.\n\n{}", source())));
    reconcile(&mut coordinator, &compiler.snapshot());
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .release(request.attempt_id)
        .unwrap();
    coordinator.poll().unwrap();
    let view = coordinator.cell_view(ids[0], false).unwrap();
    assert_eq!(view.state, CellExecutionState::StaleLastKnownGood);
    assert_eq!(view.progress, ScheduledCellState::Pending);
    assert_eq!(view.result, Some(&accepted));
    assert!(view.diagnostics.is_empty());
    assert!(coordinator.result(ids[0]).is_none());

    compiler.update_source(SourceFile::anonymous(
        source().replace("#| label:", "#| eval: false\n#| label:"),
    ));
    reconcile(&mut coordinator, &compiler.snapshot());
    assert!(coordinator.cell_view(ids[0], false).is_none());
    assert_eq!(coordinator.cell_views(false).count(), 0);
    compiler.update_source(SourceFile::anonymous(source()));
    let new_ids = reconcile(&mut coordinator, &compiler.snapshot());
    assert!(new_ids.iter().all(
        |&id| coordinator.cell_view(id, true).unwrap().state == CellExecutionState::Unavailable
    ));
}

#[test]
fn execution_view_reports_protocol_failures_without_publishing_output() {
    let snapshot = Compiler::new(SourceFile::anonymous(source())).snapshot();
    let mut coordinator = coordinator();
    let ids = reconcile(&mut coordinator, &snapshot);
    let request = dispatch(&mut coordinator);
    let invalid = tractate::OutputEvent {
        source_revision: request.source_revision,
        session_id: request.session_id,
        cell_id: request.cell_id,
        attempt_id: request.attempt_id,
        sequence: 1,
        payload: OutputPayload::Stdout("invalid output".into()),
    };
    coordinator
        .receive(request.session_id, RunnerEvent::Output(invalid))
        .unwrap();
    let failed = coordinator.cell_view(ids[0], false).unwrap();
    assert_eq!(failed.state, CellExecutionState::Failed);
    assert!(failed.result.is_none());
    assert_eq!(failed.diagnostics[0].code, DiagnosticCode::ExecutionFailed);
    assert!(failed.diagnostics[0].message.contains("EventStream"));
}
