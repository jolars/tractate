use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use tractate::{
    AcceptedExecution, ArtifactId, ArtifactReference, AttemptId, AttemptReport, CellId, Compiler,
    CompilerSnapshot, CompletionDisposition, CoordinatorError, ExecutionCoordinator,
    ExecutionInputs, ExecutionKey, ExecutionOutcome, ExecutionRequest, ExecutionRunner,
    ExecutionStatus, FakeArtifact, FakeCompletion, FakeRunner, FakeRunnerError, FakeRunnerState,
    FakeScript, FakeStep, OutputEvent, OutputPayload, RejectionReason, RunnerEvent, RunnerIdentity,
    ScheduledCellState, SessionId, SourceFile, StateKey,
};

type Coordinator = ExecutionCoordinator<FakeRunner>;

fn cell(label: &str, language: &str, session: &str) -> String {
    format!("```{{{language}}}\n#| label: {label}\n#| session: {session}\n{label}()\n```\n\n")
}

fn chain() -> String {
    ["a", "b", "c", "d"]
        .map(|label| cell(label, "r", "default"))
        .concat()
}

fn compiler(source: &str) -> Compiler {
    Compiler::new(SourceFile::new("slides.qmd", source))
}

fn coordinator(limit: usize) -> Coordinator {
    Coordinator::new(NonZeroUsize::new(limit).unwrap())
}

fn reconcile<R: ExecutionRunner>(
    coordinator: &mut ExecutionCoordinator<R>,
    snapshot: &CompilerSnapshot,
) {
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
        .collect();
    coordinator
        .reconcile(&plan.keyed(&inputs).unwrap())
        .unwrap();
}

fn ids(snapshot: &CompilerSnapshot) -> Vec<CellId> {
    snapshot
        .evaluation_plan()
        .unwrap()
        .cells
        .iter()
        .map(|cell| cell.cell_id)
        .collect()
}

fn dispatch(coordinator: &mut Coordinator) -> Vec<ExecutionRequest> {
    coordinator.dispatch_ready(FakeRunner::new).unwrap()
}

fn finish(coordinator: &mut Coordinator) -> Vec<AttemptReport> {
    let mut reports = Vec::new();
    for _ in 0..100 {
        if coordinator.is_finished() {
            return reports;
        }
        dispatch(coordinator);
        reports.extend(coordinator.poll().unwrap());
    }
    panic!("execution did not finish");
}

fn result(coordinator: &Coordinator, id: CellId) -> AcceptedExecution {
    coordinator.result(id).unwrap().clone()
}

fn script(steps: Vec<FakeStep>, completion: FakeCompletion) -> FakeScript {
    FakeScript {
        steps,
        completion,
        ..FakeScript::default()
    }
}

#[test]
fn coordinator_reconciles_pending_work_and_prose_edits_without_dispatching() {
    let source = chain();
    let mut compiler = compiler(&source);
    let original = compiler.snapshot();
    let original_ids = ids(&original);
    let mut coordinator = coordinator(2);
    reconcile(&mut coordinator, &original);
    assert_eq!(coordinator.running_count(), 0);
    assert!(
        coordinator
            .runner(coordinator.session_id(original_ids[0]).unwrap())
            .is_none()
    );

    compiler.update_source(SourceFile::new(
        "slides.qmd",
        source.replace("c()", "edited()"),
    ));
    reconcile(&mut coordinator, &compiler.snapshot());
    let reports = finish(&mut coordinator);
    assert_eq!(reports.len(), 4);
    let session_id = coordinator.session_id(original_ids[0]).unwrap();
    let runner = coordinator.runner(session_id).unwrap();
    assert_eq!(runner.execution_count(), 4);
    assert!(
        runner
            .requests()
            .iter()
            .all(|request| request.source_revision == compiler.revision())
    );
    assert!(runner.requests()[2].code.contains("edited()"));
    let current_ids = ids(&compiler.snapshot());
    let retained = current_ids
        .iter()
        .map(|&id| result(&coordinator, id))
        .collect::<Vec<_>>();

    compiler.update_source(SourceFile::new(
        "slides.qmd",
        format!("A prose edit.\n\n{}", compiler.snapshot().source().text()),
    ));
    reconcile(&mut coordinator, &compiler.snapshot());
    assert!(coordinator.is_finished());
    assert!(dispatch(&mut coordinator).is_empty());
    assert!(coordinator.poll().unwrap().is_empty());
    assert_eq!(coordinator.runner(session_id).unwrap().execution_count(), 4);
    assert_eq!(
        current_ids
            .iter()
            .map(|&id| result(&coordinator, id))
            .collect::<Vec<_>>(),
        retained
    );
    assert!(matches!(
        coordinator.reconcile(
            &original
                .evaluation_plan()
                .unwrap()
                .keyed(
                    &original_ids
                        .iter()
                        .map(|&id| (
                            id,
                            ExecutionInputs {
                                environment_digest: [1; 32],
                                computation_digest: [2; 32],
                                volatile_nonce: None
                            }
                        ))
                        .collect()
                )
                .unwrap()
        ),
        Err(CoordinatorError::ObsoleteRevision { .. })
    ));
}

