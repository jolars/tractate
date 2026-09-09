# Design: Incremental Computational Markdown

## Overview

This project is an incremental compiler for computational Markdown documents.

The initial target is presentations written in Quarto/Pandoc-style Markdown with
executable code blocks:

````markdown
---
title: Example
---

## Simulation

```{r}
x <- rnorm(1000)
hist(x)
```

## Results

The simulation illustrates ...
````

The compiler should provide:

- plain-text Markdown source;
- Quarto-like executable code-block syntax;
- incremental Markdown parsing;
- incremental code execution;
- live presentation preview;
- HTML/Reveal.js output;
- PDF output through Typst after the HTML execution path is established;
- no dependency on Quarto, knitr, Jupyter, or notebook files.

The Panache parser crate should be reused for parsing Pandoc/Quarto-style
Markdown, but this should otherwise be a separate project.

The central principle is:

> A source edit should invalidate only the work that actually depends on that
> edit.

Changing prose should not execute code. Changing one code cell should not rerun
unrelated cells. Changing one slide should not require rebuilding unrelated
slide markup.

--------------------------------------------------------------------------------

## Goals

### Primary goals

1. Use ordinary `.qmd`/Markdown files as the source format.

2. Support executable fenced blocks such as:

   ````markdown
   ```{r}
   plot(x)
   ```
   ````

3. Execute R directly, without knitr or Jupyter. The runner protocol should
   permit Python and Julia support later without changing compiler semantics.

4. Cache computation at cell/session granularity.

5. Incrementally parse and render changed document regions.

6. Provide fast live preview for presentations.

7. Support, in order:

   - Reveal.js/HTML for the MVP;
   - PDF via the Typst CLI after the MVP.

8. Make computation results backend-independent.

9. Preserve predictable execution semantics.

10. Build around a long-lived dependency graph rather than a batch rendering
    pipeline.

### Non-goals for the initial version

- Full Quarto compatibility.
- General notebook compatibility.
- `.ipynb` support.
- Reimplementing PDF layout.
- Automatic dependency inference between arbitrary variables in R/Python/Julia.
- Distributed computation.
- Interactive widgets.
- Hermetic execution.
- Interpreter-state snapshots.
- Python and Julia execution.
- PDF output.
- Supporting every Pandoc output format.
- Replacing Pandoc generally.

Hermetic execution, interpreter snapshots, Python, Julia, and PDF output are
deferred from the MVP, not rejected as longer-term features.

--------------------------------------------------------------------------------

# Architecture

The compiler has a pure incremental core and an effectful execution boundary:

```text
revisioned source, files, configuration, and tool identities
                            │
                            ▼
                     Panache parser
                            │
                            ▼
                    source semantic IR
                            │
                ┌───────────┴───────────┐
                ▼                       ▼
       computation plan          presentation IR
                │                       │
                ▼                       ├────────────► HTML / Reveal.js
       execution requests               │
                │                       └────────────► Typst ──► PDF
                ▼
       scheduler and runners
                │
                ▼
     provisional results/artifacts
                │
                ▼
       revision and key validation
                │
                ▼
          commit or discard
```

Parsing, semantic lowering, dependency analysis, and rendering preparation are
pure transformations. Starting processes, executing user code, reading volatile
external state, and committing artifacts are effects managed outside the pure
dependency graph. A dependency-tracking library such as Salsa may implement the
pure core, but tracked queries must not execute user code.

Every compilation input belongs to a monotonically increasing source revision.
Every execution request records that revision, its execution key, its required
predecessor-state key, and a unique attempt ID. A result may be committed only
if the request is still desired at publication time. Obsolete work may finish,
but it must not update session state, caches, artifacts visible to renderers, or
the preview.

Compilation should be driven by dependencies rather than phases that always
process the entire document.

A prose edit may therefore result in:

```text
text edit
  ↓
incremental parse
  ↓
one slide changed
  ↓
one rendered slide changed
```

with no execution work.

A code edit in an isolated cell might result in:

```text
cell changed
  ↓
execution node invalidated
  ↓
cell reruns
  ↓
figure changes
  ↓
containing slide invalidated
  ↓
HTML / Typst output updates
```

An incremental build and a clean full build must produce equivalent semantic and
rendered outputs when execution inputs are deterministic. The full build is the
correctness oracle; incrementality is an optimization over those semantics.

--------------------------------------------------------------------------------

# Source language

Use Pandoc/Quarto-style Markdown as understood by the Panache parser.

Executable cells use the Quarto distinction between ordinary fenced code and
executable code:

````markdown
```r
# literal code
```
````

versus:

````markdown
```{r}
# executable code
```
````

Support cell options using Quarto-style `#|` metadata:

````markdown
```{r}
#| echo: false
#| label: fig-example
#| fig-width: 8
plot(x)
```
````

Do not attempt complete Quarto option compatibility initially.

The semantic lowering layer owns slide boundaries; backends must not infer them
independently. The MVP uses these rules:

- A nonempty top-level YAML `title` scalar produces one title slide, before any
  content slides. Missing titles, YAML null values, and scalar values containing
  only whitespace produce no title slide. Quoted and multiline scalars count as
  title text; quoted `"null"` is text, while plain `null` is absent. Mappings
  and sequences do not supply title text. Other metadata cannot create a title
  slide.
- Each document-level heading of level two starts a content slide and belongs to
  that slide. Both ATX (`##`) and Setext (underlined) headings follow this rule.
  Consecutive headings, including headings with no text, each retain their
  slide.
- Nonempty body content before the first level-two heading forms one content
  slide, after the title slide if present. A body without level-two headings
  therefore forms at most one content slide. Metadata, blank lines, HTML
  comments, and link or footnote definitions do not create leading body content.
- Headings of other levels and headings nested inside quotes, lists, or fenced
  divs stay in their containing slide. Heading-like text inside code is code.
  Horizontal rules are body content and do not start additional slides.

An empty document has no slides. These are source rules: counting slides never
executes cells or depends on computation results. `tractate inspect` reports the
count, and `tests/cli/slide_rules.rs` locks down these cases. Explicit slide
construction belongs to semantic lowering.

Any additional supported boundary syntax must be defined by fixtures. Vertical
slide stacks and backend-specific boundary rules are deferred.

