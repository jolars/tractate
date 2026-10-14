//! Scripted effects for deterministic execution tests, without an interpreter.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use super::{
    AttemptId, ExecutionOutcome, ExecutionProvenance, ExecutionRequest, ExecutionRunner,
    ExecutionStatus, OutputEvent, OutputPayload, ProvisionalArtifact, RunnerEvent, RunnerIdentity,
    SessionId, StateKey,
};
use crate::{CellId, Diagnostic};

/// Provisional bytes emitted before a result can be accepted or published.
///
/// The fake does not write files or assign content digests. The receiving
/// artifact store owns staging, verification, and transactional publication.
pub type FakeArtifact = ProvisionalArtifact;

/// An observable effect or an explicit pause in one attempt's script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeStep {
    Output(OutputPayload),
    Artifact(FakeArtifact),
    /// Polling stays pending until the test releases this attempt's gate.
    Block,
}

/// The terminal behavior after all scripted steps have been consumed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FakeCompletion {
    #[default]
    Success,
    Failure,
    /// Fail the attempt and leave the runner crashed rather than poisoned.
    Crash,
}

/// Effects supplied by the test, independent of executable source and rendering.
///
/// An empty default script succeeds. Duration is supplied rather than measured,
/// so polling and gate release timing cannot change the resulting transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeScript {
    pub steps: Vec<FakeStep>,
    pub completion: FakeCompletion,
    pub diagnostics: Vec<Diagnostic>,
    pub duration: Duration,
    /// Continue emitting the original script after interruption, including a
    /// late success. Such a completion cannot restore usable interpreter state.
    pub ignore_interrupt: bool,
}

/// Lifecycle states observable by the test's execution coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeRunnerState {
    Stopped,
    Idle,
    Running,
    Interrupting,
    /// Success is provisional until the coordinator accepts or discards it.
    AwaitingAcceptance,
    Poisoned,
    Crashed,
}

/// One effect delivered by polling; terminal events are delivered exactly once.
pub type FakeRunnerEvent = RunnerEvent;

/// A command rejected before changing state or counting a dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeRunnerError {
    InvalidState {
        operation: &'static str,
        state: FakeRunnerState,
    },
    WrongSession {
        expected: SessionId,
        actual: SessionId,
    },
    PredecessorMismatch {
        required: Option<StateKey>,
        materialized: Option<StateKey>,
    },
    /// Stateful dispatches require a resulting key; isolated dispatches do not.
    InvalidResultingState,
    DuplicateAttempt(AttemptId),
    WrongAttempt {
        expected: AttemptId,
        actual: AttemptId,
    },
    NotBlocked(AttemptId),
    IncarnationExhausted,
}

impl fmt::Display for FakeRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidState { operation, state } => {
                write!(formatter, "cannot {operation} a fake runner in {state:?}")
            }
            Self::WrongSession { expected, actual } => {
                write!(
                    formatter,
                    "expected session {expected:?}, received {actual:?}"
                )
            }
            Self::PredecessorMismatch {
                required,
                materialized,
            } => write!(
                formatter,
                "required predecessor {required:?} differs from materialized state {materialized:?}"
            ),
            Self::InvalidResultingState => {
                formatter.write_str("resulting state must be present exactly for stateful requests")
            }
            Self::DuplicateAttempt(attempt) => write!(formatter, "attempt {attempt:?} already ran"),
            Self::WrongAttempt { expected, actual } => {
                write!(
                    formatter,
                    "expected attempt {expected:?}, received {actual:?}"
                )
            }
            Self::NotBlocked(attempt) => write!(formatter, "attempt {attempt:?} is not blocked"),
            Self::IncarnationExhausted => formatter.write_str("fake runner incarnations exhausted"),
        }
    }
}

impl std::error::Error for FakeRunnerError {}

#[derive(Debug)]
struct ActiveAttempt {
    request: ExecutionRequest,
    resulting_state_key: Option<StateKey>,
    steps: VecDeque<FakeStep>,
    completion: FakeCompletion,
    diagnostics: Vec<Diagnostic>,
    duration: Duration,
    ignore_interrupt: bool,
    events: Vec<OutputEvent>,
    blocked: bool,
    revoked: bool,
}

