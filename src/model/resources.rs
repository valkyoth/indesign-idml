//! Resource inventory for untyped IDML package references.

use crate::archive::{ArchiveEntry, DEFAULT_MAX_ENTRY_UNCOMPRESSED_SIZE, IdmlPath};
use crate::encoding::{Base64Mode, decode_standard, decoded_standard_len};
use crate::error::{IdmlError, Result};
use crate::model::designmap::{DesignMap, PackageResourcePointer, validate_package_element_name};
use crate::xml::{
    AnyRootTracker, DEFAULT_MAX_XML_DEPTH, DEFAULT_MAX_XML_INPUT_BYTES, XmlDepthTracker,
    configure_idml_reader, enforce_xml_input_bytes, reject_xml_doctype, validate_xml_attribute,
};
use indexmap::IndexMap;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use zip::CompressionMethod;

/// Default maximum decoded bytes accepted for one embedded resource asset.
pub const DEFAULT_MAX_EMBEDDED_ASSET_DECODED_BYTES: usize =
    DEFAULT_MAX_ENTRY_UNCOMPRESSED_SIZE as usize;

/// Default maximum total decoded embedded asset bytes accepted in one XML entry.
pub const DEFAULT_MAX_EMBEDDED_ASSET_TOTAL_DECODED_BYTES: usize =
    DEFAULT_MAX_ENTRY_UNCOMPRESSED_SIZE as usize;

/// Default maximum encoded Base64 bytes accepted for one embedded resource asset.
pub const DEFAULT_MAX_EMBEDDED_ASSET_ENCODED_BYTES: usize =
    DEFAULT_MAX_EMBEDDED_ASSET_DECODED_BYTES * 2;

/// Default maximum embedded asset payloads accepted in one XML entry.
pub const DEFAULT_MAX_EMBEDDED_ASSETS: usize = 10_000;

/// Default maximum typed resource items accepted in one resource XML entry.
pub const DEFAULT_MAX_RESOURCE_ITEMS: usize = 100_000;

/// Default maximum attributes captured for one typed resource item.
pub const DEFAULT_MAX_RESOURCE_ITEM_ATTRIBUTES: usize = 128;

/// Default maximum total UTF-8 bytes captured from one resource item's attributes.
pub const DEFAULT_MAX_RESOURCE_ITEM_ATTRIBUTE_BYTES: usize = 256 * 1024;

/// Broad resource category inferred from a `designmap.xml` package reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceKind {
    /// Master spread package entry.
    MasterSpread,
    /// Style-related package entry.
    Styles,
    /// Swatch or color-related package entry.
    Swatches,
    /// Font-related package entry.
    Fonts,
    /// Link-related package entry.
    Links,
    /// Graphic-related package entry.
    Graphics,
    /// Preferences or settings package entry.
    Preferences,
    /// XML tag metadata package entry.
    Tags,
    /// Any currently unclassified package reference.
    Other(String),
}

/// One manifest-referenced resource entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceReference {
    /// Inferred resource kind.
    pub kind: ResourceKind,
    /// Qualified `idPkg:*` element name from `designmap.xml`.
    pub element: String,
    /// Resource ID when the package reference has one.
    pub id: Option<String>,
    /// Logical archive path for the resource.
    pub path: IdmlPath,
    /// ZIP metadata when the inventory was built from an opened package.
    pub archive: Option<ResourceArchiveMetadata>,
}

/// Archive metadata for one manifest-referenced resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceArchiveMetadata {
    /// Compressed byte size from ZIP metadata.
    pub compressed_size: u64,
    /// Uncompressed byte size from ZIP metadata.
    pub uncompressed_size: u64,
    /// ZIP compression method.
    pub compression: CompressionMethod,
}

/// Integrity limits for resource inventory metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceIntegrityOptions {
    /// Require archive metadata to be present for every resource.
    pub require_archive_metadata: bool,
    /// Maximum uncompressed bytes allowed for one non-graphic resource.
    pub max_resource_uncompressed_size: u64,
    /// Maximum uncompressed bytes allowed for one graphic resource.
    pub max_graphic_uncompressed_size: u64,
    /// Maximum allowed uncompressed-to-compressed size ratio.
    pub max_compression_ratio: u64,
}

/// Parser limits for embedded Base64 resource payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmbeddedAssetParseOptions {
    /// Maximum UTF-8 bytes accepted for the XML entry.
    pub max_xml_bytes: usize,
    /// Base64 compatibility profile.
    pub base64_mode: Base64Mode,
    /// Maximum `Contents` payloads accepted in one XML entry.
    pub max_assets: usize,
    /// Maximum encoded Base64 bytes collected from one `Contents` element.
    pub max_encoded_bytes: usize,
    /// Maximum decoded bytes accepted from one `Contents` element.
    pub max_decoded_bytes: usize,
    /// Maximum total decoded bytes accepted across all `Contents` elements.
    pub max_total_decoded_bytes: usize,
    /// Maximum XML element nesting depth accepted while parsing.
    pub max_xml_depth: usize,
}

/// Parser limits for typed resource item inventories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceItemParseOptions {
    /// Maximum UTF-8 bytes accepted for the XML entry.
    pub max_xml_bytes: usize,
    /// Maximum resource items collected from one XML entry.
    pub max_items: usize,
    /// Maximum attributes captured for one item.
    pub max_item_attributes: usize,
    /// Maximum total UTF-8 bytes captured from one item's attribute names and values.
    pub max_item_attribute_bytes: usize,
    /// Maximum XML element nesting depth accepted while parsing.
    pub max_xml_depth: usize,
}

impl Default for EmbeddedAssetParseOptions {
    fn default() -> Self {
        Self {
            max_xml_bytes: DEFAULT_MAX_XML_INPUT_BYTES,
            base64_mode: Base64Mode::Strict,
            max_assets: DEFAULT_MAX_EMBEDDED_ASSETS,
            max_encoded_bytes: DEFAULT_MAX_EMBEDDED_ASSET_ENCODED_BYTES,
            max_decoded_bytes: DEFAULT_MAX_EMBEDDED_ASSET_DECODED_BYTES,
            max_total_decoded_bytes: DEFAULT_MAX_EMBEDDED_ASSET_TOTAL_DECODED_BYTES,
            max_xml_depth: DEFAULT_MAX_XML_DEPTH,
        }
    }
}

impl Default for ResourceItemParseOptions {
    fn default() -> Self {
        Self {
            max_xml_bytes: DEFAULT_MAX_XML_INPUT_BYTES,
            max_items: DEFAULT_MAX_RESOURCE_ITEMS,
            max_item_attributes: DEFAULT_MAX_RESOURCE_ITEM_ATTRIBUTES,
            max_item_attribute_bytes: DEFAULT_MAX_RESOURCE_ITEM_ATTRIBUTE_BYTES,
            max_xml_depth: DEFAULT_MAX_XML_DEPTH,
        }
    }
}