## Cell option contract

The following table defines the complete MVP option vocabulary. These are
Tractate semantics, independent of the rendering backend. Defaults are explicit
compiler values, not values inherited from an R installation or graphics device.
An option marked **Both** is supported in a cell's leading `#|` YAML mapping and
in the document's top-level `execute` mapping. **Cell** options are local only.

  | Option       | Supported type and values                                                                           | Built-in default             | Scope | Invalidation class                |
  | ------------ | --------------------------------------------------------------------------------------------------- | ---------------------------- | ----- | --------------------------------- |
  | `label`      | Nonempty string without whitespace or control characters.                                           | Absent (unlabeled identity). | Cell  | Semantic identity and references. |
  | `eval`       | Boolean.                                                                                            | `true`                       | Both  | Execution planning.               |
  | `echo`       | Boolean.                                                                                            | `true`                       | Both  | Rendering.                        |
  | `include`    | Boolean.                                                                                            | `true`                       | Both  | Rendering.                        |
  | `results`    | String enum: `markup` or `hide`.                                                                    | `markup`                     | Both  | Rendering.                        |
  | `session`    | Nonempty string without whitespace or control characters: `default`, `isolated`, or a session name. | `default`                    | Both  | Execution-graph structure.        |
  | `cache`      | Boolean.                                                                                            | `true`                       | Both  | Result-reuse policy.              |
  | `inputs`     | Sequence of nonempty file path strings without NUL characters.                                      | `[]`                         | Both  | Execution key and file watching.  |
  | `fig-width`  | Finite positive YAML number, in inches.                                                             | `7`                          | Both  | Execution and captured artifacts. |
  | `fig-height` | Finite positive YAML number, in inches.                                                             | `5`                          | Both  | Execution and captured artifacts. |
  | `fig-cap`    | String containing Markdown, including multiline text.                                               | Absent (no caption).         | Cell  | Rendering and references.         |

### Values and resolution

Use YAML 1.2 core scalar types. Unquoted `true`, `True`, and `TRUE` are true;
`false`, `False`, and `FALSE` are false. Quoted booleans, `yes`/`no`, numeric
booleans, and sequences of line numbers do not satisfy a Boolean option. Quoted
and block scalars are strings. Plain numbers and booleans do not become strings
for `label`, `session`, `inputs`, or `fig-cap`. Enum values and names are case
sensitive. Names are preserved exactly and are not trimmed into validity.

Figure dimensions accept YAML integers and floats, including exponent notation,
and must remain finite and strictly positive when converted to the runner's
numeric representation. Zero, negative values, infinities, NaN, quoted numbers,
unit strings such as `4in`, and dimension vectors are errors. Numerically equal
dimensions such as `7`, `7.0`, and `7e0` have the same semantic value.

An omitted option inherits its document default, then its built-in default. An
explicit local value replaces the entire default, including an `inputs`
sequence; sequences are not appended. `inputs: []` clears inherited inputs. An
empty or whitespace-only `fig-cap` means no caption. Literal and folded captions
follow YAML folding and chomping before Markdown lowering. Explicit null,
including an empty `key:` value, is a type error for every option; it does not
mean inheritance or reset.

The `execute` value must be a mapping. Validate defaults even when a local
override would hide them. Reject unknown option keys, keys in the wrong scope
(including `execute.label` and `execute.fig-cap`), and duplicate keys within one
mapping. A local override of a document default is not a duplicate. Explicit
labels must be unique across the document, including across slides and sessions.
These errors must retain the declaration's QMD origin for semantic diagnostics.
Unrelated document metadata such as `title` is outside this option vocabulary.

MVP option names use exactly the lowercase, hyphenated spellings in the table.
Only leading `#|` YAML supplies cell options. R Markdown inline options and
labels, dotted aliases such as `fig.width`, executable YAML tags such as
`!expr`, other explicit tags, anchors, aliases, and merge keys are unsupported
and must be diagnosed during lowering. Parsing them must never evaluate
expressions. Ordinary fenced code does not supply options, even when its body
contains `#|`.

### Execution and display behavior

One stateful session per language is the default. `session: default` explicitly
selects it, including when overriding a named document default. Other names
select independent stateful sessions scoped to the document and language.
`default` and `isolated` are reserved, case-sensitive values.
`session: isolated` requests independent execution:

````markdown
```{r}
#| session: isolated
summary(read.csv("data.csv"))
```
````

`eval: false` excludes a cell from execution and from its session's cumulative
state chain. It requires no cached result and displays no prior result or figure
caption. Its source may still appear. `echo` controls source visibility, and
`include: false` suppresses all cell presentation: source, textual results,
figures, and captions. Neither option disables evaluation or output capture.
Compiler and execution diagnostics remain available even for hidden cells.

`results: markup` presents captured stdout, stderr, and warnings as escaped text
in event order. `results: hide` suppresses that textual output while retaining
figure display. Neither changes capture or source visibility. Raw Markdown/HTML
insertion (`asis`) and regrouping output (`hold`) are outside the MVP. A caption
applies to each displayed figure from the cell and has no effect if there is no
figure. Captions are lowered by the compiler, not passed to the runner.

`inputs` accepts block and flow sequences. Each path denotes one literal file;
there is no glob, environment-variable, or tilde expansion. Relative paths
resolve under the project root, not the current shell directory; absolute paths
retain their meaning. The normalized paths form a dependency set, so duplicate
paths and sequence reordering do not change the execution key. File identity and
content digests enter the key. Apply the project path policy below without
erasing symlink semantics. Missing, unreadable, or non-file inputs produce
diagnostics and cannot silently become an empty dependency set. Reading and
watching files belongs to dependency discovery, not syntax inspection.

`cache: true` permits reuse when the complete execution contract is valid.
`cache: false` disables reuse across executions and process restarts. It does
not repeatedly execute a current cell on unrelated source edits. When a volatile
stateful cell executes, its attempt identity changes the cumulative state key,
preventing reuse of results from its previous suffix state.

### Invalidation obligations

Compare resolved semantic values, not YAML spelling or whole option mappings.
Changing a document default affects only cells whose effective value changes.
The classes above identify which dependencies must be reconsidered:

