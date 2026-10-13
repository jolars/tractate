use std::num::NonZeroUsize;
use std::sync::mpsc;
use std::time::Duration;

use tractate::{
    CellId, Compiler, CompilerSnapshot, EvaluationPlan, EvaluationScheduleError, ExecutionStatus,
    ScheduledCellState, SourceFile,
};

fn snapshot(cells: &[(&str, &str, &str, bool)]) -> CompilerSnapshot {
    let source = cells
        .iter()
        .map(|(label, language, session, eval)| {
            format!(
                "```{{{language}}}\n#| label: {label}\n#| session: {session}\n#| eval: {eval}\n{label}()\n```\n\n"
            )
        })
        .collect::<String>();
    Compiler::new(SourceFile::new("slides.qmd", source)).snapshot()
}

fn mixed_snapshot() -> CompilerSnapshot {
    snapshot(&[
        ("a1", "r", "default", true),
        ("disabled", "r", "default", false),
        ("a2", "r", "default", true),
        ("i1", "r", "isolated", true),
        ("b1", "r", "analysis", true),
        ("p1", "python", "default", true),
        ("a3", "r", "default", true),
        ("i2", "r", "isolated", true),
        ("b2", "r", "analysis", true),
        ("p2", "python", "default", true),
    ])
}

fn id(plan: &EvaluationPlan, label: &str) -> CellId {
    plan.cells
        .iter()
        .find(|cell| cell.code == format!("{label}()\n"))
        .unwrap()
        .cell_id
}

#[test]
fn evaluation_schedule_reserves_independent_heads_within_a_global_limit() {
    let snapshot = mixed_snapshot();
    let plan = snapshot.evaluation_plan().unwrap();
    let limit = NonZeroUsize::new(4).unwrap();
    let mut schedule = plan.schedule(limit).unwrap();
    assert_eq!(schedule.max_concurrent(), limit);
    for label in ["a1", "i1", "b1", "p1"] {
        assert_eq!(schedule.reserve_next().unwrap().cell_id, id(plan, label));
    }
    assert_eq!(schedule.running_count(), 4);
    assert!(schedule.reserve_next().is_none());
    assert!(!schedule.is_finished());
    for label in ["a2", "a3", "i2", "b2", "p2"] {
        assert_eq!(
            schedule.state(id(plan, label)),
            Some(ScheduledCellState::Pending)
        );
    }

    schedule
        .complete(id(plan, "b1"), ExecutionStatus::Success)
        .unwrap();
    assert_eq!(schedule.reserve_next().unwrap().cell_id, id(plan, "i2"));
    assert!(schedule.reserve_next().is_none());
    schedule
        .complete(id(plan, "i1"), ExecutionStatus::Success)
        .unwrap();
    assert_eq!(schedule.reserve_next().unwrap().cell_id, id(plan, "b2"));
    assert_eq!(
        schedule.state(id(plan, "a1")),
        Some(ScheduledCellState::Running)
    );
    assert_eq!(
        schedule.state(id(plan, "a2")),
        Some(ScheduledCellState::Pending)
    );
}

