use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::time::Duration;

use tractate::{
    ArtifactId, ArtifactReference, AttemptId, Compiler, CompilerSnapshot, Diagnostic,
    DiagnosticCode, DisplayBundle, DisplayRepresentation, ExecutionInputs, ExecutionOutcome,
    ExecutionRequest, ExecutionStatus, FakeArtifact, FakeCompletion, FakeRunner, FakeRunnerError,
    FakeRunnerEvent, FakeRunnerState, FakeScript, FakeStep, OutputPayload, ScheduledCellState,
    SessionId, Severity, SourceFile, StateKey,
};

fn snapshot(source: &str) -> CompilerSnapshot {
    Compiler::new(SourceFile::new("slides.qmd", source)).snapshot()
}

fn chain() -> CompilerSnapshot {
    snapshot("```{r}\na()\n```\n\n```{r}\nb()\n```\n")
}

fn work(
    snapshot: &CompilerSnapshot,
    index: usize,
    attempt: u64,
    session: u64,
) -> (ExecutionRequest, Option<StateKey>) {
    let plan = snapshot.evaluation_plan().unwrap();
    let inputs = plan
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
        .collect::<BTreeMap<_, _>>();
    let keyed = plan.keyed(&inputs).unwrap();
    let cell = &keyed.cells()[index];
    (
        ExecutionRequest {
            source_revision: snapshot.revision(),
            attempt_id: AttemptId(attempt),
            cell_id: cell.cell.cell_id,
            execution_key: cell.execution_key,
            session_id: SessionId(session),
            required_predecessor_state_key: cell.required_predecessor_state_key,
            language: cell.cell.language.clone().unwrap(),
            code: cell.cell.code.clone(),
        },
        cell.resulting_state_key,
    )
}

fn warning(snapshot: &CompilerSnapshot) -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        code: DiagnosticCode::ExecutionWarning,
        message: "scripted warning".into(),
        primary: snapshot.evaluation_plan().unwrap().cells[0].origin.clone(),
        related: Vec::new(),
    }
}

fn complete(runner: &mut FakeRunner) -> ExecutionOutcome {
    let Some(FakeRunnerEvent::Complete(outcome)) = runner.poll() else {
        panic!("expected a terminal report");
    };
    assert!(
        runner.poll().is_none(),
        "terminal reports are delivered once"
    );
    outcome
}