- **Rendering** changes invalidate the cell's presentation and affected slide
  fragments. `echo`, `include`, `results`, and `fig-cap` never enter execution
  keys or require code to run when the required result is already valid.
- **Semantic identity and references** changes rebuild identity and reference
  lookups and their rendered consumers. A `label` rename does not by itself
  change the computation key. Caption edits also invalidate rendered reference
  text where it depends on the caption, without changing the figure artifact.
- **Execution planning** changes add or remove a cell from its plan. Changing
  `eval` rebuilds the affected stateful suffix because its predecessor chain
  changes. The Boolean itself is not a computation input for an enabled cell.
- **Execution-graph structure** changes move a cell between session chains or
  isolation. Reconsider the old and new stateful suffixes. Session context and
  required predecessor-state identities participate in execution validity.
- **Result-reuse policy** changes make the scheduler reconsider reuse; the
  `cache` Boolean is not a computation input. Switching to `false` schedules a
  fresh evaluation for an enabled cell. Switching to `true` may retain a valid
  current result, but does not retroactively make old volatile attempts
  reusable.
- **Execution key and file watching** changes update input dependencies and
  watch targets. Changed paths or file contents invalidate the consuming cell
  and its stateful suffix. Equivalent dependency sets do not.
- **Execution and captured artifacts** changes put both effective figure
  dimensions into the execution key for every enabled cell. This conservative
  rule covers raster and vector devices, whose layout can both depend on size. A
  dimension edit invalidates the cell and its stateful suffix even if a prior
  execution happened not to emit a figure.

Invalidation does not itself authorize execution. Disabled cells never run, and
`--no-execute` must report an unavailable required result if a newly required
key cannot be reused. Changes to `label`, `echo`, `include`, `results`, or
`fig-cap` must preserve valid computation results and artifacts, including for
`cache: false` cells. Stateful replay requirements still apply when execution is
needed; output reuse alone does not materialize interpreter state.

### Panache boundary and acceptance cases

Reuse Panache's `ExecutableCell::option_declarations()` and the embedded YAML
CST for values, styles, declaration sources, and host-document ranges. A
`cooked_value()` alone cannot distinguish a Boolean from a quoted string or
represent a sequence. In panache-parser 0.29, block scalar values also retain
their scalar header and still need context-aware YAML decoding during lowering.

Panache's `resolved_options()` is a broader compatibility helper: it
canonicalizes case and dotted keys, gives inline options precedence over
hashpipe options, and reports duplicates within the winning source as ambiguous.
Tractate must validate raw declarations against its own scopes and vocabulary
before resolving defaults. Panache's separate Quarto schema linter and formatter
conversion tables describe Quarto compatibility, not Tractate's execution or
invalidation contract; they are not part of the panache-parser dependency.

The source fixtures and `tests/cli/source_fixtures.rs` preserve the inputs for
this contract. `cell-options.qmd` covers every option, `document-defaults.qmd`
covers inheritance and overrides, and `option-type-boundaries.qmd` adds YAML
Boolean spellings, exponent dimensions, flow inputs, empty inputs and captions,
and literal and folded captions. `invalid-option-types.qmd` and
`invalid-option-values.qmd` preserve wrong types, nulls, invalid enums, empty
names, mixed input lists, and invalid dimensions. Syntax inspection continues to
accept syntactically valid declarations; type validation and diagnostics belong
to semantic lowering.

The incremental compiler and fake-runner stages must turn the following edit
cases into behavioral tests, each compared with a clean full build:

  | Edit                                                               | Required observation                                                                                                                |
  | ------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
  | Rename a label or edit `echo`, `include`, `results`, or `fig-cap`. | Update presentation/references as applicable; preserve result keys and artifacts; execute zero cells, also with `cache: false`.     |
  | Disable a stateful cell with `eval: false`.                        | Remove it from the plan; invalidate its suffix; require no result for the disabled cell.                                            |
  | Re-enable a cell or move it to another `session`.                  | Rebuild the affected chains and reuse only results with valid session/predecessor identities.                                       |
  | Change `cache` from `true` to `false`.                             | Execute the enabled cell freshly when authorized; change volatile predecessor identity for its suffix; leave unrelated cells valid. |
  | Change an input path or its file contents.                         | Update watches; invalidate its consumers and their stateful suffixes.                                                               |
  | Reorder or repeat identical input paths.                           | Keep dependency identities and results unchanged.                                                                                   |
  | Change either figure dimension.                                    | Invalidate computation and affected suffixes for raster and vector figures.                                                         |
  | Rewrite `7` as `7.0`, or change only YAML quoting on a string.     | Preserve effective option values and computation.                                                                                   |
  | Edit an inherited document default.                                | Apply its class only to cells whose effective value changes; preserve local overrides.                                              |
  | Make a required execution key unavailable under `--no-execute`.    | Report the missing result without starting a runner.                                                                                |

--------------------------------------------------------------------------------

# Document model

Parsing should produce a semantic document model on top of the Panache CST. The
compiler should use distinct layers rather than mutating the source model as
results arrive:

```text
Panache CST
    ↓
source semantic IR
    ↓
evaluation plan plus result references
    ↓
backend-independent presentation IR
    ↓
HTML IR or Typst IR
```

Every semantic node should retain an origin in the QMD source. Derived and
generated nodes should retain an origin chain when possible so that diagnostics
can be mapped back through every layer.

At minimum:

```rust
struct Document {
    metadata: Metadata,
    blocks: Vec<Block>,
}

enum Block {
    Heading(Heading),
    Paragraph(Paragraph),
    Code(CodeBlock),
    ExecutableCell(Cell),
    Figure(Figure),
    Table(Table),
    Raw(RawBlock),
    // ...
}
```

The example is schematic. The real representation must preserve nested block and
inline structure, attributes, source origins, and unsupported syntax. Lists,
callouts, block quotes, notes, and similar constructs cannot be flattened
without losing rendering semantics.

The current source IR is internal and owns its data independently of Panache.
Inspection consumes this model. Every semantic node retains an `Origin`,
including nested blocks and inlines, attributes, list items, code and cell
declarations, YAML entries and properties, and preserved unsupported syntax.
Recognized descendants remain available to inspection. Display math can occur
inside a paragraph without losing its display mode or surrounding prose.