#[test]
fn coordinator_replays_a_valid_prefix_in_a_fresh_incarnation_after_a_suffix_edit() {
    let source = chain();
    let mut compiler = compiler(&source);
    let original = compiler.snapshot();
    let ids = ids(&original);
    let mut coordinator = coordinator(1);
    reconcile(&mut coordinator, &original);
    finish(&mut coordinator);
    let prefix = [result(&coordinator, ids[0]), result(&coordinator, ids[1])];
    let session_id = coordinator.session_id(ids[0]).unwrap();
    let original_state = coordinator.materialized_state_key(session_id);

    compiler.update_source(SourceFile::new(
        "slides.qmd",
        source.replace("c()", "new_c()"),
    ));
    reconcile(&mut coordinator, &compiler.snapshot());
    let current_ids = self::ids(&compiler.snapshot());
    assert_eq!(
        coordinator.materialized_state_key(session_id),
        original_state
    );
    assert_eq!(
        coordinator.state(ids[0]),
        Some(ScheduledCellState::Succeeded)
    );
    assert!(coordinator.result(ids[2]).is_none());
    let reports = finish(&mut coordinator);
    assert_eq!(
        reports
            .iter()
            .map(|report| (report.cell_id, report.replay))
            .collect::<Vec<_>>(),
        vec![
            (ids[0], true),
            (ids[1], true),
            (current_ids[2], false),
            (current_ids[3], false)
        ]
    );
    assert_eq!(
        [result(&coordinator, ids[0]), result(&coordinator, ids[1])],
        prefix
    );
    let runner = coordinator.runner(session_id).unwrap();
    assert_eq!(runner.identity().incarnation, 2);
    assert_eq!(runner.execution_count(), 8);
    assert_eq!(runner.requests()[6].code.trim(), "new_c()");
    let keyed_plan = compiler.snapshot();
    let plan = keyed_plan.evaluation_plan().unwrap();
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
        .collect();
    let keyed = plan.keyed(&inputs).unwrap();
    assert_eq!(
        coordinator.materialized_state_key(session_id),
        keyed.cells()[3].resulting_state_key
    );
}

#[test]
fn coordinator_late_success_after_edit_cannot_change_results_or_materialized_state() {
    for ignore_interrupt in [false, true] {
        let source = chain();
        let mut compiler = compiler(&source);
        let original = compiler.snapshot();
        let ids = ids(&original);
        let mut coordinator = coordinator(1);
        reconcile(&mut coordinator, &original);
        // Materialize a valid prefix before blocking the first invalidated cell.
        for _ in 0..2 {
            dispatch(&mut coordinator);
            coordinator.poll().unwrap();
        }
        let session_id = coordinator.session_id(ids[0]).unwrap();
        let prefix = [result(&coordinator, ids[0]), result(&coordinator, ids[1])];
        let artifact = FakeArtifact {
            reference: ArtifactReference {
                id: ArtifactId("late".into()),
                media_type: "text/plain".into(),
            },
            bytes: b"obsolete".to_vec(),
        };
        coordinator
            .runner_mut(session_id)
            .unwrap()
            .queue_script(FakeScript {
                steps: vec![
                    FakeStep::Block,
                    FakeStep::Output(OutputPayload::Stdout("obsolete".into())),
                    FakeStep::Artifact(artifact),
                ],
                ignore_interrupt,
                ..FakeScript::default()
            });
        let obsolete = dispatch(&mut coordinator).pop().unwrap();
        coordinator.poll().unwrap();
        compiler.update_source(SourceFile::new(
            "slides.qmd",
            source.replace("c()", "new_c()"),
        ));
        reconcile(&mut coordinator, &compiler.snapshot());
        let current_ids = self::ids(&compiler.snapshot());
        assert_eq!(
            coordinator.runner(session_id).unwrap().state(),
            FakeRunnerState::Interrupting
        );
        assert_eq!(coordinator.materialized_state_key(session_id), None);
        assert!(dispatch(&mut coordinator).is_empty());
        if ignore_interrupt {
            coordinator
                .runner_mut(session_id)
                .unwrap()
                .release(obsolete.attempt_id)
                .unwrap();
        }
        let mut reports = Vec::new();
        for _ in 0..4 {
            reports.extend(coordinator.poll().unwrap());
        }
        assert!(!reports.is_empty());
        assert!(reports.iter().all(|report| report.disposition
            == CompletionDisposition::Rejected(RejectionReason::Revoked)));
        assert_eq!(coordinator.running_count(), 0);
        assert_eq!(coordinator.materialized_state_key(session_id), None);
        assert!(coordinator.result(ids[2]).is_none());
        assert_eq!(
            [result(&coordinator, ids[0]), result(&coordinator, ids[1])],
            prefix
        );
        let replay = finish(&mut coordinator);
        assert_eq!(replay.iter().filter(|report| report.replay).count(), 2);
        assert_eq!(
            coordinator
                .runner(session_id)
                .unwrap()
                .identity()
                .incarnation,
            2
        );
        assert!(
            coordinator
                .result(current_ids[2])
                .unwrap()
                .artifacts
                .is_empty()
        );
    }
}

