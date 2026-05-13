# indesign-idml Release Plan

This plan defines the path from the initial secure reader to a fully functional
IDML library. Dates are intentionally omitted; releases happen when the exit
criteria pass.

## Release Rules

Every release candidate must pass:

- `scripts/checks.sh`
- `cargo deny check`
- `cargo audit`
- `cargo license`
- SBOM generation in SPDX and CycloneDX formats
- reproducible release artifact check
- review of new public APIs for panic-free behavior on untrusted input
- dependency review against current crates.io versions

Published releases must not depend on unpublished path crates. `base64-ng` is
used from crates.io, currently `0.2.0`.

## 0.1.x: Secure Read Foundation

Goal: safely open untrusted IDML packages and parse the package manifest.

Included:

- archive path validation
- bounded ZIP inventory and bounded entry reads
- strict and legacy Base64 wrapper backed by `base64-ng`
- `designmap.xml` parser for spread, story, master spread, and unknown package
  references
- initial fuzz harness placeholders
- malicious archive and XML unit tests

Exit criteria:

- no unsafe code
- archive traversal, duplicate entry, and size-limit tests pass
- DesignMap parser handles self-closing `idPkg:*` references
- all local and CI gates pass

## 0.2.x: Text Miner

Goal: extract text from story files without loading whole documents eagerly.

Included:

- story parser for paragraphs, content text, line breaks, and common special
  characters
- lazy `StoryPointer` resolution from `DesignMap`
- package API for `read_designmap`, `story_ids`, and `resolve_story_text`
- CLI text extraction tool behind an explicit feature
- fixture tests from small exported IDML packages
- fuzz target for story parsing

Exit criteria:

- extracts text from representative current and legacy IDML fixtures
- memory usage remains bounded by configured entry limits
- malformed story XML never panics

## 0.3.x: Layout Navigator

Goal: connect page geometry to story content.

Included:

- spread parser for pages, text frames, rectangles, groups, and `ParentStory`
- f64 point/mm/in conversion module
- bounding-box queries
- resolver join from spread text frames to stories
- tests for page order and z-order preservation

Exit criteria:

- can locate text by page and bounding rectangle
- validates missing and dangling `ParentStory` references
- unit conversion round trips meet documented tolerances

## 0.4.x: Resource Reader

Goal: read enough resources to preserve and inspect real documents.

Included:

- style, swatch/color, font, link, and graphic reference inventory
- Base64 asset decoding through strict default and explicit legacy mode
- resource-level integrity checks
- unknown-resource preservation model

Exit criteria:

- resource inventory works on multi-page production fixtures
- oversized embedded assets are rejected before allocation
- license/audit gates remain clean with any added dependencies

## 0.5.x: Loss-Aware Round Trip

Goal: read and write existing IDML packages without semantic loss for supported
parts.

Included:

- XML writer traits
- namespace and qualified-name preservation
- unknown element and attribute preservation
- ZIP writer with `mimetype` first and uncompressed
- validation before write

Exit criteria:

- read-write-read round trip preserves DesignMap, story text, spread references,
  and resource inventory
- generated package structure follows Adobe IDML container expectations

## 0.6.x: Programmatic Generator

Goal: create basic IDML packages from Rust data.

Included:

- builder API for documents, spreads, pages, text frames, and stories
- deterministic ID generation
- default styles/resources required for valid packages
- writer examples for catalog-style generation

Exit criteria:

- generated fixtures open in supported InDesign versions when available
- schema validation passes where generated schemas are available

## 0.7.x: Hardening and Compatibility

Goal: broaden compatibility across real-world IDML versions and hostile input.

Included:

- expanded fixture corpus
- long-running fuzz campaigns
- duplicate-ID and cross-file integrity validator
- stricter ZIP bomb heuristics
- benchmark suite for large story and spread files

Exit criteria:

- documented compatibility matrix
- no known panics from fuzzing corpus
- performance baselines published in release evidence

## 0.8.x: Public API Stabilization

Goal: settle the API surface before 1.0.

Included:

- naming review
- error taxonomy review
- feature flag review
- migration guide from 0.7
- deprecation of unstable experimental APIs

Exit criteria:

- public API is documented and internally consistent
- no known breaking changes planned for the 1.0 core reader/writer workflow

## 0.9.x: 1.0 Release Candidate Series

Goal: validate the 1.0 contract.

Included:

- release candidate tags
- full documentation pass
- examples for reading, resolving, editing, and writing
- security review checklist
- final dependency and license review

Exit criteria:

- downstream smoke project can use the public API without internal modules
- all examples compile and run
- no critical or high known issues remain

## 1.0.0: Fully Functional Core

Definition of fully functional:

- open untrusted IDML packages safely
- parse DesignMap, stories, spreads, and core resources
- lazily resolve cross-file relationships
- extract text by story and by layout location
- validate relational integrity
- preserve unknown XML enough for safe round trips
- generate valid basic IDML packages
- write packages with correct `mimetype` handling
- expose stable, documented APIs with no unsafe code

Non-goals for 1.0:

- perfect rendering parity with InDesign
- full typography engine behavior
- every proprietary or obscure resource type
- cryptographic constant-time claims for Base64
