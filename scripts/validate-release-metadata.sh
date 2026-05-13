#!/usr/bin/env sh
set -eu

manifest="Cargo.toml"

check_contains() {
    key="$1"
    expected="$2"
    if ! grep -Eq "^${key}[[:space:]]*=[[:space:]]*\"${expected}\"" "$manifest"; then
        echo "Cargo.toml must contain ${key} = \"${expected}\"" >&2
        exit 1
    fi
}

check_contains "name" "indesign-idml"
check_contains "license" "MIT OR Apache-2.0"
check_contains "edition" "2024"
check_contains "rust-version" "1.95"

for required in description repository homepage documentation readme keywords categories; do
    if ! grep -Eq "^${required}[[:space:]]*=" "$manifest"; then
        echo "Cargo.toml missing required release metadata: ${required}" >&2
        exit 1
    fi
done
