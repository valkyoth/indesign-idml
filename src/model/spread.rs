//! Parser for IDML spread geometry.

use crate::core::units::{Millimeters, Points};
use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::escape;
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

    /// Serializes this spread into a minimal standalone spread XML document.
    pub fn to_xml(&self) -> Result<String> {
        serialize_spread(self)
    }
}

impl XmlLoadable for Spread {
    fn from_xml(xml: &str) -> Result<Self> {
        Spread::from_xml(xml)
    }
}

impl XmlSaveable for Spread {
    fn to_xml(&self) -> Result<String> {
        serialize_spread(self)
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
    /// Creates a rectangle in point coordinates.
    #[must_use]
    pub const fn new(top: Points, left: Points, bottom: Points, right: Points) -> Self {
        Self {
            top,
            left,
            bottom,
            right,
        }
    }

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

    /// Returns true if the rectangle has positive area.
    #[must_use]
    pub fn has_positive_area(self) -> bool {
        self.width().as_f64() > 0.0 && self.height().as_f64() > 0.0
    }

    /// Returns true when this rectangle intersects another rectangle.
    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.has_positive_area()
            && other.has_positive_area()
            && self.left.as_f64() < other.right.as_f64()
            && self.right.as_f64() > other.left.as_f64()
            && self.top.as_f64() < other.bottom.as_f64()
            && self.bottom.as_f64() > other.top.as_f64()
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

fn serialize_spread(spread: &Spread) -> Result<String> {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Spread");
    if let Some(id) = &spread.id {
        push_attr(&mut xml, "Self", id);
    }
    xml.push_str(">\n");

    for frame in &spread.text_frames {
        serialize_text_frame(&mut xml, frame)?;
    }

    xml.push_str("</Spread>\n");
    Ok(xml)
}

fn serialize_text_frame(xml: &mut String, frame: &TextFrame) -> Result<()> {
    xml.push_str("  <TextFrame");
    if let Some(id) = &frame.id {
        push_attr(xml, "Self", id);
    }
    if let Some(parent_story) = &frame.parent_story {
        push_attr(xml, "ParentStory", parent_story);
    }
    if let Some(bounds) = frame.geometric_bounds {
        let bounds = format_geometric_bounds(bounds)?;
        push_attr(xml, "GeometricBounds", &bounds);
    }
    xml.push_str(" />\n");
    Ok(())
}

fn format_geometric_bounds(bounds: Rect) -> Result<String> {
    Ok(format!(
        "{} {} {} {}",
        format_point(bounds.top)?,
        format_point(bounds.left)?,
        format_point(bounds.bottom)?,
        format_point(bounds.right)?
    ))
}

fn format_point(point: Points) -> Result<String> {
    let value = point.as_f64();
    if !value.is_finite() {
        return Err(IdmlError::InvalidAttribute {
            element: "TextFrame".to_owned(),
            attribute: "GeometricBounds",
            reason: "coordinate must be finite",
        });
    }
    if value == 0.0 {
        return Ok("0".to_owned());
    }
    Ok(value.to_string())
}

fn push_attr(xml: &mut String, name: &str, value: &str) {
    xml.push(' ');
    xml.push_str(name);
    xml.push_str("=\"");
    xml.push_str(escape(value).as_ref());
    xml.push('"');
}

#[cfg(test)]
mod tests {
    use super::{Rect, Spread, TextFrame};
    use crate::IdmlError;
    use crate::core::units::Points;

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

    #[test]
    fn detects_rectangle_intersection() {
        let a = super::Rect::new(
            crate::core::units::Points::new(0.0),
            crate::core::units::Points::new(0.0),
            crate::core::units::Points::new(100.0),
            crate::core::units::Points::new(100.0),
        );
        let b = super::Rect::new(
            crate::core::units::Points::new(50.0),
            crate::core::units::Points::new(50.0),
            crate::core::units::Points::new(150.0),
            crate::core::units::Points::new(150.0),
        );
        let c = super::Rect::new(
            crate::core::units::Points::new(100.0),
            crate::core::units::Points::new(100.0),
            crate::core::units::Points::new(150.0),
            crate::core::units::Points::new(150.0),
        );

        assert!(a.intersects(b));
        assert!(!a.intersects(c));
    }

    #[test]
    fn serializes_spread_text_frames_and_round_trips() {
        let spread = Spread {
            id: Some("u&\"10".to_owned()),
            text_frames: vec![TextFrame {
                id: Some("tf&1".to_owned()),
                parent_story: Some("u2".to_owned()),
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(72.5),
                    Points::new(144.0),
                    Points::new(216.25),
                )),
            }],
        };

        let xml = <Spread as crate::XmlSaveable>::to_xml(&spread).unwrap();
        let parsed = <Spread as crate::XmlLoadable>::from_xml(&xml).unwrap();

        assert!(xml.contains("Self=\"u&amp;&quot;10\""));
        assert!(xml.contains("Self=\"tf&amp;1\""));
        assert!(xml.contains("GeometricBounds=\"0 72.5 144 216.25\""));
        assert_eq!(parsed, spread);
    }

    #[test]
    fn serializer_rejects_non_finite_coordinates() {
        let spread = Spread {
            id: Some("u1".to_owned()),
            text_frames: vec![TextFrame {
                id: Some("tf1".to_owned()),
                parent_story: Some("u1".to_owned()),
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(f64::NAN),
                    Points::new(144.0),
                    Points::new(216.0),
                )),
            }],
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                attribute: "GeometricBounds",
                reason: "coordinate must be finite",
                ..
            }
        ));
    }
}
