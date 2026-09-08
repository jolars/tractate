use std::fs;
use std::process::Command;

use super::{CliOutput, TestProject};

/// Returns every process launch after the traced program's initial exec.
pub fn trace(project: &TestProject, program: &str, args: &[&str]) -> (CliOutput, Vec<String>) {
    let trace_file = tempfile::NamedTempFile::new().expect("create process trace");
    let output = Command::new("strace")
        .current_dir(project.root())
        .args(["-f", "-qq", "-e", "trace=%process", "-o"])
        .arg(trace_file.path())
        .arg("--")
        .arg(program)
        .args(args)
        .output()
        .expect("run strace; install strace or enter the devenv shell");
    let trace = fs::read_to_string(trace_file.path()).expect("read process trace");
    let mut launches: Vec<String> = trace
        .lines()
        .filter(|line| {
            let syscall = line.split_whitespace().nth(1).unwrap_or_default();
            [
                "execve(",
                "execveat(",
                "clone(",
                "clone3(",
                "fork(",
                "vfork(",
            ]
            .iter()
            .any(|name| syscall.starts_with(name))
        })
        .map(str::to_owned)
        .collect();

    // A missing or failed initial exec means tracing failed, not safe inspection.
    assert!(
        launches
            .first()
            .is_some_and(|line| line.contains("execve(") && line.ends_with("= 0")),
        "tracer did not start {program}:\n{trace}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    launches.remove(0);
    (output.into(), launches)
}

#[test]
fn tracer_detects_child_processes() {
    let project = TestProject::new("");
    let (output, launches) = trace(&project, "/bin/sh", &["-c", "/bin/sh -c ':'; :"]);

    assert!(output.status.success(), "{output:?}");
    assert!(
        launches.iter().any(|line| line.contains("execve(")),
        "tracer missed the child shell: {launches:?}"
    );
}