#[test]
fn fake_runner_transcripts_are_deterministic_and_artifacts_are_provisional() {
    let snapshot = chain();
    let (request, resulting) = work(&snapshot, 0, 1, 1);
    let artifact = FakeArtifact {
        reference: ArtifactReference {
            id: ArtifactId("plot".into()),
            media_type: "image/png".into(),
        },
        bytes: vec![0, 128, 255],
    };
    let table = FakeArtifact {
        reference: ArtifactReference {
            id: ArtifactId("table".into()),
            media_type: "text/csv".into(),
        },
        bytes: b"value\n2\n".to_vec(),
    };
    let payloads = vec![
        OutputPayload::Stdout("output\n".into()),
        OutputPayload::Stderr("message\n".into()),
        OutputPayload::Warning(warning(&snapshot)),
        OutputPayload::Display(DisplayBundle {
            representations: vec![
                DisplayRepresentation::Png(artifact.reference.clone()),
                DisplayRepresentation::Artifact(table.reference.clone()),
                DisplayRepresentation::Text("2".into()),
                DisplayRepresentation::Html("<b>2</b>".into()),
                DisplayRepresentation::Svg("<svg/>".into()),
            ],
        }),
    ];
    let mut steps = vec![FakeStep::Block, FakeStep::Artifact(artifact.clone())];
    steps.extend(payloads.iter().cloned().map(FakeStep::Output));
    steps.push(FakeStep::Block);
    steps.push(FakeStep::Artifact(table.clone()));
    let script = FakeScript {
        steps,
        duration: Duration::from_millis(37),
        ..FakeScript::default()
    };
    let run = |extra_polls: usize| {
        let mut runner = FakeRunner::new(request.session_id);
        runner
            .start(request.required_predecessor_state_key)
            .unwrap();
        runner
            .dispatch(request.clone(), resulting, script.clone())
            .unwrap();
        let mut transcript = Vec::new();
        let mut gates = 0;
        loop {
            if let Some(event) = runner.poll() {
                let finished = matches!(event, FakeRunnerEvent::Complete(_));
                transcript.push(event);
                if finished {
                    break;
                }
            } else {
                assert!(runner.is_blocked());
                gates += 1;
                for _ in 0..extra_polls {
                    assert!(runner.poll().is_none());
                    assert_eq!(runner.execution_count(), 1);
                    assert_eq!(runner.state(), FakeRunnerState::Running);
                }
                assert_eq!(
                    runner.release(AttemptId(99)),
                    Err(FakeRunnerError::WrongAttempt {
                        expected: request.attempt_id,
                        actual: AttemptId(99),
                    })
                );
                assert!(runner.is_blocked());
                runner.release(request.attempt_id).unwrap();
            }
        }
        assert_eq!(gates, 2);
        assert_eq!(runner.requests(), std::slice::from_ref(&request));
        assert_eq!(runner.execution_count_for(request.cell_id), 1);
        assert_eq!(runner.materialized_state_key(), None);
        assert_eq!(runner.state(), FakeRunnerState::AwaitingAcceptance);
        assert!(runner.poll().is_none());
        transcript
    };
    let transcript = run(0);
    assert_eq!(transcript, run(50));
    assert_eq!(
        transcript[0],
        FakeRunnerEvent::Artifact {
            request: request.clone(),
            artifact
        }
    );
    assert_eq!(
        transcript[5],
        FakeRunnerEvent::Artifact {
            request: request.clone(),
            artifact: table
        }
    );
    let FakeRunnerEvent::Complete(outcome) = &transcript[6] else {
        panic!()
    };
    assert_eq!(outcome.status, ExecutionStatus::Success);
    assert_eq!(outcome.provenance.request, request);
    assert_eq!(
        outcome.provenance.effective_execution_key,
        request.execution_key
    );
    assert_eq!(outcome.provenance.duration, Duration::from_millis(37));
    assert_eq!(outcome.provenance.runner.name, "fake");
    assert_eq!(outcome.provenance.runner.incarnation, 1);
    assert_eq!(outcome.events.len(), payloads.len());
    for (sequence, (event, payload)) in outcome.events.iter().zip(payloads).enumerate() {
        assert_eq!(
            transcript[sequence + 1],
            FakeRunnerEvent::Output(event.clone())
        );
        assert_eq!(event.sequence, sequence as u64);
        assert_eq!(event.source_revision, request.source_revision);
        assert_eq!(event.attempt_id, request.attempt_id);
        assert_eq!(event.session_id, request.session_id);
        assert_eq!(event.cell_id, request.cell_id);
        assert_eq!(event.payload, payload);
    }
}

