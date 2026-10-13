//! Pure source-to-computation lowering, independent of slide layout and backends.

use std::collections::BTreeMap;

use crate::document::*;

use super::SourceRevision;
use super::options::{boolean, execute_mappings};
use super::queries::{DependencyGraph, QueryKey, Reads};

mod keys;
mod schedule;

pub use keys::{
    EvaluationKeyError, ExecutionInputs, KeyedCell, KeyedEvaluationPlan, SessionPreparation,
};
pub use schedule::{EvaluationSchedule, EvaluationScheduleError, ScheduledCellState};

/// Desired computations in semantic document order at one source revision.
///
/// Only cells with effective `eval: true` appear here. Constructing or reading
/// this plan performs no I/O and does not authorize execution. Predecessor links
/// describe the cumulative state contributors, not materialized interpreter
/// state. [`Self::keyed`] adds computation identities once execution inputs have
/// been resolved. Dispatch identities remain the coordinator's responsibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationPlan {
    pub source_revision: SourceRevision,
    pub cells: Vec<PlannedCell>,
}

/// One enabled source cell before execution inputs and attempts are assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCell {
    pub cell_id: CellId,
    pub origin: Origin,
    /// The source language; planning does not require an installed runner.
    pub language: Option<String>,
    /// Executable source without leading option declarations or container prefixes.
    pub code: String,
    pub session: EvaluationSession,
    /// The preceding enabled cell in the same language and stateful session.
    ///
    /// `None` denotes either the initial state of a stateful session or an
    /// isolated cell. Disabled cells never contribute to this chain.
    pub predecessor: Option<CellId>,
}

/// A source-level session selection, scoped to the document and language.
///
/// This does not identify a running interpreter. Each isolated cell runs
/// independently, even though all use the same selection value here.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvaluationSession {
    #[default]
    Default,
    Named(String),
    Isolated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EvaluationOptions {
    eval: bool,
    session: EvaluationSession,
}

impl Default for EvaluationOptions {
    fn default() -> Self {
        Self {
            eval: true,
            session: EvaluationSession::Default,
        }
    }
}

impl EvaluationOptions {
    fn apply_metadata(&mut self, metadata: &Metadata, diagnostics: &mut Vec<Diagnostic>) {
        for mapping in execute_mappings(metadata) {
            self.apply(mapping, diagnostics);
        }
    }

    fn apply(&mut self, mapping: &YamlValue, diagnostics: &mut Vec<Diagnostic>) {
        for entry in mapping.mapping().unwrap_or_default() {
            let YamlKind::Scalar { text: key, .. } = &entry.key.kind else {
                continue;
            };
            match key.as_str() {
                "eval" => {
                    if let Some(value) = boolean(key, &entry.value, diagnostics) {
                        self.eval = value;
                    }
                }
                "session" => {
                    if let Some(name) = super::validation::name(&entry.value) {
                        self.session = match name.as_str() {
                            "default" => EvaluationSession::Default,
                            "isolated" => EvaluationSession::Isolated,
                            _ => EvaluationSession::Named(name),
                        };
                    } else {
                        diagnostics.push(error(
                            DiagnosticCode::InvalidOptionValue,
                            &entry.value.origin,
                            "`session` must be a nonempty string without whitespace or control characters",
                        ));
                    }
                }
                _ => {}
            }
        }
    }
}

pub(super) fn lower(
    document: &SourceDocument,
    revision: SourceRevision,
    source_diagnostics: &[Diagnostic],
    graph: &mut DependencyGraph,
) -> Result<EvaluationPlan, Vec<Diagnostic>> {
    let reads = Reads::default();
    let result = plan(document, revision, source_diagnostics, graph, &reads);
    graph.finish(QueryKey::EvaluationPlan, reads);
    result
}

fn plan(
    document: &SourceDocument,
    revision: SourceRevision,
    source_diagnostics: &[Diagnostic],
    graph: &mut DependencyGraph,
    reads: &Reads,
) -> Result<EvaluationPlan, Vec<Diagnostic>> {
    let mut diagnostics = reads
        .read(QueryKey::SourceDiagnostics, source_diagnostics)
        .to_vec();
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Err(diagnostics);
    }
    let defaults = graph.query(QueryKey::EvaluationDefaults, |reads| {
        let mut defaults = EvaluationOptions::default();
        let mut diagnostics = Vec::new();
        for metadata in reads.read(QueryKey::Metadata, &document.metadata) {
            defaults.apply_metadata(metadata, &mut diagnostics);
        }
        reads
            .read(QueryKey::SourceDocument, document)
            .visit_blocks(&mut |block| {
                if let BlockKind::Metadata(metadata) = &block.kind {
                    defaults.apply_metadata(
                        reads.read(QueryKey::SemanticBlock(block.id), metadata),
                        &mut diagnostics,
                    );
                }
            });
        (defaults, diagnostics)
    });
    let (defaults, default_diagnostics) = reads.read(QueryKey::EvaluationDefaults, defaults);
    diagnostics.extend(default_diagnostics);
    let mut cells = Vec::new();
    let mut predecessors = BTreeMap::new();
    reads
        .read(QueryKey::SourceDocument, document)
        .visit_blocks(&mut |block| {
            let BlockKind::Cell(cell) = &block.kind else {
                return;
            };
            let resolved = graph.query(QueryKey::CellEvaluation(cell.id), |reads| {
                let cell = reads.read(QueryKey::SemanticBlock(block.id), cell);
                let mut options = reads.read(QueryKey::EvaluationDefaults, defaults.clone());
                let mut diagnostics = Vec::new();
                if let Some(preamble) = &cell.preamble {
                    options.apply(preamble, &mut diagnostics);
                }
                (options, diagnostics)
            });
            let (options, cell_diagnostics) =
                reads.read(QueryKey::CellEvaluation(cell.id), resolved);
            diagnostics.extend(cell_diagnostics);
            if !options.eval {
                return;
            }
            let predecessor = if options.session == EvaluationSession::Isolated {
                None
            } else {
                predecessors.insert(
                    (cell.code.language.clone(), options.session.clone()),
                    cell.id,
                )
            };
            cells.push(PlannedCell {
                cell_id: cell.id,
                origin: cell.origin.derived("evaluation-cell"),
                language: cell.code.language.clone(),
                code: cell.code.source.clone(),
                session: options.session,
                predecessor,
            });
        });
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        Err(diagnostics)
    } else {
        Ok(EvaluationPlan {
            source_revision: revision,
            cells,
        })
    }
}
