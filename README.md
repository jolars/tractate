# Tractate

Tractate is an incremental compiler for computational Markdown. It uses plain
Quarto-style Markdown as its source format and is designed to execute and render
only the work invalidated by an edit.

The initial implementation provides a safe inspection path that parses a
document with `panache-parser` and identifies executable cells without running
them:

```console
cargo run -- inspect slides.qmd
```

The compiler will grow within this single crate. Its internal modules will keep
parsing, computation, and backend rendering separate; the planned execution and
rendering behavior is described in [DESIGN.md](DESIGN.md).

## Development

Enter the development environment and run the complete local check:

```console
devenv shell
task check
```

To verify the exact archive intended for crates.io:

```console
task package
```

## License

Tractate is licensed under the [MIT License](LICENSE).
