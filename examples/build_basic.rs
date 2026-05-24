//! Build a minimal typed document and resolve layout-linked text.

use indesign_idml::prelude::*;
use std::io::Cursor;

fn main() -> Result<()> {
    let mut document = IdmlDocument::new(DesignMap {
        id: "d1".to_owned(),
        ..DesignMap::default()
    });

    document.add_story(
        "u1",
        IdmlPath::new("Stories/Story_u1.xml")?,
        Story {
            id: Some("u1".to_owned()),
            text: "Catalog entry".to_owned(),
            metadata: Default::default(),
            preference: None,
            incopy_export_option: None,
        },
    )?;
    document.add_spread(
        "u10",
        IdmlPath::new("Spreads/Spread_u10.xml")?,
        Spread {
            id: Some("u10".to_owned()),
            pages: vec![Page {
                id: Some("p1".to_owned()),
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(0.0),
                    Points::new(792.0),
                    Points::new(612.0),
                )),
            }],
            guides: Vec::new(),
            text_frames: vec![TextFrame {
                id: Some("tf1".to_owned()),
                layer_id: None,
                parent_story: Some("u1".to_owned()),
                previous_text_frame: None,
                next_text_frame: None,
                geometric_bounds: Some(Rect::new(
                    Points::new(72.0),
                    Points::new(72.0),
                    Points::new(144.0),
                    Points::new(360.0),
                )),
                metadata: Default::default(),
                path_geometry: None,
            }],
            page_items: Vec::new(),
        },
    )?;

    let bytes = document.write_to(Cursor::new(Vec::new()))?.into_inner();
    let mut package = IdmlPackage::new(Cursor::new(bytes))?;
    let design_map = package.read_designmap()?;
    let frames = package.resolve_spread_text_on_page(&design_map, "u10", "p1")?;

    for frame in frames {
        println!("{}: {}", frame.story_id, frame.text);
    }

    Ok(())
}
