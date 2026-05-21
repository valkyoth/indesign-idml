//! Relational joins between parsed IDML model objects.

use crate::error::{IdmlError, Result};
use crate::model::spread::{Page, Rect, Spread, TextFrame};
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

/// Returns resolved text frames whose bounds intersect the requested page.
///
/// Frames without direct geometric bounds are skipped. A page without direct
/// geometric bounds is rejected because page-scoped lookup would otherwise be
/// ambiguous.
pub fn text_frames_on_page<'a>(
    resolved: impl IntoIterator<Item = ResolvedTextFrame<'a>>,
    page: &Page,
) -> Result<Vec<ResolvedTextFrame<'a>>> {
    let bounds = page
        .geometric_bounds
        .ok_or_else(|| IdmlError::InvalidReference {
            kind: "spread page",
            id: page
                .id
                .clone()
                .unwrap_or_else(|| "<unidentified>".to_owned()),
            reason: "page has no geometric bounds",
        })?;
    Ok(text_frames_intersecting(resolved, bounds))
}

/// Finds a page by `Self` ID.
pub fn page_by_id<'a>(spread: &'a Spread, page_id: &str) -> Result<&'a Page> {
    spread
        .pages
        .iter()
        .find(|page| page.id.as_deref() == Some(page_id))
        .ok_or_else(|| IdmlError::MissingReference {
            kind: "spread page",
            id: page_id.to_owned(),
        })
}

/// Converts borrowed resolved frames into owned records.
pub fn to_owned_records<'a>(
    resolved: impl IntoIterator<Item = ResolvedTextFrame<'a>>,
) -> Vec<ResolvedTextFrameData> {
    resolved.into_iter().map(Into::into).collect()
}

#[cfg(test)]
mod tests {
    use super::{page_by_id, resolve_text_frames, text_frames_intersecting, text_frames_on_page};
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
    fn resolves_grouped_text_frames_against_story_text_map() {
        let spread = Spread::from_xml(
            r#"<Spread>
  <Group Self="g1">
    <TextFrame Self="tf1" ParentStory="u2" />
  </Group>
</Spread>"#,
        )
        .unwrap();
        let story_texts = IndexMap::from([("u2".to_owned(), "Grouped".to_owned())]);

        let resolved = resolve_text_frames(&spread, &story_texts).unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].frame.id.as_deref(), Some("tf1"));
        assert_eq!(resolved[0].text, "Grouped");
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
    fn filters_resolved_text_frames_by_page_bounds() {
        let spread = Spread::from_xml(
            r#"<Spread>
  <Page Self="p1" GeometricBounds="0 0 100 100" />
  <TextFrame Self="inside" ParentStory="u1" GeometricBounds="10 10 20 20" />
  <TextFrame Self="outside" ParentStory="u2" GeometricBounds="200 200 300 300" />
</Spread>"#,
        )
        .unwrap();
        let story_texts = IndexMap::from([
            ("u1".to_owned(), "Inside".to_owned()),
            ("u2".to_owned(), "Outside".to_owned()),
        ]);

        let resolved = resolve_text_frames(&spread, &story_texts).unwrap();
        let hits = text_frames_on_page(resolved, page_by_id(&spread, "p1").unwrap()).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].frame.id.as_deref(), Some("inside"));
    }

    #[test]
    fn page_lookup_reports_missing_page() {
        let spread = Spread::default();

        let err = page_by_id(&spread, "p404").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::MissingReference {
                kind: "spread page",
                id,
            } if id == "p404"
        ));
    }

    #[test]
    fn page_scoped_filter_rejects_unbounded_page() {
        let spread = Spread::from_xml(
            r#"<Spread>
  <Page Self="p1" />
  <TextFrame Self="tf1" ParentStory="u1" GeometricBounds="10 10 20 20" />
</Spread>"#,
        )
        .unwrap();
        let story_texts = IndexMap::from([("u1".to_owned(), "Text".to_owned())]);
        let resolved = resolve_text_frames(&spread, &story_texts).unwrap();

        let err = text_frames_on_page(resolved, page_by_id(&spread, "p1").unwrap()).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidReference {
                kind: "spread page",
                id,
                reason: "page has no geometric bounds",
            } if id == "p1"
        ));
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
