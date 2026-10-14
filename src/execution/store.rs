//! Content-addressed objects and atomic records at the execution effect boundary.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    AcceptedExecution, ArtifactId, DisplayRepresentation, ExecutionKey, ExecutionOutcome,
    ExecutionStatus, OutputPayload, ProvisionalArtifact,
};

const SCHEMA_VERSION: u32 = 1;

/// A project-local execution cache, with one writer across processes.
///
/// Opening the store acquires a nonblocking exclusive lock until drop. Store
/// handles cannot be cloned. Objects are staged independently of result records
/// and are invisible through this API until a validated success commits them.
/// Records and active attempts root their objects; unreferenced objects may be
/// left by rejected attempts. Garbage collection is not performed yet.
///
/// Writes use temporary files in the destination directory and atomic rename.
/// Reads verify both record and object digests. Missing, partial, corrupt, and
/// incompatible entries are cache misses. Cached provenance retains the original
/// compiler's local identities and never proves live interpreter state.
#[derive(Debug)]
pub struct ExecutionStore {
    root: PathBuf,
    _lock: File,
}

#[derive(Serialize, Deserialize)]
struct Record {
    schema_version: u32,
    execution: AcceptedExecution,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    sha256: String,
    record: String,
}

impl ExecutionStore {
    /// Open a cache directory, creating it if necessary.
    /// A second writer fails immediately while another handle holds the lock.
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("writer.lock"))?;
        lock.try_lock().map_err(io::Error::other)?;
        let root = root.join("v1");
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("results"))?;
        Ok(Self { root, _lock: lock })
    }

    /// Read a complete successful record and verify all its artifact bytes.
    ///
    /// Cache policy and execution authorization remain the caller's decisions.
    /// This method performs no execution and materializes no interpreter state.
    pub fn load(&self, key: ExecutionKey) -> Option<AcceptedExecution> {
        self.read_record(key).ok()
    }

    fn read_record(&self, key: ExecutionKey) -> io::Result<AcceptedExecution> {
        let envelope: Envelope = serde_json::from_slice(&fs::read(self.record_path(key))?)?;
        if digest(envelope.record.as_bytes()) != envelope.sha256 {
            return Err(invalid("result record digest mismatch"));
        }
        let record: Record = serde_json::from_str(&envelope.record)?;
        if record.schema_version != SCHEMA_VERSION
            || record.execution.outcome.provenance.request.execution_key != key
        {
            return Err(invalid("incompatible result record"));
        }
        let mut execution = record.execution;
        validate_outcome(&execution.outcome)?;
        for (id, artifact) in &mut execution.artifacts {
            if artifact.reference.id != *id || !artifact.bytes.is_empty() {
                return Err(invalid("invalid stored artifact metadata"));
            }
            artifact.bytes = fs::read(self.object_path(id)?)?;
            if content_id(&artifact.bytes) != *id {
                return Err(invalid("artifact digest mismatch"));
            }
        }
        validate_references(&execution, false)?;
        Ok(execution)
    }

    pub(super) fn stage(&self, artifact: &ProvisionalArtifact) -> io::Result<()> {
        let id = content_id(&artifact.bytes);
        let path = self.object_path(&id)?;
        if fs::read(&path).is_ok_and(|bytes| bytes == artifact.bytes) {
            return Ok(());
        }
        atomic_write(&path, &artifact.bytes)
    }

    /// Normalize attempt-local aliases only after checking the original stream.
    pub(super) fn prepare(
        &self,
        outcome: ExecutionOutcome,
        artifacts: BTreeMap<ArtifactId, ProvisionalArtifact>,
    ) -> io::Result<AcceptedExecution> {
        let mut execution = AcceptedExecution { outcome, artifacts };
        validate_outcome(&execution.outcome)?;
        validate_references(&execution, true)?;
        let mut aliases = BTreeMap::new();
        let mut objects = BTreeMap::new();
        for (alias, mut artifact) in execution.artifacts {
            self.stage(&artifact)?;
            let id = content_id(&artifact.bytes);
            aliases.insert(alias, id.clone());
            artifact.reference.id = id.clone();
            objects.insert(id, artifact);
        }
        for event in &mut execution.outcome.events {
            if let OutputPayload::Display(bundle) = &mut event.payload {
                for representation in &mut bundle.representations {
                    if let DisplayRepresentation::Png(reference)
                    | DisplayRepresentation::Artifact(reference) = representation
                    {
                        reference.id = aliases[&reference.id].clone();
                    }
                }
            }
        }
        execution.artifacts = objects;
        Ok(execution)
    }

    /// Only the coordinator may publish, after all publication checks succeed.
    pub(super) fn commit(&self, execution: &AcceptedExecution) -> io::Result<()> {
        validate_outcome(&execution.outcome)?;
        validate_references(execution, false)?;
        for (id, artifact) in &execution.artifacts {
            if content_id(&artifact.bytes) != *id
                || fs::read(self.object_path(id)?)? != artifact.bytes
            {
                return Err(invalid("artifact changed before result publication"));
            }
        }
        // Records root immutable objects but never duplicate their bytes.
        let mut metadata = execution.clone();
        for artifact in metadata.artifacts.values_mut() {
            artifact.bytes.clear();
        }
        let record = serde_json::to_string(&Record {
            schema_version: SCHEMA_VERSION,
            execution: metadata,
        })?;
        let bytes = serde_json::to_vec(&Envelope {
            sha256: digest(record.as_bytes()),
            record,
        })?;
        atomic_write(
            &self.record_path(execution.outcome.provenance.request.execution_key),
            &bytes,
        )
    }

    fn record_path(&self, key: ExecutionKey) -> PathBuf {
        self.root.join("results").join(hex(key.digest()))
    }

    fn object_path(&self, id: &ArtifactId) -> io::Result<PathBuf> {
        let hash =
            id.0.strip_prefix("sha256-")
                .filter(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
                .ok_or_else(|| invalid("invalid content address"))?;
        Ok(self.root.join("objects").join(&hash[..2]).join(hash))
    }
}

