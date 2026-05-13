//! Parser for the root `designmap.xml` package manifest.

use crate::archive::IdmlPath;
use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::validate_xml_attribute;
use indexmap::{IndexMap, IndexSet};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::escape;
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

/// Borrowed pointer to a story package entry listed in [`DesignMap`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoryPointer<'a> {
    id: &'a str,
    path: &'a IdmlPath,
}

impl<'a> StoryPointer<'a> {
    /// Returns the story ID derived from the package reference.
    #[must_use]
    pub const fn id(self) -> &'a str {
        self.id
    }

    /// Returns the story archive path.
    #[must_use]
    pub const fn path(self) -> &'a IdmlPath {
        self.path
    }
}

/// Borrowed pointer to a spread package entry listed in [`DesignMap`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpreadPointer<'a> {
    id: &'a str,
    path: &'a IdmlPath,
}

impl<'a> SpreadPointer<'a> {
    /// Returns the spread ID derived from the package reference.
    #[must_use]
    pub const fn id(self) -> &'a str {
        self.id
    }

    /// Returns the spread archive path.
    #[must_use]
    pub const fn path(self) -> &'a IdmlPath {
        self.path
    }
}

/// Borrowed pointer to a master spread package entry listed in [`DesignMap`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MasterSpreadPointer<'a> {
    id: &'a str,
    path: &'a IdmlPath,
}

impl<'a> MasterSpreadPointer<'a> {
    /// Returns the master spread ID derived from the package reference.
    #[must_use]
    pub const fn id(self) -> &'a str {
        self.id
    }

    /// Returns the master spread archive path.
    #[must_use]
    pub const fn path(self) -> &'a IdmlPath {
        self.path
    }
}

/// Borrowed pointer to an untyped `idPkg:*` resource listed in [`DesignMap`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageResourcePointer<'a> {
    element: &'a str,
    path: &'a IdmlPath,
}

impl<'a> PackageResourcePointer<'a> {
    /// Returns the qualified `idPkg:*` element name.
    #[must_use]
    pub const fn element(self) -> &'a str {
        self.element
    }

    /// Returns the resource archive path.
    #[must_use]
    pub const fn path(self) -> &'a IdmlPath {
        self.path
    }
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

        design_map.validate()?;
        Ok(design_map)
    }

    /// Validates known package IDs and package reference paths.
    pub fn validate(&self) -> Result<()> {
        validate_unique_known_ids(self)?;
        validate_unique_package_paths(self)?;
        Ok(())
    }

    /// Returns story IDs in package order.
    pub fn story_ids(&self) -> impl Iterator<Item = &str> {
        self.story_srcs.keys().map(String::as_str)
    }

    /// Returns spread IDs in package order.
    pub fn spread_ids(&self) -> impl Iterator<Item = &str> {
        self.spread_srcs.keys().map(String::as_str)
    }

    /// Returns master spread IDs in package order.
    pub fn master_spread_ids(&self) -> impl Iterator<Item = &str> {
        self.master_spread_srcs.keys().map(String::as_str)
    }

    /// Returns lazy story pointers in package order.
    pub fn story_pointers(&self) -> impl Iterator<Item = StoryPointer<'_>> + '_ {
        self.story_srcs
            .iter()
            .map(|(id, path)| StoryPointer { id, path })
    }

    /// Returns lazy spread pointers in package order.
    pub fn spread_pointers(&self) -> impl Iterator<Item = SpreadPointer<'_>> + '_ {
        self.spread_srcs
            .iter()
            .map(|(id, path)| SpreadPointer { id, path })
    }

    /// Returns lazy master spread pointers in package order.
    pub fn master_spread_pointers(&self) -> impl Iterator<Item = MasterSpreadPointer<'_>> + '_ {
        self.master_spread_srcs
            .iter()
            .map(|(id, path)| MasterSpreadPointer { id, path })
    }

    /// Returns lazy pointers for untyped `idPkg:*` resources in package order.
    pub fn package_resource_pointers(
        &self,
    ) -> impl Iterator<Item = PackageResourcePointer<'_>> + '_ {
        self.other_package_srcs.iter().flat_map(|(element, paths)| {
            paths.iter().map(|path| PackageResourcePointer {
                element: element.as_str(),
                path,
            })
        })
    }

    /// Serializes this design map into a standalone `designmap.xml` document.
    pub fn to_xml(&self) -> Result<String> {
        serialize_designmap(self)
    }
}

impl XmlLoadable for DesignMap {
    fn from_xml(xml: &str) -> Result<Self> {
        DesignMap::from_xml(xml)
    }
}

