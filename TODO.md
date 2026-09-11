# Tractate MVP Roadmap

This roadmap turns the architecture in `DESIGN.md` into an ordered path to the
first useful release: an R-first, Reveal-first incremental compiler for a single
QMD presentation. Complete stages in order. A later stage may be explored, but
its implementation should not become a dependency until the preceding gate has
passed.

Write the acceptance test for each behavior before implementing it. During a
stage, run focused tests; at every gate, run `task check`. Use a clean full
build as the correctness oracle for every incremental-build test.

## MVP boundary

The MVP is complete when these commands form one coherent workflow:

```console
tractate inspect slides.qmd
tractate render slides.qmd --to html
tractate render slides.qmd --to html --no-execute
tractate preview slides.qmd
tractate preview slides.qmd --no-execute
```

The workflow must parse without executing, execute R only when explicitly
authorized, reuse valid cell results, render a Reveal.js deck, and update a live
preview according to actual dependencies. It must support one QMD file, a
project root, basic Markdown and math, simple slides, isolated and stateful R
cells, named R sessions, declared file inputs, ordered textual output, and base
and ggplot2 figures.

Typst/PDF, Python, Julia, Windows process management, notebooks, broad Quarto
compatibility, rich tables, interactive widgets, interpreter snapshots, and
hermetic execution are outside the MVP.

## Stage 0—Secure the parsing foothold

The repository already has most of this foundation.

- [x] Keep the compiler library and CLI in one Rust 2024 crate with Rust 1.89 as
  the MSRV.
- [x] Parse Quarto-flavored Markdown with Panache.
- [x] Distinguish ordinary fenced code from executable fenced code.
- [x] Provide `tractate inspect` without an execution path.
- [x] Cover the inspection command with unit and integration tests.
- [x] Add a test asserting that malformed input makes `inspect` fail without
  starting any process.
- [x] Add reusable fixture and assertion helpers for full-build and
  edit-sequence tests.

### Gate 0—Safe inspection

- [x] `tractate inspect tests/fixtures/minimal.qmd` reports the expected
  structure.
- [x] Valid and malformed documents are inspected without executing code.
- [x] `task check` passes.

## Stage 1—Compile source into a static Reveal deck

This stage proves the pure source-to-presentation path before execution is
introduced.

### 1.1 Specify source semantics with fixtures

- [x] Add fixtures for YAML metadata, title slides, body content before the
  first level-two heading, level-two slide boundaries, headings, paragraphs,
  lists, math, ordinary code, and executable code.
- [x] Add fixtures for `#|` cell options, document defaults, malformed options,
  unknown options, and duplicate labels.
- [x] Lock down the MVP slide rules: nonempty title metadata creates a title
  slide, each level-two heading starts a content slide, and nonempty leading
  body content creates a content slide.
- [x] Define the supported option types and their invalidation classes for
  `label`, `eval`, `echo`, `include`, `results`, `session`, `cache`,
  `inputs`, figure dimensions, and figure captions.

### 1.2 Introduce the semantic layers

- [x] Create internal `document`, `parser`, `compiler`, and `render` module
  boundaries while keeping the public facade small.
- [x] Lower the Panache CST into a source semantic IR without flattening nested
  block or inline structure.
- [x] Attach source origins to semantic and generated nodes.
- [x] Represent diagnostics with a severity, stable code, message, primary
  origin, and related origins.
- [x] Represent presentations and slides explicitly, independently of HTML.
- [x] Keep `NodeId`/`SlideId`/`CellId`, `ExecutionKey`, and `ArtifactId` as
  distinct types.
- [x] Enforce document-scoped unique labels and report unsupported options.
- [ ] Give labeled nodes deterministic semantic identities; define a best-effort
  previous-revision matcher for unlabeled slides and cells.

### 1.3 Render static HTML

- [x] Define a backend-independent presentation IR with result references or
  result slots, not embedded runner output.
- [ ] Render each slide as a Reveal `<section>` with a stable slide ID.
- [ ] Support the MVP Markdown constructs in HTML, including source code and
  math.
- [ ] Emit an HTML directory atomically with a content manifest and pinned or
  bundled Reveal.js assets—do not depend on an unversioned CDN.
- [ ] Add `tractate render SOURCE --to html`; on source-only documents it should
  produce a complete deck without any runner infrastructure.
- [ ] Make `render --no-execute` fail when a required result is neither present
  nor reusable, while allowing documents with no required computation to
  render.

### 1.4 Establish the incremental pure core

- [ ] Introduce monotonically increasing source revisions and a long-lived
  in-memory compiler state.
