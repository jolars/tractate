# Test helpers

The CLI integration test suite starts at `cli/main.rs` and imports the helpers
in `cli/common.rs` with `mod common;`:

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

On Linux, `cli/common/process.rs` uses `strace` to follow process creation and
exec calls, including failed attempts and descendants. Only the initial exec of
the program under test is excluded. A control test deliberately launches a child
shell to verify that the tracer detects it. Missing or unusable `strace` fails
the tests. The development environment and Linux CI install it. Other platforms
still run the inspection behavior and edit-sequence tests.

The `malformed-*.qmd` fixtures contain intentional YAML syntax errors and are
excluded from Panache formatting and linting.

## Stage 1 source fixtures

`cli/source_fixtures.rs`, included by the CLI integration tests, checks the
fixture corpus through inspection and Panache's typed syntax tree. These tests
verify the inputs needed by semantic lowering: metadata, ordered headings,
nested Markdown, math, code, option declarations, and source ranges. Inspection
of every case is also traced for process launches on Linux.

The internal lowering tests in `src/parser/tests.rs` assert the resulting IR
directly. They cover nested blocks and inlines, list markers and looseness,
attributes, links, math, raw content, UTF-8 source ranges, and the separation of
code from options and container prefixes. They also verify that unsupported
containers retain recognized descendants and that metadata and options remain
unresolved declarations. YAML rejected by Panache, including duplicate mapping
keys, retains its original source even when no structured YAML tree is
available. The existing inspection and process-tracing tests exercise the same
lowering path.

  | Fixture                      | Coverage                                                                                                                                                                                                                                                        |
  | ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `source-content.qmd`         | Title, subtitle, author, date, Unicode leading prose, level-one through level-three headings, paragraphs, emphasis, strong text, inline code, a link, nested bullet lists, an ordered list, inline and display math, ordinary R code, and an executable R cell. |
  | `title-only.qmd`             | Nonempty title metadata with no body.                                                                                                                                                                                                                           |
  | `empty-title.qmd`            | An empty title followed by a level-two heading.                                                                                                                                                                                                                 |
  | `no-title.qmd`               | No metadata or leading body, two level-two headings, and a level-three heading within the first slide.                                                                                                                                                          |
  | `leading-body.qmd`           | Leading prose and a list without metadata, followed by consecutive level-two headings.                                                                                                                                                                          |
  | `cell-options.qmd`           | The initial option vocabulary, YAML scalar and sequence values, isolated and named sessions, and a declared CSV input in `data/observations.csv`. Use `tests/fixtures` as the project root when exercising file inputs.                                         |
  | `document-defaults.qmd`      | Every supported default under `execute`, one cell with no local options, and another with explicit overrides, including a reset to the default session and an empty input list.                                                                                 |
  | `option-type-boundaries.qmd` | YAML Boolean spellings, string enums, default/isolated/named sessions, exponent and fractional dimensions, duplicate flow inputs, empty inputs and captions, and literal and folded Markdown captions.                                                          |
  | `invalid-option-values.qmd`  | Invalid values for the complete option vocabulary, including nulls, quoted booleans and numbers, wrong enum values, empty names, mixed input lists, and zero, negative, nonfinite, or unit-bearing dimensions.                                                  |
  | `malformed-frontmatter.qmd`  | An unclosed YAML sequence in title metadata.                                                                                                                                                                                                                    |
  | `malformed-options.qmd`      | An unclosed YAML sequence in a `#\|` label option.                                                                                                                                                                                                              |
  | `malformed-defaults.qmd`     | An unclosed YAML sequence in document execution defaults.                                                                                                                                                                                                       |
  | `invalid-option-types.qmd`   | Valid YAML containing a quoted boolean, a scalar input list, and a nonnumeric figure width.                                                                                                                                                                     |
  | `unknown-options.qmd`        | Unknown keys in both document defaults and cell options.                                                                                                                                                                                                        |
  | `duplicate-labels.qmd`       | The same label on cells in different slides and named sessions.                                                                                                                                                                                                 |

`slide-boundaries.qmd` adds all six heading levels, level-two headings nested in
quotes, lists, and fenced divs, heading-like code, a horizontal rule, and a
Setext heading followed by an ATX heading with no body text between them. Local
Panache directives preserve the Setext spelling and allow the intentional skip
from a level-one heading to a level-three heading within a slide.

`cli/slide_rules.rs` checks the MVP slide counts through the library summary and CLI.
The fixture expectations are five slides for `source-content.qmd`, one each for
`title-only.qmd` and `empty-title.qmd`, two for `no-title.qmd`, and three each
for `leading-body.qmd` and `slide-boundaries.qmd`. Inline cases cover missing,
null, empty, whitespace-only, quoted, and multiline titles; empty documents;
comments and definitions; leading blocks; and empty or consecutive headings. The
precise rules are in `DESIGN.md`. Explicit slide construction remains a separate
roadmap item.

Malformed YAML already fails inspection. Invalid option types, unknown keys, and
duplicate labels are syntactically valid, so inspection currently succeeds.
Their fixtures retain the declarations that semantic lowering must diagnose.
Likewise, the defaults fixture preserves both scopes without resolving them in
the test: the first cell should inherit document defaults, and local options in
the second should override the corresponding defaults. Type validation, option
resolution, and diagnostics belong to semantic lowering. The supported types,
defaults, scopes, invalidation classes, and behavioral acceptance cases are now
specified in the cell option contract in `DESIGN.md`. Execution and incremental
invalidation remain later roadmap items.

The option boundary tests use Panache's raw declarations, scalar styles, and
sequence nodes without coercing values or applying its R Markdown compatibility
resolver. They also check that option ranges slice the original QMD correctly,
including Unicode and prefixed multiline captions. Block scalar values retain
their header in panache-parser 0.29, so these tests check preservation rather
than pretending to decode captions. No fixture test currently proves type
rejection, effective default resolution, or execution invalidation. A local
`panache-ignore-format` directive preserves the deliberate Boolean spellings,
quote styles, and physical caption lines in `option-type-boundaries.qmd`.

The internal origin tests in `src/parser/tests/origins.rs` walk every semantic
node in all QMD fixtures with LF and CRLF line endings, including rejected
syntax. They verify that origins and code/option spans share the correct source
snapshot, that nested Unicode declarations retain precise locations, and that
origins survive edits and dropping the containing document. Additional cases
cover anonymous and empty sources, invalid UTF-8 ranges, synthesized autolink
labels, and a chain from generated text through a semantic derivation back to
QMD. The generated-text case tests provenance independently of a renderer.

The diagnostic tests in `src/parser/tests/diagnostics.rs` check the parser
adapter and retained diagnostic model. They cover stable codes independent of
upstream wording, owned messages, QMD ranges for malformed metadata and nested
cell options with Unicode and CRLF, multiple errors, empty spans, anonymous
sources, and snapshots retained across revisions. Related origins retain their
ordering, messages, and source chains, including generated spans. Error counts
exclude warnings and notes. The quoted-list case exercises only diagnostic
conversion: Panache 0.29 duplicates a quote marker in that CST, which still
prevents full semantic lowering for the reproducer and is tracked in `TODO.md`.

Local `panache-ignore-lint` directives surround intentionally invalid types, the
duplicate label, and the cell with no local options. Formatting remains enabled
for those regions, and the Rust tests still inspect them.
