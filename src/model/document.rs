//! High-level typed IDML document aggregate.

use crate::archive::{ArchiveLimits, IdmlPackage, IdmlPackageWriter, IdmlPath};
use crate::core::resolver::{
    ResolvedTextFrameData, page_by_id, resolve_text_frames, text_frames_intersecting,
    text_frames_on_page, to_owned_records,
};
use crate::error::{IdmlError, Result};
use crate::model::designmap::{
    DesignMap, DesignMapPackageRef, DesignMapParseOptions, validate_package_element_name,
};
use crate::model::resources::{ResourceIntegrityOptions, ResourceInventory};
use crate::model::spread::{Rect, Spread, SpreadParseOptions};
use crate::model::story::{Story, StoryParseOptions};
use indexmap::{IndexMap, IndexSet};
use std::io::{Read, Seek, Write};
use zip::CompressionMethod;

/// Typed IDML document parts that can be validated and written as one package.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdmlDocument {
    /// Root package manifest.
    pub design_map: DesignMap,
    /// Story models keyed by their `DesignMap` story ID.
    pub stories: IndexMap<String, Story>,
    /// Spread models keyed by their `DesignMap` spread ID.
    pub spreads: IndexMap<String, Spread>,
    /// Raw `DesignMap`-referenced entries that are not typed yet.
    pub preserved_entries: IndexMap<IdmlPath, PreservedEntry>,
}

/// Parser limits used when eagerly loading an [`IdmlDocument`] from a package.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IdmlDocumentReadOptions {
    /// DesignMap parser limits.
    pub design_map: DesignMapParseOptions,
    /// Story parser limits.
    pub story: StoryParseOptions,
    /// Spread parser limits.
    pub spread: SpreadParseOptions,
    /// Resource metadata integrity limits applied before preserved entries are read.
    pub resources: ResourceIntegrityOptions,
}

/// Raw package entry preserved while a typed model does not exist yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreservedEntry {
    /// Entry bytes after decompression.
    pub data: Vec<u8>,
    /// ZIP compression method to use when writing this entry back.
    pub compression: CompressionMethod,
}

impl PreservedEntry {
    /// Creates a preserved entry with an explicitly supported compression method.
    pub fn with_compression(
        data: impl Into<Vec<u8>>,
        compression: CompressionMethod,
    ) -> Result<Self> {
        validate_supported_preserved_compression(compression)?;
        Ok(Self {
            data: data.into(),
            compression,
        })
    }

    /// Creates a preserved entry using deflate compression on write.
    #[must_use]
    pub fn deflated(data: impl Into<Vec<u8>>) -> Self {
        Self {
            data: data.into(),
            compression: CompressionMethod::Deflated,
        }
    }

    /// Creates a preserved entry using stored, uncompressed ZIP output.
    #[must_use]
    pub fn stored(data: impl Into<Vec<u8>>) -> Self {
        Self {
            data: data.into(),
            compression: CompressionMethod::Stored,
        }
    }
}

/// Deterministic allocator for IDML-style object IDs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdmlIdAllocator {
    prefix: String,
    next: u64,
    reserved: IndexSet<String>,
}

impl Default for IdmlIdAllocator {
    fn default() -> Self {
        Self {
            prefix: "u".to_owned(),
            next: 1,
            reserved: IndexSet::new(),
        }
    }
}

impl IdmlIdAllocator {
    /// Creates an allocator that emits IDs like `u1`, `u2`, and so on.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an allocator with a validated prefix and starting counter.
    pub fn with_prefix(prefix: impl Into<String>, next: u64) -> Result<Self> {
        let prefix = prefix.into();
        validate_id_prefix(&prefix)?;
        Ok(Self {
            prefix,
            next,
            reserved: IndexSet::new(),
        })
    }

    /// Creates an allocator with all IDs currently used by `document` reserved.
    pub fn from_document(document: &IdmlDocument) -> Result<Self> {
        let mut allocator = Self::default();
        allocator.reserve_document(document)?;
        Ok(allocator)
    }

    /// Reserves an existing ID so it will not be allocated later.
    pub fn reserve(&mut self, id: impl Into<String>) -> Result<()> {
        let id = id.into();
        if self.reserved.insert(id.clone()) {
            return Ok(());
        }
        Err(IdmlError::DuplicateId {
            kind: "allocated object",
            id,
        })
    }

    /// Returns the next unused ID and reserves it.
    pub fn allocate(&mut self) -> Result<String> {
        loop {
            let current = self.next;
            let id = format!("{}{}", self.prefix, current);
            if current == u64::MAX {
                if self.reserved.insert(id.clone()) {
                    return Ok(id);
                }
                return Err(IdmlError::LimitExceeded {
                    what: "ID allocator counter",
                    limit: u64::MAX,
                    actual: u64::MAX,
                });
            }
            self.next = current + 1;
            if self.reserved.insert(id.clone()) {
                return Ok(id);
            }
        }
    }

    fn reserve_document(&mut self, document: &IdmlDocument) -> Result<()> {
        document.validate()?;

        for id in document
            .design_map
            .story_srcs
            .keys()
            .chain(document.design_map.spread_srcs.keys())
            .chain(document.design_map.master_spread_srcs.keys())
        {
            self.reserve_existing(id);
        }
        for layer in &document.design_map.layers {
            if let Some(id) = &layer.id {
                self.reserve_existing(id);
            }
        }
        for story in document.stories.values() {
            if let Some(id) = &story.id {
                self.reserve_existing(id);
            }
        }
        for spread in document.spreads.values() {
            if let Some(id) = &spread.id {
                self.reserve_existing(id);
            }
            for page in &spread.pages {
                if let Some(id) = &page.id {
                    self.reserve_existing(id);
                }
            }
            for frame in &spread.text_frames {
                if let Some(id) = &frame.id {
                    self.reserve_existing(id);
                }
            }
            for page_item in &spread.page_items {
                reserve_page_item_object_ids(page_item, self);
            }
        }
        Ok(())
    }

    fn reserve_existing(&mut self, id: &str) {
        self.reserved.insert(id.to_owned());
    }
}

impl IdmlDocument {
    /// Creates an empty document aggregate around a `DesignMap`.
    #[must_use]
    pub fn new(design_map: DesignMap) -> Self {
        Self {
            design_map,
            stories: IndexMap::new(),
            spreads: IndexMap::new(),
            preserved_entries: IndexMap::new(),
        }
    }

    /// Inserts or replaces a story by `DesignMap` story ID.
    pub fn insert_story(&mut self, id: impl Into<String>, story: Story) -> Option<Story> {
        self.stories.insert(id.into(), story)
    }

    /// Inserts or replaces a spread by `DesignMap` spread ID.
    pub fn insert_spread(&mut self, id: impl Into<String>, spread: Spread) -> Option<Spread> {
        self.spreads.insert(id.into(), spread)
    }

    /// Adds a story model and its `DesignMap` package reference atomically.
    ///
    /// This is the preferred generator API for new story entries because it
    /// prevents the model map and `designmap.xml` manifest from drifting apart.
    pub fn add_story(&mut self, id: impl Into<String>, path: IdmlPath, story: Story) -> Result<()> {
        let id = id.into();
        ensure_new_document_id(self, &id)?;
        ensure_new_package_path(&self.design_map, &path)?;
        validate_optional_self_id("story", &id, story.id.as_deref())?;

        self.design_map
            .ensure_package_ref_capacity_for_additional(1)?;
        self.design_map.ensure_package_refs_initialized()?;
        self.design_map.story_srcs.insert(id.clone(), path.clone());
        self.design_map
            .package_refs
            .push(DesignMapPackageRef::story(id.clone(), path));
        self.stories.insert(id, story);
        Ok(())
    }

