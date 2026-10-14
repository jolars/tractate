//! Backend-independent contracts at the boundary between planning and effects.

use std::time::Duration;

use crate::{CellId, Diagnostic, ExecutionKey, SourceRevision};

mod fake;

pub use fake::{
    FakeArtifact, FakeCompletion, FakeRunner, FakeRunnerError, FakeRunnerEvent, FakeRunnerState,
    FakeScript, FakeStep,
};

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
/// currently desired computation. Events remain provisional until that check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub status: ExecutionStatus,
    pub diagnostics: Vec<Diagnostic>,
    /// Captured events in sequence order, including partial output on failure.
    pub events: Vec<OutputEvent>,
    pub provenance: ExecutionProvenance,
}

/// One captured event, attributed before any asynchronous delivery.
///
/// Sequences start at zero for each attempt and increase by one across all output
/// kinds. Receivers must reject duplicate, missing, or out-of-order sequences and
/// mismatched identities as protocol errors. A terminal outcome contains the same
/// stream in sequence order; it does not authorize publication of partial output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputEvent {
    pub source_revision: SourceRevision,
    pub session_id: SessionId,
    pub cell_id: CellId,
    pub attempt_id: AttemptId,
    pub sequence: u64,
    pub payload: OutputPayload,
}

/// Captured output, unaffected by source visibility or rendering options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputPayload {
    Stdout(String),
    Stderr(String),
    Warning(Diagnostic),
    Display(DisplayBundle),
}

/// An opaque artifact identity, resolved through the artifact store.
///
/// This is not a filesystem path or URL. During execution it refers to staged
/// data owned by the attempt; publication must commit that data transactionally.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactId(pub String);

/// A reference to stored bytes with their declared media type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReference {
    pub id: ArtifactId,
    pub media_type: String,
}

/// Representation categories that a renderer can advertise in preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepresentationKind {
    Text,
    Html,
    Svg,
    Png,
    Artifact,
}

/// Alternative representations of a single displayed value.
///
/// HTML and SVG are untrusted content and require the backend's trust policy.
/// PNG bytes live in the artifact store; their reference must declare `image/png`.
/// Generic artifacts allow additional media types without changing this enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayRepresentation {
    Text(String),
    Html(String),
    Svg(String),
    Png(ArtifactReference),
    Artifact(ArtifactReference),
}

impl DisplayRepresentation {
    pub fn kind(&self) -> RepresentationKind {
        match self {
            Self::Text(_) => RepresentationKind::Text,
            Self::Html(_) => RepresentationKind::Html,
            Self::Svg(_) => RepresentationKind::Svg,
            Self::Png(_) => RepresentationKind::Png,
            Self::Artifact(_) => RepresentationKind::Artifact,
        }
    }
}

/// Several alternatives for one value, never several successive outputs.
///
/// Producers should include a text fallback for rich output. Source captions and
/// presentation options belong to the compiler, not this runner-owned bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayBundle {
    pub representations: Vec<DisplayRepresentation>,
}

impl DisplayBundle {
    /// Select the first acceptable representation in the renderer's preference
    /// order. The predicate checks trust and, for artifacts, supported media types.
    /// Returns `None` when the backend cannot safely present any alternative.
    pub fn select(
        &self,
        preferences: &[RepresentationKind],
        mut accepts: impl FnMut(&DisplayRepresentation) -> bool,
    ) -> Option<&DisplayRepresentation> {
        for kind in preferences {
            if let Some(representation) = self
                .representations
                .iter()
                .find(|representation| representation.kind() == *kind && accepts(representation))
            {
                return Some(representation);
            }
        }
        None
    }
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
                events: Vec::new(),
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
    fn display_selection_respects_backend_preferences_and_trust() {
        let bundle = DisplayBundle {
            representations: vec![
                DisplayRepresentation::Html("<b>value</b>".into()),
                DisplayRepresentation::Text("value".into()),
                DisplayRepresentation::Svg("<svg/>".into()),
                DisplayRepresentation::Png(ArtifactReference {
                    id: ArtifactId("plot".into()),
                    media_type: "image/png".into(),
                }),
                DisplayRepresentation::Artifact(ArtifactReference {
                    id: ArtifactId("table".into()),
                    media_type: "text/csv".into(),
                }),
            ],
        };
        assert_eq!(
            bundle.select(&[RepresentationKind::Png, RepresentationKind::Text], |_| {
                true
            }),
            Some(&bundle.representations[3])
        );
        assert_eq!(
            bundle.select(&[RepresentationKind::Html, RepresentationKind::Text], |r| {
                !matches!(r, DisplayRepresentation::Html(_))
            }),
            Some(&bundle.representations[1])
        );
        assert_eq!(
            bundle.select(&[RepresentationKind::Artifact], |r| {
                matches!(r, DisplayRepresentation::Artifact(a) if a.media_type == "application/pdf")
            }),
            None
        );
        assert_eq!(
            bundle.select(&[RepresentationKind::Svg], |_| true),
            Some(&bundle.representations[2])
        );
        assert_eq!(
            bundle.select(&[RepresentationKind::Artifact], |r| {
                matches!(r, DisplayRepresentation::Artifact(a) if a.media_type == "text/csv")
            }),
            Some(&bundle.representations[4])
        );
        assert_eq!(bundle.select(&[], |_| true), None);
        assert_eq!(
            DisplayBundle {
                representations: vec![]
            }
            .select(&[RepresentationKind::Text], |_| true),
            None
        );
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