- [ ] Track dependencies from source through semantic blocks, slides, and
  rendered slide fragments; keep effects out of tracked queries.
- [ ] Reuse stable semantic nodes and rendered fragments across revisions when
  their declared inputs are unchanged.
- [ ] Return an explicit change set for inserted, removed, reordered, and
  modified slides.
- [ ] Build a full-build oracle and compare it with incremental output after
  edit sequences.

### Gate 1—Static incremental compiler

- [ ] A representative source-only QMD renders as a usable Reveal.js deck.
- [ ] Editing prose on one slide changes only that slide's rendered fragment.
- [ ] Inserting, deleting, and moving slides produces a correct structural
  change set.
- [ ] Incremental output equals a clean full build after every fixture edit
  sequence.
- [ ] Syntax, option, and duplicate-label errors point into the original QMD.
- [ ] `task check` passes.

## Stage 2—Prove execution semantics with a fake runner

This stage establishes the effect boundary and race safety without depending on R.

### 2.1 Define execution contracts

- [ ] Define execution requests with source revision, attempt ID, cell ID,
  execution key, session ID, and required predecessor-state key.
- [ ] Define success, failure, and cancellation outcomes with diagnostics,
  duration, runner identity, and the effective execution key.
- [ ] Define ordered output events for stdout, stderr, warnings, and display
  bundles.
- [ ] Define display representations for text, HTML, SVG, PNG, and artifact
  references, while allowing renderers to choose a supported representation.
- [ ] Specify runner startup, execution, interruption, reset, crash, poison,
  restart, and shutdown transitions.

### 2.2 Plan computation

- [ ] Lower executable cells into an evaluation plan independently of rendering.
- [ ] Exclude `eval: false` cells from execution and from cumulative state.
- [ ] Build conservative document-order chains for the default and named
  stateful sessions.
- [ ] Plan isolated cells independently and allow unrelated sessions to make
  progress concurrently within a configurable limit.
- [ ] Compute cumulative state keys and distinguish a logically cached result
  from interpreter state that has actually been materialized.
- [ ] Ensure render-only options do not enter execution keys.

### 2.3 Add the scheduler and transactional artifact path

- [ ] Implement a deterministic counting fake runner that can block, fail,
  crash, emit artifacts, and ignore interruption on demand.
- [ ] Reconcile a revision's desired requests with pending and running attempts.
- [ ] Cancel obsolete work when practical, and reject late results by revision,
  attempt ID, execution key, and predecessor-state key.
- [ ] Never advance a session state key after stale, failed, cancelled, or
  protocol-invalid work.
- [ ] Poison an uncertain stateful runner and reconstruct state in a fresh
  runner by replaying the valid prefix.
- [ ] Store provisional artifacts by content digest, then atomically commit a
  result record only after publication checks pass.
- [ ] Treat partial or corrupt artifact writes as cache misses.
- [ ] Expose pending, running, succeeded, failed, cancelled, unavailable, and
  stale-last-known-good states to renderers.

### Gate 2—Revision-safe execution proof

- [ ] A counting fake runner proves that a prose-only edit executes zero cells.
- [ ] In `A -> B -> C -> D`, editing `C` preserves the logical results of `A`
  and `B`, replays them when necessary, and executes only the invalidated
  suffix.
- [ ] Concurrent unrelated sessions can finish when one session fails.
- [ ] A late result from an obsolete revision changes neither visible output,
  committed cache records, nor materialized session state.
- [ ] Failed, cancelled, crashed, partial, and corrupt outcomes never become
  successful cache hits.
- [ ] Incremental and clean full builds agree for deterministic fake-runner
  fixtures.
- [ ] `task check` passes.

## Stage 3—Run R and render its results

This stage replaces the fake at the effect boundary without changing compiler
semantics.

### 3.1 Build the R bridge

- [ ] Discover R and record its executable and version as runner identity.
- [ ] Implement a small R runtime shim using a framed control channel that user
  stdout and stderr cannot imitate.
- [ ] Tag requests and events with session, cell, attempt, and sequence IDs.
- [ ] Define the project root, R working directory, inherited environment, and
  disabled or explicit stdin behavior.
- [ ] Add startup and per-cell timeouts, captured-output and artifact limits,
  graceful interruption, forced process-tree termination, crash detection,
  and cleanup for Linux and macOS.
- [ ] Avoid logging source-adjacent secrets or arbitrary environment values.

### 3.2 Capture MVP outputs

- [ ] Preserve the original order of stdout, stderr, messages, warnings, and
  errors.
- [ ] Capture base graphics and ggplot2 figures as SVG or PNG artifacts through
  a controlled graphics device.