#[test]
fn fake_runner_counts_only_accepted_dispatches_and_advances_only_accepted_success() {
    let snapshot = chain();
    let (first, first_state) = work(&snapshot, 0, 1, 1);
    let (second, second_state) = work(&snapshot, 1, 2, 1);
    let mut runner = FakeRunner::new(first.session_id);
    assert!(
        runner
            .dispatch(first.clone(), first_state, FakeScript::default())
            .is_err()
    );
    assert_eq!(runner.execution_count(), 0);
    runner.start(first.required_predecessor_state_key).unwrap();
    assert!(runner.start(first.required_predecessor_state_key).is_err());
    let mut wrong_session = first.clone();
    wrong_session.session_id = SessionId(9);
    assert!(matches!(
        runner.dispatch(wrong_session, first_state, FakeScript::default()),
        Err(FakeRunnerError::WrongSession { .. })
    ));
    assert!(matches!(
        runner.dispatch(second.clone(), second_state, FakeScript::default()),
        Err(FakeRunnerError::PredecessorMismatch { .. })
    ));
    assert_eq!(
        runner.dispatch(first.clone(), None, FakeScript::default()),
        Err(FakeRunnerError::InvalidResultingState)
    );
    assert_eq!(runner.execution_count(), 0);
    assert_eq!(
        runner.materialized_state_key(),
        first.required_predecessor_state_key
    );
    runner
        .dispatch(first.clone(), first_state, FakeScript::default())
        .unwrap();
    assert_eq!(
        runner.release(first.attempt_id),
        Err(FakeRunnerError::NotBlocked(first.attempt_id))
    );
    assert!(
        runner
            .dispatch(second.clone(), second_state, FakeScript::default())
            .is_err()
    );
    let outcome = complete(&mut runner);
    assert_eq!(outcome.events, Vec::new());
    assert_eq!(outcome.provenance.duration, Duration::ZERO);
    assert_eq!(runner.materialized_state_key(), None);
    assert!(
        runner
            .dispatch(second.clone(), second_state, FakeScript::default())
            .is_err()
    );
    assert!(runner.accept_success(second.attempt_id).is_err());
    assert_eq!(runner.state(), FakeRunnerState::AwaitingAcceptance);
    runner.accept_success(first.attempt_id).unwrap();
    assert_eq!(runner.materialized_state_key(), first_state);
    runner
        .dispatch(second.clone(), second_state, FakeScript::default())
        .unwrap();
    complete(&mut runner);
    runner.accept_success(second.attempt_id).unwrap();
    assert_eq!(runner.materialized_state_key(), second_state);
    runner.reset(first.required_predecessor_state_key).unwrap();
    assert_eq!(runner.identity().incarnation, 1);
    assert_eq!(
        runner.dispatch(first.clone(), first_state, FakeScript::default()),
        Err(FakeRunnerError::DuplicateAttempt(first.attempt_id))
    );
    let mut retry = first.clone();
    retry.attempt_id = AttemptId(3);
    runner
        .dispatch(retry.clone(), first_state, FakeScript::default())
        .unwrap();
    complete(&mut runner);
    assert!(runner.discard_success(second.attempt_id).is_err());
    runner.discard_success(retry.attempt_id).unwrap();
    assert_eq!(runner.state(), FakeRunnerState::Poisoned);
    assert_eq!(runner.materialized_state_key(), None);
    assert_eq!(runner.requests(), &[first.clone(), second.clone(), retry]);
    assert_eq!(runner.execution_count(), 3);
    assert_eq!(runner.execution_count_for(first.cell_id), 2);
    assert_eq!(runner.execution_count_for(second.cell_id), 1);
}

