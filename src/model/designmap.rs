//! Parser for the root `designmap.xml` package manifest.

use crate::archive::IdmlPath;
use crate::error::{IdmlError, Result};
use indexmap::IndexMap;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

/// Represents high-level package references found in `designmap.xml`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignMap {
    /// Document `Self` identifier.
    pub id: String,
    /// Ordered spread ID to archive path mapping.
    pub spread_srcs: IndexMap<String, IdmlPath>,
    /// Ordered story ID to archive path mapping.
    pub story_srcs: IndexMap<String, IdmlPath>,
    /// Ordered master spread ID to archive path mapping.
    pub master_spread_srcs: IndexMap<String, IdmlPath>,
    /// Other `idPkg:*` references preserved by qualified tag name.
    pub other_package_srcs: IndexMap<String, Vec<IdmlPath>>,
}

impl DesignMap {
    /// Parses a `designmap.xml` document.
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml_content);
        reader.config_mut().trim_text(true);

        let mut design_map = Self::default();

        loop {
            match reader.read_event()? {
                Event::Start(e) if e.name().as_ref() == b"Document" => {
                    if let Some(id) = optional_attr(&e, "Self", &reader)? {
                        design_map.id = id;
                    }
                }
                Event::Empty(e) if e.name().as_ref() == b"idPkg:Spread" => {
                    parse_package_ref(&e, "idPkg:Spread", &reader, &mut design_map.spread_srcs)?;
                }
                Event::Empty(e) if e.name().as_ref() == b"idPkg:Story" => {
                    parse_package_ref(&e, "idPkg:Story", &reader, &mut design_map.story_srcs)?;
                }
                Event::Empty(e) if e.name().as_ref() == b"idPkg:MasterSpread" => {
                    parse_package_ref(
                        &e,
                        "idPkg:MasterSpread",
                        &reader,
                        &mut design_map.master_spread_srcs,
                    )?;
                }
                Event::Empty(e) if e.name().as_ref().starts_with(b"idPkg:") => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                    let src = required_attr(&e, &name, "src", &reader)?;
                    design_map
                        .other_package_srcs
                        .entry(name)
                        .or_default()
                        .push(IdmlPath::new(src)?);
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(design_map)
    }

    /// Returns story IDs in package order.
    pub fn story_ids(&self) -> impl Iterator<Item = &str> {
        self.story_srcs.keys().map(String::as_str)
    }
}

fn parse_package_ref(
    event: &BytesStart<'_>,
    element: &'static str,
    reader: &Reader<&[u8]>,
    map: &mut IndexMap<String, IdmlPath>,
) -> Result<()> {
    let src = required_attr(event, element, "src", reader)?;
    let id = id_from_package_src(&src);
    map.insert(id, IdmlPath::new(src)?);
    Ok(())
}

fn required_attr(
    event: &BytesStart<'_>,
    element: &str,
    attribute: &'static str,
    reader: &Reader<&[u8]>,
) -> Result<String> {
    optional_attr(event, attribute, reader)?.ok_or_else(|| IdmlError::MissingAttribute {
        element: element.to_owned(),
        attribute,
    })
}

fn optional_attr(
    event: &BytesStart<'_>,
    attribute: &str,
    reader: &Reader<&[u8]>,
) -> Result<Option<String>> {
    for attr in event.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == attribute.as_bytes() {
            return Ok(Some(
                attr.decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

fn id_from_package_src(src: &str) -> String {
    let file_name = src.rsplit('/').next().unwrap_or(src);
    let stem = file_name.strip_suffix(".xml").unwrap_or(file_name);
    stem.strip_prefix("Spread_")
        .or_else(|| stem.strip_prefix("Story_"))
        .or_else(|| stem.strip_prefix("MasterSpread_"))
        .unwrap_or(stem)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::DesignMap;
    use crate::IdmlError;
    use crate::archive::IdmlPath;

    #[test]
    fn parses_designmap_package_refs_in_order() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document Self="d1" xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <idPkg:Spread src="Spreads/Spread_u1.xml" />
  <idPkg:Story src="Stories/Story_u2.xml" />
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_u3.xml" />
  <idPkg:Graphic src="Resources/Graphic.xml" />
</Document>"#;

        let design_map = DesignMap::from_xml(xml).unwrap();

        assert_eq!(design_map.id, "d1");
        assert_eq!(
            design_map.spread_srcs.get("u1"),
            Some(&IdmlPath::new("Spreads/Spread_u1.xml").unwrap())
        );
        assert_eq!(
            design_map.story_srcs.get("u2"),
            Some(&IdmlPath::new("Stories/Story_u2.xml").unwrap())
        );
        assert_eq!(
            design_map.master_spread_srcs.get("u3"),
            Some(&IdmlPath::new("MasterSpreads/MasterSpread_u3.xml").unwrap())
        );
        assert_eq!(
            design_map.other_package_srcs["idPkg:Graphic"],
            vec![IdmlPath::new("Resources/Graphic.xml").unwrap()]
        );
    }

    #[test]
    fn reports_missing_src_on_package_ref() {
        let err = DesignMap::from_xml("<Document><idPkg:Story /></Document>").unwrap_err();
        assert!(matches!(
            err,
            IdmlError::MissingAttribute {
                element,
                attribute: "src"
            } if element == "idPkg:Story"
        ));
    }

    #[test]
    fn story_ids_follow_designmap_order() {
        let xml = r#"<Document>
  <idPkg:Story src="Stories/Story_u2.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#;

        let design_map = DesignMap::from_xml(xml).unwrap();
        let ids = design_map.story_ids().collect::<Vec<_>>();

        assert_eq!(ids, ["u2", "u1"]);
    }

    #[test]
    fn rejects_dangerous_package_srcs() {
        let err = DesignMap::from_xml(
            r#"<Document><idPkg:Story src="../Stories/Story_u1.xml" /></Document>"#,
        )
        .unwrap_err();

        assert!(matches!(err, IdmlError::InvalidArchivePath { .. }));
    }
}
