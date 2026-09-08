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

## Stage 1 source fixtures

`source_fixtures/mod.rs`, included by the CLI integration tests, checks the
fixture corpus through inspection and Panache's typed syntax tree. These tests
verify the inputs needed by semantic lowering: metadata, ordered headings,
nested Markdown, math, code, option declarations, and source ranges. Inspection
of every case is also traced for process launches on Linux.

  | Fixture                     | Coverage                                                                                                                                                                                                                                                        |
  | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `source-content.qmd`        | Title, subtitle, author, date, Unicode leading prose, level-one through level-three headings, paragraphs, emphasis, strong text, inline code, a link, nested bullet lists, an ordered list, inline and display math, ordinary R code, and an executable R cell. |
  | `title-only.qmd`            | Nonempty title metadata with no body.                                                                                                                                                                                                                           |
  | `empty-title.qmd`           | An empty title followed by a level-two heading.                                                                                                                                                                                                                 |
  | `no-title.qmd`              | No metadata or leading body, two level-two headings, and a level-three heading within the first slide.                                                                                                                                                          |
  | `leading-body.qmd`          | Leading prose and a list without metadata, followed by consecutive level-two headings.                                                                                                                                                                          |
  | `cell-options.qmd`          | The initial option vocabulary, YAML scalar and sequence values, isolated and named sessions, and a declared CSV input in `data/observations.csv`. Use `tests/fixtures` as the project root when exercising file inputs.                                         |
  | `document-defaults.qmd`     | Defaults under `execute`, one cell with no local options, and another with explicit overrides.                                                                                                                                                                  |
  | `malformed-frontmatter.qmd` | An unclosed YAML sequence in title metadata.                                                                                                                                                                                                                    |
  | `malformed-options.qmd`     | An unclosed YAML sequence in a `#\|` label option.                                                                                                                                                                                                              |
  | `malformed-defaults.qmd`    | An unclosed YAML sequence in document execution defaults.                                                                                                                                                                                                       |
  | `invalid-option-types.qmd`  | Valid YAML containing a quoted boolean, a scalar input list, and a nonnumeric figure width.                                                                                                                                                                     |
  | `unknown-options.qmd`       | Unknown keys in both document defaults and cell options.                                                                                                                                                                                                        |
  | `duplicate-labels.qmd`      | The same label on cells in different slides and named sessions.                                                                                                                                                                                                 |

The slide cases are inputs for the rules in `DESIGN.md`: a nonempty title
creates a title slide, each level-two heading starts a content slide, and
nonempty leading body content creates a content slide. The tests currently
assert source structure; slide construction remains a separate roadmap item.

Malformed YAML already fails inspection. Invalid option types, unknown keys, and
duplicate labels are syntactically valid, so inspection currently succeeds.
Their fixtures retain the declarations that semantic lowering must diagnose.
Likewise, the defaults fixture preserves both scopes without resolving them in
the test: the first cell should inherit document defaults, and local options in
the second should override the corresponding defaults. Type validation, option
resolution, diagnostics, and invalidation rules belong to later roadmap items.

Local `panache-ignore-lint` directives surround intentionally invalid types, the
duplicate label, and the cell with no local options. Formatting remains enabled
for these fixtures, and the Rust tests still inspect those regions.