- [ ] Map R failures to structured diagnostics whose actionable origin is the
  initiating QMD cell.
- [ ] Render textual events and select suitable figure representations in HTML.

### 3.3 Materialize stateful sessions

- [ ] Implement one default stateful R session and independent named sessions.
- [ ] Execute a cell directly only when the live R process materializes its
  required predecessor-state key.
- [ ] Otherwise reset, replay the valid prefix without republishing its output,
  and execute the invalidated suffix.
- [ ] Block a failed session suffix, but allow unrelated sessions to complete.
- [ ] Restart and reconstruct state after interruption, crash, or protocol
  failure.
- [ ] Keep `inspect` and all parsing/editor-facing APIs physically incapable of
  starting R.

### Gate 3—R vertical slice

- [ ] `tractate render slides.qmd --to html` explicitly starts R and produces a
  deck containing ordered text output and base and ggplot2 figures.
- [ ] Default and named stateful sessions have the same reset-and-replay
  behavior proven with the fake runner.
- [ ] Runner interruption, timeout, error, and crash tests leave no publishable
  partial result and recover on a subsequent valid build.
- [ ] `inspect` starts no R process, and `render --no-execute` starts no R
  process.
- [ ] The supported R and ggplot2 versions and platform assumptions are recorded
  in user-facing documentation.
- [ ] `task check` passes with focused R integration tests run in the
  development environment.

## Stage 4—Make reuse durable, selective, and explainable

This stage completes the computation contract needed by both one-shot builds and
preview.

### 4.1 Complete execution-key semantics

- [ ] Include language and interpreter identity, Tractate and shim versions,
  normalized source, execution-affecting options, working-directory
  semantics, relevant declared environment, OS/architecture where needed,
  lockfile digest when available, and cache schema version.
- [ ] Add isolated R cells whose keys do not depend on document neighbors.
- [ ] Resolve declared `inputs` under the project path policy and include their
  content digests in execution and cumulative state keys.
- [ ] Keep missing declared inputs as diagnostics and watchable dependencies.
- [ ] Implement `cache: false` so unrelated edits do not schedule the cell, but
  every actual execution gets a new attempt identity that invalidates its
  stateful suffix.
- [ ] Test each option's declared invalidation class, especially render-only
  `echo`, `include`, and captions versus execution-affecting figure
  dimensions.

### 4.2 Finish the persistent cache

- [ ] Persist result records and immutable artifacts in a project-local,
  schema-versioned, content-addressed store.
- [ ] Use atomic replacement and inter-process locking or a documented
  single-writer rule.
- [ ] Verify object digests on read; treat missing, corrupt, and incompatible
  entries as misses rather than fatal errors.
- [ ] Define roots for current results and active builds, and provide a safe
  garbage-collection path.
- [ ] Exclude cache and build directories from source discovery and watching.
- [ ] Test cold builds, warm builds, process restarts, cache corruption, and
  interrupted commits.

### 4.3 Explain decisions and enforce no-execute

- [ ] Add an inspection or diagnostic surface that explains why each cell ran,
  replayed, hit the cache, missed the cache, or was invalidated.
- [ ] Distinguish logical cache reuse from physical prefix replay in that
  output.
- [ ] Under `render --no-execute`, use valid cached results and fail if any
  required result is missing.
- [ ] Under preview's future `--no-execute` path, represent a missing result as
  unavailable rather than scheduling it.
- [ ] State clearly that R inherits the invoking user's filesystem and network
  authority and that undeclared external state is the author's
  responsibility.

### Gate 4—Correct persistent reuse

- [ ] Editing an isolated cell executes only that cell and rerenders only its
  dependent slide.
- [ ] Editing a declared input invalidates its consumers and the required
  stateful suffix, but no unrelated session.
- [ ] Warm and cold-process cache tests reuse exactly the expected results.
- [ ] A stateful edit visibly distinguishes cached prefix results, physical
  replay, and invalidated suffix execution.
- [ ] Render-only option and prose edits execute zero cells.
- [ ] `--no-execute` behavior is correct for complete, missing, stale, and
  corrupt cache states.
- [ ] `task check` passes.

## Stage 5—Deliver the live preview

This stage connects the long-lived compiler, scheduler, renderer, and browser
into the MVP workflow.

### 5.1 Watch and rebuild

- [ ] Add `tractate preview SOURCE` around one persistent compiler and scheduler
  state.
- [ ] Watch the QMD, declared inputs, and supported source resources under the
  project root.
- [ ] Tolerate atomic-save renames, duplicate events, coalesced edits, and
  temporarily missing files.