#[test]
fn coordinator_force_cleanup_and_same_revision_retry_reject_retired_attempts() {
    let snapshot = compiler(&chain()).snapshot();
    let ids = ids(&snapshot);
    let mut coordinator = coordinator(1);
    reconcile(&mut coordinator, &snapshot);
    let old = coordinator
        .dispatch_ready(|session_id| {
            let mut runner = FakeRunner::new(session_id);
            runner.queue_script(FakeScript {
                steps: vec![FakeStep::Block],
                ignore_interrupt: true,
                ..FakeScript::default()
            });
            runner
        })
        .unwrap()
        .pop()
        .unwrap();
    coordinator.poll().unwrap();
    coordinator.retry(ids[0]).unwrap();
    assert!(dispatch(&mut coordinator).is_empty());
    coordinator.terminate_obsolete().unwrap();
    let fresh = dispatch(&mut coordinator).pop().unwrap();
    assert_ne!(old.attempt_id, fresh.attempt_id);
    assert_eq!(old.source_revision, fresh.source_revision);
    assert_eq!(old.session_id, fresh.session_id);
    assert_eq!(
        coordinator
            .runner(old.session_id)
            .unwrap()
            .identity()
            .incarnation,
        2
    );
    let late = ExecutionOutcome {
        status: ExecutionStatus::Success,
        diagnostics: Vec::new(),
        events: Vec::new(),
        provenance: tractate::ExecutionProvenance {
            effective_execution_key: old.execution_key,
            request: old,
            runner: RunnerIdentity {
                name: "fake".into(),
                version: "1".into(),
                incarnation: 1,
            },
            duration: std::time::Duration::ZERO,
        },
    };
    let report = coordinator
        .receive(fresh.session_id, RunnerEvent::Complete(late))
        .unwrap()
        .unwrap();
    assert_eq!(
        report.disposition,
        CompletionDisposition::Rejected(RejectionReason::Attempt)
    );
    assert_eq!(coordinator.running_count(), 1);
    assert_eq!(coordinator.materialized_state_key(fresh.session_id), None);
    assert!(coordinator.result(ids[0]).is_none());
    coordinator.poll().unwrap();
    assert_eq!(
        coordinator
            .result(ids[0])
            .unwrap()
            .outcome
            .provenance
            .request
            .attempt_id,
        fresh.attempt_id
    );
    finish(&mut coordinator);
}