impl XmlSaveable for DesignMap {
    fn to_xml(&self) -> Result<String> {
        serialize_designmap(self)
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
    if map.contains_key(&id) {
        return Err(IdmlError::DuplicateId {
            kind: "DesignMap package",
            id,
        });
    }
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

fn serialize_designmap(design_map: &DesignMap) -> Result<String> {
    design_map.validate()?;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Document");
    if !design_map.id.is_empty() {
        push_attr(&mut xml, "Document", "Self", &design_map.id)?;
    }
    xml.push_str(" xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\">\n");

    push_package_refs(&mut xml, "idPkg:Spread", design_map.spread_srcs.values())?;
    push_package_refs(
        &mut xml,
        "idPkg:MasterSpread",
        design_map.master_spread_srcs.values(),
    )?;
    push_package_refs(&mut xml, "idPkg:Story", design_map.story_srcs.values())?;

    for (element, srcs) in &design_map.other_package_srcs {
        push_package_refs(&mut xml, element, srcs.iter())?;
    }

    xml.push_str("</Document>\n");
    Ok(xml)
}

fn validate_unique_known_ids(design_map: &DesignMap) -> Result<()> {
    let mut seen = IndexSet::new();

    for id in design_map
        .spread_srcs
        .keys()
        .chain(design_map.story_srcs.keys())
        .chain(design_map.master_spread_srcs.keys())
    {
        if !seen.insert(id.as_str()) {
            return Err(IdmlError::DuplicateId {
                kind: "DesignMap package",
                id: id.clone(),
            });
        }
    }

    Ok(())
}

fn validate_unique_package_paths(design_map: &DesignMap) -> Result<()> {
    let mut seen = IndexSet::new();

    for path in design_map
        .spread_srcs
        .values()
        .chain(design_map.story_srcs.values())
        .chain(design_map.master_spread_srcs.values())
        .chain(design_map.other_package_srcs.values().flatten())
    {
        if !seen.insert(path.clone()) {
            return Err(IdmlError::InvalidReference {
                kind: "DesignMap package path",
                id: path.to_string(),
                reason: "path is referenced more than once",
            });
        }
    }

    Ok(())
}

fn push_package_refs<'a>(
    xml: &mut String,
    element: &str,
    srcs: impl IntoIterator<Item = &'a IdmlPath>,
) -> Result<()> {
    validate_package_element_name(element)?;
    for src in srcs {
        xml.push_str("  <");
        xml.push_str(element);
        push_attr(xml, element, "src", src.as_str())?;
        xml.push_str(" />\n");
    }
    Ok(())
}

fn push_attr(
    xml: &mut String,
    element: impl Into<String>,
    name: &'static str,
    value: &str,
) -> Result<()> {
    validate_xml_attribute(element, name, value)?;
    xml.push(' ');
    xml.push_str(name);
    xml.push_str("=\"");
    xml.push_str(escape(value).as_ref());
    xml.push('"');
    Ok(())
}

pub(crate) fn validate_package_element_name(element: &str) -> Result<()> {
    let Some(local) = element.strip_prefix("idPkg:") else {
        return Err(invalid_package_element_name());
    };
    if local.is_empty() {
        return Err(invalid_package_element_name());
    }
    if !local
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(invalid_package_element_name());
    }
    Ok(())
}