    /// Adds a spread model and its `DesignMap` package reference atomically.
    ///
    /// This is the preferred generator API for new spread entries because it
    /// prevents the model map and `designmap.xml` manifest from drifting apart.
    pub fn add_spread(
        &mut self,
        id: impl Into<String>,
        path: IdmlPath,
        spread: Spread,
    ) -> Result<()> {
        let id = id.into();
        ensure_new_document_id(self, &id)?;
        ensure_new_package_path(&self.design_map, &path)?;
        validate_optional_self_id("spread", &id, spread.id.as_deref())?;

        self.design_map
            .ensure_package_ref_capacity_for_additional(1)?;
        self.design_map.ensure_package_refs_initialized()?;
        self.design_map.spread_srcs.insert(id.clone(), path.clone());
        self.design_map
            .package_refs
            .push(DesignMapPackageRef::spread(id.clone(), path));
        self.spreads.insert(id, spread);
        Ok(())
    }

    /// Inserts or replaces a raw package entry that should be preserved on write.
    ///
    /// The path must be referenced by `DesignMap` as a master spread or another
    /// package resource before [`IdmlDocument::validate`] will accept it.
    pub fn insert_preserved_entry(
        &mut self,
        path: IdmlPath,
        data: impl Into<Vec<u8>>,
    ) -> Option<PreservedEntry> {
        self.preserved_entries
            .insert(path, PreservedEntry::deflated(data))
    }

    /// Inserts or replaces a raw preserved package entry with explicit metadata.
    pub fn insert_preserved_entry_with_options(
        &mut self,
        path: IdmlPath,
        entry: PreservedEntry,
    ) -> Option<PreservedEntry> {
        self.preserved_entries.insert(path, entry)
    }

    /// Adds a raw master spread entry and its `DesignMap` reference atomically.
    pub fn add_master_spread_entry(
        &mut self,
        id: impl Into<String>,
        path: IdmlPath,
        entry: PreservedEntry,
    ) -> Result<()> {
        let id = id.into();
        ensure_new_document_id(self, &id)?;
        ensure_new_package_path(&self.design_map, &path)?;
        validate_supported_preserved_compression(entry.compression)?;

        self.design_map
            .ensure_package_ref_capacity_for_additional(1)?;
        self.design_map.ensure_package_refs_initialized()?;
        self.design_map
            .master_spread_srcs
            .insert(id.clone(), path.clone());
        self.design_map
            .package_refs
            .push(DesignMapPackageRef::master_spread(id, path.clone()));
        self.preserved_entries.insert(path, entry);
        Ok(())
    }

    /// Adds a raw `idPkg:*` resource entry and its `DesignMap` reference atomically.
    ///
    /// Use this for package references that do not have a typed model yet, such
    /// as graphics, fonts, preferences, and other resource XML files.
    pub fn add_package_entry(
        &mut self,
        element: impl Into<String>,
        path: IdmlPath,
        entry: PreservedEntry,
    ) -> Result<()> {
        let element = element.into();
        validate_raw_package_element_name(&element)?;
        ensure_new_package_path(&self.design_map, &path)?;
        validate_supported_preserved_compression(entry.compression)?;

        self.design_map
            .ensure_package_ref_capacity_for_additional(1)?;
        self.design_map.ensure_package_refs_initialized()?;
        self.design_map
            .other_package_srcs
            .entry(element.clone())
            .or_default()
            .push(path.clone());
        self.design_map
            .package_refs
            .push(DesignMapPackageRef::resource(element, path.clone())?);
        self.preserved_entries.insert(path, entry);
        Ok(())
    }

    /// Creates a deterministic allocator with all current document IDs reserved.
    pub fn id_allocator(&self) -> Result<IdmlIdAllocator> {
        IdmlIdAllocator::from_document(self)
    }

    /// Validates model presence and cross-file story references.
    pub fn validate(&self) -> Result<()> {
        self.design_map.validate()?;
        validate_manifest_models("story model", &self.design_map.story_srcs, &self.stories)?;
        validate_manifest_models("spread model", &self.design_map.spread_srcs, &self.spreads)?;
        validate_no_unreferenced_models("story", &self.design_map.story_srcs, &self.stories)?;
        validate_no_unreferenced_models("spread", &self.design_map.spread_srcs, &self.spreads)?;
        self.validate_preserved_entries()?;
        self.validate_story_ids()?;
        self.validate_spread_ids()?;
        self.validate_spread_models()?;
        self.validate_unique_object_ids()?;
        self.validate_parent_stories()?;
        self.validate_item_layers()?;
        Ok(())
    }

    /// Reads all manifest-referenced stories and spreads from a package.
    ///
    /// This eagerly loads the typed document model. Archive entry validation and
    /// per-entry size limits are still enforced by [`IdmlPackage`].
    pub fn read_from_package<R>(package: &mut IdmlPackage<R>) -> Result<Self>
    where
        R: Read + Seek,
    {
        Self::read_from_package_with_options(package, IdmlDocumentReadOptions::default())
    }

    /// Reads all manifest-referenced stories and spreads with explicit limits.
    pub fn read_from_package_with_options<R>(
        package: &mut IdmlPackage<R>,
        options: IdmlDocumentReadOptions,
    ) -> Result<Self>
    where
        R: Read + Seek,
    {
        let design_map = package.read_designmap_with_options(options.design_map)?;
        let story_refs = design_map
            .story_srcs
            .iter()
            .map(|(id, path)| (id.clone(), path.clone()))
            .collect::<Vec<_>>();
        let spread_refs = design_map
            .spread_srcs
            .iter()
            .map(|(id, path)| (id.clone(), path.clone()))
            .collect::<Vec<_>>();
        let preserved_refs = referenced_preserved_paths(&design_map)
            .cloned()
            .collect::<Vec<_>>();

        ResourceInventory::from_designmap(&design_map)?
            .with_archive_metadata(package.entries())?
            .validate_integrity_with_options(options.resources)?;

        let mut document = Self::new(design_map);
        for (id, path) in story_refs {
            document.insert_story(id, package.read_story_with_options(&path, options.story)?);
        }
        for (id, path) in spread_refs {
            document.insert_spread(id, package.read_spread_with_options(&path, options.spread)?);
        }
        for path in preserved_refs {
            let compression = package
                .entries()
                .get(&path)
                .ok_or_else(|| IdmlError::MissingArchiveEntry(path.to_string()))?
                .compression;
            let data = package.read_entry(&path)?;
            document
                .insert_preserved_entry_with_options(path, PreservedEntry { data, compression });
        }

        document.validate()?;
        Ok(document)
    }

    /// Returns all story text in `designmap.xml` order.
    pub fn story_texts(&self) -> Result<IndexMap<String, String>> {
        let mut texts = IndexMap::with_capacity(self.design_map.story_srcs.len());
        for story_id in self.design_map.story_srcs.keys() {
            let story = self
                .stories
                .get(story_id)
                .ok_or_else(|| IdmlError::MissingReference {
                    kind: "story model",
                    id: story_id.clone(),
                })?;
            texts.insert(story_id.clone(), story.text.clone());
        }
        Ok(texts)
    }

    /// Resolves all text frames on a spread into owned story text records.
    pub fn resolve_spread_text_frames(
        &self,
        spread_id: &str,
    ) -> Result<Vec<ResolvedTextFrameData>> {
        let spread = self.spread(spread_id)?;
        let story_texts = self.story_texts()?;
        Ok(to_owned_records(resolve_text_frames(spread, &story_texts)?))
    }

