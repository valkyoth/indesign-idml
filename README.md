# indesign-idml

Secure Rust library for parsing, inspecting, and generating Adobe InDesign IDML
packages.

This project is an independent implementation. InDesign and IDML are trademarks
of Adobe Inc. This project is not affiliated with, endorsed by, or sponsored by
Adobe.

See [SECURITY_IMPLEMENTATION_PLAN.md](SECURITY_IMPLEMENTATION_PLAN.md) for the
implementation plan, threat model, dependency policy, and local verification
gate.

See [RELEASE_PLAN.md](RELEASE_PLAN.md) for the milestone plan through the fully
functional 1.0 release.

## Library

For large IDML packages, prefer visitor-style APIs when writing extracted text
or metadata to another sink. `try_for_each_story_text_with_options` resolves one
story at a time in `designmap.xml` order and allows the caller to stop on I/O or
application errors without collecting every story into one map first.
For layout-aware extraction, `try_for_each_spread_text_frame_with_options`,
`try_for_each_spread_text_on_page_with_options`, and
`try_for_each_spread_text_in_rect_with_options` resolve only matching frame
stories as they are visited.

## CLI

The crate includes an optional companion binary, `idml-text`, behind the `cli`
feature. It extracts story text from an IDML package in `designmap.xml` order
or layout-resolved text from one spread, page, rectangle, or every spread. It
exposes the same archive, manifest, spread, and story parser limits used by the
library.

```sh
cargo run --features cli --bin idml-text -- INPUT.idml
cargo run --features cli --bin idml-text -- --all-spreads INPUT.idml
cargo run --features cli --bin idml-text -- --spread-id u10 --page-id p1 INPUT.idml
cargo run --features cli --bin idml-text -- --spread-id u10 --rect-points 0 0 100 100 INPUT.idml
cargo run --features cli --bin idml-text -- --max-entry-compression-ratio 100 INPUT.idml
cargo run --features cli --bin idml-text -- --max-archive-path-bytes 4096 INPUT.idml
```

Release binaries for `idml-text` are intended to be attached to GitHub Releases
as companion artifacts. The crates.io package remains the Rust library API.

To build a native CLI release artifact locally:

```sh
scripts/build-cli-release.sh
```

The script writes `dist/idml-text-v<VERSION>-<TARGET>` and a matching
`.sha256` checksum file.

## License

This project is dual-licensed:

```toml
license = "MIT OR Apache-2.0"
```

The full license texts are available in [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).
