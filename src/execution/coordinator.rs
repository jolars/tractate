//! Revision reconciliation and verified progress at the effect boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroUsize;

use crate::{
    CellId, EvaluationSession, ExecutionKey, KeyedEvaluationPlan, PlannedCell, ScheduledCellState,
    SourceRevision,
};

use super::{
    ArtifactId, AttemptId, ExecutionOutcome, ExecutionRequest, ExecutionRunner, ExecutionStatus,
    OutputEvent, ProvisionalArtifact, RunnerEvent, RunnerIdentity, SessionId, StateKey,
};

/// An accepted logical result retained in this coordinator's memory.
///
/// Provenance keeps the original dispatch revision. Reconciliation can retain
/// this result for a later revision with identical computation inputs. Artifact
/// bytes remain provisional; this is not a persistent cache or artifact store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedExecution {
    pub outcome: ExecutionOutcome,
    pub artifacts: BTreeMap<ArtifactId, ProvisionalArtifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    Revision,
    Attempt,
    ExecutionKey,
    PredecessorState,
    Session,
    Request,
    Runner,
    EventStream,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionDisposition {
    Completed(ExecutionStatus),
    Rejected(RejectionReason),
}

/// Physical execution is reported even when replay preserves logical output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptReport {
    pub attempt_id: AttemptId,
    pub cell_id: CellId,
    pub target_cell_id: CellId,
    pub replay: bool,
    pub disposition: CompletionDisposition,
}

#[derive(Debug)]
pub enum CoordinatorError<E> {
    ObsoleteRevision {
        current: SourceRevision,
        received: SourceRevision,
    },
    UnknownCell(CellId),
    IdentityExhausted,
    Runner {
        session_id: SessionId,
        source: E,
    },
    Protocol {
        session_id: SessionId,
        reason: RejectionReason,
    },
}