/// Decoded embedded asset payload found in a resource or spread XML file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddedAsset {
    /// Qualified XML element that carried the Base64 payload.
    pub source_element: String,
    /// Decoded asset bytes.
    pub bytes: Vec<u8>,
}

/// A typed item found inside a resource XML entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceItem {
    /// Broad inferred resource kind.
    pub kind: ResourceKind,
    /// Qualified XML element name.
    pub element: String,
    /// Resource `Self` identifier, when present.
    pub id: Option<String>,
    /// Human-readable name, when present.
    pub name: Option<String>,
    /// Captured XML attributes in source order.
    pub attributes: IndexMap<String, String>,
}

/// Typed item inventory for one manifest-referenced resource entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceItemGroup {
    /// Manifest resource reference.
    pub resource: ResourceReference,
    /// Items extracted from that resource XML entry.
    pub items: Vec<ResourceItem>,
}

impl ResourceItemGroup {
    /// Validates resource item IDs within this resource entry.
    pub fn validate(&self) -> Result<()> {
        validate_unique_resource_item_ids(&self.items)
    }
}

impl Default for ResourceIntegrityOptions {
    fn default() -> Self {
        Self {
            require_archive_metadata: true,
            max_resource_uncompressed_size: DEFAULT_MAX_ENTRY_UNCOMPRESSED_SIZE,
            max_graphic_uncompressed_size: DEFAULT_MAX_ENTRY_UNCOMPRESSED_SIZE,
            max_compression_ratio: 100,
        }
    }
}

/// Ordered resource inventory derived from a [`DesignMap`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceInventory {
    /// Resource references in `designmap.xml` order by category.
    pub resources: Vec<ResourceReference>,
}

impl ResourceInventory {
    /// Builds a resource inventory without reading referenced entry bodies.
    pub fn from_designmap(design_map: &DesignMap) -> Result<Self> {
        design_map.validate()?;

        let mut resources = Vec::with_capacity(
            design_map.master_spread_srcs.len()
                + design_map
                    .other_package_srcs
                    .values()
                    .map(Vec::len)
                    .sum::<usize>(),
        );

        if design_map.package_refs.is_empty() {
            for pointer in design_map.master_spread_pointers() {
                resources.push(ResourceReference {
                    kind: ResourceKind::MasterSpread,
                    element: "idPkg:MasterSpread".to_owned(),
                    id: Some(pointer.id().to_owned()),
                    path: pointer.path().clone(),
                    archive: None,
                });
            }

            for pointer in design_map.package_resource_pointers() {
                push_resource_pointer(&mut resources, pointer)?;
            }
        } else {
            for reference in &design_map.package_refs {
                if reference.element == "idPkg:MasterSpread" {
                    resources.push(ResourceReference {
                        kind: ResourceKind::MasterSpread,
                        element: reference.element.clone(),
                        id: reference.id.clone(),
                        path: reference.path.clone(),
                        archive: None,
                    });
                } else if !matches!(reference.element.as_str(), "idPkg:Story" | "idPkg:Spread") {
                    validate_package_element_name(&reference.element)?;
                    resources.push(ResourceReference {
                        kind: ResourceKind::from_reference(&reference.element, &reference.path),
                        element: reference.element.clone(),
                        id: None,
                        path: reference.path.clone(),
                        archive: None,
                    });
                }
            }
        }

        Ok(Self { resources })
    }

    /// Returns all references with the requested resource kind.
    pub fn by_kind(&self, kind: ResourceKind) -> impl Iterator<Item = &ResourceReference> {
        self.resources
            .iter()
            .filter(move |resource| resource.kind == kind)
    }

    /// Attaches archive metadata for all inventory entries.
    pub fn attach_archive_metadata(
        &mut self,
        entries: &IndexMap<IdmlPath, ArchiveEntry>,
    ) -> Result<()> {
        for resource in &mut self.resources {
            let entry = entries
                .get(&resource.path)
                .ok_or_else(|| IdmlError::MissingArchiveEntry(resource.path.to_string()))?;
            resource.archive = Some(ResourceArchiveMetadata {
                compressed_size: entry.compressed_size,
                uncompressed_size: entry.uncompressed_size,
                compression: entry.compression,
            });
        }
        Ok(())
    }

    /// Returns a new inventory with archive metadata attached.
    pub fn with_archive_metadata(
        mut self,
        entries: &IndexMap<IdmlPath, ArchiveEntry>,
    ) -> Result<Self> {
        self.attach_archive_metadata(entries)?;
        Ok(self)
    }

    /// Validates resource archive metadata against secure default limits.
    pub fn validate_integrity(&self) -> Result<()> {
        self.validate_integrity_with_options(ResourceIntegrityOptions::default())
    }

    /// Validates resource archive metadata against explicit limits.
    pub fn validate_integrity_with_options(&self, options: ResourceIntegrityOptions) -> Result<()> {
        for resource in &self.resources {
            let Some(metadata) = &resource.archive else {
                if options.require_archive_metadata {
                    return Err(IdmlError::InvalidReference {
                        kind: "resource archive metadata",
                        id: resource.path.to_string(),
                        reason: "metadata is missing",
                    });
                }
                continue;
            };

            validate_resource_compression(resource, metadata)?;
            validate_resource_size(resource, metadata, options)?;
            validate_resource_compression_ratio(resource, metadata, options)?;
        }
        Ok(())
    }
}

fn push_resource_pointer(
    resources: &mut Vec<ResourceReference>,
    pointer: PackageResourcePointer<'_>,
) -> Result<()> {
    validate_package_element_name(pointer.element())?;
    resources.push(ResourceReference {
        kind: ResourceKind::from_reference(pointer.element(), pointer.path()),
        element: pointer.element().to_owned(),
        id: None,
        path: pointer.path().clone(),
        archive: None,
    });
    Ok(())
}

/// Extracts embedded Base64 assets from `Contents` elements with secure defaults.
///
/// IDML can store embedded image bytes in `Contents` elements. The payload may
/// be split across multiple text or CDATA chunks, so this parser joins chunks
/// only within one `Contents` element and applies explicit encoded and decoded
/// size limits before returning bytes.
pub fn extract_embedded_assets(xml_content: &str) -> Result<Vec<EmbeddedAsset>> {
    extract_embedded_assets_with_options(xml_content, EmbeddedAssetParseOptions::default())
}