    /// Resolves text frames on a spread whose direct bounds intersect `query`.
    pub fn resolve_spread_text_in_rect(
        &self,
        spread_id: &str,
        query: Rect,
    ) -> Result<Vec<ResolvedTextFrameData>> {
        let spread = self.spread(spread_id)?;
        let story_texts = self.story_texts()?;
        let resolved = resolve_text_frames(spread, &story_texts)?;
        Ok(to_owned_records(text_frames_intersecting(resolved, query)))
    }

    /// Resolves text frames on a spread whose direct bounds intersect a page.
    pub fn resolve_spread_text_on_page(
        &self,
        spread_id: &str,
        page_id: &str,
    ) -> Result<Vec<ResolvedTextFrameData>> {
        let spread = self.spread(spread_id)?;
        let story_texts = self.story_texts()?;
        let resolved = resolve_text_frames(spread, &story_texts)?;
        let page = page_by_id(spread, page_id)?;
        Ok(to_owned_records(text_frames_on_page(resolved, page)?))
    }

    /// Validates and writes the aggregate as a complete IDML package.
    pub fn write_to<W>(&self, writer: W) -> Result<W>
    where
        W: Write + Seek,
    {
        self.write_to_with_limits(writer, ArchiveLimits::default())
    }

    /// Validates and writes the aggregate with explicit archive output limits.
    pub fn write_to_with_limits<W>(&self, writer: W, limits: ArchiveLimits) -> Result<W>
    where
        W: Write + Seek,
    {
        self.validate()?;

        let mut package = IdmlPackageWriter::with_limits(writer, limits)?;
        package.add_designmap(&self.design_map)?;
        for (spread_id, path) in &self.design_map.spread_srcs {
            let spread =
                self.spreads
                    .get(spread_id)
                    .ok_or_else(|| IdmlError::MissingReference {
                        kind: "spread model",
                        id: spread_id.clone(),
                    })?;
            package.add_spread(path.as_str(), spread)?;
        }
        for (story_id, path) in &self.design_map.story_srcs {
            let story = self
                .stories
                .get(story_id)
                .ok_or_else(|| IdmlError::MissingReference {
                    kind: "story model",
                    id: story_id.clone(),
                })?;
            package.add_story(path.as_str(), story)?;
        }
        for path in referenced_preserved_paths(&self.design_map) {
            let entry = self
                .preserved_entries
                .get(path)
                .ok_or_else(|| IdmlError::MissingArchiveEntry(path.to_string()))?;
            match entry.compression {
                CompressionMethod::Stored => package.add_stored_file(path.as_str(), &entry.data)?,
                CompressionMethod::Deflated => package.add_file(path.as_str(), &entry.data)?,
                _ => return Err(unsupported_preserved_compression_error()),
            }
        }
        package.finish()
    }

    fn spread(&self, spread_id: &str) -> Result<&Spread> {
        self.spreads
            .get(spread_id)
            .ok_or_else(|| IdmlError::MissingReference {
                kind: "spread model",
                id: spread_id.to_owned(),
            })
    }

    fn validate_story_ids(&self) -> Result<()> {
        for (id, story) in &self.stories {
            validate_optional_self_id("story", id, story.id.as_deref())?;
        }
        Ok(())
    }

    fn validate_spread_ids(&self) -> Result<()> {
        for (id, spread) in &self.spreads {
            validate_optional_self_id("spread", id, spread.id.as_deref())?;
        }
        Ok(())
    }

    fn validate_spread_models(&self) -> Result<()> {
        for spread in self.spreads.values() {
            spread.validate()?;
        }
        Ok(())
    }

