//! Interpreter adapters used by the revision-aware execution coordinator.

use serde::{Deserialize, Serialize};

use super::{
    ArtifactReference, AttemptId, ExecutionOutcome, ExecutionRequest, OutputEvent, RunnerIdentity,
    StateKey,
};

/// Provisional bytes owned by an attempt, before artifact-store publication.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProvisionalArtifact {
    pub reference: ArtifactReference,
    pub bytes: Vec<u8>,
}

/// Effects delivered through an adapter's control channel.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum RunnerEvent {
    Output(OutputEvent),
    Artifact {
        request: ExecutionRequest,
        artifact: ProvisionalArtifact,
    },
    Complete(ExecutionOutcome),
}

/// A single interpreter process, controlled outside the pure compiler.
///
/// Polling is nonblocking. Success remains provisional, with no materialized
/// state, until `accept_success` verifies it. Interruption revokes active work,
/// but may still deliver late events. Shutdown must quiesce and clean up the
/// process before returning success; a subsequent start uses a fresh incarnation.
/// Failure must clear any claim of materialized state. Commands that fail must
/// never leave the interpreter eligible for unverified execution.
pub trait ExecutionRunner {
    type Error: std::error::Error;

    fn identity(&self) -> &RunnerIdentity;
    fn materialized_state_key(&self) -> Option<StateKey>;
    fn start(&mut self, initial_state_key: Option<StateKey>) -> Result<(), Self::Error>;
    fn execute(
        &mut self,
        request: ExecutionRequest,
        resulting_state_key: Option<StateKey>,
    ) -> Result<(), Self::Error>;
    fn poll(&mut self) -> Option<RunnerEvent>;
    fn interrupt(&mut self);
    fn accept_success(&mut self, attempt_id: AttemptId) -> Result<(), Self::Error>;
    fn shutdown(&mut self) -> Result<(), Self::Error>;
}