#[test]
fn evaluation_schedule_handles_out_of_order_completions_and_configured_limits() {
    let snapshot = mixed_snapshot();
    let plan = snapshot.evaluation_plan().unwrap();
    for limit in [1, 2, 4, plan.cells.len() + 2] {
        for complete_last in [false, true] {
            let mut schedule = plan.schedule(NonZeroUsize::new(limit).unwrap()).unwrap();
            let mut running = Vec::new();
            let mut completed = Vec::new();
            while !schedule.is_finished() {
                while let Some(cell) = schedule.reserve_next() {
                    assert!(!completed.contains(&cell.cell_id));
                    assert!(!running.contains(&cell.cell_id));
                    if let Some(predecessor) = cell.predecessor {
                        assert!(completed.contains(&predecessor));
                    }
                    running.push(cell.cell_id);
                    assert!(running.len() <= limit);
                    assert_eq!(schedule.running_count(), running.len());
                }
                assert!(!running.is_empty());
                let index = if complete_last { running.len() - 1 } else { 0 };
                let cell_id = running.remove(index);
                schedule
                    .complete(cell_id, ExecutionStatus::Success)
                    .unwrap();
                completed.push(cell_id);
            }
            assert_eq!(completed.len(), plan.cells.len());
            for cell in &plan.cells {
                assert_eq!(
                    schedule.state(cell.cell_id),
                    Some(ScheduledCellState::Succeeded)
                );
            }
            assert_eq!(schedule.running_count(), 0);
            assert!(schedule.reserve_next().is_none());
            if limit == 1 {
                assert_eq!(
                    completed,
                    plan.cells
                        .iter()
                        .map(|cell| cell.cell_id)
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn evaluation_schedule_failure_and_cancellation_block_only_the_session_suffix() {
    let snapshot = mixed_snapshot();
    let plan = snapshot.evaluation_plan().unwrap();
    for (outcome, state) in [
        (ExecutionStatus::Failure, ScheduledCellState::Failed),
        (ExecutionStatus::Cancelled, ScheduledCellState::Cancelled),
    ] {
        let mut schedule = plan.schedule(NonZeroUsize::new(2).unwrap()).unwrap();
        assert_eq!(schedule.reserve_next().unwrap().cell_id, id(plan, "a1"));
        assert_eq!(schedule.reserve_next().unwrap().cell_id, id(plan, "i1"));
        schedule.complete(id(plan, "a1"), outcome).unwrap();
        assert_eq!(schedule.state(id(plan, "a1")), Some(state));
        for label in ["a2", "a3"] {
            assert_eq!(
                schedule.state(id(plan, label)),
                Some(ScheduledCellState::Blocked)
            );
        }
        let mut running = vec![id(plan, "i1")];
        while !schedule.is_finished() {
            while let Some(cell) = schedule.reserve_next() {
                assert!(![id(plan, "a2"), id(plan, "a3")].contains(&cell.cell_id));
                running.push(cell.cell_id);
            }
            schedule
                .complete(running.pop().unwrap(), ExecutionStatus::Success)
                .unwrap();
        }
        for label in ["i1", "i2", "b1", "b2", "p1", "p2"] {
            assert_eq!(
                schedule.state(id(plan, label)),
                Some(ScheduledCellState::Succeeded)
            );
        }
    }
}

#[test]
fn evaluation_schedule_isolated_failures_leave_other_isolated_cells_ready() {
    let snapshot = snapshot(&[
        ("first", "r", "isolated", true),
        ("second", "r", "isolated", true),
        ("third", "r", "isolated", true),
    ]);
    let plan = snapshot.evaluation_plan().unwrap();
    let mut schedule = plan.schedule(NonZeroUsize::new(2).unwrap()).unwrap();
    for label in ["first", "second"] {
        assert_eq!(schedule.reserve_next().unwrap().cell_id, id(plan, label));
    }
    schedule
        .complete(id(plan, "first"), ExecutionStatus::Failure)
        .unwrap();
    assert_eq!(schedule.reserve_next().unwrap().cell_id, id(plan, "third"));
    schedule
        .complete(id(plan, "second"), ExecutionStatus::Cancelled)
        .unwrap();
    assert!(!schedule.is_finished());
    schedule
        .complete(id(plan, "third"), ExecutionStatus::Success)
        .unwrap();
    assert!(schedule.is_finished());
    assert_eq!(
        schedule.state(id(plan, "first")),
        Some(ScheduledCellState::Failed)
    );
    assert_eq!(
        schedule.state(id(plan, "second")),
        Some(ScheduledCellState::Cancelled)
    );
    assert_eq!(
        schedule.state(id(plan, "third")),
        Some(ScheduledCellState::Succeeded)
    );
}

#[test]
fn evaluation_schedule_rejects_invalid_progress_without_releasing_slots() {
    let snapshot = mixed_snapshot();
    let original = snapshot.evaluation_plan().unwrap();
    let unknown = id(original, "i2");
    let mut owned_plan = original.clone();
    owned_plan.cells.retain(|cell| cell.cell_id != unknown);
    let plan = &owned_plan;
    let mut schedule = plan.schedule(NonZeroUsize::new(1).unwrap()).unwrap();
    assert_eq!(schedule.state(unknown), None);
    assert_eq!(
        schedule.complete(unknown, ExecutionStatus::Success),
        Err(EvaluationScheduleError::UnknownCell(unknown))
    );
    assert_eq!(
        schedule.complete(id(plan, "a2"), ExecutionStatus::Success),
        Err(EvaluationScheduleError::CellNotRunning {
            cell_id: id(plan, "a2"),
            state: ScheduledCellState::Pending,
        })
    );
    assert_eq!(schedule.running_count(), 0);
    let first = schedule.reserve_next().unwrap().cell_id;
    schedule.complete(first, ExecutionStatus::Success).unwrap();
    let second = schedule.reserve_next().unwrap().cell_id;
    assert_eq!(
        schedule.complete(first, ExecutionStatus::Success),
        Err(EvaluationScheduleError::CellNotRunning {
            cell_id: first,
            state: ScheduledCellState::Succeeded,
        })
    );
    assert_eq!(schedule.running_count(), 1);
    assert!(schedule.reserve_next().is_none());
    schedule.complete(second, ExecutionStatus::Failure).unwrap();
    assert_eq!(
        schedule.complete(id(plan, "a3"), ExecutionStatus::Success),
        Err(EvaluationScheduleError::CellNotRunning {
            cell_id: id(plan, "a3"),
            state: ScheduledCellState::Blocked,
        })
    );
}

#[test]
fn evaluation_schedule_rejects_malformed_caller_constructed_plans() {
    let snapshot = mixed_snapshot();
    let plan = snapshot.evaluation_plan().unwrap();
    let limit = NonZeroUsize::new(2).unwrap();
    let mut duplicate = plan.clone();
    duplicate.cells.push(plan.cells[0].clone());
    assert_eq!(
        duplicate.schedule(limit).unwrap_err(),
        EvaluationScheduleError::DuplicateCell(plan.cells[0].cell_id)
    );
    for (label, predecessor) in [
        ("a1", Some(id(plan, "a1"))),
        ("a1", Some(id(plan, "a2"))),
        ("a2", None),
        ("a3", Some(id(plan, "a1"))),
        ("i1", Some(id(plan, "a1"))),
        ("b2", Some(id(plan, "a1"))),
        ("p2", Some(id(plan, "a1"))),
    ] {
        let mut malformed = plan.clone();
        let cell = malformed
            .cells
            .iter_mut()
            .find(|cell| cell.cell_id == id(plan, label))
            .unwrap();
        let expected = cell.predecessor;
        cell.predecessor = predecessor;
        assert_eq!(
            malformed.schedule(limit).unwrap_err(),
            EvaluationScheduleError::InvalidPredecessor {
                cell_id: id(plan, label),
                expected,
                actual: predecessor,
            }
        );
    }
}

#[test]
fn evaluation_schedule_empty_and_disabled_plans_finish_without_reservations() {
    for snapshot in [
        snapshot(&[]),
        snapshot(&[("disabled", "r", "default", false)]),
    ] {
        let mut schedule = snapshot
            .evaluation_plan()
            .unwrap()
            .schedule(NonZeroUsize::MIN)
            .unwrap();
        assert!(schedule.is_finished());
        assert_eq!(schedule.running_count(), 0);
        assert!(schedule.reserve_next().is_none());
    }
}

#[test]
fn evaluation_schedule_unrelated_workers_progress_while_one_session_is_blocked() {
    let snapshot = snapshot(&[
        ("slow", "r", "default", true),
        ("slow_suffix", "r", "default", true),
        ("fast1", "r", "analysis", true),
        ("fast2", "r", "analysis", true),
        ("isolated1", "r", "isolated", true),
        ("isolated2", "r", "isolated", true),
    ]);
    let plan = snapshot.evaluation_plan().unwrap();
    let mut schedule = plan.schedule(NonZeroUsize::new(2).unwrap()).unwrap();
    let (completed_tx, completed_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let timeout = Duration::from_secs(10);
    std::thread::scope(|scope| {
        let slow = schedule.reserve_next().unwrap().cell_id;
        assert_eq!(slow, id(plan, "slow"));
        let slow_completed_tx = completed_tx.clone();
        scope.spawn(move || {
            started_tx.send(()).unwrap();
            // Dropping the sender also unblocks this worker if the test fails.
            if release_rx.recv().is_ok() {
                slow_completed_tx.send(slow).unwrap();
            }
        });
        // Own the release sender inside the scope so unwinding cannot deadlock.
        let release_slow = release_tx;
        started_rx.recv_timeout(timeout).unwrap();
        for label in ["fast1", "fast2", "isolated1", "isolated2"] {
            let cell = schedule.reserve_next().unwrap();
            assert_eq!(cell.cell_id, id(plan, label));
            let cell_id = cell.cell_id;
            let completed_tx = completed_tx.clone();
            scope.spawn(move || completed_tx.send(cell_id).unwrap());
            assert!(schedule.reserve_next().is_none());
            let completed = completed_rx.recv_timeout(timeout).unwrap();
            assert_eq!(completed, cell_id);
            schedule
                .complete(completed, ExecutionStatus::Success)
                .unwrap();
            assert_eq!(schedule.state(slow), Some(ScheduledCellState::Running));
            assert_eq!(
                schedule.state(id(plan, "slow_suffix")),
                Some(ScheduledCellState::Pending)
            );
        }
        release_slow.send(()).unwrap();
        assert_eq!(completed_rx.recv_timeout(timeout).unwrap(), slow);
        schedule.complete(slow, ExecutionStatus::Success).unwrap();
    });
    assert_eq!(
        schedule.reserve_next().unwrap().cell_id,
        id(plan, "slow_suffix")
    );
    schedule
        .complete(id(plan, "slow_suffix"), ExecutionStatus::Success)
        .unwrap();
    assert!(schedule.is_finished());
}