#[test]
fn fake_runner_failure_and_crash_preserve_partial_output_and_require_recovery() {
    let snapshot = chain();
    let (request, resulting) = work(&snapshot, 0, 1, 1);
    for (completion, state) in [
        (FakeCompletion::Failure, FakeRunnerState::Poisoned),
        (FakeCompletion::Crash, FakeRunnerState::Crashed),
    ] {
        let mut runner = FakeRunner::new(request.session_id);
        runner
            .start(request.required_predecessor_state_key)
            .unwrap();
        runner
            .dispatch(
                request.clone(),
                resulting,
                FakeScript {
                    steps: vec![
                        FakeStep::Output(OutputPayload::Stdout("partial".into())),
                        FakeStep::Block,
                    ],
                    completion,
                    diagnostics: vec![warning(&snapshot)],
                    ..FakeScript::default()
                },
            )
            .unwrap();
        let Some(FakeRunnerEvent::Output(event)) = runner.poll() else {
            panic!()
        };
        assert!(runner.poll().is_none());
        runner.release(request.attempt_id).unwrap();
        let outcome = complete(&mut runner);
        assert_eq!(outcome.status, ExecutionStatus::Failure);
        assert_eq!(outcome.events, vec![event]);
        assert_eq!(outcome.diagnostics, vec![warning(&snapshot)]);
        assert_eq!(runner.state(), state);
        assert_eq!(runner.materialized_state_key(), None);
        assert!(runner.accept_success(request.attempt_id).is_err());
        assert!(
            runner
                .dispatch(request.clone(), resulting, FakeScript::default())
                .is_err()
        );
        runner
            .restart(request.required_predecessor_state_key)
            .unwrap();
        assert_eq!(runner.identity().incarnation, 2);
        let mut retry = request.clone();
        retry.attempt_id = AttemptId(2);
        runner
            .dispatch(retry.clone(), resulting, FakeScript::default())
            .unwrap();
        assert_eq!(complete(&mut runner).provenance.runner.incarnation, 2);
        runner.accept_success(retry.attempt_id).unwrap();
        assert_eq!(runner.execution_count(), 2);
        assert_eq!(runner.materialized_state_key(), resulting);
    }
}

#[test]
fn fake_runner_interruption_cancels_or_delivers_ineligible_late_success() {
    let source = "```{r}\na()\n```\n";
    let mut compiler = Compiler::new(SourceFile::new("slides.qmd", source));
    let snapshot = compiler.snapshot();
    let (request, resulting) = work(&snapshot, 0, 1, 1);
    compiler.update_source(SourceFile::new(
        "slides.qmd",
        source.replace("a()", "edited()"),
    ));
    assert_ne!(request.source_revision, compiler.revision());
    for ignore_interrupt in [false, true] {
        let mut runner = FakeRunner::new(request.session_id);
        runner
            .start(request.required_predecessor_state_key)
            .unwrap();
        runner
            .dispatch(
                request.clone(),
                resulting,
                FakeScript {
                    steps: vec![
                        FakeStep::Output(OutputPayload::Stdout("before".into())),
                        FakeStep::Block,
                        FakeStep::Output(OutputPayload::Stdout("late".into())),
                    ],
                    ignore_interrupt,
                    ..FakeScript::default()
                },
            )
            .unwrap();
        let Some(FakeRunnerEvent::Output(before)) = runner.poll() else {
            panic!()
        };
        assert!(runner.poll().is_none());
        runner.interrupt();
        runner.interrupt();
        assert_eq!(runner.state(), FakeRunnerState::Interrupting);
        assert_eq!(runner.materialized_state_key(), None);
        assert!(
            runner
                .reset(request.required_predecessor_state_key)
                .is_err()
        );
        assert!(
            runner
                .restart(request.required_predecessor_state_key)
                .is_err()
        );
        let mut expected = vec![before];
        if ignore_interrupt {
            for _ in 0..10 {
                assert!(runner.poll().is_none());
            }
            runner.release(request.attempt_id).unwrap();
            let Some(FakeRunnerEvent::Output(late)) = runner.poll() else {
                panic!()
            };
            assert_eq!(late.payload, OutputPayload::Stdout("late".into()));
            expected.push(late);
        }
        let outcome = complete(&mut runner);
        assert_eq!(
            outcome.status,
            if ignore_interrupt {
                ExecutionStatus::Success
            } else {
                ExecutionStatus::Cancelled
            }
        );
        assert_eq!(outcome.provenance.request, request);
        assert_eq!(outcome.events, expected);
        assert_eq!(runner.state(), FakeRunnerState::Poisoned);
        assert!(runner.accept_success(request.attempt_id).is_err());
        runner.interrupt();
        assert_eq!(runner.state(), FakeRunnerState::Poisoned);
        assert_eq!(runner.materialized_state_key(), None);
    }
}