/// Extracts embedded Base64 assets from `Contents` elements with explicit limits.
pub fn extract_embedded_assets_with_options(
    xml_content: &str,
    options: EmbeddedAssetParseOptions,
) -> Result<Vec<EmbeddedAsset>> {
    enforce_xml_input_bytes(
        "embedded asset XML bytes",
        xml_content,
        options.max_xml_bytes,
    )?;

    let mut reader = Reader::from_str(xml_content);
    configure_idml_reader(&mut reader, false);

    let mut assets = Vec::new();
    let mut contents_depth = 0usize;
    let mut total_decoded_bytes = 0usize;
    let mut source_element = String::new();
    let mut encoded = String::new();
    let mut xml_depth = XmlDepthTracker::new(options.max_xml_depth, "embedded asset XML depth");
    let mut root = AnyRootTracker::default();

    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                root.enter(e.name().as_ref())?;
                xml_depth.enter()?;
                if is_contents_element(&e) {
                    if contents_depth == 0 {
                        source_element = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        encoded.clear();
                    }
                    contents_depth = contents_depth.saturating_add(1);
                }
            }
            Event::Empty(e) => {
                root.visit_empty(e.name().as_ref())?;
                xml_depth.visit_empty()?;
                if is_contents_element(&e) {
                    let bytes =
                        decode_embedded_asset_limited(&[], options, &mut total_decoded_bytes)?;
                    push_asset_limited(
                        &mut assets,
                        options.max_assets,
                        EmbeddedAsset {
                            source_element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                            bytes,
                        },
                    )?;
                }
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"Contents" && contents_depth > 0 => {
                contents_depth -= 1;
                if contents_depth == 0 {
                    let bytes = decode_embedded_asset_limited(
                        encoded.as_bytes(),
                        options,
                        &mut total_decoded_bytes,
                    )?;
                    push_asset_limited(
                        &mut assets,
                        options.max_assets,
                        EmbeddedAsset {
                            source_element: source_element.clone(),
                            bytes,
                        },
                    )?;
                    encoded.clear();
                }
                xml_depth.exit();
                root.exit();
            }
            Event::End(_) => {
                xml_depth.exit();
                root.exit();
            }
            Event::Text(e) => {
                root.visit_text(e.as_ref())?;
                if contents_depth > 0 {
                    push_encoded_limited(
                        &mut encoded,
                        e.decode()?.as_ref(),
                        options.max_encoded_bytes,
                    )?;
                }
            }
            Event::CData(e) => {
                root.visit_cdata()?;
                if contents_depth > 0 {
                    push_encoded_limited(
                        &mut encoded,
                        e.xml_content(XmlVersion::Implicit1_0)?.as_ref(),
                        options.max_encoded_bytes,
                    )?;
                }
            }
            Event::GeneralRef(_) => {
                root.visit_reference()?;
                if contents_depth > 0 {
                    return Err(IdmlError::InvalidElement {
                        element: "#reference".to_owned(),
                        reason: "entity reference inside embedded asset contents",
                    });
                }
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => break,
            _ => {}
        }
    }

    if contents_depth != 0 {
        return Err(IdmlError::InvalidElement {
            element: source_element,
            reason: "unclosed Contents element",
        });
    }
    xml_depth.finish()?;
    root.finish()?;
    Ok(assets)
}

/// Extracts typed resource items from a resource XML entry with secure defaults.
pub fn extract_resource_items(xml_content: &str, kind: ResourceKind) -> Result<Vec<ResourceItem>> {
    extract_resource_items_with_options(xml_content, kind, ResourceItemParseOptions::default())
}

/// Extracts typed resource items from a resource XML entry with explicit limits.
pub fn extract_resource_items_with_options(
    xml_content: &str,
    kind: ResourceKind,
    options: ResourceItemParseOptions,
) -> Result<Vec<ResourceItem>> {
    enforce_xml_input_bytes(
        "resource item XML bytes",
        xml_content,
        options.max_xml_bytes,
    )?;

    let mut reader = Reader::from_str(xml_content);
    configure_idml_reader(&mut reader, true);

    let mut items = Vec::new();
    let mut xml_depth = XmlDepthTracker::new(options.max_xml_depth, "resource item XML depth");
    let mut root = AnyRootTracker::default();

    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                root.enter(e.name().as_ref())?;
                xml_depth.enter()?;
                maybe_push_resource_item(&mut items, &e, &reader, &kind, options)?;
            }
            Event::Empty(e) => {
                root.visit_empty(e.name().as_ref())?;
                xml_depth.visit_empty()?;
                maybe_push_resource_item(&mut items, &e, &reader, &kind, options)?;
            }
            Event::End(_) => {
                xml_depth.exit();
                root.exit();
            }
            Event::Text(e) => {
                root.visit_text(e.as_ref())?;
            }
            Event::CData(_) => {
                root.visit_cdata()?;
            }
            Event::GeneralRef(_) => {
                root.visit_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => break,
            _ => {}
        }
    }

    xml_depth.finish()?;
    root.finish()?;
    validate_unique_resource_item_ids(&items)?;
    Ok(items)
}

impl ResourceKind {
    /// Classifies a package reference using the element first, then the archive path.
    #[must_use]
    pub fn from_reference(element: &str, path: &IdmlPath) -> Self {
        match Self::from_idpkg_element(element) {
            Self::Other(_) => {
                Self::from_path(path).unwrap_or_else(|| Self::Other(element.to_owned()))
            }
            known => known,
        }
    }

    /// Classifies an `idPkg:*` element name into a broad resource kind.
    #[must_use]
    pub fn from_idpkg_element(element: &str) -> Self {
        let local = element.strip_prefix("idPkg:").unwrap_or(element);
        match local {
            "Style" | "Styles" => Self::Styles,
            "Swatch" | "Swatches" | "Color" | "Colors" => Self::Swatches,
            "Font" | "Fonts" => Self::Fonts,
            "Link" | "Links" => Self::Links,
            "Graphic" | "Graphics" => Self::Graphics,
            "Preference" | "Preferences" => Self::Preferences,
            "Tag" | "Tags" => Self::Tags,
            _ => Self::Other(element.to_owned()),
        }
    }

    /// Classifies a resource by conventional IDML archive path.
    #[must_use]
    pub fn from_path(path: &IdmlPath) -> Option<Self> {
        let path = path.as_str().to_ascii_lowercase();
        let file = path.rsplit('/').next().unwrap_or(path.as_str());

        if path.contains("masterspreads/") {
            Some(Self::MasterSpread)
        } else if file.contains("style") {
            Some(Self::Styles)
        } else if file.contains("swatch") || file.contains("color") {
            Some(Self::Swatches)
        } else if file.contains("font") {
            Some(Self::Fonts)
        } else if file.contains("link") {
            Some(Self::Links)
        } else if file.contains("graphic") || path.contains("links/") {
            Some(Self::Graphics)
        } else if file.contains("preference") || file.contains("setting") {
            Some(Self::Preferences)
        } else if file.contains("tag") {
            Some(Self::Tags)
        } else {
            None
        }
    }
}

fn validate_resource_compression(
    resource: &ResourceReference,
    metadata: &ResourceArchiveMetadata,
) -> Result<()> {
    match metadata.compression {
        CompressionMethod::Stored | CompressionMethod::Deflated => Ok(()),
        _ => Err(IdmlError::InvalidReference {
            kind: "resource archive compression",
            id: resource.path.to_string(),
            reason: "unsupported ZIP compression method",
        }),
    }
}

