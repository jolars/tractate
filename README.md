# Tractate

Tractate is an incremental compiler for computational Markdown. It uses plain
Quarto-style Markdown as its source format and is designed to execute and render
only the work invalidated by an edit.

The implementation provides a safe inspection path that parses a document with
`panache-parser`, lowers it into a recursive source model, counts slides under
the MVP rules, identifies executable cells, and checks labels and option
declarations without running code:

```console
cargo run -- inspect slides.qmd
```

The compiler lives in a single crate with private `document`, `parser`,
`compiler`, and `render` modules. The library offers `summarize_document` for a
syntax-only structural summary and `inspect_document` for a summary with
structured diagnostics. Diagnostics retain their source snapshots, UTF-8 byte
ranges, stable codes, and related declarations. The CLI prints their QMD line
and column locations and exits unsuccessfully on syntax or semantic errors.
Inspection checks document-wide label uniqueness, label values, option names and
scopes, and unsupported option syntax.

Library callers can keep a `Compiler` across edits and retain immutable
snapshots:

```rust
use tractate::{Compiler, SourceFile};

let mut compiler = Compiler::new(SourceFile::new("slides.qmd", "## First\n"));
let original = compiler.snapshot();
compiler.update_source(SourceFile::new("slides.qmd", "## Revised\n"));
assert!(compiler.revision() > original.revision());
assert_eq!(original.source().text(), "## First\n");
```

Each compiler starts at source revision zero. Changes to the supplied path or
source bytes advance the revision, including invalid edits and reversions.
Identical input keeps the existing snapshot. Snapshots share their presentation
tree and inspection diagnostics, and remain valid after later updates. These
operations perform no filesystem access or execution. Changed input currently
rebuilds the complete tree; dependency tracking and reuse across edits remain
roadmap items.

Render a source-only presentation with:

```console
cargo run -- render slides.qmd --to html
```

This writes `slides_html/index.html` and `slides_html/manifest.json`. HTML is
the default format. Use `--output deck` (or `-o deck`) to choose a directory
relative to the source's parent, or provide an absolute path. The output's
parent must already exist. A previous Tractate deck is replaced atomically;
failed builds preserve it. Unrelated directories and directories containing
build inputs cannot be replaced.

Local Markdown images and linked files resolve from the source's parent and are
copied into the deck. Direct and reference links, URL-encoded filenames,
queries, and fragments are supported. Remote URLs remain external. Resource URLs
inside raw HTML or copied files are not rewritten.

Rendering starts no processes. Ordinary code fences are displayed as source.
Executable cells require `eval: false`, either locally or under the document's
`execute` defaults; they honor `echo` and `include`. Rendering validates these
three Boolean options, including overridden defaults. Cells that require
execution fail with a QMD diagnostic because runners are not implemented yet.

Use `--no-execute` to forbid execution explicitly:

```console
cargo run -- render slides.qmd --to html --no-execute
```

Documents with no required computation render normally. A missing required
result fails with `render.result-unavailable` at the cell's QMD location and
preserves any previous deck. `echo: false`, `include: false`, and
`results: hide` do not disable evaluation. Because runners and result caching
are not implemented yet, every cell with effective `eval: true` has an
unavailable result, even if a previous HTML deck exists.

Other option value types, execution, and caching remain roadmap items. The
library exposes `render_html` and `render_html_with_options`; pass
`RenderOptions { no_execute: true }` to the latter for the same execution
policy. Both return structured `RenderError` diagnostics. The private `build`
module owns filesystem I/O around the pure compiler.

The internal HTML backend renders Markdown, source code, and math markup into
Reveal sections keyed by semantic slide IDs. Cell visibility and validated
result content are supplied separately, without execution. Its directory writer
publishes complete decks atomically, with a SHA-256 content manifest, supplied
local assets, and exact-version CDN references to Reveal.js and KaTeX. Viewing
requires network access. Atomic publication is supported on Linux, Android, and
Apple platforms when the filesystem supports directory exchange. Identity
matching across revisions remains a roadmap item. The planned execution and
rendering behavior is described in [DESIGN.md](DESIGN.md).

## Development

Enter the development environment and run the complete local check:

```console
devenv shell
task check
```

On Linux, the inspection and rendering safety tests require `strace`, which the
development environment provides. They trace process creation and execution for
valid and malformed documents. See [tests/README.md](tests/README.md) for the
shared fixture and full-build assertion helpers.

To verify the exact archive intended for crates.io:

```console
task package
```

## License

Tractate is licensed under the [MIT License](LICENSE).