impl<E: fmt::Display> fmt::Display for CoordinatorError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ObsoleteRevision { current, received } => write!(
                formatter,
                "cannot reconcile revision {received} after revision {current}"
            ),
            Self::UnknownCell(cell_id) => write!(formatter, "cell {cell_id:?} is not desired"),
            Self::IdentityExhausted => formatter.write_str("execution identities exhausted"),
            Self::Runner { session_id, source } => {
                write!(formatter, "runner for session {session_id:?}: {source}")
            }
            Self::Protocol { session_id, reason } => {
                write!(
                    formatter,
                    "invalid protocol for session {session_id:?}: {reason:?}"
                )
            }
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CoordinatorError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runner { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesiredCell {
    cell: PlannedCell,
    execution_key: ExecutionKey,
    required: Option<StateKey>,
    resulting: Option<StateKey>,
    volatile_nonce: Option<[u8; 32]>,
}

impl DesiredCell {
    fn scope(&self) -> SessionScope {
        if self.cell.session == EvaluationSession::Isolated {
            SessionScope::Isolated(self.cell.cell_id)
        } else {
            SessionScope::Stateful(self.cell.language.clone(), self.cell.session.clone())
        }
    }

    fn same_computation(&self, other: &Self) -> bool {
        self.execution_key == other.execution_key
            && self.required == other.required
            && self.resulting == other.resulting
            && self.volatile_nonce == other.volatile_nonce
            && self.scope() == other.scope()
            && self.cell.code == other.cell.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SessionScope {
    Stateful(Option<String>, EvaluationSession),
    Isolated(CellId),
}

#[derive(Debug)]
struct ActiveAttempt {
    request: ExecutionRequest,
    resulting: Option<StateKey>,
    target: CellId,
    replay: bool,
    runner: RunnerIdentity,
    events: Vec<OutputEvent>,
    artifacts: BTreeMap<ArtifactId, ProvisionalArtifact>,
    revoked: bool,
}

impl ActiveAttempt {
    fn report(&self, disposition: CompletionDisposition) -> AttemptReport {
        AttemptReport {
            attempt_id: self.request.attempt_id,
            cell_id: self.request.cell_id,
            target_cell_id: self.target,
            replay: self.replay,
            disposition,
        }
    }
}

#[derive(Debug)]
struct Session<R> {
    runner: R,
    verified_state: Option<StateKey>,
    active: Option<ActiveAttempt>,
    replay_target: Option<CellId>,
    cleanup_pending: bool,
}

/// Effectful, bounded execution for one compiler's revision sequence.
///
/// `reconcile` updates desired work without starting interpreters. Calling
/// `dispatch_ready` explicitly authorizes execution, including prefix replay.
/// Only validated current completions change logical results or verified state.
/// Pending requests use the latest revision; each physical dispatch, including
/// replay and retry, receives a fresh attempt ID. Obsolete attempts retain their
/// concurrency slots until termination or explicit forced cleanup.
///
/// Logical results remain separate from interpreter state. Replay never replaces
/// a retained prefix result. Persistent publication and rendering are separate
/// consumers of accepted results and are not performed by this coordinator.
#[derive(Debug)]
pub struct ExecutionCoordinator<R: ExecutionRunner> {
    max_concurrent: NonZeroUsize,
    revision: Option<SourceRevision>,
    desired: Vec<DesiredCell>,
    states: BTreeMap<CellId, ScheduledCellState>,
    results: BTreeMap<CellId, AcceptedExecution>,
    session_ids: BTreeMap<SessionScope, SessionId>,
    sessions: BTreeMap<SessionId, Session<R>>,
    retired: BTreeSet<AttemptId>,
    next_attempt: u64,
    next_session: u64,
}

impl<R: ExecutionRunner> ExecutionCoordinator<R> {
    pub fn new(max_concurrent: NonZeroUsize) -> Self {
        Self {
            max_concurrent,
            revision: None,
            desired: Vec::new(),
            states: BTreeMap::new(),
            results: BTreeMap::new(),
            session_ids: BTreeMap::new(),
            sessions: BTreeMap::new(),
            retired: BTreeSet::new(),
            next_attempt: 1,
            next_session: 1,
        }
    }

    /// Reconcile desired computation, retaining only matching logical results.
    ///
    /// Repeating the same plan is idempotent. A new revision revokes every older
    /// active attempt, even if its computation key is unchanged. Interruptions
    /// are best effort; their late events remain ineligible for acceptance.
    pub fn reconcile(
        &mut self,
        plan: &KeyedEvaluationPlan<'_>,
    ) -> Result<(), CoordinatorError<R::Error>> {
        let revision = plan.source_revision();
        let desired: Vec<_> = plan
            .cells()
            .iter()
            .map(|cell| DesiredCell {
                cell: cell.cell.clone(),
                execution_key: cell.execution_key,
                required: cell.required_predecessor_state_key,
                resulting: cell.resulting_state_key,
                volatile_nonce: cell.volatile_nonce,
            })
            .collect();
        if let Some(current) = self.revision {
            if revision < current {
                return Err(CoordinatorError::ObsoleteRevision {
                    current,
                    received: revision,
                });
            }
            if revision == current && desired == self.desired {
                return Ok(());
            }
        }
        let scopes: BTreeSet<_> = desired.iter().map(DesiredCell::scope).collect();
        let missing = scopes
            .iter()
            .filter(|scope| !self.session_ids.contains_key(*scope))
            .count();
        self.next_session
            .checked_add(missing as u64)
            .ok_or(CoordinatorError::IdentityExhausted)?;
        for scope in scopes {
            if !self.session_ids.contains_key(&scope) {
                self.session_ids.insert(scope, SessionId(self.next_session));
                self.next_session += 1;
            }
        }
        self.results.retain(|cell_id, _| {
            let old = self
                .desired
                .iter()
                .find(|cell| cell.cell.cell_id == *cell_id);
            let new = desired.iter().find(|cell| cell.cell.cell_id == *cell_id);
            matches!((old, new), (Some(old), Some(new)) if new.same_computation(old))
        });
        for session in self.sessions.values_mut() {
            session.replay_target = None;
            if let Some(active) = &mut session.active {
                if !active.revoked {
                    active.revoked = true;
                    session.runner.interrupt();
                }
                session.verified_state = None;
            }
        }
        self.states = desired
            .iter()
            .map(|cell| {
                let id = cell.cell.cell_id;
                (
                    id,
                    if self.results.contains_key(&id) {
                        ScheduledCellState::Succeeded
                    } else {
                        ScheduledCellState::Pending
                    },
                )
            })
            .collect();
        self.desired = desired;
        self.revision = Some(revision);
        Ok(())
    }

    /// Retry a cell and its stateful suffix at the current revision.
    /// Existing attempts in that suffix are revoked before new attempts can run.
    pub fn retry(&mut self, cell_id: CellId) -> Result<(), CoordinatorError<R::Error>> {
        if self.desired_cell(cell_id).is_none() {
            return Err(CoordinatorError::UnknownCell(cell_id));
        }
        let mut affected = BTreeSet::from([cell_id]);
        for cell in &self.desired {
            if cell
                .cell
                .predecessor
                .is_some_and(|id| affected.contains(&id))
            {
                affected.insert(cell.cell.cell_id);
            }
        }
        for &id in &affected {
            self.results.remove(&id);
            self.states.insert(id, ScheduledCellState::Pending);
        }
        for session in self.sessions.values_mut() {
            if let Some(active) = &mut session.active
                && affected.contains(&active.target)
            {
                active.revoked = true;
                session.verified_state = None;
                session.replay_target = None;
                session.runner.interrupt();
            }
        }
        Ok(())
    }

    /// Dispatch ready targets or the next physical step in their reconstruction.
    /// The factory is called when a stateful session first needs a runner or an
    /// isolated cell needs a fresh execution environment.
    pub fn dispatch_ready(
        &mut self,
        mut make_runner: impl FnMut(SessionId) -> R,
    ) -> Result<Vec<ExecutionRequest>, CoordinatorError<R::Error>> {
        let mut requests = Vec::new();
        for index in 0..self.desired.len() {
            if self.running_count() >= self.max_concurrent.get() {
                break;
            }
            let target = self.desired[index].clone();
            let target_id = target.cell.cell_id;
            if self.state(target_id) != Some(ScheduledCellState::Pending)
                || target
                    .cell
                    .predecessor
                    .is_some_and(|id| self.state(id) != Some(ScheduledCellState::Succeeded))
            {
                continue;
            }
            let scope = target.scope();
            let mut session_id = self.session_ids[&scope];
            if target.required.is_none() && self.sessions.contains_key(&session_id) {
                let next = self
                    .next_session
                    .checked_add(1)
                    .ok_or(CoordinatorError::IdentityExhausted)?;
                session_id = SessionId(self.next_session);
                self.next_session = next;
                self.session_ids.insert(scope, session_id);
            }
            let prefix = self.prefix(target_id);
            let session = self.sessions.entry(session_id).or_insert_with(|| Session {
                runner: make_runner(session_id),
                verified_state: None,
                active: None,
                replay_target: None,
                cleanup_pending: false,
            });
            if session.active.is_some() || session.cleanup_pending {
                continue;
            }
            let next_attempt = self
                .next_attempt
                .checked_add(1)
                .ok_or(CoordinatorError::IdentityExhausted)?;
            let isolated = target.required.is_none();
            let ready = !isolated
                && session.verified_state == target.required
                && session.runner.materialized_state_key() == target.required;
            let continuing_replay = session.replay_target == Some(target_id)
                && session.verified_state.is_some()
                && session.runner.materialized_state_key() == session.verified_state;
            if !ready && !continuing_replay {
                let initial = prefix.first().map_or(target.required, |cell| cell.required);
                session.verified_state = None;
                if let Err(source) = session.runner.shutdown() {
                    session.cleanup_pending = true;
                    self.fail_target(target_id, ExecutionStatus::Failure);
                    return Err(CoordinatorError::Runner { session_id, source });
                }
                let previous_incarnation = session.runner.identity().incarnation;
                if let Err(source) = session.runner.start(initial) {
                    session.cleanup_pending = true;
                    self.fail_target(target_id, ExecutionStatus::Failure);
                    return Err(CoordinatorError::Runner { session_id, source });
                }
                if session.runner.identity().incarnation == previous_incarnation {
                    return self.invalid_start(session_id, target_id, RejectionReason::Runner);
                }
                if session.runner.materialized_state_key() != initial {
                    return self.invalid_start(
                        session_id,
                        target_id,
                        RejectionReason::PredecessorState,
                    );
                }
                session.verified_state = initial;
                session.replay_target = (!isolated).then_some(target_id);
            }
            let physical = if session.verified_state == target.required {
                target.clone()
            } else {
                prefix
                    .into_iter()
                    .find(|cell| cell.required == session.verified_state)
                    .expect("reconstruction materializes only keys in the desired prefix")
            };
            let replay = physical.cell.cell_id != target_id;
            let request = ExecutionRequest {
                source_revision: self.revision.expect("desired cells have a revision"),
                attempt_id: AttemptId(self.next_attempt),
                cell_id: physical.cell.cell_id,
                execution_key: physical.execution_key,
                session_id,
                required_predecessor_state_key: physical.required,
                language: physical.cell.language.clone().unwrap_or_default(),
                code: physical.cell.code.clone(),
            };
            self.next_attempt = next_attempt;
            session.verified_state = None;
            if let Err(source) = session.runner.execute(request.clone(), physical.resulting) {
                self.retired.insert(request.attempt_id);
                session.replay_target = None;
                session.cleanup_pending = true;
                self.fail_target(target_id, ExecutionStatus::Failure);
                return Err(CoordinatorError::Runner { session_id, source });
            }
            session.active = Some(ActiveAttempt {
                request: request.clone(),
                resulting: physical.resulting,
                target: target_id,
                replay,
                runner: session.runner.identity().clone(),
                events: Vec::new(),
                artifacts: BTreeMap::new(),
                revoked: false,
            });
            self.states.insert(target_id, ScheduledCellState::Running);
            requests.push(request);
        }
        Ok(requests)
    }

    fn invalid_start(
        &mut self,
        session_id: SessionId,
        target: CellId,
        reason: RejectionReason,
    ) -> Result<Vec<ExecutionRequest>, CoordinatorError<R::Error>> {
        let session = self.sessions.get_mut(&session_id).unwrap();
        session.replay_target = None;
        let cleanup = session.runner.shutdown();
        session.cleanup_pending = cleanup.is_err();
        self.fail_target(target, ExecutionStatus::Failure);
        cleanup.map_err(|source| CoordinatorError::Runner { session_id, source })?;
        Err(CoordinatorError::Protocol { session_id, reason })
    }

    /// Poll each active session once, allowing unrelated work to make progress.
    /// Output and artifact events remain provisional and are not exposed as results.
    pub fn poll(&mut self) -> Result<Vec<AttemptReport>, CoordinatorError<R::Error>> {
        let ids: Vec<_> = self.sessions.keys().copied().collect();
        let mut reports = Vec::new();
        for session_id in ids {
            if let Some(event) = self.sessions.get_mut(&session_id).unwrap().runner.poll()
                && let Some(report) = self.receive(session_id, event)?
            {
                reports.push(report);
            }
        }
        Ok(reports)
    }

    /// Validate an event delivered by the indicated adapter's control channel.
    /// This also supports adapters that deliver events asynchronously.
    pub fn receive(
        &mut self,
        session_id: SessionId,
        event: RunnerEvent,
    ) -> Result<Option<AttemptReport>, CoordinatorError<R::Error>> {
        let (attempt_id, cell_id) = match &event {
            RunnerEvent::Output(event) => (event.attempt_id, event.cell_id),
            RunnerEvent::Artifact { request, .. } => (request.attempt_id, request.cell_id),
            RunnerEvent::Complete(outcome) => (
                outcome.provenance.request.attempt_id,
                outcome.provenance.request.cell_id,
            ),
        };
        let ignored = |reason| {
            Some(AttemptReport {
                attempt_id,
                cell_id,
                target_cell_id: cell_id,
                replay: false,
                disposition: CompletionDisposition::Rejected(reason),
            })
        };
        let Some(active) = self
            .sessions
            .get(&session_id)
            .and_then(|session| session.active.as_ref())
        else {
            return Ok(ignored(RejectionReason::Attempt));
        };
        if active.request.attempt_id != attempt_id {
            if self.retired.contains(&attempt_id) {
                return Ok(ignored(RejectionReason::Attempt));
            }
            return self.reject(session_id, RejectionReason::Attempt).map(Some);
        }
        if active.revoked {
            return if matches!(event, RunnerEvent::Complete(_)) {
                self.reject(session_id, RejectionReason::Revoked).map(Some)
            } else {
                Ok(ignored(RejectionReason::Revoked))
            };
        }
        let reason = match &event {
            RunnerEvent::Output(event) => Self::validate_output(active, event),
            RunnerEvent::Artifact { request, artifact } => {
                Self::validate_request(&active.request, request).or_else(|| {
                    active
                        .artifacts
                        .get(&artifact.reference.id)
                        .filter(|previous| *previous != artifact)
                        .map(|_| RejectionReason::EventStream)
                })
            }
            RunnerEvent::Complete(outcome) => {
                Self::validate_request(&active.request, &outcome.provenance.request)
                    .or_else(|| {
                        (outcome.provenance.effective_execution_key != active.request.execution_key)
                            .then_some(RejectionReason::ExecutionKey)
                    })
                    .or_else(|| {
                        (outcome.provenance.runner != active.runner
                            || self.sessions[&session_id].runner.identity() != &active.runner)
                            .then_some(RejectionReason::Runner)
                    })
                    .or_else(|| {
                        (outcome.events != active.events).then_some(RejectionReason::EventStream)
                    })
            }
        };
        if let Some(reason) = reason {
            return self.reject(session_id, reason).map(Some);
        }
        let session = self.sessions.get_mut(&session_id).unwrap();
        match event {
            RunnerEvent::Output(event) => {
                session.active.as_mut().unwrap().events.push(event);
                Ok(None)
            }
            RunnerEvent::Artifact { artifact, .. } => {
                session
                    .active
                    .as_mut()
                    .unwrap()
                    .artifacts
                    .insert(artifact.reference.id.clone(), artifact);
                Ok(None)
            }
            RunnerEvent::Complete(outcome) => self.complete(session_id, outcome).map(Some),
        }
    }

    fn validate_request(
        expected: &ExecutionRequest,
        actual: &ExecutionRequest,
    ) -> Option<RejectionReason> {
        if actual.source_revision != expected.source_revision {
            Some(RejectionReason::Revision)
        } else if actual.attempt_id != expected.attempt_id {
            Some(RejectionReason::Attempt)
        } else if actual.execution_key != expected.execution_key {
            Some(RejectionReason::ExecutionKey)
        } else if actual.required_predecessor_state_key != expected.required_predecessor_state_key {
            Some(RejectionReason::PredecessorState)
        } else if actual.session_id != expected.session_id {
            Some(RejectionReason::Session)
        } else if actual != expected {
            Some(RejectionReason::Request)
        } else {
            None
        }
    }

    fn validate_output(active: &ActiveAttempt, event: &OutputEvent) -> Option<RejectionReason> {
        let request = &active.request;
        if event.source_revision != request.source_revision {
            Some(RejectionReason::Revision)
        } else if event.session_id != request.session_id {
            Some(RejectionReason::Session)
        } else if event.cell_id != request.cell_id || event.sequence != active.events.len() as u64 {
            Some(RejectionReason::EventStream)
        } else {
            None
        }
    }

    fn complete(
        &mut self,
        session_id: SessionId,
        outcome: ExecutionOutcome,
    ) -> Result<AttemptReport, CoordinatorError<R::Error>> {
        let session = self.sessions.get_mut(&session_id).unwrap();
        if outcome.status == ExecutionStatus::Success {
            let active = session.active.as_ref().unwrap();
            if let Err(source) = session.runner.accept_success(active.request.attempt_id) {
                self.reject(session_id, RejectionReason::Runner)?;
                return Err(CoordinatorError::Runner { session_id, source });
            }
            if session.runner.materialized_state_key() != active.resulting {
                return self.reject(session_id, RejectionReason::PredecessorState);
            }
        }
        let active = session.active.take().unwrap();
        self.retired.insert(active.request.attempt_id);
        let report = active.report(CompletionDisposition::Completed(outcome.status));
        if outcome.status == ExecutionStatus::Success {
            if active.resulting.is_none() {
                let cleanup = session.runner.shutdown();
                session.cleanup_pending = cleanup.is_err();
                if let Err(source) = cleanup {
                    self.fail_target(active.target, ExecutionStatus::Failure);
                    return Err(CoordinatorError::Runner { session_id, source });
                }
            }
            session.verified_state = active.resulting;
            if active.replay {
                self.states
                    .insert(active.target, ScheduledCellState::Pending);
            } else {
                session.replay_target = None;
                self.states
                    .insert(active.target, ScheduledCellState::Succeeded);
                self.results.insert(
                    active.target,
                    AcceptedExecution {
                        outcome,
                        artifacts: active.artifacts,
                    },
                );
            }
        } else {
            session.verified_state = None;
            session.replay_target = None;
            let cleanup = session.runner.shutdown();
            session.cleanup_pending = cleanup.is_err();
            self.fail_target(active.target, outcome.status);
            cleanup.map_err(|source| CoordinatorError::Runner { session_id, source })?;
        }
        Ok(report)
    }

    fn reject(
        &mut self,
        session_id: SessionId,
        reason: RejectionReason,
    ) -> Result<AttemptReport, CoordinatorError<R::Error>> {
        let session = self.sessions.get_mut(&session_id).unwrap();
        let active = session.active.take().unwrap();
        self.retired.insert(active.request.attempt_id);
        session.verified_state = None;
        session.replay_target = None;
        let cleanup = session.runner.shutdown();
        session.cleanup_pending = cleanup.is_err();
        if !active.revoked {
            self.fail_target(active.target, ExecutionStatus::Failure);
        }
        cleanup.map_err(|source| CoordinatorError::Runner { session_id, source })?;
        Ok(active.report(CompletionDisposition::Rejected(reason)))
    }

    /// Force cleanup when obsolete work ignores graceful interruption or a prior
    /// adapter operation failed to confirm cleanup.
    /// Slots are released only after the adapter confirms cleanup.
    pub fn terminate_obsolete(&mut self) -> Result<(), CoordinatorError<R::Error>> {
        for (&session_id, session) in &mut self.sessions {
            if session.cleanup_pending
                || session.active.as_ref().is_some_and(|active| active.revoked)
            {
                session
                    .runner
                    .shutdown()
                    .map_err(|source| CoordinatorError::Runner { session_id, source })?;
                if let Some(active) = session.active.take() {
                    self.retired.insert(active.request.attempt_id);
                }
                session.verified_state = None;
                session.replay_target = None;
                session.cleanup_pending = false;
            }
        }
        Ok(())
    }

    fn fail_target(&mut self, target: CellId, status: ExecutionStatus) {
        self.states.insert(
            target,
            if status == ExecutionStatus::Cancelled {
                ScheduledCellState::Cancelled
            } else {
                ScheduledCellState::Failed
            },
        );
        let mut blocked = BTreeSet::from([target]);
        for cell in &self.desired {
            if cell
                .cell
                .predecessor
                .is_some_and(|id| blocked.contains(&id))
            {
                blocked.insert(cell.cell.cell_id);
                self.states
                    .insert(cell.cell.cell_id, ScheduledCellState::Blocked);
            }
        }
    }

    fn desired_cell(&self, cell_id: CellId) -> Option<&DesiredCell> {
        self.desired
            .iter()
            .find(|cell| cell.cell.cell_id == cell_id)
    }

    fn prefix(&self, cell_id: CellId) -> Vec<DesiredCell> {
        let mut prefix = Vec::new();
        let mut cell = self.desired_cell(cell_id).unwrap();
        while let Some(predecessor) = cell.cell.predecessor {
            cell = self.desired_cell(predecessor).unwrap();
            prefix.push(cell.clone());
        }
        prefix.reverse();
        prefix
    }

    pub fn state(&self, cell_id: CellId) -> Option<ScheduledCellState> {
        self.states.get(&cell_id).copied()
    }

    pub fn result(&self, cell_id: CellId) -> Option<&AcceptedExecution> {
        self.results.get(&cell_id)
    }

    pub fn session_id(&self, cell_id: CellId) -> Option<SessionId> {
        self.desired_cell(cell_id)
            .map(|cell| self.session_ids[&cell.scope()])
    }

    pub fn materialized_state_key(&self, session_id: SessionId) -> Option<StateKey> {
        self.sessions.get(&session_id).and_then(|session| {
            session
                .verified_state
                .filter(|&state| session.runner.materialized_state_key() == Some(state))
        })
    }

    pub fn runner(&self, session_id: SessionId) -> Option<&R> {
        self.sessions
            .get(&session_id)
            .map(|session| &session.runner)
    }

    /// Access adapter controls, such as fake-runner scripts and gates.
    /// Lifecycle and execution commands must remain coordinator-owned.
    pub fn runner_mut(&mut self, session_id: SessionId) -> Option<&mut R> {
        self.sessions
            .get_mut(&session_id)
            .map(|session| &mut session.runner)
    }

    pub fn running_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.active.is_some() || session.cleanup_pending)
            .count()
    }

    pub fn is_finished(&self) -> bool {
        self.running_count() == 0
            && self.states.values().all(|state| {
                !matches!(
                    state,
                    ScheduledCellState::Pending | ScheduledCellState::Running
                )
            })
    }
}