fn validate_resource_size(
    resource: &ResourceReference,
    metadata: &ResourceArchiveMetadata,
    options: ResourceIntegrityOptions,
) -> Result<()> {
    let limit = if resource.kind == ResourceKind::Graphics {
        options.max_graphic_uncompressed_size
    } else {
        options.max_resource_uncompressed_size
    };

    if metadata.uncompressed_size > limit {
        return Err(IdmlError::LimitExceeded {
            what: "resource uncompressed size",
            limit,
            actual: metadata.uncompressed_size,
        });
    }
    Ok(())
}

fn validate_resource_compression_ratio(
    resource: &ResourceReference,
    metadata: &ResourceArchiveMetadata,
    options: ResourceIntegrityOptions,
) -> Result<()> {
    if metadata.compression != CompressionMethod::Deflated || metadata.uncompressed_size == 0 {
        return Ok(());
    }
    if metadata.compressed_size == 0 {
        return Err(IdmlError::InvalidReference {
            kind: "resource archive compression",
            id: resource.path.to_string(),
            reason: "compressed size is zero for non-empty deflated resource",
        });
    }
    let allowed_uncompressed = metadata
        .compressed_size
        .checked_mul(options.max_compression_ratio)
        .ok_or(IdmlError::LimitExceeded {
            what: "resource compression ratio limit product",
            limit: u64::MAX,
            actual: u64::MAX,
        })?;
    if metadata.uncompressed_size > allowed_uncompressed {
        return Err(IdmlError::LimitExceeded {
            what: "resource compression ratio",
            limit: options.max_compression_ratio,
            actual: compression_ratio_ceil(metadata.uncompressed_size, metadata.compressed_size),
        });
    }
    Ok(())
}

fn maybe_push_resource_item(
    items: &mut Vec<ResourceItem>,
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    kind: &ResourceKind,
    options: ResourceItemParseOptions,
) -> Result<()> {
    if !resource_kind_matches_element(kind, event.name().as_ref()) {
        return Ok(());
    }

    let attributes = collect_resource_attrs_limited(event, reader, options)?;
    let id = first_attr_value(&attributes, &["Self", "Id", "ID"]).map(str::to_owned);
    let name = first_attr_value(&attributes, &["Name", "FullName", "AppliedFont", "Label"])
        .map(str::to_owned);
    push_resource_item_limited(
        items,
        options.max_items,
        ResourceItem {
            kind: kind.clone(),
            element: String::from_utf8_lossy(event.name().as_ref()).into_owned(),
            id,
            name,
            attributes,
        },
    )
}

fn resource_kind_matches_element(kind: &ResourceKind, name: &[u8]) -> bool {
    let local = local_name(name).to_ascii_lowercase();
    match kind {
        ResourceKind::MasterSpread => local == b"masterspread",
        ResourceKind::Styles => {
            matches!(
                local.as_slice(),
                b"style"
                    | b"paragraphstyle"
                    | b"characterstyle"
                    | b"objectstyle"
                    | b"tablestyle"
                    | b"cellstyle"
            )
        }
        ResourceKind::Swatches => matches!(
            local.as_slice(),
            b"swatch" | b"color" | b"gradient" | b"tint" | b"mixedink"
        ),
        ResourceKind::Fonts => matches!(local.as_slice(), b"font" | b"fontfamily" | b"fontstyle"),
        ResourceKind::Links => matches!(
            local.as_slice(),
            b"link" | b"hyperlink" | b"hyperlinkurlsource" | b"hyperlinktextdestination"
        ),
        ResourceKind::Graphics => matches!(
            local.as_slice(),
            b"graphic" | b"image" | b"pdf" | b"eps" | b"wmf" | b"pict" | b"importedpage"
        ),
        ResourceKind::Preferences => {
            local.ends_with(b"preference") || local.ends_with(b"preferences")
        }
        ResourceKind::Tags => matches!(local.as_slice(), b"tag" | b"xmltag"),
        ResourceKind::Other(_) => false,
    }
}

fn collect_resource_attrs_limited(
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    options: ResourceItemParseOptions,
) -> Result<IndexMap<String, String>> {
    let mut attributes = IndexMap::new();
    let mut total_bytes = 0usize;

    for attr in event.attributes() {
        let attr = attr?;
        let name = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        if attributes.contains_key(&name) {
            return Err(IdmlError::InvalidAttribute {
                element: String::from_utf8_lossy(event.name().as_ref()).into_owned(),
                attribute: "resource item attribute",
                reason: "duplicate attribute",
            });
        }
        let value = attr
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())?
            .into_owned();
        validate_xml_attribute(
            String::from_utf8_lossy(event.name().as_ref()).into_owned(),
            "resource item attribute",
            &value,
        )?;
        let next_count = attributes
            .len()
            .checked_add(1)
            .ok_or(IdmlError::LimitExceeded {
                what: "resource item attribute count",
                limit: options.max_item_attributes as u64,
                actual: u64::MAX,
            })?;
        if next_count > options.max_item_attributes {
            return Err(IdmlError::LimitExceeded {
                what: "resource item attribute count",
                limit: options.max_item_attributes as u64,
                actual: next_count as u64,
            });
        }
        total_bytes = total_bytes
            .checked_add(name.len())
            .and_then(|total| total.checked_add(value.len()))
            .ok_or(IdmlError::LimitExceeded {
                what: "resource item attribute bytes",
                limit: options.max_item_attribute_bytes as u64,
                actual: u64::MAX,
            })?;
        if total_bytes > options.max_item_attribute_bytes {
            return Err(IdmlError::LimitExceeded {
                what: "resource item attribute bytes",
                limit: options.max_item_attribute_bytes as u64,
                actual: total_bytes as u64,
            });
        }
        attributes.insert(name, value);
    }
    Ok(attributes)
}

fn first_attr_value<'a>(
    attributes: &'a IndexMap<String, String>,
    names: &[&str],
) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| attributes.get(*name).map(String::as_str))
}

fn push_resource_item_limited(
    items: &mut Vec<ResourceItem>,
    max_items: usize,
    item: ResourceItem,
) -> Result<()> {
    if items.len() >= max_items {
        return Err(IdmlError::LimitExceeded {
            what: "resource item count",
            limit: max_items as u64,
            actual: items.len().saturating_add(1) as u64,
        });
    }
    items.push(item);
    Ok(())
}

fn validate_unique_resource_item_ids(items: &[ResourceItem]) -> Result<()> {
    let mut seen = indexmap::IndexSet::new();
    for item in items {
        let Some(id) = &item.id else {
            continue;
        };
        if !seen.insert(id.as_str()) {
            return Err(IdmlError::DuplicateId {
                kind: "resource item",
                id: id.clone(),
            });
        }
    }
    Ok(())
}

