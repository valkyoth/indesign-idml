#!/usr/bin/env sh
set -eu

read_text_output="$(cargo run --quiet --example read_text)"
if [ "$read_text_output" != "u1: Hello from IDML" ]; then
    echo "read_text example produced unexpected output:" >&2
    printf '%s\n' "$read_text_output" >&2
    exit 1
fi

build_basic_output="$(cargo run --quiet --example build_basic)"
if [ "$build_basic_output" != "u1: Catalog entry" ]; then
    echo "build_basic example produced unexpected output:" >&2
    printf '%s\n' "$build_basic_output" >&2
    exit 1
fi