A source origin holds a `SourceSpan`: a validated half-open UTF-8 byte range and
a shared, immutable `SourceFile` snapshot. The snapshot stores the supplied path
and original text once. Creating it does not read or canonicalize a path.
String-only inspection uses an anonymous snapshot. Clones share snapshot
identity; a new snapshot is distinct even at the same path, so retained nodes
continue to resolve against their own source after an edit. Snapshot identity is
not semantic node identity or a computation cache key. Discontiguous code
segments and option key/value spans retain the same file snapshot.

Metadata and cell options remain declarations at this stage. Their order, YAML
shape and scalar style, and source ranges are retained without resolving
defaults or applying the option contract. When Panache rejects YAML before
building its structured tree, including duplicate mapping keys, the IR retains
the rejected source as unsupported syntax and inspection reports the parser
errors. Option resolution and semantic diagnostics remain subsequent work.

Presentations should additionally expose slides explicitly:

```rust
struct Presentation {
    metadata: Metadata,
    slides: Vec<Slide>,
}

struct Slide {
    id: SlideId,
    blocks: Vec<BlockId>,
}
```

Slide identity should remain stable across ordinary edits whenever possible.

Do not make backend-specific HTML or Typst structures part of the primary
document IR.

--------------------------------------------------------------------------------

# Incremental compilation

The preview process should maintain an in-memory build graph across source
revisions. Persistent execution results and artifacts live in the filesystem
cache; the dependency graph itself need not survive process restarts initially.

Conceptually:

```text
SourceFile
    ↓
SyntaxNode
    ↓
SemanticBlock
    ↓
Slide
    ↓
RenderedSlide
```

Executable blocks additionally participate in:

```text
ExecutableCell
    ↓
ExecutionResult
    ↓
Artifact
    ↓
RenderedSlide
```

A dependency-tracking system such as Salsa is appropriate for this pure graph,
but the exact implementation should remain an internal choice. Source ranges are
locations, not stable graph keys.

The important invariant is:

> A pure derived value should be reused exactly when its declared inputs remain
> valid. Effectful execution may be reused only under the explicit
> cache-validity contract.

The compiler should be able to explain important decisions. At minimum, a
diagnostic or inspection mode should report why a cell ran, replayed, hit the
cache, or was invalidated. Invisible cache policy is too difficult to debug.

--------------------------------------------------------------------------------

# Code execution

The project owns code execution directly.

Do not delegate execution to:

- knitr;
- Jupyter;
- Quarto;
- notebook kernels.

Language support should be implemented through runner adapters.

Conceptually:

```rust
trait Runner {
    fn start(&mut self, config: SessionConfig) -> Result<()>;

    fn execute(&mut self, request: ExecutionRequest) -> Result<ExecutionOutcome>;

    fn interrupt(&mut self) -> Result<()>;

    fn reset(&mut self) -> Result<()>;

    fn stop(&mut self) -> Result<()>;
}
```

Exact APIs may differ, especially because execution and cancellation are
asynchronous. The lifecycle and state transitions are part of the runner
contract even if the final Rust trait has a different shape.

The initial real runner is R. Before it, a deterministic fake runner should be
used to test scheduling, invalidation, cancellation races, and artifact commits
without depending on an interpreter.

Later languages:

1. Python
2. Julia

Later runners could include:

- shell;
- Rust;
- JavaScript;
- arbitrary user-defined executors.

--------------------------------------------------------------------------------

# Execution results

Execution should produce a backend-independent result representation.

For example:

```rust
struct CellResult {
    events: Vec<OutputEvent>,
    artifacts: Vec<Artifact>,
    provenance: ExecutionProvenance,
}
```

with:

```rust
enum OutputEvent {
    Stdout(String),
    Stderr(String),
    Warning(Diagnostic),
    Display(DisplayBundle),
}

struct DisplayBundle {
    representations: Vec<Representation>,
    metadata: DisplayMetadata,
}
```

Events retain their original order. A display bundle may contain several
representations of one value, such as text, HTML, SVG, PNG, or structured table
data. A renderer selects the best representation it supports. Large binary
representations should be stored as artifacts and referenced by ID rather than
held as unbounded in-memory byte vectors.

An execution outcome must distinguish success, failure, and cancellation and
carry structured diagnostics, duration, runner identity, and the effective
execution key. Failed and cancelled outcomes must never be published as
successful cache entries.

This should be deliberately smaller and simpler than the Jupyter messaging
protocol, but it still needs enough structure to preserve event order and
backend choices.

Its purpose is to let:

```text
R / Python / Julia
        ↓
   CellResult
      ↙   ↘
   HTML   Typst
```

share the same execution results.

Raw HTML is not universally backend-independent. It requires an explicit trust
policy and a fallback representation for non-HTML backends.

--------------------------------------------------------------------------------

# Language bridges

Each language requires a small runtime shim to capture rich output. The shim
must communicate over a framed control channel that cannot be confused with user
stdout or stderr. Requests and events carry session, cell, attempt, and sequence
identifiers.

## R

Initially support:

- stdout;
- stderr;
- warnings/messages;
- base graphics;
- ggplot2 graphics.

Graphics can be captured by opening a controlled graphics device around cell
execution.

## Python

Later support:

- stdout;
- stderr;
- exceptions;
- matplotlib figures;
- basic rich values/tables.

Avoid depending on IPython.

Provide a small injected runtime module if necessary.

## Julia

Later support:

- stdout;
- stderr;
- exceptions;
- `display`;
- common plotting systems where practical.

Use Julia's display mechanisms rather than notebook infrastructure.

Python and Julia are post-MVP runners. Their descriptions constrain the common
runner protocol but do not enlarge the first implementation milestone.

--------------------------------------------------------------------------------

# Runner lifecycle

A runner adapter owns an interpreter process and its process tree. It must
define:

- executable discovery and version reporting;
- runtime-shim negotiation;
- working directory and inherited environment;
- startup and per-cell timeouts;
- whether stdin is disabled or explicitly connected;
- graceful interruption and forced termination;
- output and artifact-size limits;
- crash detection, cleanup, and restart;
- whether a session remains usable after each failure mode.

