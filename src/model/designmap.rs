//! Parser for the root `designmap.xml` package manifest.

use crate::archive::IdmlPath;
use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::{
    DEFAULT_MAX_XML_DEPTH, DEFAULT_MAX_XML_INPUT_BYTES, XmlDepthTracker, enforce_xml_input_bytes,
    validate_xml_attribute,
};
use indexmap::{IndexMap, IndexSet};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};

/// Represents high-level package references found in `designmap.xml`.
#[derive(Clone, Debug, Default, Eq)]
pub struct DesignMap {
    /// Document `Self` identifier.
    pub id: String,
    /// Layers in document order.
    pub layers: Vec<Layer>,
    /// Ordered spread ID to archive path mapping.
    pub spread_srcs: IndexMap<String, IdmlPath>,
    /// Ordered story ID to archive path mapping.
    pub story_srcs: IndexMap<String, IdmlPath>,
    /// Ordered master spread ID to archive path mapping.
    pub master_spread_srcs: IndexMap<String, IdmlPath>,
    /// Other `idPkg:*` references preserved by qualified tag name.
    pub other_package_srcs: IndexMap<String, Vec<IdmlPath>>,
    /// Exact `idPkg:*` package reference order as parsed from `designmap.xml`.
    ///
    /// This list is optional for manually constructed maps. When empty, writer
    /// output is derived from the typed lookup maps. Parsed maps populate this
    /// list so mixed spread/story/resource ordering can be preserved.
    pub package_refs: Vec<DesignMapPackageRef>,
}

impl PartialEq for DesignMap {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.layers == other.layers
            && self.spread_srcs == other.spread_srcs
            && self.story_srcs == other.story_srcs
            && self.master_spread_srcs == other.master_spread_srcs
            && self.other_package_srcs == other.other_package_srcs
            && compatible_package_ref_order(self, other)
    }
}

/// Layer metadata from `designmap.xml`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layer {
    /// Layer `Self` identifier, when present.
    pub id: Option<String>,
    /// Human-readable layer name, when present.
    pub name: Option<String>,
    /// Layer visibility flag, when present.
    pub visible: Option<bool>,
    /// Layer lock flag, when present.
    pub locked: Option<bool>,
}

impl Layer {
    fn from_xml_event(event: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<Self> {
        Ok(Self {
            id: optional_attr(event, "Self", reader)?,
            name: optional_attr(event, "Name", reader)?,
            visible: optional_bool_attr(event, "Visible", reader)?,
            locked: optional_bool_attr(event, "Locked", reader)?,
        })
    }
}

/// One ordered package reference from `designmap.xml`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignMapPackageRef {
    /// Qualified `idPkg:*` element name.
    pub element: String,
    /// Derived ID for typed package references.
    pub id: Option<String>,
    /// Logical archive path from the `src` attribute.
    pub path: IdmlPath,
}

impl DesignMapPackageRef {
    /// Creates an ordered spread package reference.
    #[must_use]
    pub fn spread(id: impl Into<String>, path: IdmlPath) -> Self {
        Self {
            element: "idPkg:Spread".to_owned(),
            id: Some(id.into()),
            path,
        }
    }

    /// Creates an ordered story package reference.
    #[must_use]
    pub fn story(id: impl Into<String>, path: IdmlPath) -> Self {
        Self {
            element: "idPkg:Story".to_owned(),
            id: Some(id.into()),
            path,
        }
    }

    /// Creates an ordered master spread package reference.
    #[must_use]
    pub fn master_spread(id: impl Into<String>, path: IdmlPath) -> Self {
        Self {
            element: "idPkg:MasterSpread".to_owned(),
            id: Some(id.into()),
            path,
        }
    }

    /// Creates an ordered untyped package resource reference.
    pub fn resource(element: impl Into<String>, path: IdmlPath) -> Result<Self> {
        let element = element.into();
        validate_package_element_name(&element)?;
        Ok(Self {
            element,
            id: None,
            path,
        })
    }
}

/// Default maximum package references accepted in one `designmap.xml`.
pub const DEFAULT_MAX_DESIGNMAP_PACKAGE_REFS: usize = 20_000;

/// Default maximum layers accepted in one `designmap.xml`.
pub const DEFAULT_MAX_DESIGNMAP_LAYERS: usize = 10_000;

