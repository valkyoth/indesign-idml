#!/usr/bin/env sh
set -eu

fixture_dir="${PRIVATE_IDML_FIXTURES:-tests/fixtures/private-idml}"

if [ ! -d "$fixture_dir" ]; then
    echo "private IDML fixture directory not found; skipping"
    exit 0
fi

tmp_list="$(mktemp)"
trap 'rm -f "$tmp_list"' EXIT HUP INT TERM

find "$fixture_dir" -type f -name '*.idml' -print | sort >"$tmp_list"

if [ ! -s "$tmp_list" ]; then
    echo "no private .idml fixtures found; skipping"
    exit 0
fi

count=0
while IFS= read -r fixture; do
    count=$((count + 1))
    echo "smoke: $(basename "$fixture")"
    cargo run --quiet --features cli --bin idml-text -- "$fixture" >/dev/null
    cargo run --quiet --features cli --bin idml-text -- --all-spreads "$fixture" >/dev/null
done <"$tmp_list"

echo "private IDML fixture smoke passed for $count file(s)"