An interrupted, crashed, or protocol-invalid stateful runner is poisoned unless
the adapter can prove that its state remains at a known cumulative state key.
Recovery otherwise requires a fresh process and prefix replay. Killing work is
not sufficient by itself; the scheduler must also reject any late result from
the cancelled attempt.

The initial platform target is Linux and macOS. Windows support is deferred
until interruption and descendant-process cleanup have an explicit, tested
implementation.

--------------------------------------------------------------------------------

# Execution semantics

The MVP supports stateful and isolated execution. One stateful session per
language is the default because it matches the expectation that later cells can
use values defined by earlier cells. Named sessions create independent stateful
chains. `session: isolated` opts a cell into independent execution.

Cells in a stateful session are ordered by their semantic document order.
Moving, inserting, or removing a cell changes the cumulative chain from that
point onward. Different sessions have no implicit data dependencies, although
their arbitrary filesystem side effects may still conflict.

## Isolated cells

An isolated cell executes independently.

```text
A

B

C
```

Changing `B` invalidates only `B`.

This provides the strongest caching guarantees. Isolated cells may run in
parallel subject to configured resource limits, but Tractate cannot prove that
two arbitrary cells do not contend for an undeclared external resource.

A cache key should include at least:

```text
language and interpreter identity
Tractate and runtime-shim versions
source
execution options
declared input paths and content digests
relevant environment variables
working-directory semantics
operating system and architecture where relevant
package or environment lock digest when available
cache schema version
```

--------------------------------------------------------------------------------

## Stateful sessions

Cells share interpreter state by default within their language:

```text
A → B → C → D
```

For example:

````markdown
```{r}
#| session: analysis
x <- rnorm(1000)
```

```{r}
#| session: analysis
mean(x)
```
````

Changing `C` invalidates `C` and all subsequent cells in that session.

Do not initially attempt to infer that `D` is independent of `C`.

Conservative forward invalidation gives understandable and correct semantics.
The scheduler must distinguish logical invalidation from physical execution: an
unchanged prefix can remain logically valid even when it must be replayed to
reconstruct interpreter state.

--------------------------------------------------------------------------------

# Stateful cache model

For a stateful session, use cumulative execution identities:

```text
state_0 = hash(environment)

state_1 = hash(state_0, cell_A)
state_2 = hash(state_1, cell_B)
state_3 = hash(state_2, cell_C)
```

This makes invalidation straightforward.

However, cached output is not equivalent to cached interpreter state.

Every live runner records the cumulative key of the interpreter state it has
actually materialized. It may execute the next cell directly only when that key
equals the cell's required predecessor-state key. If not, the MVP must start a
fresh process and replay the required prefix from `state_0` before executing the
invalidated suffix.

For example, after executing `A → B → C → D`, editing `C` does not permit the
old process to execute the new `C`: the effects of the old `C` and `D` are still
present. Without a snapshot, correct recovery is:

```text
reset → replay A → replay B → execute new C → execute D
```

The cached outputs of `A` and `B` can remain valid, but their code still runs
during replay. Long-lived interpreters avoid startup and replay when their
materialized key already matches the required state; they do not provide
rollback. Persistent interpreter snapshots may be investigated later but are not
part of the MVP.

If replayed prefix code observes time, randomness, the network, or other
undeclared state, its reconstructed interpreter state may disagree with its
cached output. Tractate cannot correct this automatically. Authors must make
such code repeatable, isolate it, or disable caching. The preview should explain
when physical replay occurred even though a cell remained logically valid.

--------------------------------------------------------------------------------

# Side effects and cache correctness

Arbitrary code makes perfect cache correctness impossible without additional
information.

Examples include:

```r
read.csv("data.csv")
Sys.time()
runif(10)
```

or:

```python
requests.get(...)
open("data.csv")
random.random()
```

The system should therefore distinguish between:

- source dependencies known to the compiler;
- user-declared external inputs;
- inherently volatile computation.

The MVP supports declarations such as:

```markdown
#| inputs:
#|   - data/raw.csv
```

or equivalent metadata.

Those files become part of the cell cache key.

Also provide an explicit mechanism such as:

```markdown
#| cache: false
```

for volatile cells.

`cache: false` prevents reuse across executions and process restarts; it does
not cause an unrelated prose edit to schedule the cell. When a volatile stateful
cell does execute, its attempt identity enters the cumulative state key so that
downstream results from a previous state cannot be reused.

Changing a declared input invalidates its consumers and the appropriate stateful
suffix. Undeclared file, environment, network, clock, random, and process
dependencies remain the author's responsibility. Direct filesystem writes are
side effects, not managed artifacts, unless a later explicit output declaration
adopts them.

Do not claim hermetic execution unless a future sandboxed execution mode
actually provides it.

--------------------------------------------------------------------------------

# Artifact store

Generated figures and other binary outputs should live in a content-addressed
artifact store.

For example:

```text
.tractate/
  cache/
    objects/
      ab/
        abcdef...
```

References from execution results should use artifact IDs rather than temporary
filenames.

Benefits include:

- deduplication;
- stable caching;
- backend reuse;
- atomic replacement;
- easy garbage collection.

Objects are immutable and verified by content digest when read. An execution
attempt first writes provisional objects and then atomically commits a result
record that references them after the revision check succeeds. Partial writes,
cancelled attempts, and process crashes must not create valid result records.

The store needs a schema version, inter-process locking or an equivalent
single-writer rule, and garbage-collection roots for current results and active
builds. Corrupt or incompatible entries should be treated as cache misses, not
fatal compiler errors. A project-local cache is sufficient initially; a shared
global store can be considered later.

--------------------------------------------------------------------------------

# Presentation rendering

Presentations are the initial document type.

The compiler should expose slides as first-class semantic nodes.

This gives a useful incremental unit:

```text
SlideId
  ↓
HTML fragment

SlideId
  ↓
Typst module
```

--------------------------------------------------------------------------------

# HTML / Reveal.js backend

Reveal.js should be the first renderer.

Each slide maps naturally to a `<section>`.

For example:

```text
Slide 17
  ↓
<section data-slide-id="...">
    ...
</section>
```

During live preview, an edit to slide 17 should send only its new rendered
representation to the browser.