#[test]
fn coordinator_failure_crash_and_cancellation_block_only_the_affected_suffix() {
    for completion in [FakeCompletion::Failure, FakeCompletion::Crash] {
        for cancel in [false, true] {
            let source = format!(
                "{}{}{}{}",
                cell("a", "r", "default"),
                cell("b", "r", "default"),
                cell("c", "r", "other"),
                cell("d", "r", "isolated")
            );
            let snapshot = compiler(&source).snapshot();
            let ids = ids(&snapshot);
            let mut coordinator = coordinator(2);
            reconcile(&mut coordinator, &snapshot);
            let default = coordinator.session_id(ids[0]).unwrap();
            coordinator
                .dispatch_ready(|session_id| {
                    let mut runner = FakeRunner::new(session_id);
                    if session_id == default {
                        runner.queue_script(script(vec![FakeStep::Block], completion));
                    }
                    runner
                })
                .unwrap();
            coordinator.poll().unwrap();
            assert_eq!(
                coordinator.state(ids[2]),
                Some(ScheduledCellState::Succeeded)
            );
            dispatch(&mut coordinator);
            coordinator.poll().unwrap();
            assert_eq!(
                coordinator.state(ids[3]),
                Some(ScheduledCellState::Succeeded)
            );
            assert_eq!(coordinator.running_count(), 1);
            if cancel {
                coordinator.runner_mut(default).unwrap().interrupt();
            } else {
                let attempt = coordinator.runner(default).unwrap().requests()[0].attempt_id;
                coordinator
                    .runner_mut(default)
                    .unwrap()
                    .release(attempt)
                    .unwrap();
            }
            coordinator.poll().unwrap();
            assert_eq!(
                coordinator.state(ids[0]),
                Some(if cancel {
                    ScheduledCellState::Cancelled
                } else {
                    ScheduledCellState::Failed
                })
            );
            assert_eq!(coordinator.state(ids[1]), Some(ScheduledCellState::Blocked));
            assert!(coordinator.result(ids[0]).is_none());
            assert_eq!(coordinator.materialized_state_key(default), None);
            assert!(coordinator.is_finished());
            coordinator.retry(ids[0]).unwrap();
            finish(&mut coordinator);
            assert_eq!(
                coordinator.runner(default).unwrap().identity().incarnation,
                2
            );
            assert_eq!(coordinator.runner(default).unwrap().execution_count(), 3);
        }
    }
}

#[test]
fn coordinator_validates_every_completion_identity_before_advancing_state() {
    for reason in [
        RejectionReason::Revision,
        RejectionReason::Attempt,
        RejectionReason::ExecutionKey,
        RejectionReason::PredecessorState,
        RejectionReason::Session,
        RejectionReason::Request,
        RejectionReason::Runner,
        RejectionReason::EventStream,
    ] {
        let mut compiler = compiler(&chain());
        let original = compiler.snapshot();
        let ids = ids(&original);
        let mut coordinator = coordinator(1);
        reconcile(&mut coordinator, &original);
        let request = dispatch(&mut coordinator).pop().unwrap();
        let Some(RunnerEvent::Complete(mut outcome)) =
            coordinator.runner_mut(request.session_id).unwrap().poll()
        else {
            panic!()
        };
        match reason {
            RejectionReason::Revision => {
                compiler.update_source(SourceFile::new("slides.qmd", "Prose.\n"));
                outcome.provenance.request.source_revision = compiler.revision();
            }
            RejectionReason::Attempt => outcome.provenance.request.attempt_id = AttemptId(999),
            RejectionReason::ExecutionKey => {
                outcome.provenance.effective_execution_key = ExecutionKey::from_digest([9; 32])
            }
            RejectionReason::PredecessorState => {
                outcome.provenance.request.required_predecessor_state_key =
                    Some(StateKey::from_digest([9; 32]))
            }
            RejectionReason::Session => outcome.provenance.request.session_id = SessionId(999),
            RejectionReason::Request => outcome.provenance.request.code = "different()".into(),
            RejectionReason::Runner => outcome.provenance.runner.incarnation += 1,
            RejectionReason::EventStream => outcome.events.push(OutputEvent {
                source_revision: request.source_revision,
                session_id: request.session_id,
                cell_id: request.cell_id,
                attempt_id: request.attempt_id,
                sequence: 0,
                payload: OutputPayload::Stdout("undelivered".into()),
            }),
            RejectionReason::Revoked => unreachable!(),
        }
        let report = coordinator
            .receive(request.session_id, RunnerEvent::Complete(outcome))
            .unwrap()
            .unwrap();
        assert_eq!(report.disposition, CompletionDisposition::Rejected(reason));
        assert_eq!(coordinator.materialized_state_key(request.session_id), None);
        assert!(coordinator.result(ids[0]).is_none());
        assert_eq!(coordinator.state(ids[1]), Some(ScheduledCellState::Blocked));
        coordinator.retry(ids[0]).unwrap();
        finish(&mut coordinator);
        assert_eq!(
            coordinator
                .runner(request.session_id)
                .unwrap()
                .identity()
                .incarnation,
            2
        );
    }
}

