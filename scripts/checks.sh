#!/usr/bin/env sh
set -eu

cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo test --no-default-features
cargo test --no-default-features --features std
cargo test --no-default-features --features serde
cargo doc --no-deps --all-features
scripts/validate-release-metadata.sh
cargo deny check
cargo audit
cargo license --avoid-dev-deps --json >/tmp/indesign-idml-cargo-license.json
scripts/generate-sbom.sh
scripts/reproducible_build_check.sh
