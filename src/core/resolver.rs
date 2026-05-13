//! Relational joins between parsed IDML model objects.

use crate::error::{IdmlError, Result};
use crate::model::spread::{Rect, Spread, TextFrame};
use indexmap::IndexMap;

/// A text frame joined with extracted story text.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTextFrame<'a> {
    /// Text frame from the spread.
    pub frame: &'a TextFrame,
    /// Linked story ID.
    pub story_id: &'a str,
    /// Extracted story text.
    pub text: &'a str,
}

/// Owned text-frame/story join result.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTextFrameData {
    /// Text frame ID, when present.
    pub frame_id: Option<String>,
    /// Linked story ID.
    pub story_id: String,
    /// Text frame bounds, when represented directly.
    pub bounds: Option<Rect>,
    /// Extracted story text.
    pub text: String,
}

impl<'a> From<ResolvedTextFrame<'a>> for ResolvedTextFrameData {
    fn from(resolved: ResolvedTextFrame<'a>) -> Self {
        Self {
            frame_id: resolved.frame.id.clone(),
            story_id: resolved.story_id.to_owned(),
            bounds: resolved.frame.geometric_bounds,
            text: resolved.text.to_owned(),
        }
    }
}

/// Joins text frames in a spread with extracted story text.
///
/// Frames without `ParentStory` are ignored. Frames with a `ParentStory` that is
/// absent from `story_texts` are rejected because that indicates broken
/// relational integrity.
pub fn resolve_text_frames<'a>(
    spread: &'a Spread,
    story_texts: &'a IndexMap<String, String>,
) -> Result<Vec<ResolvedTextFrame<'a>>> {
    let mut resolved = Vec::new();

    for frame in &spread.text_frames {
        let Some(story_id) = frame.parent_story.as_deref() else {
            continue;
        };
        let text = story_texts
            .get(story_id)
            .ok_or_else(|| IdmlError::MissingReference {
                kind: "text frame parent story",
                id: story_id.to_owned(),
            })?;
        resolved.push(ResolvedTextFrame {
            frame,
            story_id,
            text,
        });
    }

    Ok(resolved)
}

/// Returns resolved text frames whose bounds intersect `query`.
///
/// Frames without direct geometric bounds are skipped.
pub fn text_frames_intersecting<'a>(
    resolved: impl IntoIterator<Item = ResolvedTextFrame<'a>>,
    query: Rect,
) -> Vec<ResolvedTextFrame<'a>> {
    resolved
        .into_iter()
        .filter(|resolved| {
            resolved
                .frame
                .geometric_bounds
                .is_some_and(|bounds| bounds.intersects(query))
        })
        .collect()
}

/// Converts borrowed resolved frames into owned records.
pub fn to_owned_records<'a>(
    resolved: impl IntoIterator<Item = ResolvedTextFrame<'a>>,
) -> Vec<ResolvedTextFrameData> {
    resolved.into_iter().map(Into::into).collect()
}

#[cfg(test)]
mod tests {
    use super::{resolve_text_frames, text_frames_intersecting};
    use crate::IdmlError;
    use crate::core::units::Points;
    use crate::model::spread::{Rect, Spread};
    use indexmap::IndexMap;

    #[test]
    fn resolves_text_frames_against_story_text_map() {
        let spread = Spread::from_xml(
            r#"<Spread>
  <TextFrame Self="tf1" ParentStory="u2" GeometricBounds="0 0 72 72" />
  <TextFrame Self="tf2" />
</Spread>"#,
        )
        .unwrap();
        let story_texts = IndexMap::from([("u2".to_owned(), "Hello".to_owned())]);

        let resolved = resolve_text_frames(&spread, &story_texts).unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].frame.id.as_deref(), Some("tf1"));
        assert_eq!(resolved[0].story_id, "u2");
        assert_eq!(resolved[0].text, "Hello");
    }

    #[test]
    fn rejects_dangling_parent_story() {
        let spread =
            Spread::from_xml(r#"<Spread><TextFrame ParentStory="missing" /></Spread>"#).unwrap();
        let err = resolve_text_frames(&spread, &IndexMap::new()).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "text frame parent story",
                id
            } if id == "missing"
        ));
    }

    #[test]
    fn filters_resolved_text_frames_by_intersection() {
        let spread = Spread::from_xml(
            r#"<Spread>
  <TextFrame Self="inside" ParentStory="u1" GeometricBounds="0 0 72 72" />
  <TextFrame Self="outside" ParentStory="u2" GeometricBounds="100 100 120 120" />
  <TextFrame Self="unbounded" ParentStory="u3" />
</Spread>"#,
        )
        .unwrap();
        let story_texts = IndexMap::from([
            ("u1".to_owned(), "Inside".to_owned()),
            ("u2".to_owned(), "Outside".to_owned()),
            ("u3".to_owned(), "Unbounded".to_owned()),
        ]);

        let resolved = resolve_text_frames(&spread, &story_texts).unwrap();
        let hits = text_frames_intersecting(
            resolved,
            Rect {
                top: Points::new(10.0),
                left: Points::new(10.0),
                bottom: Points::new(80.0),
                right: Points::new(80.0),
            },
        );

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].frame.id.as_deref(), Some("inside"));
        assert_eq!(hits[0].text, "Inside");
    }

    #[test]
    fn converts_resolved_frames_to_owned_records() {
        let spread =
            Spread::from_xml(r#"<Spread><TextFrame Self="tf1" ParentStory="u1" /></Spread>"#)
                .unwrap();
        let story_texts = IndexMap::from([("u1".to_owned(), "Text".to_owned())]);

        let owned = super::to_owned_records(resolve_text_frames(&spread, &story_texts).unwrap());

        assert_eq!(owned[0].frame_id.as_deref(), Some("tf1"));
        assert_eq!(owned[0].story_id, "u1");
        assert_eq!(owned[0].text, "Text");
    }
}
