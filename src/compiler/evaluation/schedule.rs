//! Bounded, in-memory progress through independent computation chains.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroUsize;

use crate::{CellId, ExecutionStatus};

use super::{EvaluationPlan, EvaluationSession, PlannedCell};

/// Structural progress of a cell within one evaluation schedule.
///
/// These states do not describe cached results or materialized interpreter state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledCellState {
    Pending,
    /// A concurrency slot is reserved, including any runner startup time.
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// A predecessor failed or was cancelled, so this cell cannot run.
    Blocked,
}

/// An invalid plan or progress update, rejected without changing the schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationScheduleError {
    DuplicateCell(CellId),
    InvalidPredecessor {
        cell_id: CellId,
        expected: Option<CellId>,
        actual: Option<CellId>,
    },
    UnknownCell(CellId),
    CellNotRunning {
        cell_id: CellId,
        state: ScheduledCellState,
    },
}

impl fmt::Display for EvaluationScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCell(cell_id) => {
                write!(formatter, "evaluation plan repeats cell {cell_id:?}")
            }
            Self::InvalidPredecessor {
                cell_id,
                expected,
                actual,
            } => write!(
                formatter,
                "cell {cell_id:?} requires predecessor {expected:?}, but the plan specifies {actual:?}"
            ),
            Self::UnknownCell(cell_id) => {
                write!(
                    formatter,
                    "cell {cell_id:?} is absent from the evaluation plan"
                )
            }
            Self::CellNotRunning { cell_id, state } => {
                write!(formatter, "cell {cell_id:?} is {state:?}, not running")
            }
        }
    }
}

impl std::error::Error for EvaluationScheduleError {}

/// A pure reservation queue for one immutable plan, with a global concurrency cap.
///
/// Isolated cells and unrelated chains by language and session are ready
/// immediately. A successful completion releases only its own successor, without
/// waiting for other running cells. Ready cells are reserved in document order.
/// Failure or cancellation blocks only the affected stateful suffix.
///
/// This queue performs no I/O, starts no runners, and does not authorize execution
/// or publish results. An effectful coordinator must validate an outcome's
/// revision, attempt, execution key, and predecessor state before reporting its
/// completion here. Revision reconciliation and retries belong to that coordinator.
#[derive(Debug)]
pub struct EvaluationSchedule<'plan> {
    plan: &'plan EvaluationPlan,
    max_concurrent: NonZeroUsize,
    indices: BTreeMap<CellId, usize>,
    states: Vec<ScheduledCellState>,
    successors: Vec<Option<usize>>,
    ready: BTreeSet<usize>,
    running: usize,
}

impl EvaluationPlan {
    pub(super) fn chain_predecessors(&self) -> Result<Vec<Option<usize>>, EvaluationScheduleError> {
        let mut indices = BTreeMap::new();
        let mut predecessors = BTreeMap::new();
        let mut chain = Vec::with_capacity(self.cells.len());
        for (index, cell) in self.cells.iter().enumerate() {
            if indices.insert(cell.cell_id, index).is_some() {
                return Err(EvaluationScheduleError::DuplicateCell(cell.cell_id));
            }
            let expected = if cell.session == EvaluationSession::Isolated {
                None
            } else {
                predecessors.insert((cell.language.as_deref(), &cell.session), cell.cell_id)
            };
            if cell.predecessor != expected {
                return Err(EvaluationScheduleError::InvalidPredecessor {
                    cell_id: cell.cell_id,
                    expected,
                    actual: cell.predecessor,
                });
            }
            chain.push(expected.map(|predecessor| indices[&predecessor]));
        }
        Ok(chain)
    }

    /// Prepare independent progress through this plan without executing code.
    ///
    /// The positive limit counts all reserved cells across all languages,
    /// sessions, and isolated cells. A limit of one serializes the plan. Plans
    /// constructed or edited by callers must preserve unique cell IDs and the
    /// immediate predecessor in each stateful chain, just like compiler plans.
    pub fn schedule(
        &self,
        max_concurrent: NonZeroUsize,
    ) -> Result<EvaluationSchedule<'_>, EvaluationScheduleError> {
        let mut indices = BTreeMap::new();
        let mut successors = vec![None; self.cells.len()];
        let mut ready = BTreeSet::new();
        for (index, predecessor) in self.chain_predecessors()?.into_iter().enumerate() {
            indices.insert(self.cells[index].cell_id, index);
            if let Some(predecessor) = predecessor {
                successors[predecessor] = Some(index);
            } else {
                ready.insert(index);
            }
        }
        Ok(EvaluationSchedule {
            plan: self,
            max_concurrent,
            indices,
            states: vec![ScheduledCellState::Pending; self.cells.len()],
            successors,
            ready,
            running: 0,
        })
    }
}

impl<'plan> EvaluationSchedule<'plan> {
    /// Reserve the earliest ready cell if a concurrency slot is available.
    ///
    /// Reservation moves the cell to `Running`. `None` can mean the limit is
    /// reached or that pending cells await their predecessors; use
    /// [`Self::is_finished`] to distinguish terminal schedules from those waits.
    pub fn reserve_next(&mut self) -> Option<&'plan PlannedCell> {
        if self.running >= self.max_concurrent.get() {
            return None;
        }
        let index = self.ready.pop_first()?;
        self.states[index] = ScheduledCellState::Running;
        self.running += 1;
        Some(&self.plan.cells[index])
    }

    /// Release a reserved cell's slot after the coordinator accepts its outcome.
    ///
    /// Success releases its successor. Failure and cancellation block the entire
    /// remaining suffix in that session. Other chains and isolated cells retain
    /// their progress. Unknown cells and cells without a reservation are rejected.
    pub fn complete(
        &mut self,
        cell_id: CellId,
        status: ExecutionStatus,
    ) -> Result<(), EvaluationScheduleError> {
        let Some(&index) = self.indices.get(&cell_id) else {
            return Err(EvaluationScheduleError::UnknownCell(cell_id));
        };
        if self.states[index] != ScheduledCellState::Running {
            return Err(EvaluationScheduleError::CellNotRunning {
                cell_id,
                state: self.states[index],
            });
        }
        self.running -= 1;
        self.states[index] = match status {
            ExecutionStatus::Success => ScheduledCellState::Succeeded,
            ExecutionStatus::Failure => ScheduledCellState::Failed,
            ExecutionStatus::Cancelled => ScheduledCellState::Cancelled,
        };
        if status == ExecutionStatus::Success {
            if let Some(successor) = self.successors[index] {
                self.ready.insert(successor);
            }
        } else {
            let mut successor = self.successors[index];
            while let Some(index) = successor {
                self.states[index] = ScheduledCellState::Blocked;
                successor = self.successors[index];
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn max_concurrent(&self) -> NonZeroUsize {
        self.max_concurrent
    }

    /// Count occupied slots, including cells reserved before runner dispatch.
    #[must_use]
    pub fn running_count(&self) -> usize {
        self.running
    }

    /// Inspect progress, or return `None` for a cell absent from this plan.
    #[must_use]
    pub fn state(&self, cell_id: CellId) -> Option<ScheduledCellState> {
        self.indices.get(&cell_id).map(|&index| self.states[index])
    }

    /// Whether every cell succeeded, failed, was cancelled, or became blocked.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.running == 0 && self.ready.is_empty()
    }
}
