//! Backend-independent contracts at the boundary between planning and effects.

use std::time::Duration;

use crate::{CellId, Diagnostic, ExecutionKey, SourceRevision};

/// A single dispatch, unique within its owning execution coordinator.
///
/// Retries receive new IDs, even when the cell and execution key are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptId(pub u64);

/// A logical interpreter session within one execution coordinator.
///
/// Named sessions retain their ID across restarts. Isolated executions receive
/// distinct session IDs. IDs from different coordinators are unrelated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u64);

/// A digest of the cumulative interpreter state required by a stateful cell.
///
/// The planner computes this from the environment and preceding computations.
/// Cached output alone does not prove that a runner has materialized this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateKey([u8; 32]);

impl StateKey {
    pub fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Desired computation and the identities needed to reject obsolete work.
///
/// Constructing a request does not authorize execution. The effectful scheduler
/// must check that execution is authorized and that the request is still desired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRequest {
    pub source_revision: SourceRevision,
    pub attempt_id: AttemptId,
    pub cell_id: CellId,
    pub execution_key: ExecutionKey,
    pub session_id: SessionId,
    /// Stateful requests include the initial environment key for the first cell.
    /// Isolated requests have no predecessor and use `None`.
    pub required_predecessor_state_key: Option<StateKey>,
    pub language: String,
    /// Executable source, excluding the cell's option declarations.
    pub code: String,
}

/// Runner implementation, version, and process incarnation that performed work.
///
/// A restarted process must receive a new incarnation, even in the same session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerIdentity {
    pub name: String,
    pub version: String,
    pub incarnation: u64,
}

/// Attribution shared by every terminal outcome, including cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProvenance {
    /// Retain the dispatched request so later publication can validate its scope.
    pub request: ExecutionRequest,
    /// The key actually used by the runner, which must be checked before reuse.
    pub effective_execution_key: ExecutionKey,
    pub runner: RunnerIdentity,
    /// Elapsed execution time, independent of wall-clock timestamps.
    pub duration: Duration,
}

/// Terminal disposition of an attempt, independent of diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Success,
    Failure,
    Cancelled,
}

/// Terminal report from the effect boundary, independent of any renderer.
///
/// Failure and cancellation must never be published as successful cache entries.
/// Success is still provisional: publication must validate the request's revision,
/// attempt, session, predecessor state, and effective execution key against the
/// currently desired computation. Output events are a separate protocol contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub status: ExecutionStatus,
    pub diagnostics: Vec<Diagnostic>,
    pub provenance: ExecutionProvenance,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::SemanticIds;
    use crate::{Compiler, SourceFile};

    #[test]
    fn execution_outcomes_preserve_dispatch_and_effective_identity() {
        let mut compiler = Compiler::new(SourceFile::anonymous("## Slide\n"));
        let revision = compiler.revision();
        let request = ExecutionRequest {
            source_revision: revision,
            attempt_id: AttemptId(1),
            cell_id: SemanticIds::default().cell(),
            execution_key: ExecutionKey::from_digest([1; 32]),
            session_id: SessionId(2),
            required_predecessor_state_key: Some(StateKey::from_digest([3; 32])),
            language: "r".into(),
            code: "1 + 1".into(),
        };
        compiler.update_source(SourceFile::anonymous("## Edited\n"));
        for status in [
            ExecutionStatus::Success,
            ExecutionStatus::Failure,
            ExecutionStatus::Cancelled,
        ] {
            let outcome = ExecutionOutcome {
                status,
                diagnostics: compiler.snapshot().inspection().diagnostics.clone(),
                provenance: ExecutionProvenance {
                    request: request.clone(),
                    effective_execution_key: ExecutionKey::from_digest([4; 32]),
                    runner: RunnerIdentity {
                        name: "fake".into(),
                        version: "1".into(),
                        incarnation: 5,
                    },
                    duration: Duration::from_millis(10),
                },
            };
            assert_eq!(outcome.status, status);
            assert_eq!(outcome.provenance.request, request);
            assert_ne!(
                outcome.provenance.request.source_revision,
                compiler.revision()
            );
            assert_ne!(
                outcome.provenance.effective_execution_key,
                outcome.provenance.request.execution_key
            );
        }
    }

    #[test]
    fn execution_identity_domains_remain_distinct() {
        use std::any::TypeId;
        use std::collections::HashSet;

        let domains = [
            TypeId::of::<CellId>(),
            TypeId::of::<AttemptId>(),
            TypeId::of::<SessionId>(),
            TypeId::of::<SourceRevision>(),
            TypeId::of::<ExecutionKey>(),
            TypeId::of::<StateKey>(),
        ];
        assert_eq!(domains.into_iter().collect::<HashSet<_>>().len(), 6);
        assert_eq!(StateKey::from_digest([3; 32]).digest(), &[3; 32]);
    }
}
