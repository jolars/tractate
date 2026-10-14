use std::collections::BTreeMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tractate::{
    ArtifactId, ArtifactReference, CellId, Compiler, CompilerSnapshot, CompletionDisposition,
    CoordinatorError, Diagnostic, DiagnosticCode, DisplayBundle, DisplayRepresentation,
    ExecutionCoordinator, ExecutionInputs, ExecutionKey, ExecutionRequest, ExecutionStatus,
    ExecutionStore, FakeArtifact, FakeCompletion, FakeRunner, FakeScript, FakeStep, Origin,
    OutputPayload, RejectionReason, ScheduledCellState, Severity, SourceFile, SourceRange,
};

type Coordinator = ExecutionCoordinator<FakeRunner>;

fn compiler() -> Compiler {
    Compiler::new(SourceFile::new("slides.qmd", "```{r}\nplot(1)\n```\n"))
}

fn reconcile(coordinator: &mut Coordinator, snapshot: &CompilerSnapshot) -> (CellId, ExecutionKey) {
    let plan = snapshot.evaluation_plan().unwrap();
    let inputs = plan
        .cells
        .iter()
        .map(|cell| {
            (
                cell.cell_id,
                ExecutionInputs {
                    environment_digest: [1; 32],
                    computation_digest: [2; 32],
                    volatile_nonce: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let keyed = plan.keyed(&inputs).unwrap();
    coordinator.reconcile(&keyed).unwrap();
    (plan.cells[0].cell_id, keyed.cells()[0].execution_key)
}

fn coordinator(root: &Path) -> Coordinator {
    Coordinator::with_store(
        NonZeroUsize::new(1).unwrap(),
        ExecutionStore::open(root).unwrap(),
    )
}

fn dispatch(coordinator: &mut Coordinator, script: FakeScript) -> ExecutionRequest {
    coordinator
        .dispatch_ready(|session_id| {
            let mut runner = FakeRunner::new(session_id);
            runner.queue_script(script.clone());
            runner
        })
        .unwrap()
        .pop()
        .unwrap()
}

fn artifact(id: &str, bytes: &[u8]) -> FakeArtifact {
    FakeArtifact {
        reference: ArtifactReference {
            id: ArtifactId(id.into()),
            media_type: "image/png".into(),
        },
        bytes: bytes.to_vec(),
    }
}

fn display(id: &str) -> FakeStep {
    FakeStep::Output(OutputPayload::Display(DisplayBundle {
        representations: vec![
            DisplayRepresentation::Text("plot".into()),
            DisplayRepresentation::Png(artifact(id, b"").reference),
        ],
    }))
}

fn record_path(root: &Path, key: ExecutionKey) -> PathBuf {
    root.join("v1/results").join(hex(key.digest()))
}

fn object_path(root: &Path, bytes: &[u8]) -> PathBuf {
    let digest = hex(&Sha256::digest(bytes));
    root.join("v1/objects").join(&digest[..2]).join(digest)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn finish(coordinator: &mut Coordinator) {
    for _ in 0..20 {
        if coordinator.is_finished() {
            return;
        }
        coordinator.poll().unwrap();
    }
    panic!("execution did not finish");
}

fn successful_result(root: &Path) -> (Coordinator, CellId, ExecutionKey) {
    let mut coordinator = coordinator(root);
    let (id, key) = reconcile(&mut coordinator, &compiler().snapshot());
    dispatch(
        &mut coordinator,
        FakeScript {
            steps: vec![
                FakeStep::Artifact(artifact("plot", b"hello")),
                display("plot"),
            ],
            ..FakeScript::default()
        },
    );
    finish(&mut coordinator);
    (coordinator, id, key)
}

#[test]
fn execution_store_stages_deduplicated_objects_before_atomic_result_publication() {
    let temp = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(temp.path());
    let snapshot = compiler().snapshot();
    let (id, key) = reconcile(&mut coordinator, &snapshot);
    let request = dispatch(
        &mut coordinator,
        FakeScript {
            steps: vec![
                FakeStep::Artifact(artifact("../plot", b"hello")),
                FakeStep::Artifact(artifact("other-alias", b"hello")),
                display("../plot"),
                display("other-alias"),
                FakeStep::Block,
            ],
            ..FakeScript::default()
        },
    );
    for _ in 0..5 {
        coordinator.poll().unwrap();
        assert!(coordinator.result(id).is_none());
        assert!(coordinator.store().unwrap().load(key).is_none());
        assert!(!record_path(temp.path(), key).exists());
    }
    assert_eq!(
        fs::read(object_path(temp.path(), b"hello")).unwrap(),
        b"hello"
    );
    assert_eq!(
        fs::read_dir(temp.path().join("v1/objects"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        fs::read_dir(object_path(temp.path(), b"hello").parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    coordinator
        .runner_mut(request.session_id)
        .unwrap()
        .release(request.attempt_id)
        .unwrap();
    finish(&mut coordinator);
    let accepted = coordinator.result(id).unwrap().clone();
    assert_eq!(accepted.artifacts.len(), 1);
    let digest_id = ArtifactId(format!("sha256-{}", hex(&Sha256::digest(b"hello"))));
    assert_eq!(accepted.artifacts[&digest_id].bytes, b"hello");
    for event in &accepted.outcome.events {
        let OutputPayload::Display(bundle) = &event.payload else {
            panic!("expected display")
        };
        assert!(
            matches!(&bundle.representations[1], DisplayRepresentation::Png(reference) if reference.id == digest_id)
        );
    }
    assert_eq!(
        coordinator.store().unwrap().load(key),
        Some(accepted.clone())
    );
    drop(coordinator);
    let store = ExecutionStore::open(temp.path()).unwrap();
    assert_eq!(store.load(key), Some(accepted));
}

#[test]
fn execution_store_replacement_keeps_the_previous_complete_record_until_commit() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let temp = tempfile::tempdir().unwrap();
    let (mut coordinator, id, key) = successful_result(temp.path());
    let path = record_path(temp.path(), key);
    let previous = fs::read(&path).unwrap();
    let previous_result = coordinator.store().unwrap().load(key).unwrap();
    let session_id = coordinator.session_id(id).unwrap();
    coordinator.retry(id).unwrap();
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .queue_script(FakeScript {
            steps: vec![
                FakeStep::Artifact(artifact("plot", b"new bytes")),
                display("plot"),
                FakeStep::Block,
            ],
            ..FakeScript::default()
        });
    let request = coordinator
        .dispatch_ready(|_| unreachable!())
        .unwrap()
        .pop()
        .unwrap();
    for _ in 0..3 {
        coordinator.poll().unwrap();
    }
    assert_eq!(
        coordinator.store().unwrap().load(key),
        Some(previous_result)
    );
    assert_eq!(fs::read(&path).unwrap(), previous);
    coordinator
        .runner_mut(session_id)
        .unwrap()
        .release(request.attempt_id)
        .unwrap();

    let stopped = AtomicBool::new(false);
    let reading = std::sync::Barrier::new(2);
    let observed = std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let mut observed = vec![fs::read(&path).unwrap()];
            reading.wait();
            while !stopped.load(Ordering::Acquire) {
                observed.push(fs::read(&path).unwrap());
            }
            observed.push(fs::read(&path).unwrap());
            observed
        });
        reading.wait();
        coordinator.poll().unwrap();
        stopped.store(true, Ordering::Release);
        reader.join().unwrap()
    });
    let current = fs::read(&path).unwrap();
    assert_ne!(previous, current);
    assert!(
        observed
            .iter()
            .all(|bytes| bytes == &previous || bytes == &current)
    );
    assert_eq!(
        coordinator.store().unwrap().load(key),
        coordinator.result(id).cloned()
    );
}

#[test]
fn execution_store_round_trips_output_provenance_and_diagnostic_origins() {
    let temp = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(temp.path());
    let (id, key) = reconcile(&mut coordinator, &compiler().snapshot());
    let source = SourceFile::new("warning.qmd", "Unicode: λ\n");
    let origin = Origin::Source(source.span(SourceRange { start: 9, end: 11 }).unwrap())
        .derived("execution warning");
    let diagnostic = Diagnostic {
        severity: Severity::Warning,
        code: DiagnosticCode::ExecutionWarning,
        message: "a warning".into(),
        primary: origin,
        related: vec![],
    };
    dispatch(
        &mut coordinator,
        FakeScript {
            steps: vec![
                FakeStep::Output(OutputPayload::Stdout("out".into())),
                FakeStep::Output(OutputPayload::Stderr("err".into())),
                FakeStep::Output(OutputPayload::Warning(diagnostic.clone())),
                FakeStep::Artifact(artifact("empty", b"")),
                display("empty"),
            ],
            diagnostics: vec![diagnostic],
            duration: std::time::Duration::from_millis(42),
            ..FakeScript::default()
        },
    );
    finish(&mut coordinator);
    let expected = serde_json::to_value(coordinator.result(id).unwrap()).unwrap();
    drop(coordinator);
    let loaded = ExecutionStore::open(temp.path())
        .unwrap()
        .load(key)
        .unwrap();
    assert_eq!(serde_json::to_value(&loaded).unwrap(), expected);
    let origin = &loaded.outcome.diagnostics[0].primary;
    assert_eq!(origin.source_span().text(), "λ");
    assert_eq!(origin.chain().count(), 2);
    assert_eq!(
        origin.source_span().file().path(),
        Some(Path::new("warning.qmd"))
    );
    rewrite_record(&record_path(temp.path(), key), |record| {
        record["execution"]["outcome"]["diagnostics"][0]["primary"]["Derived"]["parent"]["Source"]
            ["range"]["end"] = json!(usize::MAX);
    });
    assert!(
        ExecutionStore::open(temp.path())
            .unwrap()
            .load(key)
            .is_none()
    );
}

#[test]
fn execution_store_missing_partial_and_corrupt_objects_are_cache_misses() {
    let temp = tempfile::tempdir().unwrap();
    let (coordinator, _, key) = successful_result(temp.path());
    let path = object_path(temp.path(), b"hello");
    for bytes in [b"hel".as_slice(), b"HELLO", b""] {
        fs::write(&path, bytes).unwrap();
        assert!(coordinator.store().unwrap().load(key).is_none());
    }
    fs::remove_file(&path).unwrap();
    assert!(coordinator.store().unwrap().load(key).is_none());
    fs::create_dir(&path).unwrap();
    assert!(coordinator.store().unwrap().load(key).is_none());
    fs::remove_dir(&path).unwrap();
    fs::write(&path, b"hello").unwrap();
    assert!(coordinator.store().unwrap().load(key).is_some());
}

fn rewrite_record(path: &Path, mutate: impl FnOnce(&mut Value)) {
    let mut envelope: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let mut record: Value = serde_json::from_str(envelope["record"].as_str().unwrap()).unwrap();
    mutate(&mut record);
    let record = serde_json::to_string(&record).unwrap();
    envelope["sha256"] = json!(hex(&Sha256::digest(record.as_bytes())));
    envelope["record"] = json!(record);
    fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
}

#[test]
fn execution_store_incomplete_corrupt_incompatible_and_invalid_records_are_misses() {
    let temp = tempfile::tempdir().unwrap();
    let (coordinator, _, key) = successful_result(temp.path());
    let path = record_path(temp.path(), key);
    let valid = fs::read(&path).unwrap();
    for bytes in [b"{".as_slice(), b"", &valid[..valid.len() / 2], b"{}"] {
        fs::write(&path, bytes).unwrap();
        assert!(coordinator.store().unwrap().load(key).is_none());
    }
    fs::write(&path, &valid).unwrap();
    let mut corrupt: Value = serde_json::from_slice(&valid).unwrap();
    corrupt["record"] = json!(corrupt["record"].as_str().unwrap().replace("plot", "oops"));
    fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    assert!(coordinator.store().unwrap().load(key).is_none());

    let mutations: &[fn(&mut Value)] = &[
        |record| record["schema_version"] = json!(99),
        |record| record["execution"]["outcome"]["status"] = json!("Failure"),
        |record| record["execution"]["outcome"]["status"] = json!("Cancelled"),
        |record| record["execution"]["outcome"]["events"][0]["sequence"] = json!(10),
        |record| {
            record["execution"]["outcome"]["provenance"]["effective_execution_key"] =
                json!([0; 32].to_vec())
        },
        |record| {
            record["execution"]["artifacts"] = json!({});
        },
        |record| {
            let artifacts = record["execution"]["artifacts"].as_object_mut().unwrap();
            let key = artifacts.keys().next().unwrap().clone();
            let mut artifact = artifacts.remove(&key).unwrap();
            artifact["reference"]["id"] = json!("../../escape");
            artifacts.insert("../../escape".into(), artifact);
        },
    ];
    for mutate in mutations {
        fs::write(&path, &valid).unwrap();
        rewrite_record(&path, mutate);
        assert!(coordinator.store().unwrap().load(key).is_none());
    }
    fs::remove_file(&path).unwrap();
    assert!(coordinator.store().unwrap().load(key).is_none());
    fs::write(path.with_extension("partial"), &valid).unwrap();
    assert!(coordinator.store().unwrap().load(key).is_none());
    fs::write(&path, &valid).unwrap();
    assert!(coordinator.store().unwrap().load(key).is_some());
}

#[test]
fn execution_store_failed_crashed_cancelled_and_revoked_attempts_never_commit() {
    for completion in [
        FakeCompletion::Failure,
        FakeCompletion::Crash,
        FakeCompletion::Success,
    ] {
        for revoke in [false, true] {
            if completion == FakeCompletion::Success && !revoke {
                continue;
            }
            let temp = tempfile::tempdir().unwrap();
            let mut compiler = compiler();
            let mut coordinator = coordinator(temp.path());
            let (id, key) = reconcile(&mut coordinator, &compiler.snapshot());
            let request = dispatch(
                &mut coordinator,
                FakeScript {
                    steps: vec![
                        FakeStep::Artifact(artifact("plot", b"hello")),
                        display("plot"),
                        FakeStep::Block,
                    ],
                    completion,
                    ignore_interrupt: revoke,
                    ..FakeScript::default()
                },
            );
            for _ in 0..3 {
                coordinator.poll().unwrap();
            }
            assert!(object_path(temp.path(), b"hello").exists());
            if revoke {
                compiler.update_source(SourceFile::new("slides.qmd", "```{r}\nplot(2)\n```\n"));
                reconcile(&mut coordinator, &compiler.snapshot());
            }
            coordinator
                .runner_mut(request.session_id)
                .unwrap()
                .release(request.attempt_id)
                .unwrap();
            coordinator.poll().unwrap();
            assert!(coordinator.result(id).is_none());
            assert!(coordinator.store().unwrap().load(key).is_none());
            assert!(!record_path(temp.path(), key).exists());
            assert_eq!(coordinator.materialized_state_key(request.session_id), None);
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(temp.path());
    let (_, key) = reconcile(&mut coordinator, &compiler().snapshot());
    let request = dispatch(
        &mut coordinator,
        FakeScript {
            steps: vec![
                FakeStep::Artifact(artifact("plot", b"hello")),
                FakeStep::Block,
            ],
            ..FakeScript::default()
        },
    );
    coordinator.poll().unwrap();
    coordinator
        .runner_mut(request.session_id)
        .unwrap()
        .interrupt();
    assert_eq!(
        coordinator.poll().unwrap()[0].disposition,
        CompletionDisposition::Completed(ExecutionStatus::Cancelled)
    );
    assert!(coordinator.store().unwrap().load(key).is_none());
    assert!(!record_path(temp.path(), key).exists());
}

#[test]
fn execution_store_rejects_missing_artifacts_before_accepting_runner_state() {
    let temp = tempfile::tempdir().unwrap();
    let mut coordinator = coordinator(temp.path());
    let (id, key) = reconcile(&mut coordinator, &compiler().snapshot());
    let request = dispatch(
        &mut coordinator,
        FakeScript {
            steps: vec![display("missing")],
            ..FakeScript::default()
        },
    );
    coordinator.poll().unwrap();
    assert_eq!(
        coordinator.poll().unwrap()[0].disposition,
        CompletionDisposition::Rejected(RejectionReason::Artifact)
    );
    assert_eq!(coordinator.state(id), Some(ScheduledCellState::Failed));
    assert!(coordinator.result(id).is_none());
    assert!(coordinator.store().unwrap().load(key).is_none());
    assert_eq!(coordinator.materialized_state_key(request.session_id), None);
}

#[test]
fn execution_store_write_failures_publish_nothing_and_poison_the_session() {
    for fail_object in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut coordinator = coordinator(temp.path());
        let (id, key) = reconcile(&mut coordinator, &compiler().snapshot());
        let directory = temp.path().join(if fail_object {
            "v1/objects"
        } else {
            "v1/results"
        });
        fs::remove_dir(&directory).unwrap();
        fs::write(&directory, b"cannot write through a file").unwrap();
        let request = dispatch(
            &mut coordinator,
            FakeScript {
                steps: vec![
                    FakeStep::Artifact(artifact("plot", b"hello")),
                    display("plot"),
                ],
                ..FakeScript::default()
            },
        );
        let mut failed = false;
        for _ in 0..3 {
            if let Err(error) = coordinator.poll() {
                assert!(matches!(error, CoordinatorError::Store(_)));
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert_eq!(coordinator.state(id), Some(ScheduledCellState::Failed));
        assert!(coordinator.result(id).is_none());
        assert!(coordinator.store().unwrap().load(key).is_none());
        assert_eq!(coordinator.materialized_state_key(request.session_id), None);
        assert_eq!(coordinator.running_count(), 0);
    }
}

#[test]
fn execution_store_lock_child() {
    if let Some(root) = std::env::var_os("TRACTATE_TEST_LOCK_ROOT") {
        assert!(ExecutionStore::open(root).is_err());
    }
}

#[test]
fn execution_store_serializes_writers_across_processes_and_releases_on_drop() {
    let temp = tempfile::tempdir().unwrap();
    let store = ExecutionStore::open(temp.path()).unwrap();
    assert!(ExecutionStore::open(temp.path()).is_err());
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "execution_store_lock_child"])
        .env("TRACTATE_TEST_LOCK_ROOT", temp.path())
        .status()
        .unwrap();
    assert!(status.success());
    drop(store);
    ExecutionStore::open(temp.path()).unwrap();
}
