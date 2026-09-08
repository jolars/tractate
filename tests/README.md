# Test helpers

Integration tests can import `mod common;` to use the helpers in
`common/mod.rs`:

- `fixture(name)` reads a UTF-8 fixture relative to the crate, independently of
  the test's working directory.
- `TestProject::new(source)` creates an isolated temporary project containing
  `slides.qmd`. The directory is removed when the project is dropped. Use
  `write_source` for successive revisions, and `root` for project-local inputs,
  caches, and outputs.
- `assert_matches_full_build(initial, edits, update, full_build)` compares the
  initial revision and every edit with a clean full build. Each named `Edit`
  replaces exactly one occurrence of its target and must change the source.
  Failures identify the revision and edit name.

The `update` closure retains the project or compiler state under test. The
`full_build` closure must create a fresh project or compiler for every call.
Return comparable observations from both closures: for example, a semantic IR,
rendered artifacts, or CLI status, stdout, and stderr. Include every output that
the behavior under test can affect.

The current inspection sequence rewrites a persistent project and compares its
CLI output with inspection in a new temporary project after each edit.
Inspection parses from scratch. When the incremental compiler exists, its
retained state can use the same assertion helper.

On Linux, `common/process.rs` uses `strace` to follow process creation and exec
calls, including failed attempts and descendants. Only the initial exec of the
program under test is excluded. A control test deliberately launches a child
shell to verify that the tracer detects it. Missing or unusable `strace` fails
the tests. The development environment and Linux CI install it. Other platforms
still run the inspection behavior and edit-sequence tests.

The `malformed-*.qmd` fixtures contain intentional YAML syntax errors and are
excluded from Panache formatting and linting.