/// DesignMap parser limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignMapParseOptions {
    /// Maximum UTF-8 bytes accepted for `designmap.xml`.
    pub max_xml_bytes: usize,
    /// Maximum `Layer` elements collected from one manifest.
    pub max_layers: usize,
    /// Maximum `idPkg:*` package references collected from one manifest.
    pub max_package_refs: usize,
    /// Maximum XML element nesting depth accepted while parsing.
    pub max_xml_depth: usize,
}

impl Default for DesignMapParseOptions {
    fn default() -> Self {
        Self {
            max_xml_bytes: DEFAULT_MAX_XML_INPUT_BYTES,
            max_layers: DEFAULT_MAX_DESIGNMAP_LAYERS,
            max_package_refs: DEFAULT_MAX_DESIGNMAP_PACKAGE_REFS,
            max_xml_depth: DEFAULT_MAX_XML_DEPTH,
        }
    }
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
        Self::from_xml_with_options(xml_content, DesignMapParseOptions::default())
    }

    /// Parses a `designmap.xml` document with explicit limits.
    pub fn from_xml_with_options(
        xml_content: &str,
        options: DesignMapParseOptions,
    ) -> Result<Self> {
        enforce_xml_input_bytes("DesignMap XML bytes", xml_content, options.max_xml_bytes)?;

        let mut reader = Reader::from_str(xml_content);
        reader.config_mut().trim_text(true);

        let mut design_map = Self::default();
        let mut package_refs = 0usize;
        let mut xml_depth = XmlDepthTracker::new(options.max_xml_depth, "DesignMap XML depth");

        loop {
            match reader.read_event()? {
                Event::Start(e) => {
                    xml_depth.enter()?;
                    if e.name().as_ref() == b"Document"
                        && let Some(id) = optional_attr(&e, "Self", &reader)?
                    {
                        design_map.id = id;
                    } else if e.name().as_ref() == b"Layer" {
                        push_layer_limited(
                            &mut design_map.layers,
                            Layer::from_xml_event(&e, &reader)?,
                            options.max_layers,
                        )?;
                    }
                }
                Event::Empty(e) => {
                    xml_depth.visit_empty()?;
                    if e.name().as_ref() == b"Document"
                        && let Some(id) = optional_attr(&e, "Self", &reader)?
                    {
                        design_map.id = id;
                    } else if e.name().as_ref() == b"Layer" {
                        push_layer_limited(
                            &mut design_map.layers,
                            Layer::from_xml_event(&e, &reader)?,
                            options.max_layers,
                        )?;
                    } else if e.name().as_ref() == b"idPkg:Spread" {
                        increment_package_refs(&mut package_refs, options.max_package_refs)?;
                        let package_ref = parse_package_ref(
                            &e,
                            "idPkg:Spread",
                            &reader,
                            &mut design_map.spread_srcs,
                        )?;
                        design_map.package_refs.push(package_ref);
                    } else if e.name().as_ref() == b"idPkg:Story" {
                        increment_package_refs(&mut package_refs, options.max_package_refs)?;
                        let package_ref = parse_package_ref(
                            &e,
                            "idPkg:Story",
                            &reader,
                            &mut design_map.story_srcs,
                        )?;
                        design_map.package_refs.push(package_ref);
                    } else if e.name().as_ref() == b"idPkg:MasterSpread" {
                        increment_package_refs(&mut package_refs, options.max_package_refs)?;
                        let package_ref = parse_package_ref(
                            &e,
                            "idPkg:MasterSpread",
                            &reader,
                            &mut design_map.master_spread_srcs,
                        )?;
                        design_map.package_refs.push(package_ref);
                    } else if e.name().as_ref().starts_with(b"idPkg:") {
                        increment_package_refs(&mut package_refs, options.max_package_refs)?;
                        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        let src = required_attr(&e, &name, "src", &reader)?;
                        let path = IdmlPath::new(src)?;
                        design_map
                            .other_package_srcs
                            .entry(name.clone())
                            .or_default()
                            .push(path.clone());
                        design_map
                            .package_refs
                            .push(DesignMapPackageRef::resource(name, path)?);
                    }
                }
                Event::End(_) => {
                    xml_depth.exit();
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
        validate_ordered_package_refs(self)?;
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
        let pointers = if self.package_refs.is_empty() {
            self.other_package_srcs
                .iter()
                .flat_map(|(element, paths)| {
                    paths.iter().map(|path| PackageResourcePointer {
                        element: element.as_str(),
                        path,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            self.package_refs
                .iter()
                .filter(|reference| is_untyped_package_element(&reference.element))
                .map(|reference| PackageResourcePointer {
                    element: reference.element.as_str(),
                    path: &reference.path,
                })
                .collect::<Vec<_>>()
        };
        pointers.into_iter()
    }

    /// Serializes this design map into a standalone `designmap.xml` document.
    pub fn to_xml(&self) -> Result<String> {
        serialize_designmap(self)
    }

    pub(crate) fn ensure_package_refs_initialized(&mut self) {
        if !self.package_refs.is_empty() {
            return;
        }
        self.package_refs = package_refs_from_maps(self);
    }
}

fn increment_package_refs(package_refs: &mut usize, max_package_refs: usize) -> Result<()> {
    let next = package_refs
        .checked_add(1)
        .ok_or(IdmlError::LimitExceeded {
            what: "DesignMap package reference count",
            limit: max_package_refs as u64,
            actual: u64::MAX,
        })?;

    if next > max_package_refs {
        return Err(IdmlError::LimitExceeded {
            what: "DesignMap package reference count",
            limit: max_package_refs as u64,
            actual: next as u64,
        });
    }

    *package_refs = next;
    Ok(())
}

fn push_layer_limited(layers: &mut Vec<Layer>, layer: Layer, max_layers: usize) -> Result<()> {
    let next = layers
        .len()
        .checked_add(1)
        .ok_or(IdmlError::LimitExceeded {
            what: "DesignMap layer count",
            limit: max_layers as u64,
            actual: u64::MAX,
        })?;

    if next > max_layers {
        return Err(IdmlError::LimitExceeded {
            what: "DesignMap layer count",
            limit: max_layers as u64,
            actual: next as u64,
        });
    }

    layers.push(layer);
    Ok(())
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
) -> Result<DesignMapPackageRef> {
    let src = required_attr(event, element, "src", reader)?;
    let id = id_from_package_src(&src);
    if map.contains_key(&id) {
        return Err(IdmlError::DuplicateId {
            kind: "DesignMap package",
            id,
        });
    }
    let path = IdmlPath::new(src)?;
    map.insert(id.clone(), path.clone());
    Ok(DesignMapPackageRef {
        element: element.to_owned(),
        id: Some(id),
        path,
    })
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
    attribute: &'static str,
    reader: &Reader<&[u8]>,
) -> Result<Option<String>> {
    let mut value = None;
    for attr in event.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == attribute.as_bytes() {
            if value.is_some() {
                return Err(IdmlError::InvalidAttribute {
                    element: String::from_utf8_lossy(event.name().as_ref()).into_owned(),
                    attribute,
                    reason: "duplicate attribute",
                });
            }
            let decoded = attr
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())?
                .into_owned();
            validate_xml_attribute(
                String::from_utf8_lossy(event.name().as_ref()).into_owned(),
                attribute,
                &decoded,
            )?;
            value = Some(decoded);
        }
    }
    Ok(value)
}

fn optional_bool_attr(
    event: &BytesStart<'_>,
    attribute: &'static str,
    reader: &Reader<&[u8]>,
) -> Result<Option<bool>> {
    let Some(value) = optional_attr(event, attribute, reader)? else {
        return Ok(None);
    };
    match value.as_str() {
        "true" | "True" | "TRUE" => Ok(Some(true)),
        "false" | "False" | "FALSE" => Ok(Some(false)),
        _ => Err(IdmlError::InvalidAttribute {
            element: "Layer".to_owned(),
            attribute,
            reason: "expected boolean value",
        }),
    }
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

    for layer in &design_map.layers {
        push_layer(&mut xml, layer)?;
    }

    if design_map.package_refs.is_empty() {
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
    } else {
        for reference in &design_map.package_refs {
            push_package_ref(&mut xml, &reference.element, &reference.path)?;
        }
    }

    xml.push_str("</Document>\n");
    Ok(xml)
}

fn push_layer(xml: &mut String, layer: &Layer) -> Result<()> {
    xml.push_str("  <Layer");
    if let Some(id) = &layer.id {
        push_attr(xml, "Layer", "Self", id)?;
    }
    if let Some(name) = &layer.name {
        push_attr(xml, "Layer", "Name", name)?;
    }
    if let Some(visible) = layer.visible {
        push_attr(xml, "Layer", "Visible", bool_str(visible))?;
    }
    if let Some(locked) = layer.locked {
        push_attr(xml, "Layer", "Locked", bool_str(locked))?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn validate_unique_known_ids(design_map: &DesignMap) -> Result<()> {
    let mut seen = IndexSet::new();

    for layer in &design_map.layers {
        if let Some(id) = &layer.id {
            remember_designmap_id(&mut seen, id)?;
        }
    }

    for id in design_map
        .spread_srcs
        .keys()
        .chain(design_map.story_srcs.keys())
        .chain(design_map.master_spread_srcs.keys())
    {
        remember_designmap_id(&mut seen, id)?;
    }

    Ok(())
}

fn remember_designmap_id(seen: &mut IndexSet<String>, id: &str) -> Result<()> {
    if !seen.insert(id.to_owned()) {
        return Err(IdmlError::DuplicateId {
            kind: "DesignMap object",
            id: id.to_owned(),
        });
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

fn validate_ordered_package_refs(design_map: &DesignMap) -> Result<()> {
    if design_map.package_refs.is_empty() {
        return Ok(());
    }

    let expected = expected_package_ref_keys(design_map)?;
    if design_map.package_refs.len() != expected.len() {
        return Err(IdmlError::InvalidReference {
            kind: "DesignMap package order",
            id: "package_refs".to_owned(),
            reason: "ordered package reference count does not match package maps",
        });
    }

    let mut actual = IndexSet::with_capacity(design_map.package_refs.len());
    for reference in &design_map.package_refs {
        validate_package_element_name(&reference.element)?;
        let key = package_ref_key(reference)?;
        if !expected.contains(&key) {
            return Err(IdmlError::InvalidReference {
                kind: "DesignMap package order",
                id: reference.path.to_string(),
                reason: "ordered package reference is not present in package maps",
            });
        }
        if !actual.insert(key) {
            return Err(IdmlError::InvalidReference {
                kind: "DesignMap package order",
                id: reference.path.to_string(),
                reason: "ordered package reference is duplicated",
            });
        }
    }

    Ok(())
}

fn expected_package_ref_keys(design_map: &DesignMap) -> Result<IndexSet<PackageRefKey>> {
    let mut expected = IndexSet::new();

    for (id, path) in &design_map.spread_srcs {
        expected.insert(PackageRefKey::typed("idPkg:Spread", id, path));
    }
    for (id, path) in &design_map.story_srcs {
        expected.insert(PackageRefKey::typed("idPkg:Story", id, path));
    }
    for (id, path) in &design_map.master_spread_srcs {
        expected.insert(PackageRefKey::typed("idPkg:MasterSpread", id, path));
    }
    for (element, paths) in &design_map.other_package_srcs {
        validate_package_element_name(element)?;
        for path in paths {
            expected.insert(PackageRefKey::resource(element, path));
        }
    }

    Ok(expected)
}

fn package_ref_key(reference: &DesignMapPackageRef) -> Result<PackageRefKey> {
    match reference.element.as_str() {
        "idPkg:Spread" | "idPkg:Story" | "idPkg:MasterSpread" => {
            let Some(id) = &reference.id else {
                return Err(IdmlError::InvalidReference {
                    kind: "DesignMap package order",
                    id: reference.path.to_string(),
                    reason: "typed ordered package reference is missing its ID",
                });
            };
            Ok(PackageRefKey::typed(
                &reference.element,
                id,
                &reference.path,
            ))
        }
        _ => {
            if reference.id.is_some() {
                return Err(IdmlError::InvalidReference {
                    kind: "DesignMap package order",
                    id: reference.path.to_string(),
                    reason: "untyped ordered package reference must not carry an ID",
                });
            }
            Ok(PackageRefKey::resource(&reference.element, &reference.path))
        }
    }
}

fn compatible_package_ref_order(left: &DesignMap, right: &DesignMap) -> bool {
    left.package_refs.is_empty()
        || right.package_refs.is_empty()
        || left.package_refs == right.package_refs
}

fn package_refs_from_maps(design_map: &DesignMap) -> Vec<DesignMapPackageRef> {
    let mut refs = Vec::with_capacity(
        design_map.spread_srcs.len()
            + design_map.master_spread_srcs.len()
            + design_map.story_srcs.len()
            + design_map
                .other_package_srcs
                .values()
                .map(Vec::len)
                .sum::<usize>(),
    );

    refs.extend(
        design_map
            .spread_srcs
            .iter()
            .map(|(id, path)| DesignMapPackageRef::spread(id.clone(), path.clone())),
    );
    refs.extend(
        design_map
            .master_spread_srcs
            .iter()
            .map(|(id, path)| DesignMapPackageRef::master_spread(id.clone(), path.clone())),
    );
    refs.extend(
        design_map
            .story_srcs
            .iter()
            .map(|(id, path)| DesignMapPackageRef::story(id.clone(), path.clone())),
    );
    for (element, paths) in &design_map.other_package_srcs {
        refs.extend(paths.iter().map(|path| DesignMapPackageRef {
            element: element.clone(),
            id: None,
            path: path.clone(),
        }));
    }

    refs
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PackageRefKey {
    element: String,
    id: Option<String>,
    path: IdmlPath,
}

impl PackageRefKey {
    fn typed(element: &str, id: &str, path: &IdmlPath) -> Self {
        Self {
            element: element.to_owned(),
            id: Some(id.to_owned()),
            path: path.clone(),
        }
    }

    fn resource(element: &str, path: &IdmlPath) -> Self {
        Self {
            element: element.to_owned(),
            id: None,
            path: path.clone(),
        }
    }
}

fn push_package_refs<'a>(
    xml: &mut String,
    element: &str,
    srcs: impl IntoIterator<Item = &'a IdmlPath>,
) -> Result<()> {
    validate_package_element_name(element)?;
    for src in srcs {
        push_package_ref(xml, element, src)?;
    }
    Ok(())
}

fn push_package_ref(xml: &mut String, element: &str, src: &IdmlPath) -> Result<()> {
    validate_package_element_name(element)?;
    xml.push_str("  <");
    xml.push_str(element);
    push_attr(xml, element, "src", src.as_str())?;
    xml.push_str(" />\n");
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

fn is_untyped_package_element(element: &str) -> bool {
    !matches!(
        element,
        "idPkg:Story" | "idPkg:Spread" | "idPkg:MasterSpread"
    )
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
    use super::{DesignMap, DesignMapParseOptions, Layer};
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
        let ordered_refs = design_map
            .package_refs
            .iter()
            .map(|reference| {
                (
                    reference.element.as_str(),
                    reference.id.as_deref(),
                    reference.path.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            ordered_refs,
            [
                ("idPkg:Spread", Some("u1"), "Spreads/Spread_u1.xml"),
                ("idPkg:Story", Some("u2"), "Stories/Story_u2.xml"),
                (
                    "idPkg:MasterSpread",
                    Some("u3"),
                    "MasterSpreads/MasterSpread_u3.xml"
                ),
                ("idPkg:Graphic", None, "Resources/Graphic.xml"),
            ]
        );
    }

    #[test]
    fn parses_empty_document_root_id() {
        let design_map = DesignMap::from_xml(r#"<Document Self="doc1" />"#).unwrap();

        assert_eq!(design_map.id, "doc1");
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
    fn enforces_package_reference_count_limit() {
        let xml = r#"<Document>
  <idPkg:Spread src="Spreads/Spread_u1.xml" />
  <idPkg:Story src="Stories/Story_u2.xml" />
</Document>"#;

        let err = DesignMap::from_xml_with_options(
            xml,
            DesignMapParseOptions {
                max_package_refs: 1,
                ..DesignMapParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "DesignMap package reference count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn rejects_duplicate_package_src_attributes() {
        let err = DesignMap::from_xml(
            r#"<Document><idPkg:Story src="Stories/Story_u1.xml" src="Stories/Story_u2.xml" /></Document>"#,
        )
        .unwrap_err();

        assert!(
            matches!(
                &err,
                IdmlError::InvalidAttribute {
                    element,
                    attribute: "src",
                    reason: "duplicate attribute",
                } if element == "idPkg:Story"
            ) || matches!(&err, IdmlError::XmlAttribute(_))
        );
    }

    #[test]
    fn rejects_xml_forbidden_document_id_attribute_on_parse() {
        let err = DesignMap::from_xml("<Document Self=\"bad\0id\"></Document>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Document"
        ));
    }

    #[test]
    fn rejects_xml_forbidden_empty_document_id_attribute_on_parse() {
        let err = DesignMap::from_xml("<Document Self=\"bad\0id\" />").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Document"
        ));
    }

    #[test]
    fn parses_layers_in_document_order() {
        let xml = r#"<Document>
  <Layer Self="layer1" Name="Text" Visible="true" Locked="false" />
  <Layer Self="layer2" Name="Art" Visible="False" Locked="True" />
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#;

        let design_map = DesignMap::from_xml(xml).unwrap();

        assert_eq!(design_map.layers.len(), 2);
        assert_eq!(design_map.layers[0].id.as_deref(), Some("layer1"));
        assert_eq!(design_map.layers[0].name.as_deref(), Some("Text"));
        assert_eq!(design_map.layers[0].visible, Some(true));
        assert_eq!(design_map.layers[0].locked, Some(false));
        assert_eq!(design_map.layers[1].id.as_deref(), Some("layer2"));
        assert_eq!(design_map.layers[1].visible, Some(false));
        assert_eq!(design_map.layers[1].locked, Some(true));
    }

    #[test]
    fn enforces_layer_count_limit() {
        let err = DesignMap::from_xml_with_options(
            r#"<Document><Layer Self="layer1" /><Layer Self="layer2" /></Document>"#,
            DesignMapParseOptions {
                max_layers: 1,
                ..DesignMapParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "DesignMap layer count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn enforces_xml_byte_limit() {
        let err = DesignMap::from_xml_with_options(
            "<Document/>",
            DesignMapParseOptions {
                max_xml_bytes: 4,
                ..DesignMapParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "DesignMap XML bytes",
                limit: 4,
                actual: 11,
            }
        ));
    }

    #[test]
    fn rejects_invalid_layer_booleans() {
        let err =
            DesignMap::from_xml(r#"<Document><Layer Self="layer1" Visible="maybe" /></Document>"#)
                .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Visible",
                reason: "expected boolean value",
            } if element == "Layer"
        ));
    }

    #[test]
    fn enforces_xml_depth_limit() {
        let xml = r#"<Document>
  <Wrapper><idPkg:Story src="Stories/Story_u2.xml" /></Wrapper>
</Document>"#;

        let err = DesignMap::from_xml_with_options(
            xml,
            DesignMapParseOptions {
                max_xml_depth: 2,
                ..DesignMapParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "DesignMap XML depth",
                limit: 2,
                actual: 3,
            }
        ));
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
    fn rejects_layer_id_colliding_with_package_id() {
        let err = DesignMap::from_xml(
            r#"<Document>
  <Layer Self="u1" />
  <idPkg:Story src="Stories/Story_u1.xml" />
</Document>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "DesignMap object",
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
                kind: "DesignMap object",
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
            layers: vec![Layer {
                id: Some("layer&1".to_owned()),
                name: Some("Text \"Layer\"".to_owned()),
                visible: Some(true),
                locked: Some(false),
            }],
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
        assert!(xml.contains("Self=\"layer&amp;1\""));
        assert!(xml.contains("Name=\"Text &quot;Layer&quot;\""));
        assert!(xml.contains("Visible=\"true\""));
        assert!(xml.contains("Locked=\"false\""));
        assert!(xml.contains("src=\"Resources/A&amp;B.xml\""));
        assert_eq!(round_trip.id, design_map.id);
        assert_eq!(round_trip.layers, design_map.layers);
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

    #[test]
    fn serializer_preserves_explicit_mixed_package_order() {
        let design_map = DesignMap::from_xml(
            r#"<Document>
  <idPkg:Story src="Stories/Story_u2.xml" />
  <idPkg:Graphic src="Resources/Graphic.xml" />
  <idPkg:Spread src="Spreads/Spread_u1.xml" />
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_u3.xml" />
</Document>"#,
        )
        .unwrap();

        let xml = design_map.to_xml().unwrap();
        let story = xml.find("<idPkg:Story").unwrap();
        let graphic = xml.find("<idPkg:Graphic").unwrap();
        let spread = xml.find("<idPkg:Spread").unwrap();
        let master = xml.find("<idPkg:MasterSpread").unwrap();

        assert!(story < graphic);
        assert!(graphic < spread);
        assert!(spread < master);
        assert_eq!(
            DesignMap::from_xml(&xml).unwrap().package_refs,
            design_map.package_refs
        );
    }

    #[test]
    fn validation_rejects_stale_ordered_package_refs() {
        let mut design_map = DesignMap::default();
        design_map.story_srcs.insert(
            "u1".to_owned(),
            IdmlPath::new("Stories/Story_u1.xml").unwrap(),
        );
        design_map
            .package_refs
            .push(super::DesignMapPackageRef::story(
                "u2",
                IdmlPath::new("Stories/Story_u2.xml").unwrap(),
            ));

        let err = design_map.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "DesignMap package order",
                reason: "ordered package reference is not present in package maps",
                ..
            }
        ));
    }
}