#[test]
fn coordinator_rejects_duplicate_missing_out_of_order_and_misattributed_output() {
    for fault in 0..7 {
        let snapshot = compiler(&chain()).snapshot();
        let ids = ids(&snapshot);
        let mut coordinator = coordinator(1);
        reconcile(&mut coordinator, &snapshot);
        let request = coordinator
            .dispatch_ready(|session_id| {
                let mut runner = FakeRunner::new(session_id);
                runner.queue_script(script(
                    vec![
                        FakeStep::Output(OutputPayload::Stdout("one".into())),
                        FakeStep::Output(OutputPayload::Stderr("two".into())),
                    ],
                    FakeCompletion::Success,
                ));
                runner
            })
            .unwrap()
            .pop()
            .unwrap();
        let Some(RunnerEvent::Output(first)) =
            coordinator.runner_mut(request.session_id).unwrap().poll()
        else {
            panic!()
        };
        let Some(RunnerEvent::Output(mut second)) =
            coordinator.runner_mut(request.session_id).unwrap().poll()
        else {
            panic!()
        };
        if fault != 1 {
            coordinator
                .receive(request.session_id, RunnerEvent::Output(first.clone()))
                .unwrap();
        }
        match fault {
            0 => second = first,
            1 => {}
            2 => second.sequence = 9,
            3 => second.cell_id = ids[1],
            4 => second.session_id = SessionId(999),
            5 => second.attempt_id = AttemptId(999),
            6 => second.payload = OutputPayload::Stdout("changed".into()),
            _ => unreachable!(),
        }
        let report = coordinator
            .receive(request.session_id, RunnerEvent::Output(second))
            .unwrap();
        if fault == 6 {
            assert!(report.is_none());
            assert!(matches!(
                coordinator.poll().unwrap()[0].disposition,
                CompletionDisposition::Rejected(RejectionReason::EventStream)
            ));
        } else {
            assert!(matches!(
                report.unwrap().disposition,
                CompletionDisposition::Rejected(_)
            ));
        }
        assert_eq!(coordinator.materialized_state_key(request.session_id), None);
        assert!(coordinator.result(ids[0]).is_none());
    }
}

#[test]
fn coordinator_replay_failure_preserves_logical_prefix_and_blocks_the_target() {
    let source = chain();
    let mut compiler = compiler(&source);
    let mut coordinator = coordinator(1);
    let original = compiler.snapshot();
    let ids = ids(&original);
    reconcile(&mut coordinator, &original);
    finish(&mut coordinator);
    let prefix = [result(&coordinator, ids[0]), result(&coordinator, ids[1])];
    compiler.update_source(SourceFile::new(
        "slides.qmd",
        source.replace("c()", "edited()"),
    ));
    reconcile(&mut coordinator, &compiler.snapshot());
    let current_ids = self::ids(&compiler.snapshot());
    let session_id = coordinator.session_id(ids[0]).unwrap();
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .queue_script(script(Vec::new(), FakeCompletion::Failure));
    assert_eq!(dispatch(&mut coordinator)[0].cell_id, ids[0]);
    let report = coordinator.poll().unwrap().pop().unwrap();
    assert!(report.replay);
    assert_eq!(report.target_cell_id, current_ids[2]);
    assert_eq!(
        [result(&coordinator, ids[0]), result(&coordinator, ids[1])],
        prefix
    );
    assert_eq!(
        coordinator.state(current_ids[2]),
        Some(ScheduledCellState::Failed)
    );
    assert_eq!(
        coordinator.state(current_ids[3]),
        Some(ScheduledCellState::Blocked)
    );
    assert_eq!(coordinator.materialized_state_key(session_id), None);
    coordinator.retry(current_ids[2]).unwrap();
    assert_eq!(
        finish(&mut coordinator)
            .iter()
            .filter(|report| report.replay)
            .count(),
        2
    );
    assert_eq!(
        coordinator
            .runner(session_id)
            .unwrap()
            .identity()
            .incarnation,
        3
    );
}

/// Deterministic output makes the full-build oracle sensitive to stale results.
struct EchoRunner(FakeRunner);

