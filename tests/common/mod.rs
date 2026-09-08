use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

use tempfile::TempDir;

#[cfg(target_os = "linux")]
pub mod process;

pub fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Owns a fresh project directory so tests cannot share source, cache, or output.
pub struct TestProject {
    directory: TempDir,
}

impl TestProject {
    pub fn new(source: &str) -> Self {
        let project = Self {
            directory: tempfile::tempdir().expect("create test project"),
        };
        project.write_source(source);
        project
    }

    pub fn root(&self) -> &Path {
        self.directory.path()
    }

    pub fn source_path(&self) -> PathBuf {
        self.root().join("slides.qmd")
    }

    pub fn write_source(&self, source: &str) {
        fs::write(self.source_path(), source).expect("write test source");
    }

    pub fn inspect(&self) -> CliOutput {
        Command::new(env!("CARGO_BIN_EXE_tractate"))
            .current_dir(self.root())
            .args(["inspect", "slides.qmd"])
            .output()
            .expect("run tractate inspect")
            .into()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CliOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl From<Output> for CliOutput {
    fn from(output: Output) -> Self {
        Self {
            status: output.status,
            stdout: String::from_utf8(output.stdout).expect("UTF-8 stdout"),
            stderr: String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        }
    }
}

pub struct Edit<'a> {
    pub name: &'a str,
    pub old: &'a str,
    pub new: &'a str,
}

/// Compares the initial source and every edit with a clean full-build oracle.
///
/// `update` may retain compiler state between calls. `full_build` must create
/// fresh state and outputs on each call. Both return comparable observations,
/// such as a semantic IR, rendered artifacts, or CLI output.
pub fn assert_matches_full_build<T: Debug + PartialEq>(
    initial: &str,
    edits: &[Edit<'_>],
    mut update: impl FnMut(&str) -> T,
    full_build: impl Fn(&str) -> T,
) {
    let mut source = initial.to_owned();
    assert_eq!(update(&source), full_build(&source), "initial source");

    for (index, edit) in edits.iter().enumerate() {
        let context = format!("after edit {} ({})", index + 1, edit.name);
        assert!(!edit.old.is_empty(), "empty edit target: {context}");
        assert_ne!(edit.old, edit.new, "edit must change source: {context}");
        assert_eq!(
            source.matches(edit.old).count(),
            1,
            "edit target must occur exactly once: {context}"
        );
        source = source.replacen(edit.old, edit.new, 1);
        assert_eq!(update(&source), full_build(&source), "{context}");
    }
}

#[test]
#[should_panic(expected = "after edit 1 (change heading)")]
fn oracle_catches_a_stale_intermediate_revision() {
    let initial = "# Original";
    let edits = [
        Edit {
            name: "change heading",
            old: "Original",
            new: "Changed",
        },
        Edit {
            name: "restore heading",
            old: "Changed",
            new: "Original",
        },
    ];

    assert_matches_full_build(initial, &edits, |_| initial.to_owned(), str::to_owned);
}