The patch protocol carries the source revision and stable slide ID. The client
applies patches in revision order, replaces the corresponding DOM subtree, and
asks Reveal.js to resynchronize the affected slide. Structural changes such as
slide insertion, removal, reordering, or nesting may require deck-level
synchronization. A full-reload message is the correctness fallback whenever a
local patch cannot preserve deck state.

The browser should preserve:

- current slide;
- current fragment/overlay position where possible;
- scroll/focus state;
- presenter state where practical.

Global changes such as themes or Reveal configuration may require broader
invalidation.

The initial HTML output is a directory with pinned or bundled Reveal assets and
a content manifest. The design should not depend on an unversioned CDN. Raw HTML
and SVG originating in source or execution results must follow the preview trust
policy.

--------------------------------------------------------------------------------

# Typst / PDF backend

Do not implement a PDF renderer.

PDF output follows the R and Reveal MVP; it is not required to validate the
initial compiler and execution architecture.

Generate Typst and use Typst for layout and PDF compilation.

Conceptually:

```text
Presentation IR
      ↓
Typst representation
      ↓
Typst incremental compiler
      ↓
PDF
```

Touying is a reasonable initial presentation layer, but the backend should avoid
unnecessarily coupling the core compiler to Touying-specific concepts.

--------------------------------------------------------------------------------

# Generated Typst representation

Prefer generated modules over one monolithic generated file.

For example:

```text
.tractate/build/typst/
  main.typ
  config.typ
  slides/
    0001.typ
    0002.typ
    0003.typ
  artifacts/
    ...
```

`main.typ` may contain:

```typst
#include "config.typ"
#include "slides/0001.typ"
#include "slides/0002.typ"
#include "slides/0003.typ"
```

A source edit to slide 2 should normally rewrite only:

```text
slides/0002.typ
```

Typst can then perform its own incremental layout invalidation.

Generated files should be replaced atomically so that Typst never observes a
partially written module. The generated tree must define a Typst project root
that can read its modules and copied or linked artifacts without granting
unintended access outside that root.

--------------------------------------------------------------------------------

# Typst integration strategy

## First PDF implementation

Use a long-running `typst watch` process.

Advantages:

- minimal integration work;
- preserves Typst's incremental compilation state;
- easy to inspect generated `.typ` files;
- isolates the compiler from Typst internals.

## Later

Consider embedding Typst's Rust compiler directly.

Potential benefits:

- virtual generated files;
- no filesystem synchronization;
- direct diagnostics;
- cancellation;
- tighter incremental scheduling;
- source-map integration.

Do not begin with embedded Typst unless the CLI/watch approach proves
insufficient. Embedding also couples Tractate to Typst's Rust API and MSRV. It
requires an explicit decision to raise Tractate's Rust 1.89 baseline, pin an
older Typst release, or isolate the embedded backend in a separately compiled
package. The CLI backend preserves the current MSRV and remains the default
until measured limitations justify that change.

--------------------------------------------------------------------------------

# Source mapping

Diagnostics should ultimately point to the original QMD source rather than
generated Typst.

Maintain mappings such as:

```text
QMD source range
    ↕
semantic node
    ↕
generated Typst range
```

This should allow a Typst error in generated slide 12 to be reported against the
corresponding Markdown construct where possible.

Source-map support should influence the IR/render architecture early even if
comprehensive diagnostics come later.

The internal `Origin` type supports a source span or a derivation with an
operation name, a parent origin, and an optional generated span. Semantic
transformations without physical output use a derivation without a span;
generated nodes add their output file and range. Each step retains its parent,
and `source_span()` follows the complete chain to the initiating QMD location.
For example, a title metadata value can remain the source origin through a
title-slide derivation and a generated Typst span. Synthesized autolink labels
already use a derivation from the link's source text. Backend emission and
generated-offset lookup will consume these origins when the backends exist.

Diagnostics should have a severity, message, primary origin, related origins,
and stable code. Runner failures whose internal stack frames cannot be mapped to
QMD should still point to the executable cell that initiated them. Generated
backend diagnostics must never expose a generated path as the only actionable
location when a source origin is known.

--------------------------------------------------------------------------------

# Project and file model

The initial compilation unit is one QMD file within a project root. The root
defaults to the source file's parent directory and may later be overridden by
configuration. All relative document options, declared inputs, resources, and
outputs resolve under a single documented path policy. R runners use this
project root as their working directory.

The compiler input model must include more than QMD text. Presentations may
depend on:

- images, video, and other media;
- included Markdown or raw files;
- stylesheets, scripts, themes, and templates;
- fonts and bibliography files;
- declared computational inputs;
- Reveal.js assets;
- Typst modules, packages, fonts, and tool identity;
- generated execution artifacts.

Each discovered file becomes a graph input and, in preview, a watch target.
Paths should be normalized without erasing symlink or case-sensitivity semantics
needed by the host platform. Missing or unreadable inputs produce diagnostics
and remain watched where the platform permits.

Filesystem watching must tolerate atomic-save renames, duplicate events, and
coalesced edits. Cache and build directories must be excluded to prevent output
feedback loops. A disk-based preview sees saved content only; future editor
integration may provide an unsaved in-memory source overlay without changing
compiler semantics.

--------------------------------------------------------------------------------

# Cross-references

Some document features introduce dependencies across slides:

- citations;
- figure numbering;
- equation numbering;
- references;
- table numbering;
- section numbering.

These should be represented explicitly in the dependency graph rather than
forcing unconditional whole-document rendering.

For example:

```text
figure label
      ↓
reference index
      ↓
slides containing references
```

Changing a figure number may therefore invalidate several slides, but only those
depending on it.

Global metadata and themes may legitimately invalidate the entire presentation.

Incremental compilation does not mean every change must be local; it means
invalidation should correspond to actual dependencies.

--------------------------------------------------------------------------------

# Preview server

Provide a command such as:

```sh
tractate preview slides.qmd
```

The preview process owns the long-lived compilation state:

```text
filesystem watcher
      ↓
incremental source update
      ↓
parser / compiler graph
      ↓
execution scheduler
      ↓
renderer
      ↓
preview client
```

Invoking `preview` explicitly authorizes execution. A `--no-execute` mode
renders only source and reusable cached results. Merely opening or parsing a
document never starts a runner.

