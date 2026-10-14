//! Computation identities and pure decisions about interpreter reconstruction.

use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest, Sha256};

use crate::{CellId, ExecutionKey, SourceRevision, StateKey};

use super::{EvaluationPlan, EvaluationScheduleError, EvaluationSession, PlannedCell};

/// Resolved inputs supplied by dependency discovery, without performing I/O here.
///
/// Digests must encode semantic values, using an unambiguous, versioned format.
/// Missing or unreadable dependencies must prevent keying rather than becoming
/// an empty dependency digest. Neither digest includes rendering options,
/// semantic IDs, source positions, or source revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionInputs {
    /// Interpreter, runtime shim, Tractate and cache schema versions, environment
    /// variables, package locks, platform, and project and working-directory semantics.
    /// This must be constant within a language and stateful session.
    pub environment_digest: [u8; 32],
    /// Effective execution-affecting options, including figure dimensions, and
    /// declared input paths and content digests. Encode resolved semantic values,
    /// never complete YAML mappings or the document source. Exclude `echo`,
    /// `include`, `results`, `fig-cap`, labels, and the `eval` and `cache` Booleans.
    /// Language, executable source, session, and predecessor state are added by
    /// the planner.
    pub computation_digest: [u8; 32],
    /// A globally unique token for an authorized volatile execution, retained
    /// while that attempt remains current. Its stateful suffix cannot reuse the
    /// previous attempt's state. It is not the `cache` Boolean or a bare,
    /// coordinator-local attempt ID. Replay retains the original token.
    pub volatile_nonce: Option<[u8; 32]>,
}

/// Invalid structure or unresolved execution inputs, with no partial keyed plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationKeyError {
    InvalidPlan(EvaluationScheduleError),
    MissingInputs(CellId),
    InconsistentEnvironment {
        cell_id: CellId,
        predecessor: CellId,
    },
}

impl fmt::Display for EvaluationKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(error) => error.fmt(formatter),
            Self::MissingInputs(cell_id) => {
                write!(
                    formatter,
                    "cell {cell_id:?} has no resolved execution inputs"
                )
            }
            Self::InconsistentEnvironment {
                cell_id,
                predecessor,
            } => write!(
                formatter,
                "cell {cell_id:?} and its predecessor {predecessor:?} require different environments"
            ),
        }
    }
}

impl std::error::Error for EvaluationKeyError {}

/// One computation's logical identity, independent of a live interpreter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyedCell<'plan> {
    pub cell: &'plan PlannedCell,
    pub execution_key: ExecutionKey,
    /// The initial environment key is required even for the first stateful cell.
    /// Isolated computations have no predecessor state.
    pub required_predecessor_state_key: Option<StateKey>,
    /// The state a verified successful execution would materialize, never proof
    /// that execution happened. Isolated computations do not advance a session.
    pub resulting_state_key: Option<StateKey>,
    /// Retained authorization token for volatile work, including isolated cells.
    pub volatile_nonce: Option<[u8; 32]>,
}

impl KeyedCell<'_> {
    /// Whether an otherwise eligible, committed successful result matches.
    ///
    /// The caller must first validate cache policy, provenance, and artifacts.
    /// A match says nothing about interpreter state and never authorizes execution.
    #[must_use]
    pub fn has_cached_result(&self, cached_key: Option<ExecutionKey>) -> bool {
        cached_key == Some(self.execution_key)
    }
}

/// Keyed computations at one revision, retaining the source plan's order.
///
/// Keys use SHA-256 with versioned domains and length-prefixed fields. They do
/// not depend on semantic IDs, labels, source locations, or revision numbers.
#[derive(Debug)]
pub struct KeyedEvaluationPlan<'plan> {
    source_revision: SourceRevision,
    cells: Vec<KeyedCell<'plan>>,
    indices: BTreeMap<CellId, usize>,
    predecessors: Vec<Option<usize>>,
}

/// Physical preparation needed before executing a desired computation.
///
/// This is independent of whether its output, or any prefix output, is cached.
/// It describes required work without starting, resetting, or dispatching a runner.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionPreparation<'keyed, 'plan> {
    /// An isolated cell requires its own fresh execution environment.
    Isolated,
    /// The live session already has exactly the required predecessor state.
    Ready,
    /// Start or reset to the initial environment, then replay this prefix in
    /// order. The prefix may be empty, but a fresh initial state is still required.
    Replay {
        initial_state_key: StateKey,
        prefix: Vec<&'keyed KeyedCell<'plan>>,
    },
}

