//! High-level typed IDML document aggregate.

use crate::archive::{IdmlPackage, IdmlPackageWriter};
use crate::core::resolver::{
    ResolvedTextFrameData, resolve_text_frames, text_frames_intersecting, to_owned_records,
};
use crate::error::{IdmlError, Result};
use crate::model::designmap::DesignMap;
use crate::model::spread::{Rect, Spread};
use crate::model::story::{Story, StoryParseOptions};
use indexmap::{IndexMap, IndexSet};
use std::io::{Read, Seek, Write};

/// Typed IDML document parts that can be validated and written as one package.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdmlDocument {
    /// Root package manifest.
    pub design_map: DesignMap,
    /// Story models keyed by their `DesignMap` story ID.
    pub stories: IndexMap<String, Story>,
    /// Spread models keyed by their `DesignMap` spread ID.
    pub spreads: IndexMap<String, Spread>,
}

/// Parser limits used when eagerly loading an [`IdmlDocument`] from a package.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IdmlDocumentReadOptions {
    /// Story parser limits.
    pub story: StoryParseOptions,
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
        for story in document.stories.values() {
            if let Some(id) = &story.id {
                self.reserve_existing(id);
            }
        }
        for spread in document.spreads.values() {
            if let Some(id) = &spread.id {
                self.reserve_existing(id);
            }
            for frame in &spread.text_frames {
                if let Some(id) = &frame.id {
                    self.reserve_existing(id);
                }
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

    /// Creates a deterministic allocator with all current document IDs reserved.
    pub fn id_allocator(&self) -> Result<IdmlIdAllocator> {
        IdmlIdAllocator::from_document(self)
    }

    /// Validates model presence and cross-file story references.
    pub fn validate(&self) -> Result<()> {
        validate_manifest_models("story model", &self.design_map.story_srcs, &self.stories)?;
        validate_manifest_models("spread model", &self.design_map.spread_srcs, &self.spreads)?;
        validate_no_unreferenced_models("story", &self.design_map.story_srcs, &self.stories)?;
        validate_no_unreferenced_models("spread", &self.design_map.spread_srcs, &self.spreads)?;
        self.validate_story_ids()?;
        self.validate_spread_ids()?;
        self.validate_unique_object_ids()?;
        self.validate_parent_stories()?;
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
        let design_map = package.read_designmap()?;
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

        let mut document = Self::new(design_map);
        for (id, path) in story_refs {
            document.insert_story(id, package.read_story_with_options(&path, options.story)?);
        }
        for (id, path) in spread_refs {
            document.insert_spread(id, package.read_spread(&path)?);
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

    /// Validates and writes the aggregate as a complete IDML package.
    pub fn write_to<W>(&self, writer: W) -> Result<W>
    where
        W: Write + Seek,
    {
        self.validate()?;

        let mut package = IdmlPackageWriter::new(writer)?;
        package.add_designmap(&self.design_map)?;
        for (spread_id, path) in &self.design_map.spread_srcs {
            let spread = self
                .spreads
                .get(spread_id)
                .expect("validated spread presence");
            package.add_spread(path.as_str(), spread)?;
        }
        for (story_id, path) in &self.design_map.story_srcs {
            let story = self
                .stories
                .get(story_id)
                .expect("validated story presence");
            package.add_story(path.as_str(), story)?;
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

    fn validate_unique_object_ids(&self) -> Result<()> {
        let mut seen = IndexSet::new();

        for id in self.design_map.story_srcs.keys() {
            remember_object_id(&mut seen, id)?;
        }
        for id in self.design_map.spread_srcs.keys() {
            remember_object_id(&mut seen, id)?;
        }
        for spread in self.spreads.values() {
            for frame in &spread.text_frames {
                if let Some(id) = &frame.id {
                    remember_object_id(&mut seen, id)?;
                }
            }
        }

        Ok(())
    }
}

fn validate_manifest_models<T>(
    kind: &'static str,
    manifest: &IndexMap<String, crate::archive::IdmlPath>,
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
    manifest: &IndexMap<String, crate::archive::IdmlPath>,
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
    use super::{IdmlDocument, IdmlDocumentReadOptions, IdmlIdAllocator};
    use crate::IdmlError;
    use crate::archive::{IdmlPackage, IdmlPackageWriter, IdmlPath};
    use crate::core::units::Points;
    use crate::model::designmap::DesignMap;
    use crate::model::spread::{Rect, Spread, TextFrame};
    use crate::model::story::{Story, StoryParseOptions};
    use std::io::Cursor;

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
                story: StoryParseOptions { max_text_bytes: 4 },
            },
        )
        .unwrap_err();

        assert!(matches!(err, IdmlError::LimitExceeded { what, .. } if what == "story text bytes"));
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
    fn rejects_duplicate_text_frame_ids() {
        let mut document = make_document();
        document
            .spreads
            .get_mut("u10")
            .unwrap()
            .text_frames
            .push(TextFrame {
                id: Some("tf1".to_owned()),
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
                text_frames: Vec::new(),
            },
        );

        let err = document.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::DuplicateId {
                kind: "document object",
                id,
            } if id == "u1"
        ));
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
                text_frames: vec![TextFrame {
                    id: Some("tf1".to_owned()),
                    parent_story: Some("u1".to_owned()),
                    geometric_bounds: Some(Rect::new(
                        Points::new(0.0),
                        Points::new(0.0),
                        Points::new(72.0),
                        Points::new(144.0),
                    )),
                }],
            },
        );
        document
    }
}