impl ExecutionRunner for EchoRunner {
    type Error = FakeRunnerError;
    fn identity(&self) -> &RunnerIdentity {
        self.0.identity()
    }
    fn materialized_state_key(&self) -> Option<StateKey> {
        self.0.materialized_state_key()
    }
    fn start(&mut self, initial: Option<StateKey>) -> Result<(), Self::Error> {
        self.0.start(initial)
    }
    fn execute(
        &mut self,
        request: ExecutionRequest,
        resulting: Option<StateKey>,
    ) -> Result<(), Self::Error> {
        let script = script(
            vec![FakeStep::Output(OutputPayload::Stdout(
                request.code.clone(),
            ))],
            FakeCompletion::Success,
        );
        self.0.dispatch(request, resulting, script)
    }
    fn poll(&mut self) -> Option<RunnerEvent> {
        self.0.poll()
    }
    fn interrupt(&mut self) {
        self.0.interrupt();
    }
    fn accept_success(&mut self, attempt: AttemptId) -> Result<(), Self::Error> {
        self.0.accept_success(attempt)
    }
    fn shutdown(&mut self) -> Result<(), Self::Error> {
        self.0.shutdown();
        Ok(())
    }
}

fn observe(
    coordinator: &mut ExecutionCoordinator<EchoRunner>,
    snapshot: &CompilerSnapshot,
) -> Vec<(ExecutionKey, Vec<OutputPayload>)> {
    reconcile(coordinator, snapshot);
    for _ in 0..100 {
        if coordinator.is_finished() {
            return ids(snapshot)
                .iter()
                .map(|&id| {
                    let outcome = &coordinator.result(id).unwrap().outcome;
                    (
                        outcome.provenance.effective_execution_key,
                        outcome
                            .events
                            .iter()
                            .map(|event| event.payload.clone())
                            .collect(),
                    )
                })
                .collect();
        }
        coordinator
            .dispatch_ready(|id| EchoRunner(FakeRunner::new(id)))
            .unwrap();
        coordinator.poll().unwrap();
    }
    panic!("oracle did not finish");
}

#[test]
fn coordinator_incremental_results_agree_with_clean_full_builds() {
    for newline in ["\n", "\r\n"] {
        let initial = format!(
            "{}{}{}",
            chain(),
            cell("e", "r", "other"),
            cell("f", "python", "isolated")
        );
        let edits = [
            initial.clone(),
            format!("Prose.\n\n{initial}"),
            initial.replace("c()", "changed()"),
            initial.replace("#| label: b", "#| eval: false\n#| label: b"),
            initial.replace(
                "#| label: c\n#| session: default",
                "#| label: c\n#| session: other",
            ),
            format!(
                "{}{}{}",
                cell("f", "python", "isolated"),
                cell("e", "r", "other"),
                chain()
            ),
            String::new(),
            initial.clone(),
        ];
        let mut compiler = compiler("");
        let mut incremental = ExecutionCoordinator::new(NonZeroUsize::new(3).unwrap());
        for source in edits {
            let source = source.replace('\n', newline);
            compiler.update_source(SourceFile::new("slides.qmd", &source));
            let actual = observe(&mut incremental, &compiler.snapshot());
            let mut full = ExecutionCoordinator::new(NonZeroUsize::new(3).unwrap());
            let expected = observe(&mut full, &self::compiler(&source).snapshot());
            assert_eq!(actual, expected, "source: {source}");
        }
    }
}

