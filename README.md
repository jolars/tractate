# Tractate

Tractate is an incremental compiler for computational Markdown. It uses plain
Quarto-style Markdown as its source format and is designed to execute and render
only the work invalidated by an edit.

The initial implementation provides a safe inspection path that parses a
document with `panache-parser`, lowers it into a recursive source model, counts
slides under the MVP rules, identifies executable cells, and checks labels and
option declarations without running code:

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
scopes, and unsupported option syntax. Other option value types and default
resolution remain subsequent compiler work.

The `render` module reserves the boundary for future presentation backends; the
planned execution and rendering behavior is described in [DESIGN.md](DESIGN.md).

## Development

Enter the development environment and run the complete local check:

```console
devenv shell
task check
```

On Linux, the inspection safety tests require `strace`, which the development
environment provides. They trace process creation and execution for valid and
malformed documents. See [tests/README.md](tests/README.md) for the shared
fixture and full-build assertion helpers.

To verify the exact archive intended for crates.io:

```console
task package
```

## License

Tractate is licensed under the [MIT License](LICENSE).
