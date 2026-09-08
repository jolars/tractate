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
- PDF output through Typst;
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

3. Execute R, Python, and Julia directly, without knitr or Jupyter.

4. Cache computation at cell/session granularity.

5. Incrementally parse and render changed document regions.

6. Provide fast live preview for presentations.

7. Support:

   - Reveal.js/HTML;
   - PDF via Typst.

8. Make computation results backend-independent.

9. Preserve predictable execution semantics.

10. Build around a persistent dependency graph rather than a batch rendering
    pipeline.

### Non-goals for the initial version

- Full Quarto compatibility.
- General notebook compatibility.
- `.ipynb` support.
- Reimplementing PDF layout.
- Automatic dependency inference between arbitrary variables in R/Python/Julia.
- Distributed computation.
- Interactive widgets.
- Supporting every Pandoc output format.
- Replacing Pandoc generally.

--------------------------------------------------------------------------------

# Architecture

The compiler is conceptually:

```text
source
  │
  ▼
Panache parser
  │
  ▼
document model / IR
  │
  ├───────────────┐
  ▼               ▼
computation       presentation
graph             graph
  │               │
  ▼               ├────────────► HTML / Reveal.js
language          │
runners           └────────────► Typst ──► PDF
  │
  ▼
results/artifacts
```

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

A code edit might result in:

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

A small initial option vocabulary is sufficient:

- `label`
- `eval`
- `echo`
- `include`
- `results`
- `session`
- figure width/height
- figure caption

--------------------------------------------------------------------------------

# Document model

Parsing should produce a semantic document model on top of the Panache CST.

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

The project should maintain a persistent build graph.

Conceptually:

```text
SourceRange
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

A dependency-tracking system such as Salsa is appropriate, but the exact
implementation should remain an internal choice.

The important invariant is:

> Unchanged inputs must reuse unchanged outputs.

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

    fn execute(&mut self, cell: &Cell) -> Result<CellResult>;

    fn reset(&mut self) -> Result<()>;
}
```

Exact APIs may differ, especially if asynchronous execution is useful.

Initial languages:

1. R
2. Python
3. Julia

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
    stdout: String,
    stderr: String,
    displays: Vec<Display>,
    artifacts: Vec<Artifact>,
}
```

with:

```rust
enum Display {
    Text(String),
    Markdown(String),
    Html(String),
    Svg(Vec<u8>),
    Png(Vec<u8>),
    Table(TableData),
}
```

This should be deliberately smaller and simpler than the Jupyter messaging
protocol.

Its purpose is to let:

```text
R / Python / Julia
        ↓
   CellResult
      ↙   ↘
   HTML   Typst
```

share the same execution results.

--------------------------------------------------------------------------------

# Language bridges

Each language requires a small runtime shim to capture rich output.

## R

Initially support:

- stdout;
- stderr;
- warnings/messages;
- base graphics;
- ggplot2 graphics;
- simple data-frame/table output.

Graphics can be captured by opening a controlled graphics device around cell
execution.

## Python

Initially support:

- stdout;
- stderr;
- exceptions;
- matplotlib figures;
- basic rich values/tables.

Avoid depending on IPython.

Provide a small injected runtime module if necessary.

## Julia

Initially support:

- stdout;
- stderr;
- exceptions;
- `display`;
- common plotting systems where practical.

Use Julia's display mechanisms rather than notebook infrastructure.

--------------------------------------------------------------------------------

# Execution semantics

Two execution modes should eventually exist.

## Isolated cells

An isolated cell executes independently.

```text
A

B

C
```

Changing `B` invalidates only `B`.

This provides the strongest caching guarantees.

A cache key should include at least:

```text
language
runner version
source
execution options
declared inputs
relevant environment information
```

--------------------------------------------------------------------------------

## Stateful sessions

Cells may optionally share interpreter state:

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

The initial implementation should therefore prefer correctness over cleverness.

Possible approaches include:

1. replay unchanged prefix cells when restoring a session;
2. keep long-lived interpreter processes during preview;
3. later investigate safe state snapshots where language runtimes support them.

Persistent interpreter snapshots are not required for the MVP.

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

Eventually support declarations such as:

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

Do not claim hermetic execution unless a future sandboxed execution mode
actually provides it.

--------------------------------------------------------------------------------

# Artifact store

Generated figures and other binary outputs should live in a content-addressed
artifact store.

For example:

```text
.cache/
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

The preview client can then replace the corresponding DOM subtree.

The browser should preserve:

- current slide;
- current fragment/overlay position where possible;
- scroll/focus state;
- presenter state where practical.

Global changes such as themes or Reveal configuration may require broader
invalidation.

--------------------------------------------------------------------------------

# Typst / PDF backend

Do not implement a PDF renderer.

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
.build/typst/
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

--------------------------------------------------------------------------------

# Typst integration strategy

## MVP

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
insufficient.

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
tool preview slides.qmd
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

The system should support cancellation.

If the user edits a cell several times while an old computation is still
running, obsolete work should be cancelled or its result discarded when it
completes.

--------------------------------------------------------------------------------

# Build mode

Also provide deterministic one-shot compilation:

```sh
tool render slides.qmd
```

Potential targets:

```sh
tool render slides.qmd --to html
tool render slides.qmd --to pdf
```

Build mode should reuse the same compiler and execution-cache infrastructure as
preview mode.

--------------------------------------------------------------------------------

# Proposed crate structure

A possible Rust workspace:

```text
crates/
  core/
      document IR
      IDs
      diagnostics

  parser/
      integration with Panache parser crate
      CST → semantic model

  compiler/
      dependency graph
      invalidation
      compilation orchestration

  exec/
      cell model
      scheduler
      cache
      artifact store
      runner interfaces

  exec-r/
      R bridge

  exec-python/
      Python bridge

  exec-julia/
      Julia bridge

  render-html/
      HTML / Reveal.js

  render-typst/
      presentation IR → Typst

  preview/
      watcher
      HTTP/WebSocket server

  cli/
      command-line application
```

Exact boundaries should evolve as implementation experience accumulates.

Avoid splitting into many crates before useful boundaries emerge.

--------------------------------------------------------------------------------

# Stable identities

Incrementality depends heavily on retaining identities across edits.

Slides and executable cells should therefore have stable IDs wherever possible.

Explicit labels are ideal:

````markdown
```{r}
#| label: fig-regression
...
````

````

For unlabeled nodes, derive identities from structural context rather than byte offsets alone.

Moving a cell should ideally not destroy its cached computation merely because its absolute source position changed.

This needs careful treatment in the parser/semantic layer.

---

# Execution scheduler

Execution should happen independently of rendering.

The scheduler receives invalidated computation nodes and resolves their dependencies.

For the initial version:

```text
isolated cells:
    execute independently

stateful session:
    execute invalidated suffix sequentially

different sessions:
    may execute concurrently
````

This naturally allows parallel execution across unrelated cells or sessions.

--------------------------------------------------------------------------------

# Security

Opening a document and rendering it must not silently execute arbitrary code
without a clearly defined trust model.

At minimum:

- execution should occur only through an explicit render/preview command;
- editor parsing/LSP activity must never execute cells;
- opening an untrusted QMD in an editor must be safe;
- preview should make execution status visible.

Sandboxed execution can be considered later.

--------------------------------------------------------------------------------

# MVP

The first useful version should be deliberately small.

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
- Python;
- Julia;
- isolated cells;
- one default stateful session per language;
- stdout/stderr;
- figures;
- basic tables;
- filesystem-backed cache.

Use conservative forward invalidation for stateful sessions.

## Output

Support:

1. Reveal.js HTML;
2. Typst/Touying PDF.

## Preview

Support:

```sh
tool preview slides.qmd
```

with:

- persistent compiler state;
- file watching;
- prose edits performing zero code execution;
- cached unchanged cells;
- changed HTML slides replaced live;
- generated Typst modules updated incrementally;
- long-running Typst watch for PDF.

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

- preserve `A` and `B`;
- invalidate `C` and `D`;
- execute the required suffix;
- update affected slides.

## Case 4: PDF prose edit

Editing prose on one slide should:

- rewrite only the relevant generated Typst module;
- not execute code;
- allow Typst's persistent compiler to incrementally rebuild the PDF.

If these cases do not produce a noticeably better workflow than existing Quarto
preview, reassess the project before expanding scope.

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

5. **Cache at the smallest safe semantic unit.**

   Prefer cells and sessions over whole-document hashes.

6. **Use conservative semantics where dynamic languages make dependency
   inference unreliable.**

7. **Results are backend-independent.**

   Execute once; render to HTML or Typst.

8. **Do not implement PDF layout.**

   Typst is the PDF compiler.

9. **Incrementality is dependency-driven, not an optimization bolted onto a
   batch renderer.**

10. **Presentations come first.**

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
- declared file/environment dependencies;
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