fn compression_ratio_ceil(uncompressed_size: u64, compressed_size: u64) -> u64 {
    uncompressed_size.saturating_add(compressed_size.saturating_sub(1)) / compressed_size
}

fn is_contents_element(event: &BytesStart<'_>) -> bool {
    local_name(event.name().as_ref()) == b"Contents"
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn push_encoded_limited(output: &mut String, text: &str, max_encoded_bytes: usize) -> Result<()> {
    let next_len = output
        .len()
        .checked_add(text.len())
        .ok_or(IdmlError::LimitExceeded {
            what: "embedded asset base64 bytes",
            limit: max_encoded_bytes as u64,
            actual: u64::MAX,
        })?;

    if next_len > max_encoded_bytes {
        return Err(IdmlError::LimitExceeded {
            what: "embedded asset base64 bytes",
            limit: max_encoded_bytes as u64,
            actual: next_len as u64,
        });
    }

    output.push_str(text);
    Ok(())
}

fn decode_embedded_asset_limited(
    input: &[u8],
    options: EmbeddedAssetParseOptions,
    total_decoded_bytes: &mut usize,
) -> Result<Vec<u8>> {
    let decoded_len = decoded_standard_len(input, options.base64_mode)?;
    let next_total =
        total_decoded_bytes
            .checked_add(decoded_len)
            .ok_or(IdmlError::LimitExceeded {
                what: "embedded asset total decoded bytes",
                limit: options.max_total_decoded_bytes as u64,
                actual: u64::MAX,
            })?;

    if next_total > options.max_total_decoded_bytes {
        return Err(IdmlError::LimitExceeded {
            what: "embedded asset total decoded bytes",
            limit: options.max_total_decoded_bytes as u64,
            actual: next_total as u64,
        });
    }

    let bytes = decode_standard(input, options.base64_mode, options.max_decoded_bytes)?;
    *total_decoded_bytes = next_total;
    Ok(bytes)
}

fn push_asset_limited(
    assets: &mut Vec<EmbeddedAsset>,
    max_assets: usize,
    asset: EmbeddedAsset,
) -> Result<()> {
    if assets.len() >= max_assets {
        return Err(IdmlError::LimitExceeded {
            what: "embedded asset count",
            limit: max_assets as u64,
            actual: assets.len().saturating_add(1) as u64,
        });
    }
    assets.push(asset);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        EmbeddedAssetParseOptions, ResourceArchiveMetadata, ResourceIntegrityOptions,
        ResourceInventory, ResourceItem, ResourceItemGroup, ResourceItemParseOptions, ResourceKind,
        ResourceReference, extract_embedded_assets, extract_embedded_assets_with_options,
        extract_resource_items, extract_resource_items_with_options,
    };
    use crate::IdmlError;
    use crate::archive::{ArchiveEntry, IdmlPath};
    use crate::encoding::Base64Mode;
    use crate::model::designmap::DesignMap;
    use indexmap::IndexMap;
    use zip::CompressionMethod;

    #[test]
    fn inventories_master_spreads_and_untyped_resources() {
        let xml = r#"<Document>
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_u20.xml" />
  <idPkg:Graphic src="Resources/Graphic.xml" />
  <idPkg:Fonts src="Resources/Fonts.xml" />
  <idPkg:Swatches src="Resources/Swatches.xml" />
  <idPkg:Custom src="Resources/Custom.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();

        let inventory = ResourceInventory::from_designmap(&design_map).unwrap();

        assert_eq!(inventory.resources.len(), 5);
        assert_eq!(inventory.resources[0].kind, ResourceKind::MasterSpread);
        assert_eq!(inventory.resources[0].id.as_deref(), Some("u20"));
        assert_eq!(
            inventory.resources[0].path,
            IdmlPath::new("MasterSpreads/MasterSpread_u20.xml").unwrap()
        );
        assert_eq!(inventory.resources[1].kind, ResourceKind::Graphics);
        assert_eq!(inventory.resources[1].archive, None);
        assert_eq!(inventory.resources[2].kind, ResourceKind::Fonts);
        assert_eq!(inventory.resources[3].kind, ResourceKind::Swatches);
        assert_eq!(
            inventory.resources[4].kind,
            ResourceKind::Other("idPkg:Custom".to_owned())
        );
    }

    #[test]
    fn inventory_classifies_generic_resources_by_path() {
        let xml = r#"<Document>
  <idPkg:Resource src="Resources/Styles.xml" />
  <idPkg:Resource src="Resources/Fonts.xml" />
  <idPkg:Resource src="Resources/Swatches.xml" />
  <idPkg:Resource src="Resources/Links.xml" />
  <idPkg:Resource src="Resources/Preferences.xml" />
  <idPkg:Resource src="XML/Tags.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();

        let inventory = ResourceInventory::from_designmap(&design_map).unwrap();
        let kinds = inventory
            .resources
            .iter()
            .map(|resource| resource.kind.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            [
                ResourceKind::Styles,
                ResourceKind::Fonts,
                ResourceKind::Swatches,
                ResourceKind::Links,
                ResourceKind::Preferences,
                ResourceKind::Tags,
            ]
        );
    }

    #[test]
    fn inventory_preserves_mixed_manifest_resource_order() {
        let xml = r#"<Document>
  <idPkg:Graphic src="Resources/Graphic.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_u20.xml" />
  <idPkg:Fonts src="Resources/Fonts.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();

        let inventory = ResourceInventory::from_designmap(&design_map).unwrap();
        let refs = inventory
            .resources
            .iter()
            .map(|resource| {
                (
                    resource.element.as_str(),
                    resource.id.as_deref(),
                    resource.path.as_str(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            refs,
            [
                ("idPkg:Graphic", None, "Resources/Graphic.xml"),
                (
                    "idPkg:MasterSpread",
                    Some("u20"),
                    "MasterSpreads/MasterSpread_u20.xml"
                ),
                ("idPkg:Fonts", None, "Resources/Fonts.xml"),
            ]
        );
    }

    #[test]
    fn filters_resources_by_kind() {
        let xml = r#"<Document>
  <idPkg:Graphic src="Resources/A.xml" />
  <idPkg:Fonts src="Resources/Fonts.xml" />
  <idPkg:Graphic src="Resources/B.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();
        let inventory = ResourceInventory::from_designmap(&design_map).unwrap();

        let graphics = inventory
            .by_kind(ResourceKind::Graphics)
            .map(|resource| resource.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(graphics, ["Resources/A.xml", "Resources/B.xml"]);
    }

    #[test]
    fn classifies_known_idpkg_elements() {
        assert_eq!(
            ResourceKind::from_idpkg_element("idPkg:Style"),
            ResourceKind::Styles
        );
        assert_eq!(
            ResourceKind::from_idpkg_element("idPkg:Color"),
            ResourceKind::Swatches
        );
        assert_eq!(
            ResourceKind::from_idpkg_element("idPkg:Link"),
            ResourceKind::Links
        );
        assert_eq!(
            ResourceKind::from_idpkg_element("idPkg:Custom"),
            ResourceKind::Other("idPkg:Custom".to_owned())
        );
    }

    #[test]
    fn known_resource_element_takes_precedence_over_path_hint() {
        let path = IdmlPath::new("Resources/Fonts.xml").unwrap();

        assert_eq!(
            ResourceKind::from_reference("idPkg:Graphic", &path),
            ResourceKind::Graphics
        );
    }

    #[test]
    fn extracts_style_resource_items() {
        let xml = r#"<Styles>
  <ParagraphStyle Self="p1" Name="Body" />
  <CharacterStyle Self="c1" Name="Emphasis" />
  <Color Self="ignored" Name="Red" />
</Styles>"#;

        let items = extract_resource_items(xml, ResourceKind::Styles).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].element, "ParagraphStyle");
        assert_eq!(items[0].id.as_deref(), Some("p1"));
        assert_eq!(items[0].name.as_deref(), Some("Body"));
        assert_eq!(items[0].attributes["Self"], "p1");
        assert_eq!(items[0].attributes["Name"], "Body");
        assert_eq!(items[1].element, "CharacterStyle");
    }

    #[test]
    fn extracts_swatch_font_link_graphic_and_tag_items() {
        let xml = r#"<Resources>
  <Color Self="c1" Name="Brand Red" />
  <Font Self="f1" Name="Minion Pro" />
  <Link Self="l1" LinkResourceURI="file:///image.tif" />
  <Image Self="i1" Name="Hero" />
  <XMLTag Self="t1" Name="Headline" />
</Resources>"#;

        assert_eq!(
            extract_resource_items(xml, ResourceKind::Swatches)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            extract_resource_items(xml, ResourceKind::Fonts)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            extract_resource_items(xml, ResourceKind::Links).unwrap()[0].attributes["LinkResourceURI"],
            "file:///image.tif"
        );
        assert_eq!(
            extract_resource_items(xml, ResourceKind::Graphics)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            extract_resource_items(xml, ResourceKind::Tags).unwrap()[0]
                .name
                .as_deref(),
            Some("Headline")
        );
    }

    #[test]
    fn resource_item_parser_enforces_attribute_count_limit() {
        let err = extract_resource_items_with_options(
            r#"<Styles><ParagraphStyle Self="p1" Name="Body" BasedOn="Base" /></Styles>"#,
            ResourceKind::Styles,
            ResourceItemParseOptions {
                max_item_attributes: 2,
                ..ResourceItemParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource item attribute count",
                limit: 2,
                actual: 3,
            }
        ));
    }

    #[test]
    fn resource_item_parser_enforces_attribute_byte_limit() {
        let err = extract_resource_items_with_options(
            r#"<Styles><ParagraphStyle Self="p1" Name="Body" /></Styles>"#,
            ResourceKind::Styles,
            ResourceItemParseOptions {
                max_item_attribute_bytes: 10,
                ..ResourceItemParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource item attribute bytes",
                limit: 10,
                ..
            }
        ));
    }

    #[test]
    fn resource_item_parser_rejects_duplicate_attributes() {
        let err = extract_resource_items(
            r#"<Styles><ParagraphStyle Self="p1" Self="p2" /></Styles>"#,
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(
            matches!(
                &err,
                IdmlError::InvalidAttribute {
                    element,
                    attribute: "resource item attribute",
                    reason: "duplicate attribute",
                } if element == "ParagraphStyle"
            ) || matches!(&err, IdmlError::XmlAttribute(_))
        );
    }

    #[test]
    fn resource_item_parser_rejects_xml_forbidden_attribute_values() {
        let err = extract_resource_items(
            "<Styles><ParagraphStyle Self=\"bad\0id\" /></Styles>",
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "resource item attribute",
                reason: "contains an XML-forbidden character",
            } if element == "ParagraphStyle"
        ));
    }

    #[test]
    fn resource_item_parser_enforces_item_limit() {
        let err = extract_resource_items_with_options(
            r#"<Styles><ParagraphStyle Self="p1" /><ParagraphStyle Self="p2" /></Styles>"#,
            ResourceKind::Styles,
            ResourceItemParseOptions {
                max_items: 1,
                ..ResourceItemParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource item count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn resource_item_parser_enforces_xml_byte_limit() {
        let err = extract_resource_items_with_options(
            "<Root/>",
            ResourceKind::Styles,
            ResourceItemParseOptions {
                max_xml_bytes: 4,
                ..ResourceItemParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource item XML bytes",
                limit: 4,
                actual: 7,
            }
        ));
    }

    #[test]
    fn resource_item_parser_enforces_xml_depth_limit() {
        let err = extract_resource_items_with_options(
            r#"<Resources><Wrapper><Color Self="c1" /></Wrapper></Resources>"#,
            ResourceKind::Swatches,
            ResourceItemParseOptions {
                max_xml_depth: 2,
                ..ResourceItemParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource item XML depth",
                limit: 2,
                actual: 3,
            }
        ));
    }

    #[test]
    fn resource_item_parser_rejects_unclosed_xml() {
        let err = extract_resource_items(
            r#"<Styles><ParagraphStyle Self="p1">"#,
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "unclosed XML element",
            } if element == "#document"
        ));
    }

    #[test]
    fn resource_item_parser_rejects_missing_root() {
        let err = extract_resource_items("   \n\t", ResourceKind::Styles).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "missing root element",
            } if element.is_empty()
        ));
    }

    #[test]
    fn resource_item_parser_rejects_multiple_roots() {
        let err = extract_resource_items(
            r#"<Styles /><Styles><ParagraphStyle Self="p1" /></Styles>"#,
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "multiple root elements",
            } if element == "Styles"
        ));
    }

    #[test]
    fn resource_item_parser_rejects_trailing_root_text() {
        let err = extract_resource_items(
            r#"<Styles><ParagraphStyle Self="p1" /></Styles>tail"#,
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "non-whitespace character data outside root element",
            } if element == "#text"
        ));
    }

    #[test]
    fn resource_item_parser_rejects_malformed_comments() {
        let err = extract_resource_items(
            r#"<Styles><!-- invalid -- comment --><ParagraphStyle Self="p1" /></Styles>"#,
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(matches!(err, IdmlError::Xml(_)));
    }

    #[test]
    fn resource_item_parser_rejects_doctype_declarations() {
        let err = extract_resource_items(
            r#"<!DOCTYPE Styles><Styles><ParagraphStyle Self="p1" /></Styles>"#,
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "DOCTYPE declarations are not supported",
            } if element == "!DOCTYPE"
        ));
    }

    #[test]
    fn resource_item_parser_rejects_duplicate_ids() {
        let err = extract_resource_items(
            r#"<Styles><ParagraphStyle Self="p1" /><CharacterStyle Self="p1" /></Styles>"#,
            ResourceKind::Styles,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "resource item",
                id,
            } if id == "p1"
        ));
    }

    #[test]
    fn resource_item_group_rejects_duplicate_ids() {
        let group = ResourceItemGroup {
            resource: ResourceReference {
                kind: ResourceKind::Styles,
                element: "idPkg:Styles".to_owned(),
                id: None,
                path: IdmlPath::new("Resources/Styles.xml").unwrap(),
                archive: None,
            },
            items: vec![
                ResourceItem {
                    kind: ResourceKind::Styles,
                    element: "ParagraphStyle".to_owned(),
                    id: Some("p1".to_owned()),
                    name: Some("Body".to_owned()),
                    attributes: IndexMap::new(),
                },
                ResourceItem {
                    kind: ResourceKind::Styles,
                    element: "CharacterStyle".to_owned(),
                    id: Some("p1".to_owned()),
                    name: Some("Duplicate".to_owned()),
                    attributes: IndexMap::new(),
                },
            ],
        };

        let err = group.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "resource item",
                id,
            } if id == "p1"
        ));
    }

    #[test]
    fn resource_item_group_allows_items_without_ids() {
        let group = ResourceItemGroup {
            resource: ResourceReference {
                kind: ResourceKind::Styles,
                element: "idPkg:Styles".to_owned(),
                id: None,
                path: IdmlPath::new("Resources/Styles.xml").unwrap(),
                archive: None,
            },
            items: vec![
                ResourceItem {
                    kind: ResourceKind::Styles,
                    element: "ParagraphStyle".to_owned(),
                    id: None,
                    name: Some("Body".to_owned()),
                    attributes: IndexMap::new(),
                },
                ResourceItem {
                    kind: ResourceKind::Styles,
                    element: "CharacterStyle".to_owned(),
                    id: None,
                    name: Some("No ID".to_owned()),
                    attributes: IndexMap::new(),
                },
            ],
        };

        group.validate().unwrap();
    }

    #[test]
    fn attaches_archive_metadata_to_resources() {
        let xml = r#"<Document>
  <idPkg:Graphic src="Resources/Graphic.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();
        let mut inventory = ResourceInventory::from_designmap(&design_map).unwrap();
        let path = IdmlPath::new("Resources/Graphic.xml").unwrap();
        let entries = IndexMap::from([(
            path.clone(),
            ArchiveEntry {
                path,
                compressed_size: 12,
                uncompressed_size: 34,
                compression: CompressionMethod::Deflated,
            },
        )]);

        inventory.attach_archive_metadata(&entries).unwrap();

        let archive = inventory.resources[0].archive.as_ref().unwrap();
        assert_eq!(archive.compressed_size, 12);
        assert_eq!(archive.uncompressed_size, 34);
        assert_eq!(archive.compression, CompressionMethod::Deflated);
    }

    #[test]
    fn archive_metadata_requires_present_entries() {
        let xml = r#"<Document>
  <idPkg:Graphic src="Resources/Graphic.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();
        let mut inventory = ResourceInventory::from_designmap(&design_map).unwrap();

        let err = inventory
            .attach_archive_metadata(&IndexMap::new())
            .unwrap_err();

        assert!(
            matches!(err, IdmlError::MissingArchiveEntry(path) if path == "Resources/Graphic.xml")
        );
    }

    #[test]
    fn validates_resource_integrity_with_archive_metadata() {
        let inventory = inventory_with_archive(
            ResourceKind::Graphics,
            "idPkg:Graphic",
            "Resources/Graphic.xml",
            ResourceArchiveMetadata {
                compressed_size: 50,
                uncompressed_size: 500,
                compression: CompressionMethod::Deflated,
            },
        );

        inventory.validate_integrity().unwrap();
    }

    #[test]
    fn integrity_validation_requires_archive_metadata_by_default() {
        let xml = r#"<Document>
  <idPkg:Graphic src="Resources/Graphic.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();
        let inventory = ResourceInventory::from_designmap(&design_map).unwrap();

        let err = inventory.validate_integrity().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "resource archive metadata",
                id,
                reason: "metadata is missing",
            } if id == "Resources/Graphic.xml"
        ));
    }

    #[test]
    fn integrity_validation_can_allow_missing_archive_metadata() {
        let xml = r#"<Document>
  <idPkg:Graphic src="Resources/Graphic.xml" />
</Document>"#;
        let design_map = DesignMap::from_xml(xml).unwrap();
        let inventory = ResourceInventory::from_designmap(&design_map).unwrap();

        inventory
            .validate_integrity_with_options(ResourceIntegrityOptions {
                require_archive_metadata: false,
                ..ResourceIntegrityOptions::default()
            })
            .unwrap();
    }

    #[test]
    fn integrity_validation_rejects_oversized_resources() {
        let inventory = inventory_with_archive(
            ResourceKind::Fonts,
            "idPkg:Fonts",
            "Resources/Fonts.xml",
            ResourceArchiveMetadata {
                compressed_size: 10,
                uncompressed_size: 101,
                compression: CompressionMethod::Deflated,
            },
        );

        let err = inventory
            .validate_integrity_with_options(ResourceIntegrityOptions {
                max_resource_uncompressed_size: 100,
                ..ResourceIntegrityOptions::default()
            })
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource uncompressed size",
                limit: 100,
                actual: 101,
            }
        ));
    }

    #[test]
    fn integrity_validation_uses_graphic_size_limit_for_graphics() {
        let inventory = inventory_with_archive(
            ResourceKind::Graphics,
            "idPkg:Graphic",
            "Resources/Graphic.xml",
            ResourceArchiveMetadata {
                compressed_size: 10,
                uncompressed_size: 150,
                compression: CompressionMethod::Deflated,
            },
        );

        inventory
            .validate_integrity_with_options(ResourceIntegrityOptions {
                max_resource_uncompressed_size: 100,
                max_graphic_uncompressed_size: 200,
                ..ResourceIntegrityOptions::default()
            })
            .unwrap();
    }

    #[test]
    fn integrity_validation_rejects_extreme_compression_ratio() {
        let inventory = inventory_with_archive(
            ResourceKind::Graphics,
            "idPkg:Graphic",
            "Resources/Graphic.xml",
            ResourceArchiveMetadata {
                compressed_size: 10,
                uncompressed_size: 2_000,
                compression: CompressionMethod::Deflated,
            },
        );

        let err = inventory
            .validate_integrity_with_options(ResourceIntegrityOptions {
                max_compression_ratio: 100,
                ..ResourceIntegrityOptions::default()
            })
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource compression ratio",
                limit: 100,
                actual: 200,
            }
        ));
    }

    #[test]
    fn integrity_validation_rejects_fractional_compression_ratio_over_limit() {
        let inventory = inventory_with_archive(
            ResourceKind::Graphics,
            "idPkg:Graphic",
            "Resources/Graphic.xml",
            ResourceArchiveMetadata {
                compressed_size: 100,
                uncompressed_size: 10_001,
                compression: CompressionMethod::Deflated,
            },
        );

        let err = inventory
            .validate_integrity_with_options(ResourceIntegrityOptions {
                max_compression_ratio: 100,
                ..ResourceIntegrityOptions::default()
            })
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource compression ratio",
                limit: 100,
                actual: 101,
            }
        ));
    }

    fn inventory_with_archive(
        kind: ResourceKind,
        element: &str,
        path: &str,
        archive: ResourceArchiveMetadata,
    ) -> ResourceInventory {
        ResourceInventory {
            resources: vec![super::ResourceReference {
                kind,
                element: element.to_owned(),
                id: None,
                path: IdmlPath::new(path).unwrap(),
                archive: Some(archive),
            }],
        }
    }

    #[test]
    fn rejects_invalid_resource_element_names() {
        let mut design_map = DesignMap::default();
        design_map.other_package_srcs.insert(
            "bad tag".to_owned(),
            vec![IdmlPath::new("Resources/Bad.xml").unwrap()],
        );

        let err = ResourceInventory::from_designmap(&design_map).unwrap_err();

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
    fn extracts_embedded_assets_from_split_contents_chunks() {
        let xml = r#"<Image>
  <Properties><Label>ignored</Label></Properties>
  <Contents><![CDATA[aGVs]]><![CDATA[bG8=]]></Contents>
</Image>"#;

        let assets = extract_embedded_assets(xml).unwrap();

        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].source_element, "Contents");
        assert_eq!(assets[0].bytes, b"hello");
    }

    #[test]
    fn extracts_prefixed_embedded_asset_contents() {
        let xml = r#"<Image><idPkg:Contents>aGk=</idPkg:Contents></Image>"#;

        let assets = extract_embedded_assets(xml).unwrap();

        assert_eq!(assets[0].source_element, "idPkg:Contents");
        assert_eq!(assets[0].bytes, b"hi");
    }

    #[test]
    fn strict_embedded_asset_mode_rejects_whitespace() {
        let err = extract_embedded_assets("<Image><Contents>aG Vs</Contents></Image>").unwrap_err();

        assert!(matches!(err, IdmlError::Base64Decode(_)));
    }

    #[test]
    fn legacy_embedded_asset_mode_allows_transport_whitespace() {
        let assets = extract_embedded_assets_with_options(
            "<Image><Contents> aG\r\nVs\tbG8= </Contents></Image>",
            EmbeddedAssetParseOptions {
                base64_mode: Base64Mode::LegacyWhitespace,
                ..EmbeddedAssetParseOptions::default()
            },
        )
        .unwrap();

        assert_eq!(assets[0].bytes, b"hello");
    }

    #[test]
    fn embedded_asset_parser_enforces_xml_byte_limit() {
        let err = extract_embedded_assets_with_options(
            "<Graphic/>",
            EmbeddedAssetParseOptions {
                max_xml_bytes: 4,
                ..EmbeddedAssetParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "embedded asset XML bytes",
                limit: 4,
                actual: 10,
            }
        ));
    }

    #[test]
    fn embedded_asset_parser_enforces_encoded_limit_before_decoding() {
        let err = extract_embedded_assets_with_options(
            "<Image><Contents>aGVsbG8=</Contents></Image>",
            EmbeddedAssetParseOptions {
                max_encoded_bytes: 4,
                ..EmbeddedAssetParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "embedded asset base64 bytes",
                limit: 4,
                actual: 8,
            }
        ));
    }

    #[test]
    fn embedded_asset_parser_enforces_decoded_limit_before_allocation() {
        let err = extract_embedded_assets_with_options(
            "<Image><Contents>aGVsbG8=</Contents></Image>",
            EmbeddedAssetParseOptions {
                max_decoded_bytes: 4,
                ..EmbeddedAssetParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, IdmlError::LimitExceeded { what, .. } if what == "base64 decoded length")
        );
    }

    #[test]
    fn embedded_asset_parser_enforces_asset_count_limit() {
        let err = extract_embedded_assets_with_options(
            "<Image><Contents></Contents><Contents></Contents></Image>",
            EmbeddedAssetParseOptions {
                max_assets: 1,
                ..EmbeddedAssetParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "embedded asset count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn embedded_asset_parser_enforces_total_decoded_limit_before_allocation() {
        let err = extract_embedded_assets_with_options(
            "<Image><Contents>aGk=</Contents><Contents>aGk=</Contents></Image>",
            EmbeddedAssetParseOptions {
                max_decoded_bytes: 8,
                max_total_decoded_bytes: 3,
                ..EmbeddedAssetParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "embedded asset total decoded bytes",
                limit: 3,
                actual: 4,
            }
        ));
    }

    #[test]
    fn embedded_asset_parser_enforces_xml_depth_limit() {
        let err = extract_embedded_assets_with_options(
            "<Image><Wrapper><Contents>aGk=</Contents></Wrapper></Image>",
            EmbeddedAssetParseOptions {
                max_xml_depth: 2,
                ..EmbeddedAssetParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "embedded asset XML depth",
                limit: 2,
                actual: 3,
            }
        ));
    }

    #[test]
    fn embedded_asset_parser_rejects_unclosed_contents() {
        let err = extract_embedded_assets("<Image><Contents>aGVsbG8=").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "unclosed Contents element",
            } if element == "Contents"
        ));
    }

    #[test]
    fn embedded_asset_parser_rejects_missing_root() {
        let err = extract_embedded_assets("   \n\t").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "missing root element",
            } if element.is_empty()
        ));
    }

    #[test]
    fn embedded_asset_parser_rejects_multiple_roots() {
        let err = extract_embedded_assets("<Image /><Image><Contents>aGk=</Contents></Image>")
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "multiple root elements",
            } if element == "Image"
        ));
    }

    #[test]
    fn embedded_asset_parser_rejects_trailing_root_text() {
        let err =
            extract_embedded_assets("<Image><Contents>aGk=</Contents></Image>tail").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "non-whitespace character data outside root element",
            } if element == "#text"
        ));
    }

    #[test]
    fn embedded_asset_parser_rejects_entity_reference_inside_contents() {
        let err =
            extract_embedded_assets("<Image><Contents>aG&amp;k=</Contents></Image>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "entity reference inside embedded asset contents",
            } if element == "#reference"
        ));
    }

    #[test]
    fn embedded_asset_parser_rejects_doctype_declarations() {
        let err =
            extract_embedded_assets("<!DOCTYPE Image><Image><Contents>aGk=</Contents></Image>")
                .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "DOCTYPE declarations are not supported",
            } if element == "!DOCTYPE"
        ));
    }

    #[test]
    fn embedded_asset_parser_ignores_story_content_elements() {
        let assets = extract_embedded_assets("<Story><Content>aGVsbG8=</Content></Story>").unwrap();

        assert!(assets.is_empty());
    }
}
