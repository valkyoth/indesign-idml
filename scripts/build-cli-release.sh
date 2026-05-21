#!/usr/bin/env sh
set -eu

out_dir="${OUT_DIR:-dist}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"

if [ -z "$version" ]; then
    echo "could not read package version from Cargo.toml" >&2
    exit 1
fi

if [ -n "${TARGET:-}" ]; then
    cargo build --locked --release --features cli --bin idml-text --target "$TARGET"
    build_dir="target/$TARGET/release"
    target_name="$TARGET"
else
    cargo build --locked --release --features cli --bin idml-text
    build_dir="target/release"
    target_name="$(rustc -vV | sed -n 's/^host: //p')"
fi

case "$target_name" in
    *windows*) exe_suffix=".exe" ;;
    *) exe_suffix="" ;;
esac

artifact_name="idml-text-v${version}-${target_name}${exe_suffix}"
artifact_path="${out_dir}/${artifact_name}"

mkdir -p "$out_dir"
cp "${build_dir}/idml-text${exe_suffix}" "$artifact_path"
sha256sum "$artifact_path" >"${artifact_path}.sha256"

printf '%s\n' "$artifact_path"
printf '%s\n' "${artifact_path}.sha256"