#[test]
fn fake_runner_shutdown_forces_uncooperative_work_to_stop_and_preserves_counts() {
    let snapshot = chain();
    let (request, resulting) = work(&snapshot, 0, 1, 1);
    let mut runner = FakeRunner::new(request.session_id);
    runner
        .start(request.required_predecessor_state_key)
        .unwrap();
    runner
        .dispatch(
            request.clone(),
            resulting,
            FakeScript {
                steps: vec![
                    FakeStep::Output(OutputPayload::Stderr("partial".into())),
                    FakeStep::Block,
                ],
                ignore_interrupt: true,
                ..FakeScript::default()
            },
        )
        .unwrap();
    let Some(FakeRunnerEvent::Output(event)) = runner.poll() else {
        panic!()
    };
    assert!(runner.poll().is_none());
    runner.interrupt();
    let outcome = runner.shutdown().unwrap();
    assert_eq!(outcome.status, ExecutionStatus::Cancelled);
    assert_eq!(outcome.events, vec![event]);
    assert_eq!(outcome.provenance.request, request);
    assert_eq!(runner.state(), FakeRunnerState::Stopped);
    assert_eq!(runner.materialized_state_key(), None);
    assert!(!runner.is_blocked());
    assert!(runner.poll().is_none());
    assert!(runner.shutdown().is_none());
    assert!(runner.release(request.attempt_id).is_err());
    runner
        .start(request.required_predecessor_state_key)
        .unwrap();
    assert_eq!(runner.identity().incarnation, 2);
    assert_eq!(runner.execution_count(), 1);
}

#[test]
fn fake_runner_blocked_session_allows_other_scheduled_work_to_finish() {
    let snapshot = snapshot(concat!(
        "```{r}\na()\n```\n\n",
        "```{r}\nb()\n```\n\n",
        "```{r}\n#| session: other\nc()\n```\n\n",
        "```{r}\n#| session: isolated\nd()\n```\n",
    ));
    let plan = snapshot.evaluation_plan().unwrap();
    let mut schedule = plan.schedule(NonZeroUsize::new(2).unwrap()).unwrap();
    let (blocked, resulting) = work(&snapshot, 0, 1, 1);
    assert_eq!(schedule.reserve_next().unwrap().cell_id, blocked.cell_id);
    let mut runner = FakeRunner::new(blocked.session_id);
    runner
        .start(blocked.required_predecessor_state_key)
        .unwrap();
    runner
        .dispatch(
            blocked.clone(),
            resulting,
            FakeScript {
                steps: vec![FakeStep::Block],
                completion: FakeCompletion::Failure,
                ..FakeScript::default()
            },
        )
        .unwrap();
    assert!(runner.poll().is_none());
    for index in [2, 3] {
        let (request, resulting) = work(&snapshot, index, index as u64 + 1, index as u64 + 1);
        assert_eq!(schedule.reserve_next().unwrap().cell_id, request.cell_id);
        let mut other = FakeRunner::new(request.session_id);
        other.start(request.required_predecessor_state_key).unwrap();
        other
            .dispatch(request.clone(), resulting, FakeScript::default())
            .unwrap();
        let outcome = complete(&mut other);
        other.accept_success(request.attempt_id).unwrap();
        schedule.complete(request.cell_id, outcome.status).unwrap();
        assert_eq!(other.execution_count(), 1);
        assert!(runner.poll().is_none());
    }
    assert_eq!(schedule.running_count(), 1);
    assert!(schedule.reserve_next().is_none());
    runner.release(blocked.attempt_id).unwrap();
    schedule
        .complete(blocked.cell_id, complete(&mut runner).status)
        .unwrap();
    assert_eq!(
        schedule.state(plan.cells[1].cell_id),
        Some(ScheduledCellState::Blocked)
    );
    assert!(schedule.is_finished());
    assert_eq!(runner.execution_count_for(plan.cells[1].cell_id), 0);
}
