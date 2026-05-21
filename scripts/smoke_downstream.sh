#!/usr/bin/env sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
workdir="$(mktemp -d "${TMPDIR:-/tmp}/indesign-idml-downstream.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT HUP INT TERM

mkdir -p "$workdir/src"

cat >"$workdir/Cargo.toml" <<EOF
[package]
name = "indesign-idml-downstream-smoke"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
indesign-idml = { path = "$repo_root" }
EOF

cat >"$workdir/src/main.rs" <<'EOF'
use indesign_idml::prelude::*;
use std::io::Cursor;

fn main() -> Result<()> {
    let mut writer = IdmlPackageWriter::new(Cursor::new(Vec::new()))?;
    writer.add_file(
        "designmap.xml",
        br#"<Document Self="d1">
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#,
    )?;
    writer.add_file(
        "Stories/Story_u1.xml",
        br#"<Story Self="u1"><Content>Downstream</Content></Story>"#,
    )?;

    let mut package = IdmlPackage::new(Cursor::new(writer.finish()?.into_inner()))?;
    let design_map = package.read_designmap()?;
    let mut seen = false;
    package.try_for_each_story_text(&design_map, |story_id, text| {
        assert_eq!(story_id, "u1");
        assert_eq!(text, "Downstream");
        seen = true;
        Ok::<(), IdmlError>(())
    })?;
    assert!(seen);

    Ok(())
}
EOF

cargo check --manifest-path "$workdir/Cargo.toml" --quiet
