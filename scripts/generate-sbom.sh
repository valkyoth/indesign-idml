#!/usr/bin/env sh
set -eu

mkdir -p target/sbom

spdx_output="target/sbom/indesign-idml.spdx.json"
cyclonedx_output="target/sbom/indesign-idml.cyclonedx.json"

cargo sbom --output-format spdx_json_2_3 > "$spdx_output"
cargo sbom --output-format cyclone_dx_json_1_4 > "$cyclonedx_output"

test -s "$spdx_output"
test -s "$cyclonedx_output"
grep -q '"spdxVersion"[[:space:]]*:[[:space:]]*"SPDX-2.3"' "$spdx_output"
grep -q '"bomFormat"[[:space:]]*:[[:space:]]*"CycloneDX"' "$cyclonedx_output"

sha256sum "$spdx_output" "$cyclonedx_output"
