//! High-level typed IDML document aggregate.

use crate::archive::IdmlPackageWriter;
use crate::error::{IdmlError, Result};
use crate::model::designmap::DesignMap;
use crate::model::spread::Spread;
use crate::model::story::Story;
use indexmap::IndexMap;
use std::io::{Seek, Write};

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
        self.validate_parent_stories()?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::IdmlDocument;
    use crate::IdmlError;
    use crate::archive::{IdmlPackage, IdmlPath};
    use crate::core::units::Points;
    use crate::model::designmap::DesignMap;
    use crate::model::spread::{Rect, Spread, TextFrame};
    use crate::model::story::Story;
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
