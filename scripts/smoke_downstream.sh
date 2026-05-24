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
  <idPkg:Spread src="Spreads/Spread_u10.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#,
    )?;
    writer.add_file(
        "Spreads/Spread_u10.xml",
        br#"<Spread Self="u10">
  <Page Self="p1" GeometricBounds="0 0 100 100" />
  <TextFrame Self="tf1" ParentStory="u1" GeometricBounds="10 10 20 20" />
</Spread>"#,
    )?;
    writer.add_file(
        "Stories/Story_u1.xml",
        br#"<Story Self="u1"><Content>Downstream</Content></Story>"#,
    )?;

    let bytes = writer.finish()?.into_inner();
    let mut package = IdmlPackage::new(Cursor::new(bytes.clone()))?;
    let document = package.read_document()?;
    assert_eq!(document.stories["u1"].text, "Downstream");
    let mut frame_seen = false;
    document.try_for_each_spread_text_on_page("u10", "p1", |frame| {
        assert_eq!(frame.frame.id.as_deref(), Some("tf1"));
        assert_eq!(frame.story_id, "u1");
        assert_eq!(frame.text, "Downstream");
        frame_seen = true;
        Ok::<(), IdmlError>(())
    })?;
    assert!(frame_seen);

    let mut package = IdmlPackage::new(Cursor::new(bytes))?;
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
