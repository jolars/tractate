//! Read-only cell results and progress for presentation backends.

use crate::{CellId, Diagnostic, ExecutionKey, ScheduledCellState, SourceRevision};

use super::{AcceptedExecution, StateKey};

/// The visible status of a cell's output at the desired source revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellExecutionState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// Execution is disabled and no current result is available in memory.
    Unavailable,
    /// Prior successful output is available only as an explicitly stale fallback.
    StaleLastKnownGood,
}

/// A renderer's view of one desired cell, without executing or reading the cache.
///
/// `state` describes the visible output. `progress` retains the current schedule
/// state even when output is stale or execution is disabled. A blocked suffix is
/// visibly failed, with a diagnostic identifying its immediate predecessor.
/// Only `Succeeded` output is valid for a complete one-shot build. Preview may
/// display `StaleLastKnownGood` output with its status and current diagnostics.
///
/// Result provenance retains the original accepted request; it can belong to an
/// earlier revision even for valid output. The desired revision and keys below
/// identify the current computation. Semantic identity alone never makes stale
/// output reusable, and this view never exposes provisional or failed output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellExecutionView<'a> {
    pub source_revision: SourceRevision,
    pub cell_id: CellId,
    pub execution_key: ExecutionKey,
    pub required_predecessor_state_key: Option<StateKey>,
    pub state: CellExecutionState,
    pub progress: ScheduledCellState,
    pub result: Option<&'a AcceptedExecution>,
    /// Diagnostics for the current computation, never a stale result's warnings.
    pub diagnostics: &'a [Diagnostic],
}
