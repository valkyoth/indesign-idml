//! Extract story text from a package using the streaming visitor API.

use indesign_idml::prelude::*;
use std::io::Cursor;

fn main() -> Result<()> {
    let sample = sample_package()?;
    let mut package = IdmlPackage::new(Cursor::new(sample))?;
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
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#,
    )?;
    writer.add_file(
        "Stories/Story_u1.xml",
        br#"<Story Self="u1"><Content>Hello from IDML</Content></Story>"#,
    )?;
    Ok(writer.finish()?.into_inner())
}