In preview, a missing result under `--no-execute` is shown as unavailable. A
one-shot `render --no-execute` fails if a required result is absent, because it
cannot produce the requested complete output.

The system should support cancellation and expose each cell as one of:

- pending;
- running;
- succeeded;
- failed;
- cancelled;
- unavailable because execution is disabled and no valid result is cached;
- stale last-known-good.

If the user edits a cell several times while an old computation is still
running, obsolete work should be cancelled when practical and its result
discarded if it completes. A preview may retain the last successful result after
a failure, but it must mark that result as stale and display the current
diagnostic. It must not assemble one apparently successful deck from mutually
incompatible source revisions.

Syntax or option errors must not execute ambiguously parsed cells. Preview may
render a recoverable source tree, but affected prior results remain visibly
stale; one-shot rendering reports failure.

--------------------------------------------------------------------------------

# Build mode

Also provide one-shot compilation:

```sh
tractate render slides.qmd
```

Potential targets:

```sh
tractate render slides.qmd --to html
tractate render slides.qmd --to pdf
```

Build mode should reuse the same compiler and execution-cache infrastructure as
preview mode. `render` explicitly authorizes execution; `--no-execute` forbids
new execution. A failed required cell makes the command unsuccessful. Failure in
one stateful session blocks its suffix, while unrelated sessions may finish to
provide complete diagnostics.

One-shot orchestration should be predictable, but the command must not claim
deterministic or reproducible output when user code observes undeclared state.

--------------------------------------------------------------------------------

# Proposed module structure

Keep the compiler library and CLI in one Cargo package. Put compiler behavior in
the library target, use internal modules as boundaries, and keep the binary
target thin:

```text
src/
  document/       semantic IR, origins, IDs, and diagnostics
  parser/         Panache integration and semantic lowering
  compiler/       pure dependency graph and invalidation
  execution/      plans, scheduler, cache, artifacts, and runner protocol
  runners/r/      R runtime bridge
  render/html/    Reveal.js output
  render/typst/   presentation IR to Typst, after the MVP
  preview/        watcher, server, and client protocol
  lib.rs          reusable compiler facade
  main.rs         thin CLI boundary
```

Exact module boundaries should evolve as implementation experience accumulates.
Do not split them into packages without an explicit change to the publication,
MSRV, or dependency model.

--------------------------------------------------------------------------------

# Stable identities

Incrementality depends heavily on retaining identities across edits.

Do not conflate three different identities:

  | Identity                         | Purpose                                           |
  | -------------------------------- | ------------------------------------------------- |
  | `NodeId`, `SlideId`, or `CellId` | track a semantic entity across revisions          |
  | `ExecutionKey`                   | decide whether a computation result can be reused |
  | `ArtifactId`                     | address immutable output bytes                    |

Slides and executable cells should have stable semantic IDs wherever possible.

Explicit labels are ideal:

````markdown
```{r}
#| label: fig-regression
...
```
````

Labels are document-scoped and unique. Duplicate labels are errors. Changing a
label may change semantic identity and references without changing the cell's
execution key.

For unlabeled nodes, derive live-session identities from structural context and
matching against the preceding revision rather than byte offsets alone. Matching
identical nodes can be ambiguous; reuse must remain a best effort and must never
change semantics.

Moving an isolated cell may preserve computation through its content-derived
execution key even if its semantic ID changes. Moving a stateful cell changes
its predecessor chain and therefore its cumulative state key.

Unlabeled semantic IDs need not survive a process restart initially. Persistent
cache reuse is based on execution keys, not database-local or source-position
node IDs.

--------------------------------------------------------------------------------

# Execution scheduler

Execution should happen independently of rendering and outside pure dependency
queries.

The scheduler receives desired execution requests, resolves their dependencies,
and controls process and resource limits.

For the initial version:

```text
isolated cells:
    execute independently

stateful session:
    reconstruct the required prefix, then execute the suffix sequentially

different sessions:
    may execute concurrently
```

This allows parallel execution across unrelated cells or sessions, but
parallelism is bounded and configurable. Arbitrary cells can still conflict
through undeclared filesystem or network side effects; scheduling independence
is not a claim of effect isolation.

On completion, the scheduler first writes provisional artifacts, then verifies
the source revision, execution key, predecessor-state key, and attempt ID. Only
a current request may commit a result or advance a session's materialized state
key. A stale completion is discarded.

--------------------------------------------------------------------------------

# Security

Opening a document and rendering it must not silently execute arbitrary code
without a clearly defined trust model.

The initial trust contract is:

- execution occurs only through an explicit `render` or `preview` command;
- `--no-execute` forbids starting runners;
- editor parsing/LSP activity must never execute cells;
- opening an untrusted QMD in an editor must be safe;
- preview makes execution and stale-result status visible;
- preview binds to loopback by default and requires an explicit option for a
  remote bind;
- browser connections use an unguessable per-process token;
- the browser protocol cannot submit arbitrary code or execution requests;
- artifact serving prevents path traversal and exposes only registered
  artifacts;
- raw HTML and SVG are treated as trusted document content in the MVP; the
  preview app shell still uses an appropriate content security policy where
  practical.

R processes initially inherit the invoking user's filesystem and network
authority. Tractate should state this plainly, define its working directory and
environment inheritance, avoid logging secrets, and place bounds on captured
output. Explicit invocation is authorization to execute; it is not a sandbox.

Sandboxed execution can be considered later.

--------------------------------------------------------------------------------

# MVP

The first useful version is an R-first, Reveal-first vertical slice. Typst,
Python, Julia, and broad rich-output compatibility are deliberately excluded.

## Source

Support:

- headings;
- paragraphs;
- lists;
- basic Markdown;
- math;
- fenced code;
- executable fenced code;
- YAML front matter;
- simple slide boundaries.

Reuse the Panache parser rather than implementing Markdown parsing.

## Execution

Support:

- R;
- isolated cells;
- one default stateful R session plus named sessions;
- ordered stdout, stderr, warnings, and errors;
- base and ggplot2 figures captured as SVG or PNG;
- declared file inputs;
- filesystem-backed result and artifact caches;
- reset-and-replay recovery without interpreter snapshots.

