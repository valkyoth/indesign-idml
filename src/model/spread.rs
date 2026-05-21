//! Parser for IDML spread geometry.

use crate::core::units::{Millimeters, Points};
use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::{DEFAULT_MAX_XML_DEPTH, XmlDepthTracker, validate_xml_attribute};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};

/// Represents a parsed IDML spread.
#[derive(Clone, Debug, Default)]
pub struct Spread {
    /// Spread `Self` identifier, when present.
    pub id: Option<String>,
    /// Text frames in XML order.
    pub text_frames: Vec<TextFrame>,
    /// Page items in XML order for z-order preservation.
    ///
    /// This list is populated by the parser. Manually constructed spreads may
    /// leave it empty; in that case serialization falls back to `text_frames`.
    pub page_items: Vec<PageItem>,
}

impl PartialEq for Spread {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.text_frames == other.text_frames
            && compatible_page_items(self, other)
    }
}

/// Default maximum text frames accepted in one spread.
pub const DEFAULT_MAX_TEXT_FRAMES_PER_SPREAD: usize = 100_000;

/// Default maximum page items accepted in one spread.
pub const DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD: usize = 200_000;

/// Spread parser limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpreadParseOptions {
    /// Maximum `TextFrame` elements collected from one spread.
    pub max_text_frames: usize,
    /// Maximum supported page items collected from one spread.
    pub max_page_items: usize,
    /// Maximum XML element nesting depth accepted while parsing.
    pub max_xml_depth: usize,
}

impl Default for SpreadParseOptions {
    fn default() -> Self {
        Self {
            max_text_frames: DEFAULT_MAX_TEXT_FRAMES_PER_SPREAD,
            max_page_items: DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD,
            max_xml_depth: DEFAULT_MAX_XML_DEPTH,
        }
    }
}

