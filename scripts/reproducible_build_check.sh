#!/usr/bin/env sh
set -eu

tmp_a="$(mktemp -d)"
tmp_b="$(mktemp -d)"
trap 'rm -rf "$tmp_a" "$tmp_b"' EXIT

if command -v git >/dev/null 2>&1 && git rev-parse --git-dir >/dev/null 2>&1; then
    SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || printf 0)}"
else
    SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
fi
export SOURCE_DATE_EPOCH

build_once() {
    target_dir="$1"
    remap_flags="--remap-path-prefix=$(pwd)=/source --remap-path-prefix=${target_dir}=/target"
    if [ -n "${RUSTFLAGS:-}" ]; then
        remap_flags="${RUSTFLAGS} ${remap_flags}"
    fi
    CARGO_TARGET_DIR="$target_dir/target" RUSTFLAGS="$remap_flags" cargo build --locked --release --all-features
}

build_once "$tmp_a"
build_once "$tmp_b"

artifact="release/libindesign_idml.rlib"
if ! cmp "$tmp_a/target/$artifact" "$tmp_b/target/$artifact" >/dev/null 2>&1; then
    echo "release library artifact is not reproducible across two clean target dirs" >&2
    exit 1
fi

sha256sum "$tmp_a/target/$artifact"