#[test]
fn coordinator_rekeys_same_revision_and_retains_only_current_volatile_results() {
    for session in ["default", "isolated"] {
        let snapshot = compiler(&cell("a", "r", session)).snapshot();
        let plan = snapshot.evaluation_plan().unwrap();
        let id = plan.cells[0].cell_id;
        let mut coordinator = coordinator(1);
        let mut inputs = BTreeMap::from([(
            id,
            ExecutionInputs {
                environment_digest: [1; 32],
                computation_digest: [2; 32],
                volatile_nonce: Some([3; 32]),
            },
        )]);
        coordinator
            .reconcile(&plan.keyed(&inputs).unwrap())
            .unwrap();
        finish(&mut coordinator);
        let original = result(&coordinator, id);
        coordinator
            .reconcile(&plan.keyed(&inputs).unwrap())
            .unwrap();
        assert!(dispatch(&mut coordinator).is_empty());
        inputs.get_mut(&id).unwrap().volatile_nonce = Some([4; 32]);
        coordinator
            .reconcile(&plan.keyed(&inputs).unwrap())
            .unwrap();
        assert!(coordinator.result(id).is_none());
        finish(&mut coordinator);
        assert_ne!(
            result(&coordinator, id)
                .outcome
                .provenance
                .request
                .attempt_id,
            original.outcome.provenance.request.attempt_id
        );
        assert_eq!(
            coordinator
                .runner(coordinator.session_id(id).unwrap())
                .unwrap()
                .identity()
                .incarnation,
            if session == "isolated" { 1 } else { 2 }
        );
        if session == "isolated" {
            assert_ne!(
                coordinator.session_id(id).unwrap(),
                original.outcome.provenance.request.session_id
            );
            assert_eq!(
                coordinator
                    .runner(coordinator.session_id(id).unwrap())
                    .unwrap()
                    .state(),
                FakeRunnerState::Stopped
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdapterFault {
    None,
    Start,
    Execute,
    Accept,
    Shutdown,
    InitialState,
    AcceptedState,
    Incarnation,
}

struct FaultRunner {
    inner: FakeRunner,
    fault: AdapterFault,
    invalid_identity: RunnerIdentity,
}

impl ExecutionRunner for FaultRunner {
    type Error = FakeRunnerError;

    fn identity(&self) -> &RunnerIdentity {
        if self.fault == AdapterFault::Incarnation {
            &self.invalid_identity
        } else {
            self.inner.identity()
        }
    }

    fn materialized_state_key(&self) -> Option<StateKey> {
        self.inner.materialized_state_key()
    }

    fn start(&mut self, initial: Option<StateKey>) -> Result<(), Self::Error> {
        self.inner.start(initial)?;
        match self.fault {
            AdapterFault::Start => Err(FakeRunnerError::InvalidResultingState),
            AdapterFault::InitialState => self.inner.reset(Some(StateKey::from_digest([99; 32]))),
            _ => Ok(()),
        }
    }

    fn execute(
        &mut self,
        request: ExecutionRequest,
        resulting: Option<StateKey>,
    ) -> Result<(), Self::Error> {
        let completion = if self.fault == AdapterFault::Shutdown {
            FakeCompletion::Failure
        } else {
            FakeCompletion::Success
        };
        self.inner
            .dispatch(request, resulting, script(Vec::new(), completion))?;
        if self.fault == AdapterFault::Execute {
            Err(FakeRunnerError::InvalidResultingState)
        } else {
            Ok(())
        }
    }

    fn poll(&mut self) -> Option<RunnerEvent> {
        self.inner.poll()
    }
    fn interrupt(&mut self) {
        self.inner.interrupt();
    }

    fn accept_success(&mut self, attempt: AttemptId) -> Result<(), Self::Error> {
        self.inner.accept_success(attempt)?;
        match self.fault {
            AdapterFault::Accept => Err(FakeRunnerError::InvalidResultingState),
            AdapterFault::AcceptedState => self.inner.reset(Some(StateKey::from_digest([99; 32]))),
            _ => Ok(()),
        }
    }

    fn shutdown(&mut self) -> Result<(), Self::Error> {
        if self.fault == AdapterFault::Shutdown && self.inner.identity().incarnation > 0 {
            return Err(FakeRunnerError::InvalidResultingState);
        }
        self.inner.shutdown();
        Ok(())
    }
}

#[test]
fn coordinator_adapter_errors_and_unverified_state_never_publish_or_release_unclean_slots() {
    for fault in [
        AdapterFault::Start,
        AdapterFault::Execute,
        AdapterFault::Accept,
        AdapterFault::Shutdown,
        AdapterFault::InitialState,
        AdapterFault::AcceptedState,
        AdapterFault::Incarnation,
    ] {
        let snapshot = compiler(&chain()).snapshot();
        let ids = ids(&snapshot);
        let mut coordinator = ExecutionCoordinator::new(NonZeroUsize::new(1).unwrap());
        reconcile(&mut coordinator, &snapshot);
        let dispatched = coordinator.dispatch_ready(|session_id| FaultRunner {
            inner: FakeRunner::new(session_id),
            fault,
            invalid_identity: RunnerIdentity {
                name: "fake".into(),
                version: "1".into(),
                incarnation: 0,
            },
        });
        if matches!(
            fault,
            AdapterFault::Start
                | AdapterFault::Execute
                | AdapterFault::InitialState
                | AdapterFault::Incarnation
        ) {
            assert!(dispatched.is_err());
        } else {
            dispatched.unwrap();
            let completed = coordinator.poll();
            if fault == AdapterFault::AcceptedState {
                assert_eq!(
                    completed.unwrap()[0].disposition,
                    CompletionDisposition::Rejected(RejectionReason::PredecessorState)
                );
            } else {
                assert!(completed.is_err());
            }
        }
        let session_id = coordinator.session_id(ids[0]).unwrap();
        assert_eq!(coordinator.materialized_state_key(session_id), None);
        assert!(coordinator.result(ids[0]).is_none());
        assert_eq!(coordinator.state(ids[0]), Some(ScheduledCellState::Failed));
        assert_eq!(coordinator.state(ids[1]), Some(ScheduledCellState::Blocked));
        if matches!(
            fault,
            AdapterFault::Start | AdapterFault::Execute | AdapterFault::Shutdown
        ) {
            assert_eq!(coordinator.running_count(), 1);
            assert!(!coordinator.is_finished());
            assert!(
                coordinator
                    .dispatch_ready(|_| unreachable!())
                    .unwrap()
                    .is_empty()
            );
        }
        coordinator.runner_mut(session_id).unwrap().fault = AdapterFault::None;
        coordinator.terminate_obsolete().unwrap();
        assert_eq!(coordinator.running_count(), 0);
        coordinator.retry(ids[0]).unwrap();
        for step in 0..4 {
            coordinator.dispatch_ready(|_| unreachable!()).unwrap();
            if fault == AdapterFault::Execute && step == 0 {
                let old = coordinator.runner(session_id).unwrap().inner.requests()[0].clone();
                let late = ExecutionOutcome {
                    status: ExecutionStatus::Success,
                    diagnostics: Vec::new(),
                    events: Vec::new(),
                    provenance: tractate::ExecutionProvenance {
                        effective_execution_key: old.execution_key,
                        request: old,
                        runner: RunnerIdentity {
                            name: "fake".into(),
                            version: "1".into(),
                            incarnation: 1,
                        },
                        duration: std::time::Duration::ZERO,
                    },
                };
                assert_eq!(
                    coordinator
                        .receive(session_id, RunnerEvent::Complete(late))
                        .unwrap()
                        .unwrap()
                        .disposition,
                    CompletionDisposition::Rejected(RejectionReason::Attempt)
                );
                assert_eq!(coordinator.running_count(), 1);
            }
            coordinator.poll().unwrap();
        }
        assert!(coordinator.is_finished());
        assert_eq!(
            coordinator
                .runner(session_id)
                .unwrap()
                .identity()
                .incarnation,
            2
        );
    }
}

#[test]
fn coordinator_repeating_an_active_plan_is_idempotent_and_removed_work_stays_revoked() {
    let source = chain();
    let mut compiler = compiler(&source);
    let original = compiler.snapshot();
    let ids = ids(&original);
    let mut coordinator = coordinator(1);
    reconcile(&mut coordinator, &original);
    let request = coordinator
        .dispatch_ready(|session_id| {
            let mut runner = FakeRunner::new(session_id);
            runner.queue_script(FakeScript {
                steps: vec![FakeStep::Block],
                ignore_interrupt: true,
                ..FakeScript::default()
            });
            runner
        })
        .unwrap()
        .pop()
        .unwrap();
    coordinator.poll().unwrap();
    reconcile(&mut coordinator, &original);
    assert_eq!(coordinator.state(ids[0]), Some(ScheduledCellState::Running));
    assert_eq!(
        coordinator.runner(request.session_id).unwrap().state(),
        FakeRunnerState::Running
    );
    assert!(dispatch(&mut coordinator).is_empty());
    assert_eq!(
        coordinator
            .runner(request.session_id)
            .unwrap()
            .execution_count(),
        1
    );
    compiler.update_source(SourceFile::new("slides.qmd", "Prose only.\n"));
    reconcile(&mut coordinator, &compiler.snapshot());
    assert!(coordinator.state(ids[0]).is_none());
    assert!(!coordinator.is_finished());
    coordinator
        .runner_mut(request.session_id)
        .unwrap()
        .release(request.attempt_id)
        .unwrap();
    assert_eq!(
        coordinator.poll().unwrap()[0].disposition,
        CompletionDisposition::Rejected(RejectionReason::Revoked)
    );
    assert!(coordinator.is_finished());
    assert!(coordinator.result(ids[0]).is_none());
    assert_eq!(coordinator.materialized_state_key(request.session_id), None);
    assert!(dispatch(&mut coordinator).is_empty());
}