pub(super) fn validate_references(execution: &AcceptedExecution, aliases: bool) -> io::Result<()> {
    for (id, artifact) in &execution.artifacts {
        if artifact.reference.id != *id || artifact.reference.media_type.is_empty() {
            return Err(invalid("invalid artifact metadata"));
        }
    }
    for event in &execution.outcome.events {
        if let OutputPayload::Display(bundle) = &event.payload {
            for representation in &bundle.representations {
                let reference = match representation {
                    DisplayRepresentation::Png(reference)
                        if reference.media_type != "image/png" =>
                    {
                        return Err(invalid("PNG representation must declare image/png"));
                    }
                    DisplayRepresentation::Png(reference)
                    | DisplayRepresentation::Artifact(reference) => reference,
                    _ => continue,
                };
                let artifact = execution
                    .artifacts
                    .get(&reference.id)
                    .ok_or_else(|| invalid("display references an unstaged artifact"))?;
                if reference.media_type.is_empty()
                    || (aliases && artifact.reference.media_type != reference.media_type)
                {
                    return Err(invalid("artifact media type mismatch"));
                }
            }
        }
    }
    Ok(())
}

fn validate_outcome(outcome: &ExecutionOutcome) -> io::Result<()> {
    let request = &outcome.provenance.request;
    if outcome.status != ExecutionStatus::Success
        || outcome.provenance.effective_execution_key != request.execution_key
        || outcome.events.iter().enumerate().any(|(sequence, event)| {
            event.sequence != sequence as u64
                || event.source_revision != request.source_revision
                || event.attempt_id != request.attempt_id
                || event.cell_id != request.cell_id
                || event.session_id != request.session_id
        })
    {
        return Err(invalid("invalid successful execution record"));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().expect("cache paths have parents");
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn content_id(bytes: &[u8]) -> ArtifactId {
    ArtifactId(format!("sha256-{}", digest(bytes)))
}

fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
