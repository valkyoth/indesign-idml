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

test -s LICENSE-MIT
test -s LICENSE-APACHE
test -s README.md
test -s SECURITY_IMPLEMENTATION_PLAN.md
test -s RELEASE_PLAN.md

if ! grep -q '^The MIT License (MIT)$' LICENSE-MIT; then
    echo "LICENSE-MIT does not look like the canonical MIT license used by this repo" >&2
    exit 1
fi

if ! grep -q 'Apache License' LICENSE-APACHE || ! grep -q 'Version 2.0, January 2004' LICENSE-APACHE; then
    echo "LICENSE-APACHE does not look like the canonical Apache 2.0 license" >&2
    exit 1
fi

package_list="$(
    cargo package --locked --allow-dirty --list
)"

for required_package_file in \
    "Cargo.toml" \
    "LICENSE-APACHE" \
    "LICENSE-MIT" \
    "README.md" \
    "RELEASE_PLAN.md" \
    "SECURITY_IMPLEMENTATION_PLAN.md" \
    "src/lib.rs"
do
    if ! printf '%s\n' "$package_list" | grep -qx "$required_package_file"; then
        echo "published package is missing $required_package_file" >&2
        exit 1
    fi
done
