# indesign-idml Secure Implementation Plan

## Verified Baseline

This plan was prepared on 2026-05-13 after checking the local repository,
`/home/eldryoth/Work/codex-projects/fluxheim`, and current upstream package
metadata.

Current baseline:

- Rust stable: 1.95.0.
- Crate license: `MIT OR Apache-2.0`.
- Core dependencies: `quick-xml 0.40.0`, `zip 8.6.0`, `indexmap 2.14.0`,
  `thiserror 2.0.18`, `serde 1.0.228`, `base64-ng 0.2.0`.
- Test dependencies: `assert_cmd 2.2.2`, `tempfile 3.27.0`.
- Security tools: `cargo-deny 0.19.6`, `cargo-audit`, `cargo-license`,
  `cargo-sbom`.

Adobe currently documents IDML as an InDesign export format and exposes
`generateIDMLSchema` for generating an IDML schema from the installed
application. The old PDF specification is useful historical context, but the
implementation must validate against generated schemas and real exported IDML
fixtures instead of trusting only the old PDF.

## Security Model

Threat assumptions:

- IDML files are untrusted input.
- ZIP entries may attempt path traversal, duplicate names, decompression bombs,
  unsupported compression methods, or malformed metadata.
- XML may contain oversized attributes, malformed namespaces, invalid UTF-8,
  entity tricks, malicious ID references, and resource exhaustion payloads.
- Embedded base64 assets may be non-canonical, oversized, padded incorrectly, or
  intentionally ambiguous for cache/key confusion.

Hard rules:

- `#![forbid(unsafe_code)]` for the crate.
- No network access while parsing or writing IDML.
- No filesystem writes during read-only parsing.
- No extraction APIs that write archive paths directly to disk.
- All archive paths are normalized as logical ZIP paths, never host paths.
- All parsing APIs accept explicit size limits.
- All public error types are typed, non-panicking, and non-exhaustive.
- No `unwrap`, `expect`, or panics in parser, resolver, writer, or CLI paths.

## Architecture

Primary modules:

- `archive`: ZIP reader/writer, path policy, compression policy, mimetype
  placement, entry inventory.
- `model`: typed structures for `DesignMap`, `Spread`, `Story`, and resources.
- `core::resolver`: lazy ID-to-entry resolution and relational integrity.
- `core::units`: f64 point/mm/in conversions with round-trip tests.
- `encoding`: base64 engines for strict modern and legacy-compatible decoding.
- `traits`: `XmlLoadable`, `XmlSaveable`, and validation traits.
- `validate`: cross-file integrity checks and writer preflight.

Parsing flow:

1. Open archive through `IdmlPackage`.
2. Inventory entries with strict path normalization and configured byte limits.
3. Parse `designmap.xml` first.
4. Build ordered indexes with `IndexMap`.
5. Return lazy pointers for spreads, stories, and resources.
6. Resolve on demand with lifetime-aware borrowed buffers where possible.
7. Validate before serialization.
8. Write a new archive with `mimetype` first and stored, then XML/resources.

## Base64 Policy

Use published `base64-ng 0.2.0` through explicit engines only. Do not use
deprecated global helpers or implicit configs. The local sibling checkout may
be reviewed as development context, but published `indesign-idml` releases must
not depend on unpublished path crates.

Modern default:

- RFC 4648 standard alphabet.
- Canonical padding required unless the IDML field explicitly requires no pad.
- Reject non-zero trailing bits.
- Reject whitespace and non-base64 bytes.
- Decode into caller-provided buffers when possible.
- Enforce decoded size limits before allocation.

Legacy compatibility:

- Exposed only through a named `LegacyBase64` mode.
- Allows documented legacy padding and whitespace behavior only when requested.
- Returns diagnostics that identify the compatibility relaxation used.
- Never silently canonicalizes for equality or cache keys.

## IDML-Specific Correctness

- Preserve namespaces and original qualified names unless the writer owns the
  whole element.
- Treat self-closing package references as XML empty events.
- Preserve order for spreads, layers, page items, and resources.
- Model `Self`, `ParentStory`, and package `src` references as typed IDs.
- Validate that every `ParentStory` resolves to a story entry.
- Validate that every designmap entry points to an existing archive entry.
- Keep unknown XML attributes/elements in lossless extension fields for writer
  round trips.
- Use `f64` for geometry and deterministic formatting on write.

## Testing System

The local gate mirrors Fluxheim's style, adapted for a Rust library:

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- default, no-default, `std`, and `serde` feature tests
- `cargo doc --no-deps --all-features`
- release metadata validation
- `cargo deny check`
- `cargo audit`
- `cargo license`
- SBOM generation
- reproducible release build check
- libFuzzer harness placeholders for parser targets

Test categories to add as implementation grows:

- Unit tests for XML event handling, attributes, IDs, and unit conversions.
- Golden fixtures exported from current InDesign and older IDML versions.
- Malicious ZIP fixture tests for traversal, absolute paths, duplicates, bombs,
  unsupported compression, and invalid mimetype placement.
- Malicious XML tests for huge tokens, malformed namespaces, broken encodings,
  invalid references, and text extraction corner cases.
- Property tests for unit conversion and base64 round trips.
- Fuzz targets for designmap, story, spread, archive inventory, and base64.
- Round-trip writer tests compared with schema generation and InDesign-opened
  fixtures when available.

## Milestones

### 1. Secure Foundation

- Keep the crate compiling under Rust 1.95.0.
- Implement `IdmlError`, size limit config, archive inventory, and path policy.
- Implement strict dependency policy and local gate.
- Add first malicious archive fixtures.

Exit criteria: all local checks pass and unsafe code remains forbidden.

### 2. DesignMap Reader

- Implement event-based `DesignMap` parsing with `quick-xml`.
- Capture ordered `Spread`, `Story`, `MasterSpread`, and resource references.
- Preserve unknown package references.
- Validate missing `src`, malformed IDs, duplicates, and missing archive entries.

Exit criteria: designmap golden tests, malformed XML tests, and fuzz target pass.

### 3. Text Miner

- Implement story parsing for visible text extraction.
- Stream large stories without collecting all story XML in one string.
- Add CLI binary behind a feature or separate package if needed.
- Decode XML entities correctly and keep text order stable.

Exit criteria: extract text from representative IDML fixtures under memory
limits.

### 4. Layout Navigator

- Implement spread parsing, text frame geometry, `ParentStory` links, and unit
  conversion.
- Add resolver joins from spread frames to story content.
- Add bounding-box queries in points, mm, and inches.

Exit criteria: location-based text queries work against golden fixtures.

### 5. Writer

- Implement loss-aware XML writing.
- Preserve namespaces, unknown fields, and stable ordering.
- Write `mimetype` first and uncompressed.
- Validate relational integrity before writing.

Exit criteria: generated IDML opens in InDesign and passes schema validation.

### 6. Hardened Release

- Expand fuzz corpus and run long fuzz campaigns before release.
- Generate SBOM and license reports.
- Run `cargo audit`, `cargo deny`, `cargo license`, and reproducibility checks.
- Tag only after clean local and CI gates.

Exit criteria: release candidate has zero known advisories, approved licenses,
documented fixtures, and reproducible artifacts.

## Commit Policy

Every implementation step should end with:

1. `sh scripts/checks.sh`
2. `git status --short`
3. `git add ...`
4. `git commit`

Push remains manual.