- [ ] Normalize paths without erasing relevant symlink or case-sensitivity
  behavior.
- [ ] Cancel obsolete execution attempts when practical and always reject their
  late results.

### 5.2 Patch the browser safely

- [ ] Define a browser protocol carrying the source revision, stable slide ID,
  slide state, and structural change information.
- [ ] Replace a changed slide's DOM subtree and ask Reveal.js to resynchronize.
- [ ] Use a full reload as the correctness fallback for structural or global
  changes that cannot be patched safely.
- [ ] Preserve the current slide, fragment position, focus/scroll state, and
  presenter state where practical.
- [ ] Show pending, running, failed, unavailable, and stale-last-known-good
  results without combining outputs from incompatible revisions.
- [ ] Keep the previous successful output visibly stale after an error while
  displaying the current diagnostic.

### 5.3 Secure and measure preview

- [ ] Bind to loopback by default and require an explicit option for remote
  binding.
- [ ] Authenticate browser connections with an unguessable per-process token.
- [ ] Prevent the browser protocol from submitting arbitrary source or execution
  requests.
- [ ] Serve only registered artifacts, reject path traversal, and apply a
  practical content security policy to the preview shell.
- [ ] Treat raw source/execution HTML and SVG according to the documented MVP
  trust policy.
- [ ] Instrument parse, semantic update, scheduling, rendering, browser-patch,
  replay, and user-computation time separately.
- [ ] Add a representative 100-slide benchmark fixture with many expensive fake
  computations and record latency budgets before optimizing.

### Gate 5—End-to-end preview

- [ ] A prose edit executes zero cells, rerenders one slide, and patches it
  without losing the presenter's location.
- [ ] An isolated-cell edit executes only that cell and updates only dependent
  rendering.
- [ ] A stateful edit performs the required prefix replay and suffix execution.
- [ ] A declared-input edit invalidates only its consumers and required
  suffixes.
- [ ] Rapid edits cannot publish an obsolete result, artifact, session state, or
  browser patch.
- [ ] Failure and crash recovery retain visibly stale last-known-good output and
  rebuild valid state after a corrective edit.
- [ ] Duplicate watcher events and atomic-save renames converge to the same
  result as a clean full build.
- [ ] `preview --no-execute` never starts R and marks missing results
  unavailable.
- [ ] The measured workflow is noticeably faster than a full Quarto preview for
  the representative edit cases; otherwise, reassess before broadening
  scope.
- [ ] `task check` passes.

## Stage 6—Cut the MVP

- [ ] Run all six success cases from `DESIGN.md` as named end-to-end acceptance
  tests.
- [ ] Verify incremental/full-build equivalence across cold and warm starts,
  cancellation, crashes, corrupt caches, and watcher edge cases.
- [ ] Confirm parsing, editor-facing APIs, and `--no-execute` paths cannot reach
  runner startup.
- [ ] Audit CLI errors, diagnostics, cache paths, project-root resolution,
  process cleanup, output bounds, preview authentication, and artifact
  serving.
- [ ] Document the supported source subset, option semantics, cache contract,
  trust model, platform support, and known limitations.
- [ ] Keep the binary thin and expose the reusable compiler behavior through the
  library.
- [ ] Run `task check` on Rust 1.89 and the normal development toolchain.
- [ ] Run `task package` and inspect the distributable archive.

### MVP gate—Ready for an initial release

- [ ] `inspect`, one-shot HTML `render`, and live HTML `preview` satisfy their
  documented execution and `--no-execute` contracts.
- [ ] All stage gates and the six design acceptance cases pass.
- [ ] No post-MVP backend or language has leaked into the core abstractions
  beyond the portability required by the established runner and display
  protocols.
- [ ] The clean full build remains the tested semantic and rendered correctness
  oracle.

## Parser follow-ups

- [ ] Fix Panache 0.29's duplicated quote marker for a fenced cell starting a
  list item inside a block quote. The resulting CST ranges can make source
  lowering panic. The reproducer in
  `diagnostics_map_quoted_list_errors_independently_of_cst_ranges` verifies
  only the diagnostic adapter, whose QMD offsets remain correct. Restore
  lossless full lowering before Gate 1.

## After the MVP

Only after the MVP gate passes, proceed in this order:

1. Generate per-slide Typst/Touying modules and drive a long-running
   `typst watch` process for PDF output.
2. Add Python, then Julia, using the proven runner protocol without changing
   compiler semantics.
3. Consider broader source syntax, cross-references, richer displays, other
   document types, sandboxing, interpreter snapshots, and embedded Typst based
   on measured needs.