/// A counting runner driven entirely by dispatch, poll, and explicit gate release.
///
/// Source code is recorded but never interpreted. Each instance owns one logical
/// session and accepts one attempt at a time. Polling never waits, sleeps, starts
/// processes, or accesses the filesystem. Independent instances can therefore
/// reproduce arbitrary completion orders without threads or clock races.
///
/// Successful state changes require explicit coordinator acceptance. This fake
/// cannot validate which source revision is current or publish any result.
#[derive(Debug)]
pub struct FakeRunner {
    session_id: SessionId,
    identity: RunnerIdentity,
    state: FakeRunnerState,
    materialized_state_key: Option<StateKey>,
    requests: Vec<ExecutionRequest>,
    active: Option<ActiveAttempt>,
    successful: Option<(AttemptId, Option<StateKey>)>,
    scripts: VecDeque<FakeScript>,
}

impl FakeRunner {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            identity: RunnerIdentity {
                name: "fake".into(),
                version: "1".into(),
                incarnation: 0,
            },
            state: FakeRunnerState::Stopped,
            materialized_state_key: None,
            requests: Vec::new(),
            active: None,
            successful: None,
            scripts: VecDeque::new(),
        }
    }

    /// Start a fresh incarnation, using `None` for an isolated environment.
    pub fn start(&mut self, initial_state_key: Option<StateKey>) -> Result<(), FakeRunnerError> {
        self.require_state("start", &[FakeRunnerState::Stopped])?;
        self.start_incarnation(initial_state_key)
    }

    /// Recover a failed runner with a new incarnation in the same logical session.
    pub fn restart(&mut self, initial_state_key: Option<StateKey>) -> Result<(), FakeRunnerError> {
        self.require_state(
            "restart",
            &[FakeRunnerState::Poisoned, FakeRunnerState::Crashed],
        )?;
        self.start_incarnation(initial_state_key)
    }

    fn start_incarnation(
        &mut self,
        initial_state_key: Option<StateKey>,
    ) -> Result<(), FakeRunnerError> {
        self.identity.incarnation = self
            .identity
            .incarnation
            .checked_add(1)
            .ok_or(FakeRunnerError::IncarnationExhausted)?;
        self.materialized_state_key = initial_state_key;
        self.state = FakeRunnerState::Idle;
        Ok(())
    }

    /// Restore a proven initial environment in place, retaining the incarnation.
    pub fn reset(&mut self, initial_state_key: Option<StateKey>) -> Result<(), FakeRunnerError> {
        self.require_state("reset", &[FakeRunnerState::Idle, FakeRunnerState::Poisoned])?;
        self.materialized_state_key = initial_state_key;
        self.state = FakeRunnerState::Idle;
        Ok(())
    }

    /// Record an accepted dispatch; rejected commands never increase the count.
    ///
    /// The resulting key comes from the keyed evaluation plan. Acceptance later
    /// materializes it only after the coordinator validates the terminal report.
    pub fn dispatch(
        &mut self,
        request: ExecutionRequest,
        resulting_state_key: Option<StateKey>,
        script: FakeScript,
    ) -> Result<(), FakeRunnerError> {
        self.require_state("dispatch", &[FakeRunnerState::Idle])?;
        if request.session_id != self.session_id {
            return Err(FakeRunnerError::WrongSession {
                expected: self.session_id,
                actual: request.session_id,
            });
        }
        if request.required_predecessor_state_key != self.materialized_state_key {
            return Err(FakeRunnerError::PredecessorMismatch {
                required: request.required_predecessor_state_key,
                materialized: self.materialized_state_key,
            });
        }
        if request.required_predecessor_state_key.is_some() != resulting_state_key.is_some() {
            return Err(FakeRunnerError::InvalidResultingState);
        }
        if self
            .requests
            .iter()
            .any(|previous| previous.attempt_id == request.attempt_id)
        {
            return Err(FakeRunnerError::DuplicateAttempt(request.attempt_id));
        }
        self.requests.push(request.clone());
        self.active = Some(ActiveAttempt {
            request,
            resulting_state_key,
            steps: script.steps.into(),
            completion: script.completion,
            diagnostics: script.diagnostics,
            duration: script.duration,
            ignore_interrupt: script.ignore_interrupt,
            events: Vec::new(),
            blocked: false,
            revoked: false,
        });
        self.materialized_state_key = None;
        self.state = FakeRunnerState::Running;
        Ok(())
    }

    /// Deliver the next effect, or `None` while blocked or without active work.
    pub fn poll(&mut self) -> Option<FakeRunnerEvent> {
        let active = self.active.as_mut()?;
        if active.revoked && !active.ignore_interrupt {
            return Some(FakeRunnerEvent::Complete(
                self.finish(ExecutionStatus::Cancelled, false),
            ));
        }
        if active.blocked {
            return None;
        }
        match active.steps.pop_front() {
            Some(FakeStep::Block) => {
                active.blocked = true;
                None
            }
            Some(FakeStep::Output(payload)) => {
                let request = &active.request;
                let event = OutputEvent {
                    source_revision: request.source_revision,
                    session_id: request.session_id,
                    cell_id: request.cell_id,
                    attempt_id: request.attempt_id,
                    sequence: active.events.len() as u64,
                    payload,
                };
                active.events.push(event.clone());
                Some(FakeRunnerEvent::Output(event))
            }
            Some(FakeStep::Artifact(artifact)) => Some(FakeRunnerEvent::Artifact {
                request: active.request.clone(),
                artifact,
            }),
            None => {
                let completion = active.completion;
                let status = match completion {
                    FakeCompletion::Success => ExecutionStatus::Success,
                    FakeCompletion::Failure => ExecutionStatus::Failure,
                    FakeCompletion::Crash if active.revoked => ExecutionStatus::Cancelled,
                    FakeCompletion::Crash => ExecutionStatus::Failure,
                };
                Some(FakeRunnerEvent::Complete(
                    self.finish(status, completion == FakeCompletion::Crash),
                ))
            }
        }
    }

    fn finish(&mut self, status: ExecutionStatus, crashed: bool) -> ExecutionOutcome {
        let active = self
            .active
            .take()
            .expect("terminal delivery requires an active attempt");
        self.state = if crashed {
            FakeRunnerState::Crashed
        } else if status == ExecutionStatus::Success && !active.revoked {
            self.successful = Some((active.request.attempt_id, active.resulting_state_key));
            FakeRunnerState::AwaitingAcceptance
        } else {
            FakeRunnerState::Poisoned
        };
        ExecutionOutcome {
            status,
            diagnostics: if status == ExecutionStatus::Cancelled {
                Vec::new()
            } else {
                active.diagnostics
            },
            events: active.events,
            provenance: ExecutionProvenance {
                effective_execution_key: active.request.execution_key,
                request: active.request,
                runner: self.identity.clone(),
                duration: active.duration,
            },
        }
    }

    /// Release only the currently blocked attempt, preventing delayed controls
    /// from accidentally releasing a different attempt's gate.
    pub fn release(&mut self, attempt_id: AttemptId) -> Result<(), FakeRunnerError> {
        self.require_state(
            "release",
            &[FakeRunnerState::Running, FakeRunnerState::Interrupting],
        )?;
        let active = self
            .active
            .as_mut()
            .expect("running states have an active attempt");
        Self::check_attempt(active.request.attempt_id, attempt_id)?;
        if !active.blocked {
            return Err(FakeRunnerError::NotBlocked(attempt_id));
        }
        active.blocked = false;
        Ok(())
    }

    /// Revoke active work immediately. Cooperative scripts cancel on the next
    /// poll; uncooperative scripts keep their gates and remaining effects.
    /// Repeated interruption, including after termination, is idempotent.
    pub fn interrupt(&mut self) {
        if let Some(active) = &mut self.active {
            active.revoked = true;
            self.state = FakeRunnerState::Interrupting;
        }
    }

    /// Materialize a provisional success only after the coordinator has checked
    /// revision, attempt, execution key, predecessor key, and protocol validity.
    pub fn accept_success(&mut self, attempt_id: AttemptId) -> Result<(), FakeRunnerError> {
        self.require_state("accept success", &[FakeRunnerState::AwaitingAcceptance])?;
        let (expected, resulting) = self
            .successful
            .expect("awaiting acceptance requires a success");
        Self::check_attempt(expected, attempt_id)?;
        self.successful = None;
        self.materialized_state_key = resulting;
        self.state = FakeRunnerState::Idle;
        Ok(())
    }

    /// Discard stale or invalid success, retaining no usable interpreter state.
    pub fn discard_success(&mut self, attempt_id: AttemptId) -> Result<(), FakeRunnerError> {
        self.require_state("discard success", &[FakeRunnerState::AwaitingAcceptance])?;
        let (expected, _) = self
            .successful
            .expect("awaiting acceptance requires a success");
        Self::check_attempt(expected, attempt_id)?;
        self.successful = None;
        self.state = FakeRunnerState::Poisoned;
        Ok(())
    }

    /// Force cleanup, even when interruption is ignored. Active work produces a
    /// cancellation with its partial output; repeated shutdown returns `None`.
    pub fn shutdown(&mut self) -> Option<ExecutionOutcome> {
        let outcome = if self.active.is_some() {
            Some(self.finish(ExecutionStatus::Cancelled, false))
        } else {
            None
        };
        self.successful = None;
        self.materialized_state_key = None;
        self.state = FakeRunnerState::Stopped;
        outcome
    }

    pub fn state(&self) -> FakeRunnerState {
        self.state
    }

    /// The identity's incarnation is zero until the first startup.
    pub fn identity(&self) -> &RunnerIdentity {
        &self.identity
    }

    pub fn materialized_state_key(&self) -> Option<StateKey> {
        self.materialized_state_key
    }

    /// Accepted dispatches in order, retained across reset, restart, and shutdown.
    pub fn requests(&self) -> &[ExecutionRequest] {
        &self.requests
    }

    pub fn execution_count(&self) -> usize {
        self.requests.len()
    }

    pub fn execution_count_for(&self, cell_id: CellId) -> usize {
        self.requests
            .iter()
            .filter(|request| request.cell_id == cell_id)
            .count()
    }

    pub fn is_blocked(&self) -> bool {
        self.active.as_ref().is_some_and(|active| active.blocked)
    }

    /// Supply effects for the next coordinator dispatch, including prefix replay.
    /// An empty queue supplies a successful, empty script.
    pub fn queue_script(&mut self, script: FakeScript) {
        self.scripts.push_back(script);
    }

    fn require_state(
        &self,
        operation: &'static str,
        states: &[FakeRunnerState],
    ) -> Result<(), FakeRunnerError> {
        if states.contains(&self.state) {
            Ok(())
        } else {
            Err(FakeRunnerError::InvalidState {
                operation,
                state: self.state,
            })
        }
    }

    fn check_attempt(expected: AttemptId, actual: AttemptId) -> Result<(), FakeRunnerError> {
        if expected == actual {
            Ok(())
        } else {
            Err(FakeRunnerError::WrongAttempt { expected, actual })
        }
    }
}

impl ExecutionRunner for FakeRunner {
    type Error = FakeRunnerError;

    fn identity(&self) -> &RunnerIdentity {
        self.identity()
    }

    fn materialized_state_key(&self) -> Option<StateKey> {
        self.materialized_state_key()
    }

    fn start(&mut self, initial_state_key: Option<StateKey>) -> Result<(), Self::Error> {
        self.start(initial_state_key)
    }

    fn execute(
        &mut self,
        request: ExecutionRequest,
        resulting_state_key: Option<StateKey>,
    ) -> Result<(), Self::Error> {
        let script = self.scripts.pop_front().unwrap_or_default();
        self.dispatch(request, resulting_state_key, script)
    }

    fn poll(&mut self) -> Option<RunnerEvent> {
        self.poll()
    }

    fn interrupt(&mut self) {
        self.interrupt();
    }

    fn accept_success(&mut self, attempt_id: AttemptId) -> Result<(), Self::Error> {
        self.accept_success(attempt_id)
    }

    fn shutdown(&mut self) -> Result<(), Self::Error> {
        self.shutdown();
        Ok(())
    }
}
