//! Incremental compilation primitives for computational Markdown.

mod build;
mod compiler;
mod document;
mod execution;
mod parser;
mod render;

pub use build::{RenderError, render_html, render_html_with_options};
pub use compiler::{
    Compiler, CompilerSnapshot, EvaluationKeyError, EvaluationPlan, EvaluationSchedule,
    EvaluationScheduleError, EvaluationSession, ExecutionInputs, KeyedCell, KeyedEvaluationPlan,
    PlannedCell, RenderOptions, ScheduledCellState, SessionPreparation, SlideChangeSet, SlideMove,
    SlidePosition, SourceRevision, SourceUpdate, inspect_document, summarize_document,
};
pub use document::{
    CellId, Derivation, Diagnostic, DiagnosticCode, DocumentInspection, DocumentSummary,
    ExecutionKey, Origin, RelatedOrigin, Severity, SlideId, SourceFile, SourceRange, SourceSpan,
};

pub use execution::{
    AcceptedExecution, ArtifactId, ArtifactReference, AttemptId, AttemptReport, CellExecutionState,
    CellExecutionView, CompletionDisposition, CoordinatorError, DisplayBundle,
    DisplayRepresentation, ExecutionCoordinator, ExecutionOutcome, ExecutionProvenance,
    ExecutionRequest, ExecutionRunner, ExecutionStatus, ExecutionStore, FakeArtifact,
    FakeCompletion, FakeRunner, FakeRunnerError, FakeRunnerEvent, FakeRunnerState, FakeScript,
    FakeStep, OutputEvent, OutputPayload, ProvisionalArtifact, RDiscoveryError, RInstallation,
    RejectionReason, RepresentationKind, RunnerEvent, RunnerIdentity, SessionId, StateKey,
};
