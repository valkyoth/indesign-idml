//! Parser for IDML spread geometry.

use crate::core::units::{Millimeters, Points};
use crate::error::{IdmlError, Result};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

/// Represents a parsed IDML spread.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spread {
    /// Spread `Self` identifier, when present.
    pub id: Option<String>,
    /// Text frames in XML order.
    pub text_frames: Vec<TextFrame>,
}

impl Spread {
    /// Parses a spread XML document.
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml_content);
        reader.config_mut().trim_text(true);

        let mut spread = Self::default();

        loop {
            match reader.read_event()? {
                Event::Start(e) if e.name().as_ref() == b"Spread" && spread.id.is_none() => {
                    spread.id = optional_attr(&e, "Self", &reader)?;
                }
                Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"TextFrame" => {
                    spread
                        .text_frames
                        .push(TextFrame::from_xml_event(&e, &reader)?);
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(spread)
    }

    /// Returns text frames linked to a specific story ID.
    pub fn text_frames_for_story<'a>(
        &'a self,
        story_id: &'a str,
    ) -> impl Iterator<Item = &'a TextFrame> + 'a {
        self.text_frames
            .iter()
            .filter(move |frame| frame.parent_story.as_deref() == Some(story_id))
    }
}

/// Text frame geometry and story relationship.
#[derive(Clone, Debug, PartialEq)]
pub struct TextFrame {
    /// Text frame `Self` identifier, when present.
    pub id: Option<String>,
    /// Linked story ID from `ParentStory`, when present.
    pub parent_story: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
}

impl TextFrame {
    fn from_xml_event(event: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<Self> {
        let id = optional_attr(event, "Self", reader)?;
        let parent_story = optional_attr(event, "ParentStory", reader)?;
        let geometric_bounds = optional_attr(event, "GeometricBounds", reader)?
            .map(|value| parse_geometric_bounds(&value))
            .transpose()?;

        Ok(Self {
            id,
            parent_story,
            geometric_bounds,
        })
    }
}

/// Rectangle in IDML point coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Top coordinate.
    pub top: Points,
    /// Left coordinate.
    pub left: Points,
    /// Bottom coordinate.
    pub bottom: Points,
    /// Right coordinate.
    pub right: Points,
}

impl Rect {
    /// Width in points.
    #[must_use]
    pub fn width(self) -> Points {
        Points::new(self.right.as_f64() - self.left.as_f64())
    }

    /// Height in points.
    #[must_use]
    pub fn height(self) -> Points {
        Points::new(self.bottom.as_f64() - self.top.as_f64())
    }

    /// Returns the same rectangle converted to millimeters.
    #[must_use]
    pub fn to_millimeters(self) -> RectMm {
        RectMm {
            top: self.top.to_millimeters(),
            left: self.left.to_millimeters(),
            bottom: self.bottom.to_millimeters(),
            right: self.right.to_millimeters(),
        }
    }
}

/// Rectangle in millimeter coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectMm {
    /// Top coordinate.
    pub top: Millimeters,
    /// Left coordinate.
    pub left: Millimeters,
    /// Bottom coordinate.
    pub bottom: Millimeters,
    /// Right coordinate.
    pub right: Millimeters,
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

fn parse_geometric_bounds(value: &str) -> Result<Rect> {
    let values = value
        .split_whitespace()
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let [top, left, bottom, right] = values.as_slice() else {
        return Err(IdmlError::InvalidAttribute {
            element: "TextFrame".to_owned(),
            attribute: "GeometricBounds",
            reason: "expected four point coordinates",
        });
    };

    Ok(Rect {
        top: Points::new(*top),
        left: Points::new(*left),
        bottom: Points::new(*bottom),
        right: Points::new(*right),
    })
}

#[cfg(test)]
mod tests {
    use super::Spread;
    use crate::IdmlError;

    #[test]
    fn parses_text_frames_with_bounds_and_parent_story() {
        let xml = r#"<Spread Self="u10">
  <TextFrame Self="tf1" ParentStory="u2" GeometricBounds="0 72 144 216" />
  <TextFrame Self="tf2" ParentStory="u3" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.id.as_deref(), Some("u10"));
        assert_eq!(spread.text_frames.len(), 2);
        assert_eq!(spread.text_frames[0].id.as_deref(), Some("tf1"));
        assert_eq!(spread.text_frames[0].parent_story.as_deref(), Some("u2"));
        let bounds = spread.text_frames[0].geometric_bounds.unwrap();
        assert_eq!(bounds.width().as_f64(), 144.0);
        assert_eq!(bounds.height().as_f64(), 144.0);
        assert_eq!(bounds.to_millimeters().left.as_f64(), 25.4);
        assert!(spread.text_frames[1].geometric_bounds.is_none());
    }

    #[test]
    fn filters_text_frames_by_story() {
        let xml = r#"<Spread>
  <TextFrame Self="tf1" ParentStory="u2" />
  <TextFrame Self="tf2" ParentStory="u3" />
  <TextFrame Self="tf3" ParentStory="u2" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();
        let frame_ids = spread
            .text_frames_for_story("u2")
            .map(|frame| frame.id.as_deref())
            .collect::<Vec<_>>();

        assert_eq!(frame_ids, [Some("tf1"), Some("tf3")]);
    }

    #[test]
    fn rejects_malformed_geometric_bounds() {
        let err = Spread::from_xml(r#"<Spread><TextFrame GeometricBounds="1 2 3" /></Spread>"#)
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                attribute: "GeometricBounds",
                ..
            }
        ));
    }
}