Use conservative forward invalidation for stateful sessions.

## Output

Support:

1. Reveal.js HTML.

Structured tables and rich raw-HTML execution results may follow after the
common display-bundle and trust contracts are proven.

## Preview

Support:

```sh
tractate preview slides.qmd
```

with:

- persistent compiler state;
- file watching;
- prose edits performing zero code execution;
- cached unchanged cells;
- changed HTML slides replaced live;
- revision-safe cancellation and stale-result rejection;
- last-known-good output marked stale after a failure.

--------------------------------------------------------------------------------

# MVP success criteria

The prototype should demonstrate the following.

Given a 100-slide presentation containing many expensive computations:

## Case 1: prose edit

Editing one sentence should:

- incrementally update the parse;
- execute zero cells;
- rerender one affected slide;
- update the preview rapidly.

## Case 2: isolated code-cell edit

Editing one independent code cell should:

- execute only that cell;
- reuse all other cell results;
- update only dependent rendering.

## Case 3: stateful-session edit

Given:

```text
A → B → C → D
```

editing `C` should:

- preserve the logical results of `A` and `B` when their inputs remain valid;
- invalidate `C` and `D`;
- reset and replay `A` and `B` when no live runner already represents `state_B`;
- execute the invalidated suffix;
- update affected slides.

Instrumentation should distinguish replayed prefix cells from invalidated cells.

## Case 4: declared-input edit

If `C` declares `data.csv` as an input, changing that file should:

- invalidate `C` and the required stateful suffix;
- leave unrelated sessions valid;
- never reuse a result keyed by the previous file contents.

## Case 5: rapid edits

If a cell changes from revision 10 to 11 while revision 10 is executing:

- revision 10 should be interrupted when practical;
- any late revision-10 result should be discarded;
- only revision 11 may update visible output, the result cache, or session
  state.

## Case 6: failure and recovery

If a stateful cell fails or its runner crashes:

- its session suffix should not execute;
- unrelated sessions may finish;
- preview should retain and mark any last-known-good output as stale;
- a subsequent valid edit should restart the runner and reconstruct state;
- no failed or partial result should become a cache hit.

## General correctness criteria

For deterministic inputs, the rendered result after any edit sequence should
equal a clean full build of the final source. Tests should cover cold and warm
starts, cancellation, corrupt cache entries, runner crashes, duplicate watcher
events, and atomic-save renames. A counting fake runner should prove that prose
edits execute zero cells.

Performance criteria should be expressed as measured latency budgets on a
representative large presentation. Record parse, semantic update, scheduling,
rendering, and browser-patch time separately from user computation.

If these cases do not produce a noticeably better workflow than existing Quarto
preview, reassess the project before expanding scope.

--------------------------------------------------------------------------------

# Implementation sequence

Develop the compiler as a series of vertical proofs:

1. Lower Panache syntax into semantic slides and render static Reveal.js HTML.
   Establish incremental-versus-full-build equivalence.
2. Add a deterministic fake runner, revisioned scheduling, artifact storage, and
   rapid-edit race tests.
3. Add the R runner, stateful reset-and-replay semantics, stdout/stderr, and
   SVG/PNG figures.
4. Add isolated cells, declared inputs, durable cache recovery, and tools that
   explain invalidation and replay.
5. Add Typst/Touying output through generated modules and a long-running
   `typst watch` process.
6. Add Python and Julia only after the runner protocol has proved portable.

Steps 1 through 4 constitute the MVP. Steps 5 and 6 are post-MVP.

The first Typst acceptance case is:

Editing prose on one slide should:

- rewrite only the relevant generated Typst module;
- not execute code;
- allow Typst's persistent compiler to incrementally rebuild the PDF.

--------------------------------------------------------------------------------

# Key architectural principles

1. **Plain text is the source of truth.**

   There is no notebook document model.

2. **Parsing is incremental.**

   Reuse the Panache parser infrastructure.

3. **Execution is owned by this project.**

   Do not depend on knitr, Jupyter, or Quarto execution engines.

4. **Computation and rendering are separate graphs.**

   A prose edit must not invalidate computation.

5. **Pure dependency tracking and effects are separate.**

   Dependency queries decide what is required; runners execute outside those
   queries, and revision checks govern publication.

6. **Cache at the smallest safe semantic unit.**

   Prefer cells and session prefixes over whole-document hashes. Cached output
   is not cached interpreter state.

7. **Use conservative semantics where dynamic languages make dependency
   inference unreliable.**

8. **Results are backend-independent display bundles.**

   Execute once and let each backend select a supported representation.

9. **Do not implement PDF layout.**

   Typst is the PDF compiler.

10. **Incrementality is dependency-driven, not an optimization bolted onto a
    batch renderer.**

11. **A clean full build defines correctness.**

    Incremental execution and rendering must agree with it for deterministic
    inputs.

12. **Presentations come first.**

    They provide natural incremental boundaries and a constrained initial
    problem.

--------------------------------------------------------------------------------

# Longer-term possibilities

If the presentation architecture works well, the same compiler could later
support:

- articles;
- reports;
- books;
- websites;
- computational lecture notes;
- executable technical documentation.

Possible future capabilities include:

- explicit cell dependency DAGs;
- declared environment and output dependencies;
- deterministic/hermetic execution;
- remote execution;
- custom language runners;
- package/environment locking;
- persistent runtime snapshots;
- richer tables;
- interactive HTML outputs;
- citation processing;
- bibliography support;
- cross-document references;
- project-level dependency graphs.

These should not complicate the initial presentation-focused implementation.

--------------------------------------------------------------------------------

# Summary

The project should not be conceived as another static-site generator or another
notebook frontend.

It is an **incremental compiler for computational Markdown**.

The defining pipeline is:

```text
QMD
 │
 ▼
incremental Panache parse
 │
 ▼
semantic document / presentation IR
 │
 ├──── incremental computation ────► cached results
 │
 └──── incremental rendering
             │
        ┌────┴────┐
        ▼         ▼
     Reveal     Typst
       │           │
      HTML        PDF
```

The central user-facing property is simple:

> Editing a document should perform only the computation, parsing, rendering,
> and typesetting that the edit actually invalidates.
