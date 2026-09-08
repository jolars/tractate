# AGENTS.md

## Project overview

Tractate is a single Rust 2024 crate that provides both a library and the
`tractate` CLI. Internal modules separate document types (`src/document.rs`),
Panache integration (`src/parser.rs`), and pure compilation (`src/compiler.rs`).
`src/render.rs` reserves the presentation backend boundary. `src/lib.rs` exposes
the public facade, and `src/main.rs` is the command-line boundary. Integration
tests and their QMD fixtures live under `tests/`.

Keep the compiler in one crate. Introduce internal modules as boundaries become
useful, but do not turn them into separate packages without an explicit change
to the publication model.

## Validation

Run focused Rust tests while developing, then run the complete check before
handing off a nontrivial change:

```bash
cargo test <test-name>
task check
```

Verify the distributable archive after changing package metadata or packaged
paths:

```bash
task package
```

## Architectural invariants

- Plain `.qmd` or Markdown files are the source of truth; do not introduce a
  notebook document model.
- Parsing and editor-facing operations must never execute code.
- Keep computation results independent of the HTML and Typst backends.
- Tractate owns execution directly; do not route it through Quarto, knitr,
  Jupyter, or notebook kernels.
- Preserve Rust 1.89 compatibility unless a change explicitly raises the MSRV.

The full intended architecture and incremental invalidation semantics are in
`DESIGN.md`.

## Generated and release-managed files

- Update `Cargo.lock` through Cargo and `devenv.lock` through devenv; do not
  edit either by hand.
- Use Conventional Commits. Versionary derives releases from commit history and
  owns `CHANGELOG.md`; do not edit it manually.
