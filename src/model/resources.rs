//! Resource inventory for untyped IDML package references.

use crate::archive::{ArchiveEntry, IdmlPath};
use crate::error::{IdmlError, Result};
use crate::model::designmap::{DesignMap, validate_package_element_name};
use indexmap::IndexMap;
use zip::CompressionMethod;

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
            validate_package_element_name(pointer.element())?;
            resources.push(ResourceReference {
                kind: ResourceKind::from_idpkg_element(pointer.element()),
                element: pointer.element().to_owned(),
                id: None,
                path: pointer.path().clone(),
                archive: None,
            });
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
}

impl ResourceKind {
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
}

#[cfg(test)]
mod tests {
    use super::{ResourceInventory, ResourceKind};
    use crate::IdmlError;
    use crate::archive::{ArchiveEntry, IdmlPath};
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
}
