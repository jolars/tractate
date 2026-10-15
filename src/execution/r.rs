//! Explicit, effectful discovery and control of the R interpreter.

mod bridge;

pub use bridge::{RBridge, REvaluation};

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::RunnerIdentity;

/// An R executable and the version reported by that executable.
///
/// Discovery starts R to query its version. Pure compilation and inspection do
/// not call this API. Retain this value to launch the same executable later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RInstallation {
    executable: PathBuf,
    version: String,
}

/// Failure to locate or query an R installation.
#[derive(Debug)]
pub enum RDiscoveryError {
    NotFound,
    Io(io::Error),
    /// The executable failed or returned an invalid version response.
    InvalidVersion,
}

impl fmt::Display for RDiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("R was not found on PATH"),
            Self::Io(error) => write!(f, "could not query the R executable: {error}"),
            Self::InvalidVersion => f.write_str("R did not report a valid version"),
        }
    }
}

impl std::error::Error for RDiscoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for RDiscoveryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl RInstallation {
    /// Find the first executable named `R` on the inherited PATH and query it.
    pub fn discover() -> Result<Self, RDiscoveryError> {
        Self::discover_on_path(std::env::var_os("PATH").as_deref())
    }

    fn discover_on_path(path: Option<&OsStr>) -> Result<Self, RDiscoveryError> {
        let path = path.ok_or(RDiscoveryError::NotFound)?;
        for directory in std::env::split_paths(path) {
            let candidate = directory.join("R");
            if is_executable(&candidate) {
                return Self::from_executable(candidate);
            }
        }
        Err(RDiscoveryError::NotFound)
    }

    /// Query an explicitly selected executable without searching PATH.
    pub fn from_executable(path: impl AsRef<Path>) -> Result<Self, RDiscoveryError> {
        let executable = path.as_ref().canonicalize()?;
        // Vanilla startup prevents project profiles from running during discovery.
        let output = Command::new(&executable)
            .args([
                "--vanilla",
                "--slave",
                "-e",
                "cat(as.character(getRversion()))",
            ])
            .stdin(Stdio::null())
            .output()?;
        let version = std::str::from_utf8(&output.stdout)
            .ok()
            .map(str::trim)
            .filter(|version| {
                version.split('.').count() >= 2
                    && version.split('.').all(|part| {
                        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
            .filter(|_| output.status.success())
            .ok_or(RDiscoveryError::InvalidVersion)?
            .to_owned();
        Ok(Self {
            executable,
            version,
        })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Attribute work to this executable, version, and process incarnation.
    pub fn identity(&self, incarnation: u64) -> RunnerIdentity {
        RunnerIdentity {
            name: self.executable.to_string_lossy().into_owned(),
            version: self.version.clone(),
            incarnation,
        }
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(directory: &Path, body: &str) -> PathBuf {
        let path = directory.join("R");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn discovers_r_and_records_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = executable(
            directory.path(),
            "[ \"$1\" = --vanilla ] && [ \"$2\" = --slave ] && [ \"$3\" = -e ] || exit 1\nprintf 4.5.1",
        );
        let installation =
            RInstallation::discover_on_path(Some(directory.path().as_os_str())).unwrap();
        assert_eq!(installation.executable(), path.canonicalize().unwrap());
        assert_eq!(installation.version(), "4.5.1");
        assert_eq!(
            installation.identity(7),
            RunnerIdentity {
                name: path.canonicalize().unwrap().to_string_lossy().into_owned(),
                version: "4.5.1".into(),
                incarnation: 7,
            }
        );
    }

    #[test]
    fn discovery_handles_missing_and_nonexecutable_r() {
        assert!(matches!(
            RInstallation::discover_on_path(None),
            Err(RDiscoveryError::NotFound)
        ));
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("R"), "not executable").unwrap();
        assert!(matches!(
            RInstallation::discover_on_path(Some(directory.path().as_os_str())),
            Err(RDiscoveryError::NotFound)
        ));
        assert!(matches!(
            RInstallation::from_executable(directory.path().join("missing")),
            Err(RDiscoveryError::Io(_))
        ));
    }

    #[test]
    fn rejects_failed_or_malformed_version_queries() {
        let directory = tempfile::tempdir().unwrap();
        for body in [
            "printf 4.5.1; exit 1",
            "printf garbage",
            "printf 4..1",
            "printf '4.5.1\\nsecret'",
            "printf '\\377'",
        ] {
            let path = executable(directory.path(), body);
            assert!(matches!(
                RInstallation::from_executable(path),
                Err(RDiscoveryError::InvalidVersion)
            ));
        }
    }
}
