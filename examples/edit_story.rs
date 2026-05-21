//! Edit story text through the typed document model and write a new package.

use indesign_idml::prelude::*;
use std::io::Cursor;

fn main() -> Result<()> {
    let mut package = IdmlPackage::new(Cursor::new(sample_package()?))?;
    let mut document = IdmlDocument::read_from_package(&mut package)?;

    let story = document
        .stories
        .get_mut("u1")
        .ok_or_else(|| IdmlError::MissingReference {
            kind: "story model",
            id: "u1".to_owned(),
        })?;
    story.text = "Edited story text".to_owned();

    let edited = document.write_to(Cursor::new(Vec::new()))?.into_inner();
    let mut package = IdmlPackage::new(Cursor::new(edited))?;
    let design_map = package.read_designmap()?;

    package.try_for_each_story_text(&design_map, |story_id, text| {
        println!("{story_id}: {text}");
        Ok::<(), IdmlError>(())
    })?;

    Ok(())
}

fn sample_package() -> Result<Vec<u8>> {
    let mut writer = IdmlPackageWriter::new(Cursor::new(Vec::new()))?;
    writer.add_file(
        "designmap.xml",
        br#"<Document Self="d1">
  <idPkg:Spread src="Spreads/Spread_u10.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#,
    )?;
    writer.add_file("Spreads/Spread_u10.xml", br#"<Spread Self="u10" />"#)?;
    writer.add_file(
        "Stories/Story_u1.xml",
        br#"<Story Self="u1"><Content>Original story text</Content></Story>"#,
    )?;
    Ok(writer.finish()?.into_inner())
}