    fn validate_parent_stories(&self) -> Result<()> {
        for spread in self.spreads.values() {
            for frame in &spread.text_frames {
                let Some(parent_story) = frame.parent_story.as_deref() else {
                    continue;
                };
                if !self.design_map.story_srcs.contains_key(parent_story) {
                    return Err(IdmlError::MissingReference {
                        kind: "text frame parent story",
                        id: parent_story.to_owned(),
                    });
                }
                if !self.stories.contains_key(parent_story) {
                    return Err(IdmlError::MissingReference {
                        kind: "story model",
                        id: parent_story.to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_preserved_entries(&self) -> Result<()> {
        for element in self.design_map.other_package_srcs.keys() {
            validate_raw_package_element_name(element)?;
        }

        let referenced = referenced_preserved_paths(&self.design_map)
            .cloned()
            .collect::<IndexSet<_>>();

        for path in &referenced {
            if !self.preserved_entries.contains_key(path) {
                return Err(IdmlError::MissingArchiveEntry(path.to_string()));
            }
        }
        for path in self.preserved_entries.keys() {
            if !referenced.contains(path) {
                return Err(IdmlError::InvalidReference {
                    kind: "preserved package entry",
                    id: path.to_string(),
                    reason: "entry is not present in DesignMap",
                });
            }
        }
        for entry in self.preserved_entries.values() {
            validate_supported_preserved_compression(entry.compression)?;
        }

        Ok(())
    }

    fn validate_unique_object_ids(&self) -> Result<()> {
        let mut seen = IndexSet::new();

        for id in self.design_map.story_srcs.keys() {
            remember_object_id(&mut seen, id)?;
        }
        for id in self.design_map.spread_srcs.keys() {
            remember_object_id(&mut seen, id)?;
        }
        for id in self.design_map.master_spread_srcs.keys() {
            remember_object_id(&mut seen, id)?;
        }
        for layer in &self.design_map.layers {
            if let Some(id) = &layer.id {
                remember_object_id(&mut seen, id)?;
            }
        }
        for spread in self.spreads.values() {
            for page in &spread.pages {
                if let Some(id) = &page.id {
                    remember_object_id(&mut seen, id)?;
                }
            }
            for frame in &spread.text_frames {
                if let Some(id) = &frame.id {
                    remember_object_id(&mut seen, id)?;
                }
            }
            for page_item in &spread.page_items {
                remember_structural_page_item_ids(&mut seen, page_item)?;
            }
        }

        Ok(())
    }

    fn validate_item_layers(&self) -> Result<()> {
        if self.design_map.layers.is_empty() {
            return Ok(());
        }

        let layer_ids = self
            .design_map
            .layers
            .iter()
            .filter_map(|layer| layer.id.as_deref())
            .collect::<IndexSet<_>>();

        for spread in self.spreads.values() {
            for frame in &spread.text_frames {
                validate_layer_id(frame.layer_id.as_deref(), &layer_ids)?;
            }
            for page_item in &spread.page_items {
                validate_page_item_layer_id(page_item, &layer_ids)?;
            }
        }

        Ok(())
    }
}

fn validate_manifest_models<T>(
    kind: &'static str,
    manifest: &IndexMap<String, IdmlPath>,
    models: &IndexMap<String, T>,
) -> Result<()> {
    for id in manifest.keys() {
        if !models.contains_key(id) {
            return Err(IdmlError::MissingReference {
                kind,
                id: id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_no_unreferenced_models<T>(
    kind: &'static str,
    manifest: &IndexMap<String, IdmlPath>,
    models: &IndexMap<String, T>,
) -> Result<()> {
    for id in models.keys() {
        if !manifest.contains_key(id) {
            return Err(IdmlError::InvalidReference {
                kind,
                id: id.clone(),
                reason: "model is not present in DesignMap",
            });
        }
    }
    Ok(())
}

fn ensure_new_document_id(document: &IdmlDocument, id: &str) -> Result<()> {
    if document_contains_object_id(document, id) {
        return Err(IdmlError::DuplicateId {
            kind: "document object",
            id: id.to_owned(),
        });
    }
    Ok(())
}

fn document_contains_object_id(document: &IdmlDocument, id: &str) -> bool {
    document.design_map.story_srcs.contains_key(id)
        || document.design_map.spread_srcs.contains_key(id)
        || document.design_map.master_spread_srcs.contains_key(id)
        || document.stories.contains_key(id)
        || document.spreads.contains_key(id)
        || document
            .design_map
            .layers
            .iter()
            .any(|layer| layer.id.as_deref() == Some(id))
        || document.spreads.values().any(|spread| {
            spread.id.as_deref() == Some(id)
                || spread
                    .pages
                    .iter()
                    .any(|page| page.id.as_deref() == Some(id))
                || spread
                    .text_frames
                    .iter()
                    .any(|frame| frame.id.as_deref() == Some(id))
                || spread
                    .page_items
                    .iter()
                    .any(|page_item| page_item_contains_object_id(page_item, id))
        })
}

fn ensure_new_package_path(design_map: &DesignMap, path: &IdmlPath) -> Result<()> {
    let path_exists = design_map
        .story_srcs
        .values()
        .chain(design_map.spread_srcs.values())
        .chain(design_map.master_spread_srcs.values())
        .chain(design_map.other_package_srcs.values().flatten())
        .any(|existing| existing == path);

    if path_exists {
        return Err(IdmlError::InvalidReference {
            kind: "DesignMap package path",
            id: path.to_string(),
            reason: "path is referenced more than once",
        });
    }
    Ok(())
}

fn validate_raw_package_element_name(element: &str) -> Result<()> {
    validate_package_element_name(element)?;
    if matches!(
        element,
        "idPkg:Story" | "idPkg:Spread" | "idPkg:MasterSpread"
    ) {
        return Err(IdmlError::InvalidAttribute {
            element: "DesignMap".to_owned(),
            attribute: "idPkg element",
            reason: "typed package element must use a typed registration API",
        });
    }
    Ok(())
}

fn remember_structural_page_item_ids(
    seen: &mut IndexSet<String>,
    page_item: &crate::model::spread::PageItem,
) -> Result<()> {
    match page_item {
        crate::model::spread::PageItem::TextFrame(_) => Ok(()),
        crate::model::spread::PageItem::Rectangle(rectangle) => {
            if let Some(id) = &rectangle.id {
                remember_object_id(seen, id)?;
            }
            Ok(())
        }
        crate::model::spread::PageItem::Group(group) => {
            if let Some(id) = &group.id {
                remember_object_id(seen, id)?;
            }
            for child in &group.page_items {
                remember_structural_page_item_ids(seen, child)?;
            }
            Ok(())
        }
    }
}

fn reserve_page_item_object_ids(
    page_item: &crate::model::spread::PageItem,
    allocator: &mut IdmlIdAllocator,
) {
    match page_item {
        crate::model::spread::PageItem::TextFrame(frame) => {
            if let Some(id) = &frame.id {
                allocator.reserve_existing(id);
            }
        }
        crate::model::spread::PageItem::Rectangle(rectangle) => {
            if let Some(id) = &rectangle.id {
                allocator.reserve_existing(id);
            }
        }
        crate::model::spread::PageItem::Group(group) => {
            if let Some(id) = &group.id {
                allocator.reserve_existing(id);
            }
            for child in &group.page_items {
                reserve_page_item_object_ids(child, allocator);
            }
        }
    }
}

fn page_item_contains_object_id(page_item: &crate::model::spread::PageItem, id: &str) -> bool {
    match page_item {
        crate::model::spread::PageItem::TextFrame(frame) => frame.id.as_deref() == Some(id),
        crate::model::spread::PageItem::Rectangle(rectangle) => rectangle.id.as_deref() == Some(id),
        crate::model::spread::PageItem::Group(group) => {
            group.id.as_deref() == Some(id)
                || group
                    .page_items
                    .iter()
                    .any(|child| page_item_contains_object_id(child, id))
        }
    }
}

fn referenced_preserved_paths(design_map: &DesignMap) -> impl Iterator<Item = &IdmlPath> {
    design_map
        .master_spread_srcs
        .values()
        .chain(design_map.other_package_srcs.values().flatten())
}

fn validate_supported_preserved_compression(compression: CompressionMethod) -> Result<()> {
    match compression {
        CompressionMethod::Stored | CompressionMethod::Deflated => Ok(()),
        _ => Err(unsupported_preserved_compression_error()),
    }
}

fn validate_page_item_layer_id(
    page_item: &crate::model::spread::PageItem,
    layer_ids: &IndexSet<&str>,
) -> Result<()> {
    match page_item {
        crate::model::spread::PageItem::TextFrame(frame) => {
            validate_layer_id(frame.layer_id.as_deref(), layer_ids)
        }
        crate::model::spread::PageItem::Rectangle(rectangle) => {
            validate_layer_id(rectangle.layer_id.as_deref(), layer_ids)
        }
        crate::model::spread::PageItem::Group(group) => {
            validate_layer_id(group.layer_id.as_deref(), layer_ids)?;
            for child in &group.page_items {
                validate_page_item_layer_id(child, layer_ids)?;
            }
            Ok(())
        }
    }
}

fn validate_layer_id(layer_id: Option<&str>, layer_ids: &IndexSet<&str>) -> Result<()> {
    let Some(layer_id) = layer_id else {
        return Ok(());
    };
    if layer_ids.contains(layer_id) {
        return Ok(());
    }
    Err(IdmlError::MissingReference {
        kind: "page item layer",
        id: layer_id.to_owned(),
    })
}

fn unsupported_preserved_compression_error() -> IdmlError {
    IdmlError::InvalidPackage(
        "unsupported ZIP compression method; only stored and deflated entries are accepted",
    )
}

fn validate_optional_self_id(kind: &'static str, id: &str, self_id: Option<&str>) -> Result<()> {
    if let Some(self_id) = self_id
        && self_id != id
    {
        return Err(IdmlError::InvalidReference {
            kind,
            id: id.to_owned(),
            reason: "model Self ID does not match DesignMap ID",
        });
    }
    Ok(())
}

fn remember_object_id(seen: &mut IndexSet<String>, id: &str) -> Result<()> {
    if seen.insert(id.to_owned()) {
        return Ok(());
    }
    Err(IdmlError::DuplicateId {
        kind: "document object",
        id: id.to_owned(),
    })
}

fn validate_id_prefix(prefix: &str) -> Result<()> {
    let valid = !prefix.is_empty()
        && prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if valid {
        return Ok(());
    }
    Err(IdmlError::InvalidAttribute {
        element: "IdmlIdAllocator".to_owned(),
        attribute: "prefix",
        reason: "prefix must be non-empty ASCII alphanumeric, underscore, or hyphen",
    })
}

#[cfg(test)]
mod tests {
    use super::{IdmlDocument, IdmlDocumentReadOptions, IdmlIdAllocator, PreservedEntry};
    use crate::IdmlError;
    use crate::archive::{ArchiveLimits, IdmlPackage, IdmlPackageWriter, IdmlPath};
    use crate::core::units::Points;
    use crate::model::designmap::{
        DEFAULT_MAX_DESIGNMAP_PACKAGE_REFS, DesignMap, DesignMapParseOptions, Layer,
    };
    use crate::model::resources::ResourceIntegrityOptions;
    use crate::model::spread::{
        Group, Page, PageItem, Rect, Rectangle, Spread, SpreadParseOptions, TextFrame,
    };
    use crate::model::story::{Story, StoryParseOptions};
    use std::io::Cursor;
    use zip::CompressionMethod;

    #[test]
    fn validates_and_writes_document_package() {
        let document = make_document();
        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let design_map = package.read_designmap().unwrap();
        let story = package.resolve_story(&design_map, "u1").unwrap();
        let spread = package.resolve_spread(&design_map, "u10").unwrap();

        assert_eq!(story.text, "Generated");
        assert_eq!(spread.text_frames[0].parent_story.as_deref(), Some("u1"));
    }

    #[test]
    fn reads_document_from_package_and_validates_it() {
        let document = make_document();
        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let parsed = IdmlDocument::read_from_package(&mut package).unwrap();

        assert_eq!(parsed, document);
    }

    #[test]
    fn add_story_and_spread_register_manifest_and_models() {
        let design_map = DesignMap {
            id: "d1".to_owned(),
            ..DesignMap::default()
        };
        let mut document = IdmlDocument::new(design_map);

        document
            .add_story(
                "u1",
                IdmlPath::new("Stories/Story_u1.xml").unwrap(),
                Story {
                    id: Some("u1".to_owned()),
                    text: "Generated".to_owned(),
                },
            )
            .unwrap();
        document
            .add_spread(
                "u10",
                IdmlPath::new("Spreads/Spread_u10.xml").unwrap(),
                Spread {
                    id: Some("u10".to_owned()),
                    pages: Vec::new(),
                    text_frames: vec![TextFrame {
                        id: Some("tf1".to_owned()),
                        layer_id: None,
                        parent_story: Some("u1".to_owned()),
                        geometric_bounds: None,
                    }],
                    page_items: Vec::new(),
                },
            )
            .unwrap();

        document.validate().unwrap();
        assert_eq!(
            document
                .design_map
                .story_srcs
                .get("u1")
                .map(IdmlPath::as_str),
            Some("Stories/Story_u1.xml")
        );
        assert_eq!(
            document
                .design_map
                .spread_srcs
                .get("u10")
                .map(IdmlPath::as_str),
            Some("Spreads/Spread_u10.xml")
        );

        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();
        let parsed = IdmlDocument::read_from_package(&mut package).unwrap();

        assert_eq!(parsed, document);
    }

    #[test]
    fn add_story_rejects_duplicate_document_id() {
        let mut document = make_document();

        let err = document
            .add_story(
                "u10",
                IdmlPath::new("Stories/Story_u10.xml").unwrap(),
                Story {
                    id: Some("u10".to_owned()),
                    text: "duplicate".to_owned(),
                },
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "u10"
        ));
    }

    #[test]
    fn add_story_rejects_package_ref_limit_without_mutating_document() {
        let mut document = IdmlDocument::new(DesignMap::default());
        document.design_map.other_package_srcs.insert(
            "idPkg:Graphic".to_owned(),
            (0..DEFAULT_MAX_DESIGNMAP_PACKAGE_REFS)
                .map(|index| IdmlPath::new(format!("Resources/Graphic_{index}.xml")).unwrap())
                .collect(),
        );

        let err = document
            .add_story(
                "u1",
                IdmlPath::new("Stories/Story_u1.xml").unwrap(),
                Story {
                    id: Some("u1".to_owned()),
                    text: "not inserted".to_owned(),
                },
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "DesignMap package reference count",
                limit: 20_000,
                actual: 20_001,
            }
        ));
        assert!(document.design_map.package_refs.is_empty());
        assert!(!document.design_map.story_srcs.contains_key("u1"));
        assert!(!document.stories.contains_key("u1"));
    }

    #[test]
    fn add_story_rejects_layer_id_collision_without_mutating_document() {
        let mut document = make_document();
        document.design_map.layers.push(Layer {
            id: Some("layer1".to_owned()),
            name: Some("Text".to_owned()),
            visible: Some(true),
            locked: Some(false),
        });

        let err = document
            .add_story(
                "layer1",
                IdmlPath::new("Stories/Story_layer1.xml").unwrap(),
                Story {
                    id: Some("layer1".to_owned()),
                    text: "duplicate".to_owned(),
                },
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "layer1"
        ));
        assert!(!document.design_map.story_srcs.contains_key("layer1"));
        assert!(!document.stories.contains_key("layer1"));
    }

    #[test]
    fn add_story_rejects_group_id_collision_without_mutating_document() {
        let mut document = make_document();
        document
            .spreads
            .get_mut("u10")
            .unwrap()
            .page_items
            .push(PageItem::Group(Group {
                id: Some("group1".to_owned()),
                layer_id: None,
                geometric_bounds: None,
                page_items: Vec::new(),
            }));

        let err = document
            .add_story(
                "group1",
                IdmlPath::new("Stories/Story_group1.xml").unwrap(),
                Story {
                    id: Some("group1".to_owned()),
                    text: "duplicate".to_owned(),
                },
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "group1"
        ));
        assert!(!document.design_map.story_srcs.contains_key("group1"));
        assert!(!document.stories.contains_key("group1"));
    }

    #[test]
    fn add_spread_rejects_duplicate_package_path() {
        let mut document = make_document();

        let err = document
            .add_spread(
                "u11",
                IdmlPath::new("Stories/Story_u1.xml").unwrap(),
                Spread {
                    id: Some("u11".to_owned()),
                    pages: Vec::new(),
                    text_frames: Vec::new(),
                    page_items: Vec::new(),
                },
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "DesignMap package path",
                id,
                reason: "path is referenced more than once",
            } if id == "Stories/Story_u1.xml"
        ));
    }

    #[test]
    fn add_story_rejects_self_id_mismatch_without_mutating_document() {
        let mut document = make_document();

        let err = document
            .add_story(
                "u2",
                IdmlPath::new("Stories/Story_u2.xml").unwrap(),
                Story {
                    id: Some("wrong".to_owned()),
                    text: "bad".to_owned(),
                },
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "story",
                id,
                reason: "model Self ID does not match DesignMap ID",
            } if id == "u2"
        ));
        assert!(!document.design_map.story_srcs.contains_key("u2"));
        assert!(!document.stories.contains_key("u2"));
    }

    #[test]
    fn add_preserved_entries_register_manifest_and_models() {
        let mut document = make_document();
        let master_path = IdmlPath::new("MasterSpreads/MasterSpread_u20.xml").unwrap();
        let graphic_path = IdmlPath::new("Resources/Graphic.xml").unwrap();

        document
            .add_master_spread_entry(
                "u20",
                master_path.clone(),
                PreservedEntry::deflated(b"<MasterSpread Self=\"u20\" />".as_slice()),
            )
            .unwrap();
        document
            .add_package_entry(
                "idPkg:Graphic",
                graphic_path.clone(),
                PreservedEntry::stored(b"<Graphic><Data>raw</Data></Graphic>".as_slice()),
            )
            .unwrap();

        document.validate().unwrap();
        assert_eq!(
            document
                .design_map
                .master_spread_srcs
                .get("u20")
                .map(IdmlPath::as_str),
            Some("MasterSpreads/MasterSpread_u20.xml")
        );
        assert_eq!(
            document.design_map.other_package_srcs["idPkg:Graphic"][0].as_str(),
            "Resources/Graphic.xml"
        );

        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();
        let parsed = IdmlDocument::read_from_package(&mut package).unwrap();

        assert_eq!(
            parsed.preserved_entries[&master_path].data.as_slice(),
            b"<MasterSpread Self=\"u20\" />"
        );
        assert_eq!(
            parsed.preserved_entries[&graphic_path].compression,
            CompressionMethod::Stored
        );
    }

    #[test]
    fn add_master_spread_rejects_duplicate_id_without_mutating_document() {
        let mut document = make_document();

        let err = document
            .add_master_spread_entry(
                "u1",
                IdmlPath::new("MasterSpreads/MasterSpread_u1.xml").unwrap(),
                PreservedEntry::deflated(b"<MasterSpread />".as_slice()),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "u1"
        ));
        assert!(document.design_map.master_spread_srcs.is_empty());
    }

    #[test]
    fn add_package_entry_rejects_invalid_element_without_mutating_document() {
        let mut document = make_document();

        let err = document
            .add_package_entry(
                "Graphic",
                IdmlPath::new("Resources/Graphic.xml").unwrap(),
                PreservedEntry::deflated(b"<Graphic />".as_slice()),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "idPkg element",
                reason: "invalid XML element name",
            } if element == "DesignMap"
        ));
        assert!(document.design_map.other_package_srcs.is_empty());
        assert!(document.preserved_entries.is_empty());
    }

    #[test]
    fn add_package_entry_rejects_typed_element_without_mutating_document() {
        let mut document = make_document();

        let err = document
            .add_package_entry(
                "idPkg:Story",
                IdmlPath::new("Resources/StoryLike.xml").unwrap(),
                PreservedEntry::deflated(b"<Story />".as_slice()),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "idPkg element",
                reason: "typed package element must use a typed registration API",
            } if element == "DesignMap"
        ));
        assert!(document.design_map.other_package_srcs.is_empty());
        assert!(document.preserved_entries.is_empty());
    }

    #[test]
    fn add_package_entry_rejects_duplicate_path_without_mutating_document() {
        let mut document = make_document();

        let err = document
            .add_package_entry(
                "idPkg:Graphic",
                IdmlPath::new("Stories/Story_u1.xml").unwrap(),
                PreservedEntry::deflated(b"<Graphic />".as_slice()),
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "DesignMap package path",
                id,
                reason: "path is referenced more than once",
            } if id == "Stories/Story_u1.xml"
        ));
        assert!(document.design_map.other_package_srcs.is_empty());
        assert!(document.preserved_entries.is_empty());
    }

    #[test]
    fn validate_rejects_typed_element_in_raw_package_refs() {
        let mut document = make_document();
        let path = IdmlPath::new("Resources/StoryLike.xml").unwrap();
        document
            .design_map
            .other_package_srcs
            .insert("idPkg:Story".to_owned(), vec![path.clone()]);
        document.insert_preserved_entry(path, b"<Story />".as_slice());

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "idPkg element",
                reason: "typed package element must use a typed registration API",
            } if element == "DesignMap"
        ));
    }

    #[test]
    fn validate_rejects_duplicate_designmap_package_paths() {
        let mut document = make_document();
        let path = IdmlPath::new("Resources/Shared.xml").unwrap();
        document
            .design_map
            .other_package_srcs
            .insert("idPkg:Graphic".to_owned(), vec![path.clone()]);
        document
            .design_map
            .other_package_srcs
            .insert("idPkg:Font".to_owned(), vec![path.clone()]);
        document.insert_preserved_entry(path, b"<Shared />".as_slice());

        let err = document.validate().unwrap_err();

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
    fn validate_rejects_master_spread_id_colliding_with_text_frame() {
        let mut document = make_document();
        let path = IdmlPath::new("MasterSpreads/MasterSpread_tf1.xml").unwrap();
        document
            .design_map
            .master_spread_srcs
            .insert("tf1".to_owned(), path.clone());
        document.insert_preserved_entry(path, b"<MasterSpread Self=\"tf1\" />".as_slice());

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "tf1"
        ));
    }

    #[test]
    fn reads_and_writes_preserved_designmap_entries() {
        let master_path = IdmlPath::new("MasterSpreads/MasterSpread_u20.xml").unwrap();
        let resource_path = IdmlPath::new("Resources/Graphic.xml").unwrap();
        let mut writer = IdmlPackageWriter::new(Cursor::new(Vec::new())).unwrap();
        writer
            .add_file(
                "designmap.xml",
                br#"<Document Self="d1">
  <idPkg:Spread src="Spreads/Spread_u10.xml" />
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_u20.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
  <idPkg:Graphic src="Resources/Graphic.xml" />
</Document>"#,
            )
            .unwrap();
        writer
            .add_file("Spreads/Spread_u10.xml", br#"<Spread Self="u10" />"#)
            .unwrap();
        writer
            .add_file(
                "Stories/Story_u1.xml",
                br#"<Story Self="u1"><Content>Generated</Content></Story>"#,
            )
            .unwrap();
        writer
            .add_file(master_path.as_str(), b"<MasterSpread Self=\"u20\" />")
            .unwrap();
        writer
            .add_stored_file(
                resource_path.as_str(),
                b"<Graphic><Data>raw</Data></Graphic>",
            )
            .unwrap();
        let zip = writer.finish().unwrap().into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let document = IdmlDocument::read_from_package(&mut package).unwrap();

        assert_eq!(
            document.preserved_entries[&master_path].data.as_slice(),
            b"<MasterSpread Self=\"u20\" />"
        );
        assert_eq!(
            document.preserved_entries[&master_path].compression,
            CompressionMethod::Deflated
        );
        assert_eq!(
            document.preserved_entries[&resource_path].data.as_slice(),
            b"<Graphic><Data>raw</Data></Graphic>"
        );
        assert_eq!(
            document.preserved_entries[&resource_path].compression,
            CompressionMethod::Stored
        );

        let round_trip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(round_trip)).unwrap();
        assert_eq!(
            package.entries()[&master_path].compression,
            CompressionMethod::Deflated
        );
        assert_eq!(
            package.entries()[&resource_path].compression,
            CompressionMethod::Stored
        );

        assert_eq!(
            package.read_entry(&master_path).unwrap(),
            b"<MasterSpread Self=\"u20\" />"
        );
        assert_eq!(
            package.read_entry(&resource_path).unwrap(),
            b"<Graphic><Data>raw</Data></Graphic>"
        );
    }

    #[test]
    fn extracts_story_texts_in_designmap_order() {
        let document = make_document();

        let texts = document.story_texts().unwrap();

        assert_eq!(
            texts
                .iter()
                .map(|(story_id, text)| (story_id.as_str(), text.as_str()))
                .collect::<Vec<_>>(),
            [("u1", "Generated")]
        );
    }

    #[test]
    fn resolves_spread_text_frames_from_loaded_document() {
        let document = make_document();

        let resolved = document.resolve_spread_text_frames("u10").unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].frame_id.as_deref(), Some("tf1"));
        assert_eq!(resolved[0].story_id, "u1");
        assert_eq!(resolved[0].text, "Generated");
    }

    #[test]
    fn resolves_spread_text_frames_in_rect_from_loaded_document() {
        let document = make_document();

        let resolved = document
            .resolve_spread_text_in_rect(
                "u10",
                Rect::new(
                    Points::new(10.0),
                    Points::new(10.0),
                    Points::new(80.0),
                    Points::new(80.0),
                ),
            )
            .unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].frame_id.as_deref(), Some("tf1"));
    }

    #[test]
    fn resolves_spread_text_frames_on_page_from_loaded_document() {
        let mut document = make_document();
        document.spreads.get_mut("u10").unwrap().pages.push(Page {
            id: Some("p1".to_owned()),
            geometric_bounds: Some(Rect::new(
                Points::new(0.0),
                Points::new(0.0),
                Points::new(100.0),
                Points::new(100.0),
            )),
        });

        let resolved = document.resolve_spread_text_on_page("u10", "p1").unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].frame_id.as_deref(), Some("tf1"));
        assert_eq!(resolved[0].text, "Generated");
    }

    #[test]
    fn resolve_spread_text_frames_reports_unknown_spread() {
        let document = make_document();

        let err = document.resolve_spread_text_frames("missing").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "spread model",
                id,
            } if id == "missing"
        ));
    }

    #[test]
    fn id_allocator_skips_document_ids() {
        let document = make_document();
        let mut allocator = document.id_allocator().unwrap();

        assert_eq!(allocator.allocate().unwrap(), "u2");
        assert_eq!(allocator.allocate().unwrap(), "u3");
    }

    #[test]
    fn id_allocator_skips_layer_page_and_page_item_ids() {
        let mut document = make_document();
        document.design_map.layers.push(Layer {
            id: Some("u2".to_owned()),
            name: Some("Text".to_owned()),
            visible: Some(true),
            locked: Some(false),
        });
        let spread = document.spreads.get_mut("u10").unwrap();
        spread
            .page_items
            .push(PageItem::TextFrame(spread.text_frames[0].clone()));
        spread.pages.push(Page {
            id: Some("u3".to_owned()),
            geometric_bounds: None,
        });
        spread.page_items.push(PageItem::Group(Group {
            id: Some("u4".to_owned()),
            layer_id: None,
            geometric_bounds: None,
            page_items: vec![PageItem::Rectangle(Rectangle {
                id: Some("u5".to_owned()),
                layer_id: None,
                geometric_bounds: None,
            })],
        }));

        let mut allocator = document.id_allocator().unwrap();

        assert_eq!(allocator.allocate().unwrap(), "u6");
    }

    #[test]
    fn id_allocator_rejects_duplicate_reservations() {
        let mut allocator = IdmlIdAllocator::new();
        allocator.reserve("u1").unwrap();

        let err = allocator.reserve("u1").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "allocated object",
                id,
            } if id == "u1"
        ));
    }

    #[test]
    fn id_allocator_rejects_invalid_prefixes() {
        let err = IdmlIdAllocator::with_prefix("bad prefix", 1).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "prefix",
                reason: "prefix must be non-empty ASCII alphanumeric, underscore, or hyphen",
            } if element == "IdmlIdAllocator"
        ));
    }

    #[test]
    fn id_allocator_reports_counter_exhaustion() {
        let mut allocator = IdmlIdAllocator::with_prefix("u", u64::MAX).unwrap();
        allocator.reserve(format!("u{}", u64::MAX)).unwrap();

        let err = allocator.allocate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "ID allocator counter",
                ..
            }
        ));
    }

    #[test]
    fn read_from_package_with_options_enforces_story_text_limit() {
        let document = make_document();
        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let err = IdmlDocument::read_from_package_with_options(
            &mut package,
            IdmlDocumentReadOptions {
                story: StoryParseOptions {
                    max_text_bytes: 4,
                    ..StoryParseOptions::default()
                },
                ..IdmlDocumentReadOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(err, IdmlError::LimitExceeded { what, .. } if what == "story text bytes"));
    }

    #[test]
    fn read_from_package_with_options_enforces_designmap_ref_limit() {
        let document = make_document();
        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let err = IdmlDocument::read_from_package_with_options(
            &mut package,
            IdmlDocumentReadOptions {
                design_map: DesignMapParseOptions {
                    max_package_refs: 1,
                    ..DesignMapParseOptions::default()
                },
                ..IdmlDocumentReadOptions::default()
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, IdmlError::LimitExceeded { what, .. } if what == "DesignMap package reference count")
        );
    }

    #[test]
    fn read_from_package_with_options_enforces_spread_text_frame_limit() {
        let document = make_document();
        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let err = IdmlDocument::read_from_package_with_options(
            &mut package,
            IdmlDocumentReadOptions {
                spread: SpreadParseOptions {
                    max_text_frames: 0,
                    ..SpreadParseOptions::default()
                },
                ..IdmlDocumentReadOptions::default()
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, IdmlError::LimitExceeded { what, .. } if what == "spread text frame count")
        );
    }

    #[test]
    fn read_from_package_with_options_enforces_resource_integrity_before_reading() {
        let mut document = make_document();
        document
            .add_package_entry(
                "idPkg:Graphic",
                IdmlPath::new("Resources/Graphic.xml").unwrap(),
                PreservedEntry::deflated(b"<Graphic />".as_slice()),
            )
            .unwrap();
        let zip = document
            .write_to(Cursor::new(Vec::new()))
            .unwrap()
            .into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let err = IdmlDocument::read_from_package_with_options(
            &mut package,
            IdmlDocumentReadOptions {
                resources: ResourceIntegrityOptions {
                    max_graphic_uncompressed_size: 1,
                    ..ResourceIntegrityOptions::default()
                },
                ..IdmlDocumentReadOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "resource uncompressed size",
                limit: 1,
                actual,
            } if actual == b"<Graphic />".len() as u64
        ));
    }

    #[test]
    fn write_to_with_limits_enforces_archive_output_limits() {
        let document = make_document();

        let err = document
            .write_to_with_limits(
                Cursor::new(Vec::new()),
                ArchiveLimits {
                    max_entries: 1,
                    ..ArchiveLimits::default()
                },
            )
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "archive entry count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn read_from_package_rejects_invalid_parent_story() {
        let mut writer = IdmlPackageWriter::new(Cursor::new(Vec::new())).unwrap();
        writer
            .add_file(
                "designmap.xml",
                br#"<Document Self="d1">
  <idPkg:Spread src="Spreads/Spread_u10.xml" />
</Document>"#,
            )
            .unwrap();
        writer
            .add_file(
                "Spreads/Spread_u10.xml",
                br#"<Spread Self="u10">
  <TextFrame Self="tf1" ParentStory="missing" GeometricBounds="0 0 72 144" />
</Spread>"#,
            )
            .unwrap();
        let zip = writer.finish().unwrap().into_inner();
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let err = IdmlDocument::read_from_package(&mut package).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "text frame parent story",
                id
            } if id == "missing"
        ));
    }

    #[test]
    fn rejects_stale_spread_page_item_index() {
        let mut document = make_document();
        let spread = document.spreads.get_mut("u10").unwrap();
        spread.page_items.push(PageItem::TextFrame(TextFrame {
            id: Some("tf2".to_owned()),
            layer_id: None,
            parent_story: Some("u1".to_owned()),
            geometric_bounds: None,
        }));

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "spread page items",
                reason: "ordered page items do not match text frame index",
                ..
            }
        ));
    }

    #[test]
    fn rejects_non_finite_spread_geometry() {
        let mut document = make_document();
        document.spreads.get_mut("u10").unwrap().text_frames[0].geometric_bounds = Some(Rect::new(
            Points::new(0.0),
            Points::new(f64::NAN),
            Points::new(72.0),
            Points::new(144.0),
        ));

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "GeometricBounds",
                reason: "coordinate must be finite",
            } if element == "TextFrame"
        ));
    }

    #[test]
    fn rejects_missing_story_model() {
        let mut document = make_document();
        document.stories.clear();

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "story model",
                id
            } if id == "u1"
        ));
    }

    #[test]
    fn rejects_unreferenced_story_model() {
        let mut document = make_document();
        document.insert_story(
            "u2",
            Story {
                id: Some("u2".to_owned()),
                text: "extra".to_owned(),
            },
        );

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "story",
                id,
                reason: "model is not present in DesignMap",
            } if id == "u2"
        ));
    }

    #[test]
    fn rejects_story_self_id_mismatch() {
        let mut document = make_document();
        document.stories.get_mut("u1").unwrap().id = Some("wrong".to_owned());

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "story",
                id,
                reason: "model Self ID does not match DesignMap ID",
            } if id == "u1"
        ));
    }

    #[test]
    fn rejects_dangling_parent_story() {
        let mut document = make_document();
        document.spreads.get_mut("u10").unwrap().text_frames[0].parent_story =
            Some("missing".to_owned());

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "text frame parent story",
                id
            } if id == "missing"
        ));
    }

    #[test]
    fn accepts_known_text_frame_layer_reference() {
        let mut document = make_document();
        document.design_map.layers.push(Layer {
            id: Some("layer1".to_owned()),
            name: Some("Text".to_owned()),
            visible: Some(true),
            locked: Some(false),
        });
        document.spreads.get_mut("u10").unwrap().text_frames[0].layer_id =
            Some("layer1".to_owned());

        document.validate().unwrap();
    }

    #[test]
    fn rejects_dangling_text_frame_layer_reference() {
        let mut document = make_document();
        document.design_map.layers.push(Layer {
            id: Some("layer1".to_owned()),
            name: Some("Text".to_owned()),
            visible: Some(true),
            locked: Some(false),
        });
        document.spreads.get_mut("u10").unwrap().text_frames[0].layer_id =
            Some("missing".to_owned());

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "page item layer",
                id,
            } if id == "missing"
        ));
    }

    #[test]
    fn rejects_dangling_nested_page_item_layer_reference() {
        let mut document = make_document();
        document.design_map.layers.push(Layer {
            id: Some("layer1".to_owned()),
            name: Some("Text".to_owned()),
            visible: Some(true),
            locked: Some(false),
        });
        let spread = document.spreads.get_mut("u10").unwrap();
        spread
            .page_items
            .push(PageItem::TextFrame(spread.text_frames[0].clone()));
        spread
            .page_items
            .push(PageItem::Group(crate::model::spread::Group {
                id: Some("group1".to_owned()),
                layer_id: Some("layer1".to_owned()),
                geometric_bounds: None,
                page_items: vec![PageItem::Rectangle(Rectangle {
                    id: Some("rect1".to_owned()),
                    layer_id: Some("missing".to_owned()),
                    geometric_bounds: None,
                })],
            }));

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "page item layer",
                id,
            } if id == "missing"
        ));
    }

    #[test]
    fn rejects_duplicate_text_frame_ids() {
        let mut document = make_document();
        document
            .spreads
            .get_mut("u10")
            .unwrap()
            .text_frames
            .push(TextFrame {
                id: Some("tf1".to_owned()),
                layer_id: None,
                parent_story: Some("u1".to_owned()),
                geometric_bounds: None,
            });

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "tf1"
        ));
    }

    #[test]
    fn rejects_rectangle_id_colliding_with_text_frame() {
        let mut document = make_document();
        let spread = document.spreads.get_mut("u10").unwrap();
        spread
            .page_items
            .push(PageItem::TextFrame(spread.text_frames[0].clone()));
        spread.page_items.push(PageItem::Rectangle(Rectangle {
            id: Some("tf1".to_owned()),
            layer_id: None,
            geometric_bounds: None,
        }));

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "tf1"
        ));
    }

    #[test]
    fn rejects_group_id_colliding_with_text_frame() {
        let mut document = make_document();
        let spread = document.spreads.get_mut("u10").unwrap();
        spread
            .page_items
            .push(PageItem::TextFrame(spread.text_frames[0].clone()));
        spread.page_items.push(PageItem::Group(Group {
            id: Some("tf1".to_owned()),
            layer_id: None,
            geometric_bounds: None,
            page_items: Vec::new(),
        }));

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "tf1"
        ));
    }

    #[test]
    fn rejects_nested_rectangle_id_colliding_with_page() {
        let mut document = make_document();
        let spread = document.spreads.get_mut("u10").unwrap();
        spread
            .page_items
            .push(PageItem::TextFrame(spread.text_frames[0].clone()));
        spread.pages.push(Page {
            id: Some("page1".to_owned()),
            geometric_bounds: None,
        });
        spread.page_items.push(PageItem::Group(Group {
            id: Some("group1".to_owned()),
            layer_id: None,
            geometric_bounds: None,
            page_items: vec![PageItem::Rectangle(Rectangle {
                id: Some("page1".to_owned()),
                layer_id: None,
                geometric_bounds: None,
            })],
        }));

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "page1"
        ));
    }

    #[test]
    fn rejects_page_id_colliding_with_text_frame() {
        let mut document = make_document();
        document.spreads.get_mut("u10").unwrap().pages.push(Page {
            id: Some("tf1".to_owned()),
            geometric_bounds: None,
        });

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "tf1"
        ));
    }

    #[test]
    fn rejects_cross_type_id_collisions() {
        let mut document = make_document();
        document.design_map.spread_srcs.insert(
            "u1".to_owned(),
            IdmlPath::new("Spreads/Spread_u1.xml").unwrap(),
        );
        document.insert_spread(
            "u1",
            Spread {
                id: Some("u1".to_owned()),
                pages: Vec::new(),
                text_frames: Vec::new(),
                page_items: Vec::new(),
            },
        );

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "DesignMap object",
                id,
            } if id == "u1"
        ));
    }

    #[test]
    fn rejects_missing_preserved_designmap_entry() {
        let mut document = make_document();
        document.design_map.other_package_srcs.insert(
            "idPkg:Graphic".to_owned(),
            vec![IdmlPath::new("Resources/Graphic.xml").unwrap()],
        );

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingArchiveEntry(path) if path == "Resources/Graphic.xml"
        ));
    }

    #[test]
    fn rejects_unreferenced_preserved_entry() {
        let mut document = make_document();
        document.insert_preserved_entry(
            IdmlPath::new("Resources/Graphic.xml").unwrap(),
            b"raw".as_slice(),
        );

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "preserved package entry",
                id,
                reason: "entry is not present in DesignMap",
            } if id == "Resources/Graphic.xml"
        ));
    }

    #[test]
    fn rejects_unsupported_preserved_entry_compression() {
        let err = PreservedEntry::with_compression(b"raw".as_slice(), CompressionMethod::AES)
            .unwrap_err();

        assert!(matches!(err, IdmlError::InvalidPackage(_)));

        let mut document = make_document();
        let path = IdmlPath::new("Resources/Graphic.xml").unwrap();
        document
            .design_map
            .other_package_srcs
            .insert("idPkg:Graphic".to_owned(), vec![path.clone()]);
        document.insert_preserved_entry_with_options(
            path,
            PreservedEntry {
                data: b"raw".to_vec(),
                compression: CompressionMethod::AES,
            },
        );

        let err = document.validate().unwrap_err();

        assert!(matches!(err, IdmlError::InvalidPackage(_)));
    }

    fn make_document() -> IdmlDocument {
        let mut design_map = DesignMap {
            id: "d1".to_owned(),
            ..DesignMap::default()
        };
        design_map.story_srcs.insert(
            "u1".to_owned(),
            IdmlPath::new("Stories/Story_u1.xml").unwrap(),
        );
        design_map.spread_srcs.insert(
            "u10".to_owned(),
            IdmlPath::new("Spreads/Spread_u10.xml").unwrap(),
        );

        let mut document = IdmlDocument::new(design_map);
        document.insert_story(
            "u1",
            Story {
                id: Some("u1".to_owned()),
                text: "Generated".to_owned(),
            },
        );
        document.insert_spread(
            "u10",
            Spread {
                id: Some("u10".to_owned()),
                pages: Vec::new(),
                text_frames: vec![TextFrame {
                    id: Some("tf1".to_owned()),
                    layer_id: None,
                    parent_story: Some("u1".to_owned()),
                    geometric_bounds: Some(Rect::new(
                        Points::new(0.0),
                        Points::new(0.0),
                        Points::new(72.0),
                        Points::new(144.0),
                    )),
                }],
                page_items: Vec::new(),
            },
        );
        document
    }
}