impl EvaluationPlan {
    /// Compute execution and cumulative state keys from already resolved inputs.
    ///
    /// Every enabled cell requires an entry. Extra entries, including disabled
    /// cells, are ignored. Discovery of environments, execution options, and file
    /// dependencies belongs to the caller. This pass performs no I/O, consults
    /// no cache, and assigns no runner or dispatch identities. Render-only edits
    /// preserve keys when the caller retains the resolved execution inputs and
    /// any current volatile attempt token.
    pub fn keyed(
        &self,
        inputs: &BTreeMap<CellId, ExecutionInputs>,
    ) -> Result<KeyedEvaluationPlan<'_>, EvaluationKeyError> {
        let predecessors = self
            .chain_predecessors()
            .map_err(EvaluationKeyError::InvalidPlan)?;
        let mut cells: Vec<KeyedCell<'_>> = Vec::with_capacity(self.cells.len());
        let mut indices = BTreeMap::new();
        for (index, cell) in self.cells.iter().enumerate() {
            let input = inputs
                .get(&cell.cell_id)
                .ok_or(EvaluationKeyError::MissingInputs(cell.cell_id))?;
            let required = if cell.session == EvaluationSession::Isolated {
                None
            } else if let Some(predecessor) = predecessors[index] {
                let previous = &self.cells[predecessor];
                if input.environment_digest != inputs[&previous.cell_id].environment_digest {
                    return Err(EvaluationKeyError::InconsistentEnvironment {
                        cell_id: cell.cell_id,
                        predecessor: previous.cell_id,
                    });
                }
                cells[predecessor].resulting_state_key
            } else {
                Some(StateKey::from_digest(hash(
                    b"tractate/state/initial/v1",
                    &[
                        &input.environment_digest,
                        &language(cell),
                        &session(&cell.session),
                    ],
                )))
            };
            let execution_key = ExecutionKey::from_digest(hash(
                b"tractate/execution/v1",
                &[
                    &input.environment_digest,
                    &input.computation_digest,
                    &language(cell),
                    &session(&cell.session),
                    cell.code.as_bytes(),
                    required.as_ref().map_or(&[], |key| key.digest()),
                ],
            ));
            let resulting_state_key = required.map(|predecessor| {
                StateKey::from_digest(hash(
                    b"tractate/state/step/v1",
                    &[
                        predecessor.digest(),
                        execution_key.digest(),
                        input.volatile_nonce.as_ref().map_or(&[], |nonce| nonce),
                    ],
                ))
            });
            indices.insert(cell.cell_id, index);
            cells.push(KeyedCell {
                cell,
                execution_key,
                required_predecessor_state_key: required,
                resulting_state_key,
                volatile_nonce: input.volatile_nonce,
            });
        }
        Ok(KeyedEvaluationPlan {
            source_revision: self.source_revision,
            cells,
            indices,
            predecessors,
        })
    }
}

impl<'plan> KeyedEvaluationPlan<'plan> {
    #[must_use]
    pub fn source_revision(&self) -> SourceRevision {
        self.source_revision
    }

    #[must_use]
    pub fn cells(&self) -> &[KeyedCell<'plan>] {
        &self.cells
    }

    /// Inspect a keyed cell, or return `None` when it is absent from this plan.
    #[must_use]
    pub fn cell(&self, cell_id: CellId) -> Option<&KeyedCell<'plan>> {
        self.indices.get(&cell_id).map(|&index| &self.cells[index])
    }

    /// Determine physical work using a live runner's verified state, independently
    /// of logical result reuse. `None` means no known materialized state.
    ///
    /// The caller supplies state from the intended session's current runner.
    /// Cached output must never be supplied as evidence of materialization.
    /// A mismatch requires reconstruction from the initial state, even when the
    /// runner contains a later state in the same chain; there is no rollback.
    /// Returns `None` when the cell is absent from this plan.
    #[must_use]
    pub fn preparation(
        &self,
        cell_id: CellId,
        materialized_state: Option<StateKey>,
    ) -> Option<SessionPreparation<'_, 'plan>> {
        let &index = self.indices.get(&cell_id)?;
        let Some(required) = self.cells[index].required_predecessor_state_key else {
            return Some(SessionPreparation::Isolated);
        };
        if materialized_state == Some(required) {
            return Some(SessionPreparation::Ready);
        }
        let mut prefix = Vec::new();
        let mut head = index;
        while let Some(predecessor) = self.predecessors[head] {
            prefix.push(&self.cells[predecessor]);
            head = predecessor;
        }
        prefix.reverse();
        Some(SessionPreparation::Replay {
            initial_state_key: self.cells[head]
                .required_predecessor_state_key
                .expect("stateful chain heads require their initial environment"),
            prefix,
        })
    }
}

fn language(cell: &PlannedCell) -> Vec<u8> {
    let mut encoded = vec![u8::from(cell.language.is_some())];
    if let Some(language) = &cell.language {
        encoded.extend_from_slice(language.as_bytes());
    }
    encoded
}

fn session(session: &EvaluationSession) -> Vec<u8> {
    match session {
        EvaluationSession::Default => vec![0],
        EvaluationSession::Named(name) => {
            let mut encoded = vec![1];
            encoded.extend_from_slice(name.as_bytes());
            encoded
        }
        EvaluationSession::Isolated => vec![2],
    }
}

fn hash(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for field in std::iter::once(&domain).chain(fields) {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    hasher.finalize().into()
}