fn invalid_package_element_name() -> IdmlError {
    IdmlError::InvalidAttribute {
        element: "DesignMap".to_owned(),
        attribute: "idPkg element",
        reason: "invalid XML element name",
    }
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
    fn pointers_follow_designmap_order_without_copying_paths() {
        let xml = r#"<Document>
  <idPkg:Spread src="Spreads/Spread_u10.xml" />
  <idPkg:Spread src="Spreads/Spread_u11.xml" />
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_u20.xml" />
  <idPkg:Story src="Stories/Story_u2.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
  <idPkg:Graphic src="Resources/Graphic.xml" />
  <idPkg:Fonts src="Resources/Fonts.xml" />
</Document>"#;

        let design_map = DesignMap::from_xml(xml).unwrap();
        let spread_pointers = design_map
            .spread_pointers()
            .map(|pointer| (pointer.id(), pointer.path().as_str()))
            .collect::<Vec<_>>();
        let master_spread_pointers = design_map
            .master_spread_pointers()
            .map(|pointer| (pointer.id(), pointer.path().as_str()))
            .collect::<Vec<_>>();
        let story_pointers = design_map
            .story_pointers()
            .map(|pointer| (pointer.id(), pointer.path().as_str()))
            .collect::<Vec<_>>();
        let resource_pointers = design_map
            .package_resource_pointers()
            .map(|pointer| (pointer.element(), pointer.path().as_str()))
            .collect::<Vec<_>>();

        assert_eq!(
            spread_pointers,
            [
                ("u10", "Spreads/Spread_u10.xml"),
                ("u11", "Spreads/Spread_u11.xml"),
            ]
        );
        assert_eq!(
            master_spread_pointers,
            [("u20", "MasterSpreads/MasterSpread_u20.xml")]
        );
        assert_eq!(
            story_pointers,
            [
                ("u2", "Stories/Story_u2.xml"),
                ("u1", "Stories/Story_u1.xml"),
            ]
        );
        assert_eq!(
            resource_pointers,
            [
                ("idPkg:Graphic", "Resources/Graphic.xml"),
                ("idPkg:Fonts", "Resources/Fonts.xml"),
            ]
        );
    }

    #[test]
    fn rejects_dangerous_package_srcs() {
        let err = DesignMap::from_xml(
            r#"<Document><idPkg:Story src="../Stories/Story_u1.xml" /></Document>"#,
        )
        .unwrap_err();

        assert!(matches!(err, IdmlError::InvalidArchivePath { .. }));
    }

    #[test]
    fn rejects_duplicate_known_package_ids() {
        let err = DesignMap::from_xml(
            r#"<Document>
  <idPkg:Story src="Stories/Story_u1.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "DesignMap package",
                id,
            } if id == "u1"
        ));
    }

    #[test]
    fn rejects_cross_type_known_package_id_collisions() {
        let err = DesignMap::from_xml(
            r#"<Document>
  <idPkg:Spread src="Spreads/Spread_u1.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "DesignMap package",
                id,
            } if id == "u1"
        ));
    }

    #[test]
    fn rejects_duplicate_package_paths() {
        let err = DesignMap::from_xml(
            r#"<Document>
  <idPkg:Story src="Resources/Shared.xml" />
  <idPkg:Graphic src="Resources/Shared.xml" />
</Document>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "DesignMap package path",
                id,
                reason: "path is referenced more than once",
            } if id == "Resources/Shared.xml"
        ));
    }

    #[test]
    fn serializes_designmap_with_escaped_attributes_and_round_trips() {
        let mut design_map = DesignMap {
            id: "d&\"1".to_owned(),
            ..DesignMap::default()
        };
        design_map.spread_srcs.insert(
            "u1".to_owned(),
            IdmlPath::new("Spreads/Spread_u1.xml").unwrap(),
        );
        design_map.story_srcs.insert(
            "u2".to_owned(),
            IdmlPath::new("Stories/Story_u2.xml").unwrap(),
        );
        design_map.other_package_srcs.insert(
            "idPkg:Graphic".to_owned(),
            vec![IdmlPath::new("Resources/A&B.xml").unwrap()],
        );

        let xml = <DesignMap as crate::XmlSaveable>::to_xml(&design_map).unwrap();
        let round_trip = <DesignMap as crate::XmlLoadable>::from_xml(&xml).unwrap();

        assert!(xml.contains("Self=\"d&amp;&quot;1\""));
        assert!(xml.contains("src=\"Resources/A&amp;B.xml\""));
        assert_eq!(round_trip.id, design_map.id);
        assert_eq!(round_trip.spread_srcs, design_map.spread_srcs);
        assert_eq!(round_trip.story_srcs, design_map.story_srcs);
        assert_eq!(
            round_trip.other_package_srcs["idPkg:Graphic"],
            design_map.other_package_srcs["idPkg:Graphic"]
        );
    }

    #[test]
    fn serializer_rejects_invalid_package_element_names() {
        let mut design_map = DesignMap::default();
        design_map.other_package_srcs.insert(
            "bad tag".to_owned(),
            vec![IdmlPath::new("Resources/Graphic.xml").unwrap()],
        );

        let err = design_map.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "idPkg element",
                reason: "invalid XML element name",
            } if element == "DesignMap"
        ));
    }

    #[test]
    fn serializer_rejects_duplicate_package_paths() {
        let mut design_map = DesignMap::default();
        design_map.story_srcs.insert(
            "u1".to_owned(),
            IdmlPath::new("Resources/Shared.xml").unwrap(),
        );
        design_map.other_package_srcs.insert(
            "idPkg:Graphic".to_owned(),
            vec![IdmlPath::new("Resources/Shared.xml").unwrap()],
        );

        let err = design_map.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "DesignMap package path",
                id,
                reason: "path is referenced more than once",
            } if id == "Resources/Shared.xml"
        ));
    }

    #[test]
    fn serializer_rejects_xml_forbidden_document_id() {
        let design_map = DesignMap {
            id: "d\u{0}".to_owned(),
            ..DesignMap::default()
        };

        let err = design_map.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Document"
        ));
    }
}