impl Spread {
    /// Parses a spread XML document.
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        Self::from_xml_with_options(xml_content, SpreadParseOptions::default())
    }

    /// Parses a spread XML document with explicit limits.
    pub fn from_xml_with_options(xml_content: &str, options: SpreadParseOptions) -> Result<Self> {
        let mut reader = Reader::from_str(xml_content);
        reader.config_mut().trim_text(true);

        let mut spread = Self::default();
        let mut xml_depth = XmlDepthTracker::new(options.max_xml_depth, "Spread XML depth");

        loop {
            match reader.read_event()? {
                Event::Start(e) => {
                    xml_depth.enter()?;
                    if e.name().as_ref() == b"Spread" && spread.id.is_none() {
                        spread.id = optional_attr(&e, "Self", &reader)?;
                    } else if e.name().as_ref() == b"TextFrame" {
                        spread.push_text_frame_limited(
                            TextFrame::from_xml_event(&e, &reader)?,
                            options,
                        )?;
                    } else if e.name().as_ref() == b"Rectangle" {
                        spread.push_page_item_limited(
                            PageItem::Rectangle(Rectangle::from_xml_event(&e, &reader)?),
                            options.max_page_items,
                        )?;
                    }
                }
                Event::Empty(e) => {
                    xml_depth.visit_empty()?;
                    if e.name().as_ref() == b"TextFrame" {
                        spread.push_text_frame_limited(
                            TextFrame::from_xml_event(&e, &reader)?,
                            options,
                        )?;
                    } else if e.name().as_ref() == b"Rectangle" {
                        spread.push_page_item_limited(
                            PageItem::Rectangle(Rectangle::from_xml_event(&e, &reader)?),
                            options.max_page_items,
                        )?;
                    }
                }
                Event::End(_) => {
                    xml_depth.exit();
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

    fn push_text_frame_limited(
        &mut self,
        text_frame: TextFrame,
        options: SpreadParseOptions,
    ) -> Result<()> {
        push_text_frame_index_limited(
            &mut self.text_frames,
            text_frame.clone(),
            options.max_text_frames,
        )?;
        self.push_page_item_limited(PageItem::TextFrame(text_frame), options.max_page_items)
    }

    fn push_page_item_limited(&mut self, page_item: PageItem, max_page_items: usize) -> Result<()> {
        let next_len = self
            .page_items
            .len()
            .checked_add(1)
            .ok_or(IdmlError::LimitExceeded {
                what: "spread page item count",
                limit: max_page_items as u64,
                actual: u64::MAX,
            })?;

        if next_len > max_page_items {
            return Err(IdmlError::LimitExceeded {
                what: "spread page item count",
                limit: max_page_items as u64,
                actual: next_len as u64,
            });
        }

        self.page_items.push(page_item);
        Ok(())
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

/// Rectangle geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Rectangle {
    /// Rectangle `Self` identifier, when present.
    pub id: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
}

impl Rectangle {
    fn from_xml_event(event: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<Self> {
        let id = optional_attr(event, "Self", reader)?;
        let geometric_bounds = optional_attr(event, "GeometricBounds", reader)?
            .map(|value| parse_geometric_bounds("Rectangle", &value))
            .transpose()?;

        Ok(Self {
            id,
            geometric_bounds,
        })
    }
}

/// Supported spread page item in z-order.
#[derive(Clone, Debug, PartialEq)]
pub enum PageItem {
    /// A text frame page item.
    TextFrame(TextFrame),
    /// A rectangle page item.
    Rectangle(Rectangle),
}

impl TextFrame {
    fn from_xml_event(event: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<Self> {
        let id = optional_attr(event, "Self", reader)?;
        let parent_story = optional_attr(event, "ParentStory", reader)?;
        let geometric_bounds = optional_attr(event, "GeometricBounds", reader)?
            .map(|value| parse_geometric_bounds("TextFrame", &value))
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

fn parse_geometric_bounds(element: &'static str, value: &str) -> Result<Rect> {
    let values = value
        .split_whitespace()
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let [top, left, bottom, right] = values.as_slice() else {
        return Err(IdmlError::InvalidAttribute {
            element: element.to_owned(),
            attribute: "GeometricBounds",
            reason: "expected four point coordinates",
        });
    };

    let rect = Rect {
        top: Points::new(*top),
        left: Points::new(*left),
        bottom: Points::new(*bottom),
        right: Points::new(*right),
    };
    validate_finite_bounds(element, rect)?;
    Ok(rect)
}

fn push_text_frame_index_limited(
    text_frames: &mut Vec<TextFrame>,
    text_frame: TextFrame,
    max_text_frames: usize,
) -> Result<()> {
    let next_len = text_frames
        .len()
        .checked_add(1)
        .ok_or(IdmlError::LimitExceeded {
            what: "spread text frame count",
            limit: max_text_frames as u64,
            actual: u64::MAX,
        })?;

    if next_len > max_text_frames {
        return Err(IdmlError::LimitExceeded {
            what: "spread text frame count",
            limit: max_text_frames as u64,
            actual: next_len as u64,
        });
    }

    text_frames.push(text_frame);
    Ok(())
}

fn serialize_spread(spread: &Spread) -> Result<String> {
    validate_page_items(spread)?;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Spread");
    if let Some(id) = &spread.id {
        push_attr(&mut xml, "Spread", "Self", id)?;
    }
    xml.push_str(">\n");

    if spread.page_items.is_empty() {
        for frame in &spread.text_frames {
            serialize_text_frame(&mut xml, frame)?;
        }
    } else {
        for page_item in &spread.page_items {
            match page_item {
                PageItem::TextFrame(frame) => serialize_text_frame(&mut xml, frame)?,
                PageItem::Rectangle(rectangle) => serialize_rectangle(&mut xml, rectangle)?,
            }
        }
    }

    xml.push_str("</Spread>\n");
    Ok(xml)
}

fn validate_page_items(spread: &Spread) -> Result<()> {
    if spread.page_items.is_empty() {
        return Ok(());
    }

    let page_item_text_frames = spread
        .page_items
        .iter()
        .filter_map(|item| match item {
            PageItem::TextFrame(frame) => Some(frame),
            PageItem::Rectangle(_) => None,
        })
        .collect::<Vec<_>>();

    if page_item_text_frames.len() != spread.text_frames.len() {
        return Err(stale_page_items_error());
    }

    for (from_items, from_index) in page_item_text_frames.iter().zip(&spread.text_frames) {
        if *from_items != from_index {
            return Err(stale_page_items_error());
        }
    }

    Ok(())
}

fn stale_page_items_error() -> IdmlError {
    IdmlError::InvalidReference {
        kind: "spread page items",
        id: "text_frames".to_owned(),
        reason: "ordered page items do not match text frame index",
    }
}

fn compatible_page_items(left: &Spread, right: &Spread) -> bool {
    left.page_items.is_empty() || right.page_items.is_empty() || left.page_items == right.page_items
}

fn serialize_text_frame(xml: &mut String, frame: &TextFrame) -> Result<()> {
    xml.push_str("  <TextFrame");
    if let Some(id) = &frame.id {
        push_attr(xml, "TextFrame", "Self", id)?;
    }
    if let Some(parent_story) = &frame.parent_story {
        push_attr(xml, "TextFrame", "ParentStory", parent_story)?;
    }
    if let Some(bounds) = frame.geometric_bounds {
        let bounds = format_geometric_bounds("TextFrame", bounds)?;
        push_attr(xml, "TextFrame", "GeometricBounds", &bounds)?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_rectangle(xml: &mut String, rectangle: &Rectangle) -> Result<()> {
    xml.push_str("  <Rectangle");
    if let Some(id) = &rectangle.id {
        push_attr(xml, "Rectangle", "Self", id)?;
    }
    if let Some(bounds) = rectangle.geometric_bounds {
        let bounds = format_geometric_bounds("Rectangle", bounds)?;
        push_attr(xml, "Rectangle", "GeometricBounds", &bounds)?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn format_geometric_bounds(element: &'static str, bounds: Rect) -> Result<String> {
    validate_finite_bounds(element, bounds)?;
    Ok(format!(
        "{} {} {} {}",
        format_point(bounds.top),
        format_point(bounds.left),
        format_point(bounds.bottom),
        format_point(bounds.right)
    ))
}

fn validate_finite_bounds(element: &'static str, bounds: Rect) -> Result<()> {
    for value in [bounds.top, bounds.left, bounds.bottom, bounds.right] {
        if !value.as_f64().is_finite() {
            return Err(non_finite_bounds_error(element));
        }
    }
    Ok(())
}

fn non_finite_bounds_error(element: &'static str) -> IdmlError {
    IdmlError::InvalidAttribute {
        element: element.to_owned(),
        attribute: "GeometricBounds",
        reason: "coordinate must be finite",
    }
}

fn format_point(point: Points) -> String {
    let value = point.as_f64();
    if value == 0.0 {
        return "0".to_owned();
    }
    value.to_string()
}

fn push_attr(
    xml: &mut String,
    element: &'static str,
    name: &'static str,
    value: &str,
) -> Result<()> {
    validate_xml_attribute(element, name, value)?;
    xml.push(' ');
    xml.push_str(name);
    xml.push_str("=\"");
    xml.push_str(escape(value).as_ref());
    xml.push('"');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PageItem, Rect, Rectangle, Spread, SpreadParseOptions, TextFrame};
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
        assert_eq!(spread.page_items.len(), 2);
        assert!(matches!(spread.page_items[0], PageItem::TextFrame(_)));
        assert_eq!(spread.text_frames[0].id.as_deref(), Some("tf1"));
        assert_eq!(spread.text_frames[0].parent_story.as_deref(), Some("u2"));
        let bounds = spread.text_frames[0].geometric_bounds.unwrap();
        assert_eq!(bounds.width().as_f64(), 144.0);
        assert_eq!(bounds.height().as_f64(), 144.0);
        assert_eq!(bounds.to_millimeters().left.as_f64(), 25.4);
        assert!(spread.text_frames[1].geometric_bounds.is_none());
    }

    #[test]
    fn parses_rectangles_and_preserves_page_item_order() {
        let xml = r#"<Spread Self="u10">
  <Rectangle Self="r1" GeometricBounds="0 0 72 72" />
  <TextFrame Self="tf1" ParentStory="u2" GeometricBounds="72 0 144 72" />
  <Rectangle Self="r2" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 1);
        assert_eq!(spread.page_items.len(), 3);
        assert!(matches!(
            &spread.page_items[0],
            PageItem::Rectangle(Rectangle {
                id,
                geometric_bounds: Some(_)
            }) if id.as_deref() == Some("r1")
        ));
        assert!(matches!(
            &spread.page_items[1],
            PageItem::TextFrame(TextFrame { id, .. }) if id.as_deref() == Some("tf1")
        ));
        assert!(matches!(
            &spread.page_items[2],
            PageItem::Rectangle(Rectangle {
                id,
                geometric_bounds: None
            }) if id.as_deref() == Some("r2")
        ));
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
    fn rejects_non_finite_geometric_bounds_on_parse() {
        let err =
            Spread::from_xml(r#"<Spread><TextFrame GeometricBounds="0 NaN 72 72" /></Spread>"#)
                .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                attribute: "GeometricBounds",
                reason: "coordinate must be finite",
                ..
            }
        ));
    }

    #[test]
    fn enforces_text_frame_count_limit() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><TextFrame Self="tf1" /><TextFrame Self="tf2" /></Spread>"#,
            SpreadParseOptions {
                max_text_frames: 1,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread text frame count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn enforces_page_item_count_limit() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><Rectangle Self="r1" /><TextFrame Self="tf1" /></Spread>"#,
            SpreadParseOptions {
                max_page_items: 1,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread page item count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn enforces_xml_depth_limit() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><Group><TextFrame Self="tf1" /></Group></Spread>"#,
            SpreadParseOptions {
                max_xml_depth: 2,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "Spread XML depth",
                limit: 2,
                actual: 3,
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
            page_items: Vec::new(),
        };

        let xml = <Spread as crate::XmlSaveable>::to_xml(&spread).unwrap();
        let parsed = <Spread as crate::XmlLoadable>::from_xml(&xml).unwrap();

        assert!(xml.contains("Self=\"u&amp;&quot;10\""));
        assert!(xml.contains("Self=\"tf&amp;1\""));
        assert!(xml.contains("GeometricBounds=\"0 72.5 144 216.25\""));
        assert_eq!(parsed, spread);
    }

    #[test]
    fn serializer_preserves_page_item_order() {
        let text_frame = TextFrame {
            id: Some("tf1".to_owned()),
            parent_story: Some("u2".to_owned()),
            geometric_bounds: Some(Rect::new(
                Points::new(72.0),
                Points::new(0.0),
                Points::new(144.0),
                Points::new(72.0),
            )),
        };
        let spread = Spread {
            id: Some("u10".to_owned()),
            text_frames: vec![text_frame.clone()],
            page_items: vec![
                PageItem::Rectangle(Rectangle {
                    id: Some("r1".to_owned()),
                    geometric_bounds: Some(Rect::new(
                        Points::new(0.0),
                        Points::new(0.0),
                        Points::new(72.0),
                        Points::new(72.0),
                    )),
                }),
                PageItem::TextFrame(text_frame),
            ],
        };

        let xml = spread.to_xml().unwrap();
        let rectangle = xml.find("<Rectangle").unwrap();
        let frame = xml.find("<TextFrame").unwrap();
        let parsed = Spread::from_xml(&xml).unwrap();

        assert!(rectangle < frame);
        assert_eq!(parsed, spread);
        assert_eq!(parsed.page_items, spread.page_items);
    }

    #[test]
    fn serializer_rejects_stale_page_item_index() {
        let spread = Spread {
            id: Some("u10".to_owned()),
            text_frames: vec![TextFrame {
                id: Some("tf1".to_owned()),
                parent_story: Some("u2".to_owned()),
                geometric_bounds: None,
            }],
            page_items: vec![PageItem::TextFrame(TextFrame {
                id: Some("tf2".to_owned()),
                parent_story: Some("u2".to_owned()),
                geometric_bounds: None,
            })],
        };

        let err = spread.to_xml().unwrap_err();

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
            page_items: Vec::new(),
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

    #[test]
    fn serializer_rejects_xml_forbidden_frame_attributes() {
        let spread = Spread {
            id: Some("u1".to_owned()),
            text_frames: vec![TextFrame {
                id: Some("tf\u{0}".to_owned()),
                parent_story: Some("u1".to_owned()),
                geometric_bounds: None,
            }],
            page_items: Vec::new(),
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "TextFrame"
        ));
    }
}
