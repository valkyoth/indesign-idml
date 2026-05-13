//! High-level typed IDML document aggregate.

use crate::archive::{IdmlPackage, IdmlPackageWriter};
use crate::error::{IdmlError, Result};
use crate::model::designmap::DesignMap;
use crate::model::spread::Spread;
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

#[cfg(test)]
mod tests {
    use super::{IdmlDocument, IdmlDocumentReadOptions};
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
