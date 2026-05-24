//! Parser for IDML spread geometry.

use crate::core::units::{Millimeters, Points};
use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::{
    AnyRootTracker, DEFAULT_MAX_XML_DEPTH, DEFAULT_MAX_XML_INPUT_BYTES, XmlDepthTracker,
    configure_idml_reader, enforce_xml_input_bytes, reject_xml_doctype,
    reject_xml_entity_reference, validate_xml_attribute,
};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};

/// Represents a parsed IDML spread.
#[derive(Clone, Debug, Default)]
pub struct Spread {
    /// Spread `Self` identifier, when present.
    pub id: Option<String>,
    /// Pages in XML order.
    pub pages: Vec<Page>,
    /// Layout guides in XML order.
    pub guides: Vec<Guide>,
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
            && compatible_pages(self, other)
            && compatible_guides(self, other)
            && self.text_frames == other.text_frames
            && compatible_page_items(self, other)
    }
}

/// Default maximum text frames accepted in one spread.
pub const DEFAULT_MAX_TEXT_FRAMES_PER_SPREAD: usize = 100_000;

/// Default maximum page items accepted in one spread.
pub const DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD: usize = 200_000;

/// Default maximum pages accepted in one spread.
pub const DEFAULT_MAX_PAGES_PER_SPREAD: usize = 10_000;

/// Default maximum guides accepted in one spread.
pub const DEFAULT_MAX_GUIDES_PER_SPREAD: usize = 100_000;

/// Default maximum nested images accepted in one spread.
pub const DEFAULT_MAX_IMAGES_PER_SPREAD: usize = 100_000;

/// Default maximum path points accepted in one spread.
pub const DEFAULT_MAX_PATH_POINTS_PER_SPREAD: usize = 100_000;

/// Default maximum raw bytes accepted for one spread XML attribute value.
pub const DEFAULT_MAX_SPREAD_ATTRIBUTE_BYTES: usize = 256 * 1024;

/// Spread parser limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpreadParseOptions {
    /// Maximum UTF-8 bytes accepted for the spread XML document.
    pub max_xml_bytes: usize,
    /// Maximum `Page` elements collected from one spread.
    pub max_pages: usize,
    /// Maximum `Guide` elements collected from one spread.
    pub max_guides: usize,
    /// Maximum nested `Image` elements collected from one spread.
    pub max_images: usize,
    /// Maximum `PathPointType` elements collected from one spread.
    pub max_path_points: usize,
    /// Maximum `TextFrame` elements collected from one spread.
    pub max_text_frames: usize,
    /// Maximum supported page items collected from one spread.
    pub max_page_items: usize,
    /// Maximum raw bytes accepted for one XML attribute value.
    pub max_attribute_bytes: usize,
    /// Maximum XML element nesting depth accepted while parsing.
    pub max_xml_depth: usize,
}

impl Default for SpreadParseOptions {
    fn default() -> Self {
        Self {
            max_xml_bytes: DEFAULT_MAX_XML_INPUT_BYTES,
            max_pages: DEFAULT_MAX_PAGES_PER_SPREAD,
            max_guides: DEFAULT_MAX_GUIDES_PER_SPREAD,
            max_images: DEFAULT_MAX_IMAGES_PER_SPREAD,
            max_path_points: DEFAULT_MAX_PATH_POINTS_PER_SPREAD,
            max_text_frames: DEFAULT_MAX_TEXT_FRAMES_PER_SPREAD,
            max_page_items: DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD,
            max_attribute_bytes: DEFAULT_MAX_SPREAD_ATTRIBUTE_BYTES,
            max_xml_depth: DEFAULT_MAX_XML_DEPTH,
        }
    }
}

struct NestedSpreadParseState<'a> {
    options: SpreadParseOptions,
    xml_depth: &'a mut XmlDepthTracker,
    root: &'a mut AnyRootTracker,
    text_frames: &'a mut Vec<TextFrame>,
    page_item_count: &'a mut usize,
    image_count: &'a mut usize,
    path_point_count: &'a mut usize,
}

impl Spread {
    /// Parses a spread XML document.
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        Self::from_xml_with_options(xml_content, SpreadParseOptions::default())
    }

    /// Parses a spread XML document with explicit limits.
    pub fn from_xml_with_options(xml_content: &str, options: SpreadParseOptions) -> Result<Self> {
        enforce_xml_input_bytes("Spread XML bytes", xml_content, options.max_xml_bytes)?;

        let mut reader = Reader::from_str(xml_content);
        configure_idml_reader(&mut reader, true);

        let mut spread = Self::default();
        let mut seen_spread = false;
        let mut in_spread = false;
        let mut root_name = None;
        let mut page_item_count = 0usize;
        let mut image_count = 0usize;
        let mut path_point_count = 0usize;
        let mut xml_depth = XmlDepthTracker::new(options.max_xml_depth, "Spread XML depth");
        let mut root = AnyRootTracker::default();

        loop {
            match reader.read_event()? {
                Event::Start(e) => {
                    validate_spread_root(&mut root_name, e.name().as_ref())?;
                    root.enter(e.name().as_ref())?;
                    xml_depth.enter()?;
                    if e.name().as_ref() == b"Spread" {
                        enter_spread_element(&mut seen_spread, &mut in_spread)?;
                        spread.id =
                            optional_attr(&e, "Self", &reader, options.max_attribute_bytes)?;
                    } else if !in_spread {
                        continue;
                    } else if e.name().as_ref() == b"Page" {
                        push_page_limited(
                            &mut spread.pages,
                            Page::from_xml_event(&e, &reader, options.max_attribute_bytes)?,
                            options.max_pages,
                        )?;
                    } else if e.name().as_ref() == b"Guide" {
                        push_guide_limited(
                            &mut spread.guides,
                            Guide::from_xml_event(&e, &reader, options.max_attribute_bytes)?,
                            options.max_guides,
                        )?;
                    } else if e.name().as_ref() == b"TextFrame" {
                        let text_frame =
                            TextFrame::from_xml_event(&e, &reader, options.max_attribute_bytes)?;
                        let mut state = NestedSpreadParseState {
                            options,
                            xml_depth: &mut xml_depth,
                            root: &mut root,
                            text_frames: &mut spread.text_frames,
                            page_item_count: &mut page_item_count,
                            image_count: &mut image_count,
                            path_point_count: &mut path_point_count,
                        };
                        let text_frame =
                            parse_text_frame_contents(&mut reader, text_frame, &mut state)?;
                        push_text_frame_page_item_limited(
                            &mut spread.text_frames,
                            &mut spread.page_items,
                            text_frame,
                            options,
                            &mut page_item_count,
                        )?;
                    } else if e.name().as_ref() == b"TextPath" {
                        push_text_frame_index_limited(
                            &mut spread.text_frames,
                            TextFrame::from_text_path_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?,
                            options.max_text_frames,
                        )?;
                    } else if e.name().as_ref() == b"Rectangle" {
                        let rectangle =
                            Rectangle::from_xml_event(&e, &reader, options.max_attribute_bytes)?;
                        let mut state = NestedSpreadParseState {
                            options,
                            xml_depth: &mut xml_depth,
                            root: &mut root,
                            text_frames: &mut spread.text_frames,
                            page_item_count: &mut page_item_count,
                            image_count: &mut image_count,
                            path_point_count: &mut path_point_count,
                        };
                        let rectangle =
                            parse_rectangle_contents(&mut reader, rectangle, &mut state)?;
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Rectangle(rectangle),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"Oval" {
                        let oval = Oval::from_xml_event(&e, &reader, options.max_attribute_bytes)?;
                        let mut state = NestedSpreadParseState {
                            options,
                            xml_depth: &mut xml_depth,
                            root: &mut root,
                            text_frames: &mut spread.text_frames,
                            page_item_count: &mut page_item_count,
                            image_count: &mut image_count,
                            path_point_count: &mut path_point_count,
                        };
                        let oval = parse_oval_contents(&mut reader, oval, &mut state)?;
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Oval(oval),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"GraphicLine" {
                        let line =
                            GraphicLine::from_xml_event(&e, &reader, options.max_attribute_bytes)?;
                        let mut state = NestedSpreadParseState {
                            options,
                            xml_depth: &mut xml_depth,
                            root: &mut root,
                            text_frames: &mut spread.text_frames,
                            page_item_count: &mut page_item_count,
                            image_count: &mut image_count,
                            path_point_count: &mut path_point_count,
                        };
                        let line = parse_graphic_line_contents(&mut reader, line, &mut state)?;
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::GraphicLine(line),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"Polygon" {
                        let polygon =
                            Polygon::from_xml_event(&e, &reader, options.max_attribute_bytes)?;
                        let mut state = NestedSpreadParseState {
                            options,
                            xml_depth: &mut xml_depth,
                            root: &mut root,
                            text_frames: &mut spread.text_frames,
                            page_item_count: &mut page_item_count,
                            image_count: &mut image_count,
                            path_point_count: &mut path_point_count,
                        };
                        let polygon = parse_polygon_contents(&mut reader, polygon, &mut state)?;
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Polygon(polygon),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"Group" {
                        let group =
                            Group::from_xml_event(&e, &reader, options.max_attribute_bytes)?;
                        let mut state = NestedSpreadParseState {
                            options,
                            xml_depth: &mut xml_depth,
                            root: &mut root,
                            text_frames: &mut spread.text_frames,
                            page_item_count: &mut page_item_count,
                            image_count: &mut image_count,
                            path_point_count: &mut path_point_count,
                        };
                        let group = parse_group_contents(&mut reader, group, &mut state)?;
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Group(group),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    }
                }
                Event::Empty(e) => {
                    validate_spread_root(&mut root_name, e.name().as_ref())?;
                    root.visit_empty(e.name().as_ref())?;
                    xml_depth.visit_empty()?;
                    if e.name().as_ref() == b"Spread" {
                        enter_empty_spread_element(&mut seen_spread)?;
                        spread.id =
                            optional_attr(&e, "Self", &reader, options.max_attribute_bytes)?;
                    } else if !in_spread {
                        continue;
                    } else if e.name().as_ref() == b"Page" {
                        push_page_limited(
                            &mut spread.pages,
                            Page::from_xml_event(&e, &reader, options.max_attribute_bytes)?,
                            options.max_pages,
                        )?;
                    } else if e.name().as_ref() == b"Guide" {
                        push_guide_limited(
                            &mut spread.guides,
                            Guide::from_xml_event(&e, &reader, options.max_attribute_bytes)?,
                            options.max_guides,
                        )?;
                    } else if e.name().as_ref() == b"TextFrame" {
                        push_text_frame_page_item_limited(
                            &mut spread.text_frames,
                            &mut spread.page_items,
                            TextFrame::from_xml_event(&e, &reader, options.max_attribute_bytes)?,
                            options,
                            &mut page_item_count,
                        )?;
                    } else if e.name().as_ref() == b"TextPath" {
                        push_text_frame_index_limited(
                            &mut spread.text_frames,
                            TextFrame::from_text_path_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?,
                            options.max_text_frames,
                        )?;
                    } else if e.name().as_ref() == b"Rectangle" {
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Rectangle(Rectangle::from_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"Oval" {
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Oval(Oval::from_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"GraphicLine" {
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::GraphicLine(GraphicLine::from_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"Polygon" {
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Polygon(Polygon::from_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    } else if e.name().as_ref() == b"Group" {
                        push_page_item_limited(
                            &mut spread.page_items,
                            PageItem::Group(Group::from_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?),
                            &mut page_item_count,
                            options.max_page_items,
                        )?;
                    }
                }
                Event::End(e) => {
                    xml_depth.exit()?;
                    root.exit(e.name().as_ref())?;
                    if e.name().as_ref() == b"Spread" {
                        in_spread = false;
                    }
                }
                Event::Text(e) => {
                    root.visit_text(e.as_ref())?;
                }
                Event::CData(_) => {
                    root.visit_cdata()?;
                }
                Event::GeneralRef(_) => {
                    root.visit_reference()?;
                    reject_xml_entity_reference()?;
                }
                Event::DocType(_) => reject_xml_doctype()?,
                Event::Eof => break,
                _ => {}
            }
        }

        root.finish()?;
        xml_depth.finish()?;
        if !seen_spread {
            return Err(IdmlError::InvalidElement {
                element: root_name
                    .map(|name| String::from_utf8_lossy(&name).into_owned())
                    .unwrap_or_default(),
                reason: "missing Spread element",
            });
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

    /// Validates spread-local serialization invariants without allocating XML.
    pub fn validate(&self) -> Result<()> {
        validate_spread(self)
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

/// Page geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Page `Self` identifier, when present.
    pub id: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
}

impl Page {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds("Page", &value))
                .transpose()?;

        Ok(Self {
            id,
            geometric_bounds,
        })
    }
}

/// Spread ruler guide metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    /// Guide `Self` identifier, when present.
    pub id: Option<String>,
    /// Layer ID from `ItemLayer`, when present.
    pub layer_id: Option<String>,
    /// Guide orientation, such as `Horizontal` or `Vertical`.
    pub orientation: Option<String>,
    /// Guide location in points, when represented directly.
    pub location: Option<Points>,
    /// Page index from `PageIndex`, when present.
    pub page_index: Option<i32>,
}

impl Guide {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let layer_id = optional_attr(event, "ItemLayer", reader, max_attribute_bytes)?;
        let orientation = optional_attr(event, "Orientation", reader, max_attribute_bytes)?;
        let location = optional_attr(event, "Location", reader, max_attribute_bytes)?
            .map(|value| parse_point_attribute("Guide", "Location", &value))
            .transpose()?;
        let page_index = optional_attr(event, "PageIndex", reader, max_attribute_bytes)?
            .map(|value| parse_i32_attribute("Guide", "PageIndex", &value))
            .transpose()?;

        Ok(Self {
            id,
            layer_id,
            orientation,
            location,
            page_index,
        })
    }
}

/// Text frame geometry and story relationship.
#[derive(Clone, Debug, PartialEq)]
pub struct TextFrame {
    /// Text frame `Self` identifier, when present.
    pub id: Option<String>,
    /// Layer ID from `ItemLayer`, when present.
    pub layer_id: Option<String>,
    /// Linked story ID from `ParentStory`, when present.
    pub parent_story: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
    /// Vector path geometry in local point coordinates, when present.
    pub path_geometry: Option<PathGeometry>,
}

/// Rectangle geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Rectangle {
    /// Rectangle `Self` identifier, when present.
    pub id: Option<String>,
    /// Layer ID from `ItemLayer`, when present.
    pub layer_id: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
    /// Vector path geometry in local point coordinates, when present.
    pub path_geometry: Option<PathGeometry>,
    /// Nested image graphics in XML order.
    pub images: Vec<Image>,
}

impl Rectangle {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let layer_id = optional_attr(event, "ItemLayer", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds("Rectangle", &value))
                .transpose()?;

        Ok(Self {
            id,
            layer_id,
            geometric_bounds,
            path_geometry: None,
            images: Vec::new(),
        })
    }
}

/// Vector path geometry for page items.
#[derive(Clone, Debug, PartialEq)]
pub struct PathGeometry {
    /// Geometry paths in XML order.
    pub paths: Vec<GeometryPath>,
}

/// One IDML geometry path.
#[derive(Clone, Debug, PartialEq)]
pub struct GeometryPath {
    /// Whether the path is open, from `PathOpen`, when present.
    pub path_open: Option<bool>,
    /// Path points in XML order.
    pub points: Vec<PathPoint>,
}

/// One IDML path point with anchor and Bezier handle coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct PathPoint {
    /// Anchor coordinate.
    pub anchor: Point,
    /// Left handle coordinate.
    pub left_direction: Point,
    /// Right handle coordinate.
    pub right_direction: Point,
}

/// Two-dimensional point in IDML point coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// X coordinate.
    pub x: Points,
    /// Y coordinate.
    pub y: Points,
}

impl Point {
    /// Creates a point coordinate.
    #[must_use]
    pub const fn new(x: Points, y: Points) -> Self {
        Self { x, y }
    }
}

impl GeometryPath {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let path_open = optional_attr(event, "PathOpen", reader, max_attribute_bytes)?
            .map(|value| parse_bool_attribute("GeometryPathType", "PathOpen", &value))
            .transpose()?;

        Ok(Self {
            path_open,
            points: Vec::new(),
        })
    }
}

impl PathPoint {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            anchor: required_point_pair_attr(
                event,
                "PathPointType",
                "Anchor",
                reader,
                max_attribute_bytes,
            )?,
            left_direction: required_point_pair_attr(
                event,
                "PathPointType",
                "LeftDirection",
                reader,
                max_attribute_bytes,
            )?,
            right_direction: required_point_pair_attr(
                event,
                "PathPointType",
                "RightDirection",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

/// Image graphic metadata nested inside a rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Image `Self` identifier, when present.
    pub id: Option<String>,
    /// Image type name, such as `$ID/JPEG`.
    pub image_type_name: Option<String>,
    /// Image transform matrix, preserved as an IDML string.
    pub item_transform: Option<String>,
    /// Graphic bounds in points, when represented directly.
    pub graphic_bounds: Option<Rect>,
    /// Linked resource records in XML order.
    pub links: Vec<ImageLink>,
}

impl Image {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let image_type_name = optional_attr(event, "ImageTypeName", reader, max_attribute_bytes)?;
        let item_transform = optional_attr(event, "ItemTransform", reader, max_attribute_bytes)?;

        Ok(Self {
            id,
            image_type_name,
            item_transform,
            graphic_bounds: None,
            links: Vec::new(),
        })
    }
}

/// Linked image resource metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageLink {
    /// Link `Self` identifier, when present.
    pub id: Option<String>,
    /// Link resource URI, when present.
    pub resource_uri: Option<String>,
    /// Link resource format, when present.
    pub resource_format: Option<String>,
}

impl ImageLink {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let resource_uri = optional_attr(event, "LinkResourceURI", reader, max_attribute_bytes)?;
        let resource_format =
            optional_attr(event, "LinkResourceFormat", reader, max_attribute_bytes)?;

        Ok(Self {
            id,
            resource_uri,
            resource_format,
        })
    }
}

/// Oval geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Oval {
    /// Oval `Self` identifier, when present.
    pub id: Option<String>,
    /// Layer ID from `ItemLayer`, when present.
    pub layer_id: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
    /// Vector path geometry in local point coordinates, when present.
    pub path_geometry: Option<PathGeometry>,
}

impl Oval {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let layer_id = optional_attr(event, "ItemLayer", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds("Oval", &value))
                .transpose()?;

        Ok(Self {
            id,
            layer_id,
            geometric_bounds,
            path_geometry: None,
        })
    }
}

/// Graphic line geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicLine {
    /// Graphic line `Self` identifier, when present.
    pub id: Option<String>,
    /// Layer ID from `ItemLayer`, when present.
    pub layer_id: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
    /// Vector path geometry in local point coordinates, when present.
    pub path_geometry: Option<PathGeometry>,
}

impl GraphicLine {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let layer_id = optional_attr(event, "ItemLayer", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds("GraphicLine", &value))
                .transpose()?;

        Ok(Self {
            id,
            layer_id,
            geometric_bounds,
            path_geometry: None,
        })
    }
}

/// Polygon geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Polygon {
    /// Polygon `Self` identifier, when present.
    pub id: Option<String>,
    /// Layer ID from `ItemLayer`, when present.
    pub layer_id: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
    /// Vector path geometry in local point coordinates, when present.
    pub path_geometry: Option<PathGeometry>,
}

impl Polygon {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let layer_id = optional_attr(event, "ItemLayer", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds("Polygon", &value))
                .transpose()?;

        Ok(Self {
            id,
            layer_id,
            geometric_bounds,
            path_geometry: None,
        })
    }
}

/// Grouped page items in spread z-order.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Group `Self` identifier, when present.
    pub id: Option<String>,
    /// Layer ID from `ItemLayer`, when present.
    pub layer_id: Option<String>,
    /// Group geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
    /// Child page items in XML order.
    pub page_items: Vec<PageItem>,
}

impl Group {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let layer_id = optional_attr(event, "ItemLayer", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds("Group", &value))
                .transpose()?;

        Ok(Self {
            id,
            layer_id,
            geometric_bounds,
            page_items: Vec::new(),
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
    /// An oval page item.
    Oval(Oval),
    /// A graphic line page item.
    GraphicLine(GraphicLine),
    /// A polygon page item.
    Polygon(Polygon),
    /// A grouped page item.
    Group(Group),
}

impl TextFrame {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Self::from_text_bearing_xml_event(event, reader, max_attribute_bytes, "TextFrame")
    }

    fn from_text_path_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Self::from_text_bearing_xml_event(event, reader, max_attribute_bytes, "TextPath")
    }

    fn from_text_bearing_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
        element: &'static str,
    ) -> Result<Self> {
        let id = optional_attr(event, "Self", reader, max_attribute_bytes)?;
        let layer_id = optional_attr(event, "ItemLayer", reader, max_attribute_bytes)?;
        let parent_story = optional_attr(event, "ParentStory", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds(element, &value))
                .transpose()?;

        Ok(Self {
            id,
            layer_id,
            parent_story,
            geometric_bounds,
            path_geometry: None,
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
    attribute: &'static str,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<Option<String>> {
    let mut value = None;
    for attr in event.attributes() {
        let attr = attr?;
        if attr.key.as_ref() == attribute.as_bytes() {
            if value.is_some() {
                return Err(IdmlError::InvalidAttribute {
                    element: String::from_utf8_lossy(event.name().as_ref()).into_owned(),
                    attribute,
                    reason: "duplicate attribute",
                });
            }
            enforce_spread_attribute_bytes(attr.value.len(), max_attribute_bytes)?;
            let decoded = attr
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())?
                .into_owned();
            validate_xml_attribute(
                String::from_utf8_lossy(event.name().as_ref()).into_owned(),
                attribute,
                &decoded,
            )?;
            value = Some(decoded);
        }
    }
    Ok(value)
}

fn enforce_spread_attribute_bytes(actual: usize, max_attribute_bytes: usize) -> Result<()> {
    if actual > max_attribute_bytes {
        return Err(IdmlError::LimitExceeded {
            what: "Spread XML attribute bytes",
            limit: max_attribute_bytes as u64,
            actual: actual as u64,
        });
    }
    Ok(())
}

fn validate_spread_root(root_name: &mut Option<Vec<u8>>, name: &[u8]) -> Result<()> {
    if root_name.is_some() {
        return Ok(());
    }

    if matches!(name, b"Spread" | b"idPkg:Spread") {
        *root_name = Some(name.to_vec());
        return Ok(());
    }

    Err(IdmlError::InvalidElement {
        element: String::from_utf8_lossy(name).into_owned(),
        reason: "expected Spread root element",
    })
}

fn enter_spread_element(seen_spread: &mut bool, in_spread: &mut bool) -> Result<()> {
    if *in_spread {
        return Err(IdmlError::InvalidElement {
            element: "Spread".to_owned(),
            reason: "nested Spread element",
        });
    }
    if *seen_spread {
        return Err(IdmlError::InvalidElement {
            element: "Spread".to_owned(),
            reason: "multiple Spread elements",
        });
    }

    *seen_spread = true;
    *in_spread = true;
    Ok(())
}

fn enter_empty_spread_element(seen_spread: &mut bool) -> Result<()> {
    if *seen_spread {
        return Err(IdmlError::InvalidElement {
            element: "Spread".to_owned(),
            reason: "multiple Spread elements",
        });
    }

    *seen_spread = true;
    Ok(())
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

fn parse_graphic_bounds(
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<Rect> {
    let left = required_point_attr(event, "GraphicBounds", "Left", reader, max_attribute_bytes)?;
    let top = required_point_attr(event, "GraphicBounds", "Top", reader, max_attribute_bytes)?;
    let right = required_point_attr(event, "GraphicBounds", "Right", reader, max_attribute_bytes)?;
    let bottom = required_point_attr(
        event,
        "GraphicBounds",
        "Bottom",
        reader,
        max_attribute_bytes,
    )?;
    let rect = Rect::new(top, left, bottom, right);
    validate_finite_bounds("GraphicBounds", rect)?;
    Ok(rect)
}

fn required_point_attr(
    event: &BytesStart<'_>,
    element: &'static str,
    attribute: &'static str,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<Points> {
    optional_attr(event, attribute, reader, max_attribute_bytes)?
        .ok_or_else(|| IdmlError::MissingAttribute {
            element: element.to_owned(),
            attribute,
        })
        .and_then(|value| parse_point_attribute(element, attribute, &value))
}

fn required_point_pair_attr(
    event: &BytesStart<'_>,
    element: &'static str,
    attribute: &'static str,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<Point> {
    optional_attr(event, attribute, reader, max_attribute_bytes)?
        .ok_or_else(|| IdmlError::MissingAttribute {
            element: element.to_owned(),
            attribute,
        })
        .and_then(|value| parse_point_pair_attribute(element, attribute, &value))
}

fn parse_point_pair_attribute(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<Point> {
    let values = value
        .split_whitespace()
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let [x, y] = values.as_slice() else {
        return Err(IdmlError::InvalidAttribute {
            element: element.to_owned(),
            attribute,
            reason: "expected two point coordinates",
        });
    };

    let point = Point::new(Points::new(*x), Points::new(*y));
    validate_finite_point(element, attribute, point)?;
    Ok(point)
}

fn parse_point_attribute(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<Points> {
    let value = value.parse::<f64>()?;
    let point = Points::new(value);
    if !point.as_f64().is_finite() {
        return Err(non_finite_point_error(element, attribute));
    }
    Ok(point)
}

fn parse_bool_attribute(
    element: &'static str,
    attribute: &'static str,
    value: &str,
) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(IdmlError::InvalidAttribute {
            element: element.to_owned(),
            attribute,
            reason: "expected boolean",
        }),
    }
}

fn parse_i32_attribute(element: &'static str, attribute: &'static str, value: &str) -> Result<i32> {
    value
        .parse::<i32>()
        .map_err(|_| IdmlError::InvalidAttribute {
            element: element.to_owned(),
            attribute,
            reason: "expected signed integer",
        })
}

fn push_page_limited(pages: &mut Vec<Page>, page: Page, max_pages: usize) -> Result<()> {
    let next_len = pages.len().checked_add(1).ok_or(IdmlError::LimitExceeded {
        what: "spread page count",
        limit: max_pages as u64,
        actual: u64::MAX,
    })?;

    if next_len > max_pages {
        return Err(IdmlError::LimitExceeded {
            what: "spread page count",
            limit: max_pages as u64,
            actual: next_len as u64,
        });
    }

    pages.push(page);
    Ok(())
}

fn push_guide_limited(guides: &mut Vec<Guide>, guide: Guide, max_guides: usize) -> Result<()> {
    let next_len = guides
        .len()
        .checked_add(1)
        .ok_or(IdmlError::LimitExceeded {
            what: "spread guide count",
            limit: max_guides as u64,
            actual: u64::MAX,
        })?;

    if next_len > max_guides {
        return Err(IdmlError::LimitExceeded {
            what: "spread guide count",
            limit: max_guides as u64,
            actual: next_len as u64,
        });
    }

    guides.push(guide);
    Ok(())
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

fn push_text_frame_page_item_limited(
    text_frames: &mut Vec<TextFrame>,
    page_items: &mut Vec<PageItem>,
    text_frame: TextFrame,
    options: SpreadParseOptions,
    page_item_count: &mut usize,
) -> Result<()> {
    push_text_frame_index_limited(text_frames, text_frame.clone(), options.max_text_frames)?;
    push_page_item_limited(
        page_items,
        PageItem::TextFrame(text_frame),
        page_item_count,
        options.max_page_items,
    )
}

fn push_page_item_limited(
    page_items: &mut Vec<PageItem>,
    page_item: PageItem,
    page_item_count: &mut usize,
    max_page_items: usize,
) -> Result<()> {
    let next_len = page_item_count
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

    *page_item_count = next_len;
    page_items.push(page_item);
    Ok(())
}

fn reserve_image_limited(image_count: &mut usize, max_images: usize) -> Result<()> {
    let next_len = image_count.checked_add(1).ok_or(IdmlError::LimitExceeded {
        what: "spread image count",
        limit: max_images as u64,
        actual: u64::MAX,
    })?;

    if next_len > max_images {
        return Err(IdmlError::LimitExceeded {
            what: "spread image count",
            limit: max_images as u64,
            actual: next_len as u64,
        });
    }

    *image_count = next_len;
    Ok(())
}

fn reserve_path_point_limited(path_point_count: &mut usize, max_path_points: usize) -> Result<()> {
    let next_len = path_point_count
        .checked_add(1)
        .ok_or(IdmlError::LimitExceeded {
            what: "spread path point count",
            limit: max_path_points as u64,
            actual: u64::MAX,
        })?;

    if next_len > max_path_points {
        return Err(IdmlError::LimitExceeded {
            what: "spread path point count",
            limit: max_path_points as u64,
            actual: next_len as u64,
        });
    }

    *path_point_count = next_len;
    Ok(())
}

fn parse_text_frame_contents(
    reader: &mut Reader<&[u8]>,
    mut frame: TextFrame,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<TextFrame> {
    parse_page_item_path_contents(reader, "TextFrame", state, &mut frame.path_geometry)?;
    Ok(frame)
}

fn parse_oval_contents(
    reader: &mut Reader<&[u8]>,
    mut oval: Oval,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<Oval> {
    parse_page_item_path_contents(reader, "Oval", state, &mut oval.path_geometry)?;
    Ok(oval)
}

fn parse_graphic_line_contents(
    reader: &mut Reader<&[u8]>,
    mut line: GraphicLine,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<GraphicLine> {
    parse_page_item_path_contents(reader, "GraphicLine", state, &mut line.path_geometry)?;
    Ok(line)
}

fn parse_polygon_contents(
    reader: &mut Reader<&[u8]>,
    mut polygon: Polygon,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<Polygon> {
    parse_page_item_path_contents(reader, "Polygon", state, &mut polygon.path_geometry)?;
    Ok(polygon)
}

fn parse_page_item_path_contents(
    reader: &mut Reader<&[u8]>,
    end_tag: &'static str,
    state: &mut NestedSpreadParseState<'_>,
    path_geometry: &mut Option<PathGeometry>,
) -> Result<()> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"PathGeometry" {
                    assign_path_geometry(
                        path_geometry,
                        parse_path_geometry_contents(reader, state)?,
                    );
                } else if e.name().as_ref() == b"TextPath" {
                    push_text_frame_index_limited(
                        state.text_frames,
                        TextFrame::from_text_path_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?,
                        state.options.max_text_frames,
                    )?;
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"PathGeometry" {
                    assign_path_geometry(path_geometry, PathGeometry { paths: Vec::new() });
                } else if e.name().as_ref() == b"TextPath" {
                    push_text_frame_index_limited(
                        state.text_frames,
                        TextFrame::from_text_path_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?,
                        state.options.max_text_frames,
                    )?;
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == end_tag.as_bytes() {
                    break;
                }
            }
            Event::Text(e) => {
                state.root.visit_text(e.as_ref())?;
            }
            Event::CData(_) => {
                state.root.visit_cdata()?;
            }
            Event::GeneralRef(_) => {
                state.root.visit_reference()?;
                reject_xml_entity_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => {
                return Err(IdmlError::InvalidElement {
                    element: end_tag.to_owned(),
                    reason: "unclosed page item element",
                });
            }
            _ => {}
        }
    }

    Ok(())
}

fn assign_path_geometry(path_geometry: &mut Option<PathGeometry>, value: PathGeometry) {
    if path_geometry.is_none() {
        *path_geometry = Some(value);
    }
}

fn parse_path_geometry_contents(
    reader: &mut Reader<&[u8]>,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<PathGeometry> {
    let mut geometry = PathGeometry { paths: Vec::new() };

    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"GeometryPathType" {
                    let path = GeometryPath::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    geometry
                        .paths
                        .push(parse_geometry_path_contents(reader, path, state)?);
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"GeometryPathType" {
                    geometry.paths.push(GeometryPath::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"PathGeometry" {
                    break;
                }
            }
            Event::Text(e) => {
                state.root.visit_text(e.as_ref())?;
            }
            Event::CData(_) => {
                state.root.visit_cdata()?;
            }
            Event::GeneralRef(_) => {
                state.root.visit_reference()?;
                reject_xml_entity_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => {
                return Err(IdmlError::InvalidElement {
                    element: "PathGeometry".to_owned(),
                    reason: "unclosed PathGeometry element",
                });
            }
            _ => {}
        }
    }

    Ok(geometry)
}

fn parse_geometry_path_contents(
    reader: &mut Reader<&[u8]>,
    mut path: GeometryPath,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<GeometryPath> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"PathPointType" {
                    reserve_path_point_limited(
                        state.path_point_count,
                        state.options.max_path_points,
                    )?;
                    path.points.push(PathPoint::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"PathPointType" {
                    reserve_path_point_limited(
                        state.path_point_count,
                        state.options.max_path_points,
                    )?;
                    path.points.push(PathPoint::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"GeometryPathType" {
                    break;
                }
            }
            Event::Text(e) => {
                state.root.visit_text(e.as_ref())?;
            }
            Event::CData(_) => {
                state.root.visit_cdata()?;
            }
            Event::GeneralRef(_) => {
                state.root.visit_reference()?;
                reject_xml_entity_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => {
                return Err(IdmlError::InvalidElement {
                    element: "GeometryPathType".to_owned(),
                    reason: "unclosed GeometryPathType element",
                });
            }
            _ => {}
        }
    }

    Ok(path)
}

fn parse_rectangle_contents(
    reader: &mut Reader<&[u8]>,
    mut rectangle: Rectangle,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<Rectangle> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"Image" {
                    reserve_image_limited(state.image_count, state.options.max_images)?;
                    let image =
                        Image::from_xml_event(&e, reader, state.options.max_attribute_bytes)?;
                    rectangle
                        .images
                        .push(parse_image_contents(reader, image, state)?);
                } else if e.name().as_ref() == b"PathGeometry" {
                    assign_path_geometry(
                        &mut rectangle.path_geometry,
                        parse_path_geometry_contents(reader, state)?,
                    );
                } else if e.name().as_ref() == b"GraphicBounds" {
                    rectangle
                        .images
                        .last_mut()
                        .map(|image| {
                            image.graphic_bounds = Some(parse_graphic_bounds(
                                &e,
                                reader,
                                state.options.max_attribute_bytes,
                            )?);
                            Ok::<(), IdmlError>(())
                        })
                        .transpose()?;
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"Image" {
                    reserve_image_limited(state.image_count, state.options.max_images)?;
                    rectangle.images.push(Image::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"PathGeometry" {
                    assign_path_geometry(
                        &mut rectangle.path_geometry,
                        PathGeometry { paths: Vec::new() },
                    );
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"Rectangle" {
                    break;
                }
            }
            Event::Text(e) => {
                state.root.visit_text(e.as_ref())?;
            }
            Event::CData(_) => {
                state.root.visit_cdata()?;
            }
            Event::GeneralRef(_) => {
                state.root.visit_reference()?;
                reject_xml_entity_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => {
                return Err(IdmlError::InvalidElement {
                    element: "Rectangle".to_owned(),
                    reason: "unclosed Rectangle element",
                });
            }
            _ => {}
        }
    }

    Ok(rectangle)
}

fn parse_image_contents(
    reader: &mut Reader<&[u8]>,
    mut image: Image,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<Image> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"Link" {
                    image.links.push(ImageLink::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"GraphicBounds" {
                    image.graphic_bounds = Some(parse_graphic_bounds(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"Link" {
                    image.links.push(ImageLink::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"GraphicBounds" {
                    image.graphic_bounds = Some(parse_graphic_bounds(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"Image" {
                    break;
                }
            }
            Event::Text(e) => {
                state.root.visit_text(e.as_ref())?;
            }
            Event::CData(_) => {
                state.root.visit_cdata()?;
            }
            Event::GeneralRef(_) => {
                state.root.visit_reference()?;
                reject_xml_entity_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => {
                return Err(IdmlError::InvalidElement {
                    element: "Image".to_owned(),
                    reason: "unclosed Image element",
                });
            }
            _ => {}
        }
    }

    Ok(image)
}

fn parse_group_contents(
    reader: &mut Reader<&[u8]>,
    mut group: Group,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<Group> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"TextFrame" {
                    let text_frame =
                        TextFrame::from_xml_event(&e, reader, state.options.max_attribute_bytes)?;
                    let text_frame = parse_text_frame_contents(reader, text_frame, state)?;
                    push_text_frame_page_item_limited(
                        state.text_frames,
                        &mut group.page_items,
                        text_frame,
                        state.options,
                        state.page_item_count,
                    )?;
                } else if e.name().as_ref() == b"TextPath" {
                    push_text_frame_index_limited(
                        state.text_frames,
                        TextFrame::from_text_path_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?,
                        state.options.max_text_frames,
                    )?;
                } else if e.name().as_ref() == b"Rectangle" {
                    let rectangle =
                        Rectangle::from_xml_event(&e, reader, state.options.max_attribute_bytes)?;
                    let rectangle = parse_rectangle_contents(reader, rectangle, state)?;
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Rectangle(rectangle),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"Oval" {
                    let oval = Oval::from_xml_event(&e, reader, state.options.max_attribute_bytes)?;
                    let oval = parse_oval_contents(reader, oval, state)?;
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Oval(oval),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"GraphicLine" {
                    let line =
                        GraphicLine::from_xml_event(&e, reader, state.options.max_attribute_bytes)?;
                    let line = parse_graphic_line_contents(reader, line, state)?;
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::GraphicLine(line),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"Polygon" {
                    let polygon =
                        Polygon::from_xml_event(&e, reader, state.options.max_attribute_bytes)?;
                    let polygon = parse_polygon_contents(reader, polygon, state)?;
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Polygon(polygon),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"Group" {
                    let child =
                        Group::from_xml_event(&e, reader, state.options.max_attribute_bytes)?;
                    let child = parse_group_contents(reader, child, state)?;
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Group(child),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"TextFrame" {
                    push_text_frame_page_item_limited(
                        state.text_frames,
                        &mut group.page_items,
                        TextFrame::from_xml_event(&e, reader, state.options.max_attribute_bytes)?,
                        state.options,
                        state.page_item_count,
                    )?;
                } else if e.name().as_ref() == b"TextPath" {
                    push_text_frame_index_limited(
                        state.text_frames,
                        TextFrame::from_text_path_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?,
                        state.options.max_text_frames,
                    )?;
                } else if e.name().as_ref() == b"Rectangle" {
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Rectangle(Rectangle::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"Oval" {
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Oval(Oval::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"GraphicLine" {
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::GraphicLine(GraphicLine::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"Polygon" {
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Polygon(Polygon::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                } else if e.name().as_ref() == b"Group" {
                    push_page_item_limited(
                        &mut group.page_items,
                        PageItem::Group(Group::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?),
                        state.page_item_count,
                        state.options.max_page_items,
                    )?;
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"Group" {
                    break;
                }
            }
            Event::Text(e) => {
                state.root.visit_text(e.as_ref())?;
            }
            Event::CData(_) => {
                state.root.visit_cdata()?;
            }
            Event::GeneralRef(_) => {
                state.root.visit_reference()?;
                reject_xml_entity_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => {
                return Err(IdmlError::InvalidElement {
                    element: "Group".to_owned(),
                    reason: "unclosed Group element",
                });
            }
            _ => {}
        }
    }

    Ok(group)
}

fn validate_spread(spread: &Spread) -> Result<()> {
    validate_page_items(spread)?;

    if let Some(id) = &spread.id {
        validate_xml_attribute("Spread", "Self", id)?;
    }
    for page in &spread.pages {
        validate_page(page)?;
    }
    for guide in &spread.guides {
        validate_guide(guide)?;
    }
    for frame in &spread.text_frames {
        validate_text_frame(frame)?;
    }
    for page_item in &spread.page_items {
        validate_page_item(page_item)?;
    }

    Ok(())
}

fn serialize_spread(spread: &Spread) -> Result<String> {
    validate_serialized_spread_counts(spread)?;
    validate_spread(spread)?;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Spread");
    push_optional_attr(&mut xml, "Self", spread.id.as_deref());
    xml.push_str(">\n");

    for page in &spread.pages {
        serialize_page(&mut xml, page)?;
    }
    for guide in &spread.guides {
        serialize_guide(&mut xml, guide)?;
    }

    if spread.page_items.is_empty() {
        for frame in &spread.text_frames {
            serialize_text_frame(&mut xml, frame)?;
        }
    } else {
        serialize_page_items_with_unrepresented_text_frames(&mut xml, spread)?;
    }

    xml.push_str("</Spread>\n");
    Ok(xml)
}

fn validate_serialized_spread_counts(spread: &Spread) -> Result<()> {
    enforce_serialized_spread_count(
        spread.pages.len(),
        DEFAULT_MAX_PAGES_PER_SPREAD,
        "spread page count",
    )?;
    enforce_serialized_spread_count(
        spread.guides.len(),
        DEFAULT_MAX_GUIDES_PER_SPREAD,
        "spread guide count",
    )?;
    enforce_serialized_spread_count(
        spread.text_frames.len(),
        DEFAULT_MAX_TEXT_FRAMES_PER_SPREAD,
        "spread text frame count",
    )?;
    enforce_serialized_spread_count(
        serialized_image_count(spread)?,
        DEFAULT_MAX_IMAGES_PER_SPREAD,
        "spread image count",
    )?;
    enforce_serialized_spread_count(
        serialized_path_point_count(spread)?,
        DEFAULT_MAX_PATH_POINTS_PER_SPREAD,
        "spread path point count",
    )?;
    enforce_serialized_spread_count(
        serialized_page_item_count(spread)?,
        DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD,
        "spread page item count",
    )?;
    Ok(())
}

fn serialized_path_point_count(spread: &Spread) -> Result<usize> {
    if spread.page_items.is_empty() {
        return spread
            .text_frames
            .iter()
            .try_fold(0usize, count_path_points_in_text_frame);
    }

    let count = spread
        .page_items
        .iter()
        .try_fold(0usize, count_path_points_in_page_item)?;
    unrepresented_text_frames(spread)?
        .into_iter()
        .try_fold(count, count_path_points_in_text_frame)
}

fn count_path_points_in_page_item(count: usize, page_item: &PageItem) -> Result<usize> {
    match page_item {
        PageItem::TextFrame(frame) => count_path_points_in_text_frame(count, frame),
        PageItem::Rectangle(rectangle) => {
            count_path_points_in_geometry(count, rectangle.path_geometry.as_ref())
        }
        PageItem::Oval(oval) => count_path_points_in_geometry(count, oval.path_geometry.as_ref()),
        PageItem::GraphicLine(line) => {
            count_path_points_in_geometry(count, line.path_geometry.as_ref())
        }
        PageItem::Polygon(polygon) => {
            count_path_points_in_geometry(count, polygon.path_geometry.as_ref())
        }
        PageItem::Group(group) => group
            .page_items
            .iter()
            .try_fold(count, count_path_points_in_page_item),
    }
}

fn count_path_points_in_text_frame(count: usize, frame: &TextFrame) -> Result<usize> {
    count_path_points_in_geometry(count, frame.path_geometry.as_ref())
}

fn count_path_points_in_geometry(count: usize, geometry: Option<&PathGeometry>) -> Result<usize> {
    let Some(geometry) = geometry else {
        return Ok(count);
    };
    geometry.paths.iter().try_fold(count, |count, path| {
        count
            .checked_add(path.points.len())
            .ok_or(IdmlError::LimitExceeded {
                what: "spread path point count",
                limit: DEFAULT_MAX_PATH_POINTS_PER_SPREAD as u64,
                actual: u64::MAX,
            })
    })
}

fn serialized_image_count(spread: &Spread) -> Result<usize> {
    spread
        .page_items
        .iter()
        .try_fold(0usize, count_images_in_page_item)
}

fn count_images_in_page_item(count: usize, page_item: &PageItem) -> Result<usize> {
    match page_item {
        PageItem::Rectangle(rectangle) => {
            count
                .checked_add(rectangle.images.len())
                .ok_or(IdmlError::LimitExceeded {
                    what: "spread image count",
                    limit: DEFAULT_MAX_IMAGES_PER_SPREAD as u64,
                    actual: u64::MAX,
                })
        }
        PageItem::Group(group) => group
            .page_items
            .iter()
            .try_fold(count, count_images_in_page_item),
        PageItem::TextFrame(_)
        | PageItem::Oval(_)
        | PageItem::GraphicLine(_)
        | PageItem::Polygon(_) => Ok(count),
    }
}

fn serialized_page_item_count(spread: &Spread) -> Result<usize> {
    if spread.page_items.is_empty() {
        return Ok(spread.text_frames.len());
    }

    let page_item_count = spread
        .page_items
        .iter()
        .try_fold(0usize, count_serialized_page_item)?;
    page_item_count
        .checked_add(unrepresented_text_frames(spread)?.len())
        .ok_or(IdmlError::LimitExceeded {
            what: "spread page item count",
            limit: DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD as u64,
            actual: u64::MAX,
        })
}

fn count_serialized_page_item(count: usize, page_item: &PageItem) -> Result<usize> {
    let count = count.checked_add(1).ok_or(IdmlError::LimitExceeded {
        what: "spread page item count",
        limit: DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD as u64,
        actual: u64::MAX,
    })?;

    match page_item {
        PageItem::TextFrame(_)
        | PageItem::Rectangle(_)
        | PageItem::Oval(_)
        | PageItem::GraphicLine(_)
        | PageItem::Polygon(_) => Ok(count),
        PageItem::Group(group) => group
            .page_items
            .iter()
            .try_fold(count, count_serialized_page_item),
    }
}

fn enforce_serialized_spread_count(actual: usize, limit: usize, what: &'static str) -> Result<()> {
    if actual > limit {
        return Err(IdmlError::LimitExceeded {
            what,
            limit: limit as u64,
            actual: actual as u64,
        });
    }
    Ok(())
}

fn serialize_page_items_with_unrepresented_text_frames(
    xml: &mut String,
    spread: &Spread,
) -> Result<()> {
    let mut text_frame_index = 0usize;

    for page_item in &spread.page_items {
        let mut item_text_frames = Vec::new();
        collect_text_frames_from_page_item(page_item, &mut item_text_frames);

        if let Some(first_item_frame) = item_text_frames.first() {
            loop {
                let Some(indexed_frame) = spread.text_frames.get(text_frame_index) else {
                    return Err(stale_page_items_error());
                };
                if indexed_frame == *first_item_frame {
                    break;
                }
                serialize_text_frame(xml, indexed_frame)?;
                text_frame_index += 1;
            }
        }

        serialize_page_item(xml, page_item, 2)?;

        for item_frame in item_text_frames {
            let Some(indexed_frame) = spread.text_frames.get(text_frame_index) else {
                return Err(stale_page_items_error());
            };
            if indexed_frame != item_frame {
                return Err(stale_page_items_error());
            }
            text_frame_index += 1;
        }
    }

    for frame in &spread.text_frames[text_frame_index..] {
        serialize_text_frame(xml, frame)?;
    }

    Ok(())
}

fn validate_page_items(spread: &Spread) -> Result<()> {
    if spread.page_items.is_empty() {
        return Ok(());
    }

    let page_item_text_frames = spread
        .page_items
        .iter()
        .fold(Vec::new(), |mut frames, item| {
            collect_text_frames_from_page_item(item, &mut frames);
            frames
        });

    let mut text_frame_index = 0usize;
    for from_items in page_item_text_frames {
        loop {
            let Some(from_index) = spread.text_frames.get(text_frame_index) else {
                return Err(stale_page_items_error());
            };
            text_frame_index += 1;
            if from_items == from_index {
                break;
            }
            if text_frame_index == spread.text_frames.len() {
                return Err(stale_page_items_error());
            }
        }
    }

    Ok(())
}

fn unrepresented_text_frames(spread: &Spread) -> Result<Vec<&TextFrame>> {
    let page_item_text_frames = spread
        .page_items
        .iter()
        .fold(Vec::new(), |mut frames, item| {
            collect_text_frames_from_page_item(item, &mut frames);
            frames
        });

    let mut represented = vec![false; spread.text_frames.len()];
    let mut text_frame_index = 0usize;
    for from_items in page_item_text_frames {
        loop {
            let Some(from_index) = spread.text_frames.get(text_frame_index) else {
                return Err(stale_page_items_error());
            };
            let current = text_frame_index;
            text_frame_index += 1;
            if from_items == from_index {
                represented[current] = true;
                break;
            }
            if text_frame_index == spread.text_frames.len() {
                return Err(stale_page_items_error());
            }
        }
    }

    Ok(spread
        .text_frames
        .iter()
        .zip(represented)
        .filter_map(|(frame, represented)| (!represented).then_some(frame))
        .collect())
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

fn compatible_pages(left: &Spread, right: &Spread) -> bool {
    left.pages.is_empty() || right.pages.is_empty() || left.pages == right.pages
}

fn compatible_guides(left: &Spread, right: &Spread) -> bool {
    left.guides.is_empty() || right.guides.is_empty() || left.guides == right.guides
}

fn collect_text_frames_from_page_item<'a>(
    page_item: &'a PageItem,
    frames: &mut Vec<&'a TextFrame>,
) {
    match page_item {
        PageItem::TextFrame(frame) => frames.push(frame),
        PageItem::Rectangle(_)
        | PageItem::Oval(_)
        | PageItem::GraphicLine(_)
        | PageItem::Polygon(_) => {}
        PageItem::Group(group) => {
            for child in &group.page_items {
                collect_text_frames_from_page_item(child, frames);
            }
        }
    }
}

fn validate_page(page: &Page) -> Result<()> {
    validate_optional_attr("Page", "Self", page.id.as_deref())?;
    if let Some(bounds) = page.geometric_bounds {
        validate_finite_bounds("Page", bounds)?;
    }
    Ok(())
}

fn validate_guide(guide: &Guide) -> Result<()> {
    validate_optional_attr("Guide", "Self", guide.id.as_deref())?;
    validate_optional_attr("Guide", "ItemLayer", guide.layer_id.as_deref())?;
    validate_optional_attr("Guide", "Orientation", guide.orientation.as_deref())?;
    if let Some(location) = guide.location
        && !location.as_f64().is_finite()
    {
        return Err(non_finite_point_error("Guide", "Location"));
    }
    Ok(())
}

fn validate_text_frame(frame: &TextFrame) -> Result<()> {
    validate_optional_attr("TextFrame", "Self", frame.id.as_deref())?;
    validate_optional_attr("TextFrame", "ItemLayer", frame.layer_id.as_deref())?;
    validate_optional_attr("TextFrame", "ParentStory", frame.parent_story.as_deref())?;
    if let Some(bounds) = frame.geometric_bounds {
        validate_finite_bounds("TextFrame", bounds)?;
    }
    if let Some(path_geometry) = &frame.path_geometry {
        validate_path_geometry(path_geometry)?;
    }
    Ok(())
}

fn validate_page_item(page_item: &PageItem) -> Result<()> {
    match page_item {
        PageItem::TextFrame(frame) => validate_text_frame(frame),
        PageItem::Rectangle(rectangle) => validate_rectangle(rectangle),
        PageItem::Oval(oval) => validate_oval(oval),
        PageItem::GraphicLine(line) => validate_graphic_line(line),
        PageItem::Polygon(polygon) => validate_polygon(polygon),
        PageItem::Group(group) => validate_group(group),
    }
}

fn validate_rectangle(rectangle: &Rectangle) -> Result<()> {
    validate_optional_attr("Rectangle", "Self", rectangle.id.as_deref())?;
    validate_optional_attr("Rectangle", "ItemLayer", rectangle.layer_id.as_deref())?;
    if let Some(bounds) = rectangle.geometric_bounds {
        validate_finite_bounds("Rectangle", bounds)?;
    }
    if let Some(path_geometry) = &rectangle.path_geometry {
        validate_path_geometry(path_geometry)?;
    }
    for image in &rectangle.images {
        validate_image(image)?;
    }
    Ok(())
}

fn validate_image(image: &Image) -> Result<()> {
    validate_optional_attr("Image", "Self", image.id.as_deref())?;
    validate_optional_attr("Image", "ImageTypeName", image.image_type_name.as_deref())?;
    validate_optional_attr("Image", "ItemTransform", image.item_transform.as_deref())?;
    if let Some(bounds) = image.graphic_bounds {
        validate_finite_bounds("GraphicBounds", bounds)?;
    }
    for link in &image.links {
        validate_image_link(link)?;
    }
    Ok(())
}

fn validate_image_link(link: &ImageLink) -> Result<()> {
    validate_optional_attr("Link", "Self", link.id.as_deref())?;
    validate_optional_attr("Link", "LinkResourceURI", link.resource_uri.as_deref())?;
    validate_optional_attr(
        "Link",
        "LinkResourceFormat",
        link.resource_format.as_deref(),
    )?;
    Ok(())
}

fn validate_oval(oval: &Oval) -> Result<()> {
    validate_optional_attr("Oval", "Self", oval.id.as_deref())?;
    validate_optional_attr("Oval", "ItemLayer", oval.layer_id.as_deref())?;
    if let Some(bounds) = oval.geometric_bounds {
        validate_finite_bounds("Oval", bounds)?;
    }
    if let Some(path_geometry) = &oval.path_geometry {
        validate_path_geometry(path_geometry)?;
    }
    Ok(())
}

fn validate_graphic_line(line: &GraphicLine) -> Result<()> {
    validate_optional_attr("GraphicLine", "Self", line.id.as_deref())?;
    validate_optional_attr("GraphicLine", "ItemLayer", line.layer_id.as_deref())?;
    if let Some(bounds) = line.geometric_bounds {
        validate_finite_bounds("GraphicLine", bounds)?;
    }
    if let Some(path_geometry) = &line.path_geometry {
        validate_path_geometry(path_geometry)?;
    }
    Ok(())
}

fn validate_polygon(polygon: &Polygon) -> Result<()> {
    validate_optional_attr("Polygon", "Self", polygon.id.as_deref())?;
    validate_optional_attr("Polygon", "ItemLayer", polygon.layer_id.as_deref())?;
    if let Some(bounds) = polygon.geometric_bounds {
        validate_finite_bounds("Polygon", bounds)?;
    }
    if let Some(path_geometry) = &polygon.path_geometry {
        validate_path_geometry(path_geometry)?;
    }
    Ok(())
}

fn validate_group(group: &Group) -> Result<()> {
    validate_optional_attr("Group", "Self", group.id.as_deref())?;
    validate_optional_attr("Group", "ItemLayer", group.layer_id.as_deref())?;
    if let Some(bounds) = group.geometric_bounds {
        validate_finite_bounds("Group", bounds)?;
    }
    for child in &group.page_items {
        validate_page_item(child)?;
    }
    Ok(())
}

fn validate_path_geometry(path_geometry: &PathGeometry) -> Result<()> {
    for path in &path_geometry.paths {
        validate_geometry_path(path)?;
    }
    Ok(())
}

fn validate_geometry_path(path: &GeometryPath) -> Result<()> {
    for point in &path.points {
        validate_path_point(point)?;
    }
    Ok(())
}

fn validate_path_point(point: &PathPoint) -> Result<()> {
    validate_finite_point("PathPointType", "Anchor", point.anchor)?;
    validate_finite_point("PathPointType", "LeftDirection", point.left_direction)?;
    validate_finite_point("PathPointType", "RightDirection", point.right_direction)
}

fn validate_optional_attr(
    element: &'static str,
    name: &'static str,
    value: Option<&str>,
) -> Result<()> {
    if let Some(value) = value {
        validate_xml_attribute(element, name, value)?;
    }
    Ok(())
}

fn serialize_page(xml: &mut String, page: &Page) -> Result<()> {
    xml.push_str("  <Page");
    if let Some(id) = &page.id {
        push_attr(xml, "Page", "Self", id)?;
    }
    if let Some(bounds) = page.geometric_bounds {
        let bounds = format_geometric_bounds("Page", bounds)?;
        push_attr(xml, "Page", "GeometricBounds", &bounds)?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_guide(xml: &mut String, guide: &Guide) -> Result<()> {
    xml.push_str("  <Guide");
    if let Some(id) = &guide.id {
        push_attr(xml, "Guide", "Self", id)?;
    }
    if let Some(layer_id) = &guide.layer_id {
        push_attr(xml, "Guide", "ItemLayer", layer_id)?;
    }
    if let Some(orientation) = &guide.orientation {
        push_attr(xml, "Guide", "Orientation", orientation)?;
    }
    if let Some(location) = guide.location {
        let location = format_point_attr("Guide", "Location", location)?;
        push_attr(xml, "Guide", "Location", &location)?;
    }
    if let Some(page_index) = guide.page_index {
        push_attr(xml, "Guide", "PageIndex", &page_index.to_string())?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_text_frame(xml: &mut String, frame: &TextFrame) -> Result<()> {
    serialize_text_frame_with_indent(xml, frame, 2)
}

fn serialize_page_item(xml: &mut String, page_item: &PageItem, indent: usize) -> Result<()> {
    match page_item {
        PageItem::TextFrame(frame) => serialize_text_frame_with_indent(xml, frame, indent),
        PageItem::Rectangle(rectangle) => serialize_rectangle_with_indent(xml, rectangle, indent),
        PageItem::Oval(oval) => serialize_oval_with_indent(xml, oval, indent),
        PageItem::GraphicLine(line) => serialize_graphic_line_with_indent(xml, line, indent),
        PageItem::Polygon(polygon) => serialize_polygon_with_indent(xml, polygon, indent),
        PageItem::Group(group) => serialize_group(xml, group, indent),
    }
}

fn serialize_text_frame_with_indent(
    xml: &mut String,
    frame: &TextFrame,
    indent: usize,
) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<TextFrame");
    if let Some(id) = &frame.id {
        push_attr(xml, "TextFrame", "Self", id)?;
    }
    if let Some(layer_id) = &frame.layer_id {
        push_attr(xml, "TextFrame", "ItemLayer", layer_id)?;
    }
    if let Some(parent_story) = &frame.parent_story {
        push_attr(xml, "TextFrame", "ParentStory", parent_story)?;
    }
    if let Some(bounds) = frame.geometric_bounds {
        let bounds = format_geometric_bounds("TextFrame", bounds)?;
        push_attr(xml, "TextFrame", "GeometricBounds", &bounds)?;
    }
    if let Some(path_geometry) = &frame.path_geometry {
        xml.push_str(">\n");
        serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        push_indent(xml, indent);
        xml.push_str("</TextFrame>\n");
        return Ok(());
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_rectangle_with_indent(
    xml: &mut String,
    rectangle: &Rectangle,
    indent: usize,
) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<Rectangle");
    if let Some(id) = &rectangle.id {
        push_attr(xml, "Rectangle", "Self", id)?;
    }
    if let Some(layer_id) = &rectangle.layer_id {
        push_attr(xml, "Rectangle", "ItemLayer", layer_id)?;
    }
    if let Some(bounds) = rectangle.geometric_bounds {
        let bounds = format_geometric_bounds("Rectangle", bounds)?;
        push_attr(xml, "Rectangle", "GeometricBounds", &bounds)?;
    }
    if rectangle.path_geometry.is_none() && rectangle.images.is_empty() {
        xml.push_str(" />\n");
        return Ok(());
    }
    xml.push_str(">\n");
    if let Some(path_geometry) = &rectangle.path_geometry {
        serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
    }
    for image in &rectangle.images {
        serialize_image(xml, image, indent + 2)?;
    }
    push_indent(xml, indent);
    xml.push_str("</Rectangle>\n");
    Ok(())
}

fn serialize_image(xml: &mut String, image: &Image, indent: usize) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<Image");
    if let Some(id) = &image.id {
        push_attr(xml, "Image", "Self", id)?;
    }
    if let Some(image_type_name) = &image.image_type_name {
        push_attr(xml, "Image", "ImageTypeName", image_type_name)?;
    }
    if let Some(item_transform) = &image.item_transform {
        push_attr(xml, "Image", "ItemTransform", item_transform)?;
    }
    if image.graphic_bounds.is_none() && image.links.is_empty() {
        xml.push_str(" />\n");
        return Ok(());
    }

    xml.push_str(">\n");
    if let Some(bounds) = image.graphic_bounds {
        push_indent(xml, indent + 2);
        xml.push_str("<Properties>\n");
        serialize_graphic_bounds(xml, bounds, indent + 4)?;
        push_indent(xml, indent + 2);
        xml.push_str("</Properties>\n");
    }
    for link in &image.links {
        serialize_image_link(xml, link, indent + 2)?;
    }
    push_indent(xml, indent);
    xml.push_str("</Image>\n");
    Ok(())
}

fn serialize_path_geometry_properties(
    xml: &mut String,
    path_geometry: &PathGeometry,
    indent: usize,
) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<Properties>\n");
    serialize_path_geometry(xml, path_geometry, indent + 2)?;
    push_indent(xml, indent);
    xml.push_str("</Properties>\n");
    Ok(())
}

fn serialize_path_geometry(
    xml: &mut String,
    path_geometry: &PathGeometry,
    indent: usize,
) -> Result<()> {
    validate_path_geometry(path_geometry)?;
    push_indent(xml, indent);
    if path_geometry.paths.is_empty() {
        xml.push_str("<PathGeometry />\n");
        return Ok(());
    }

    xml.push_str("<PathGeometry>\n");
    for path in &path_geometry.paths {
        serialize_geometry_path(xml, path, indent + 2)?;
    }
    push_indent(xml, indent);
    xml.push_str("</PathGeometry>\n");
    Ok(())
}

fn serialize_geometry_path(xml: &mut String, path: &GeometryPath, indent: usize) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<GeometryPathType");
    if let Some(path_open) = path.path_open {
        push_attr(
            xml,
            "GeometryPathType",
            "PathOpen",
            if path_open { "true" } else { "false" },
        )?;
    }
    if path.points.is_empty() {
        xml.push_str(" />\n");
        return Ok(());
    }

    xml.push_str(">\n");
    push_indent(xml, indent + 2);
    xml.push_str("<PathPointArray>\n");
    for point in &path.points {
        serialize_path_point(xml, point, indent + 4)?;
    }
    push_indent(xml, indent + 2);
    xml.push_str("</PathPointArray>\n");
    push_indent(xml, indent);
    xml.push_str("</GeometryPathType>\n");
    Ok(())
}

fn serialize_path_point(xml: &mut String, point: &PathPoint, indent: usize) -> Result<()> {
    validate_path_point(point)?;
    push_indent(xml, indent);
    xml.push_str("<PathPointType");
    push_attr(
        xml,
        "PathPointType",
        "Anchor",
        &format_point_pair(point.anchor),
    )?;
    push_attr(
        xml,
        "PathPointType",
        "LeftDirection",
        &format_point_pair(point.left_direction),
    )?;
    push_attr(
        xml,
        "PathPointType",
        "RightDirection",
        &format_point_pair(point.right_direction),
    )?;
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_graphic_bounds(xml: &mut String, bounds: Rect, indent: usize) -> Result<()> {
    validate_finite_bounds("GraphicBounds", bounds)?;
    push_indent(xml, indent);
    xml.push_str("<GraphicBounds");
    push_attr(xml, "GraphicBounds", "Left", &format_point(bounds.left))?;
    push_attr(xml, "GraphicBounds", "Top", &format_point(bounds.top))?;
    push_attr(xml, "GraphicBounds", "Right", &format_point(bounds.right))?;
    push_attr(xml, "GraphicBounds", "Bottom", &format_point(bounds.bottom))?;
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_image_link(xml: &mut String, link: &ImageLink, indent: usize) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<Link");
    if let Some(id) = &link.id {
        push_attr(xml, "Link", "Self", id)?;
    }
    if let Some(resource_uri) = &link.resource_uri {
        push_attr(xml, "Link", "LinkResourceURI", resource_uri)?;
    }
    if let Some(resource_format) = &link.resource_format {
        push_attr(xml, "Link", "LinkResourceFormat", resource_format)?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_oval_with_indent(xml: &mut String, oval: &Oval, indent: usize) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<Oval");
    if let Some(id) = &oval.id {
        push_attr(xml, "Oval", "Self", id)?;
    }
    if let Some(layer_id) = &oval.layer_id {
        push_attr(xml, "Oval", "ItemLayer", layer_id)?;
    }
    if let Some(bounds) = oval.geometric_bounds {
        let bounds = format_geometric_bounds("Oval", bounds)?;
        push_attr(xml, "Oval", "GeometricBounds", &bounds)?;
    }
    if let Some(path_geometry) = &oval.path_geometry {
        xml.push_str(">\n");
        serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        push_indent(xml, indent);
        xml.push_str("</Oval>\n");
        return Ok(());
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_graphic_line_with_indent(
    xml: &mut String,
    line: &GraphicLine,
    indent: usize,
) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<GraphicLine");
    if let Some(id) = &line.id {
        push_attr(xml, "GraphicLine", "Self", id)?;
    }
    if let Some(layer_id) = &line.layer_id {
        push_attr(xml, "GraphicLine", "ItemLayer", layer_id)?;
    }
    if let Some(bounds) = line.geometric_bounds {
        let bounds = format_geometric_bounds("GraphicLine", bounds)?;
        push_attr(xml, "GraphicLine", "GeometricBounds", &bounds)?;
    }
    if let Some(path_geometry) = &line.path_geometry {
        xml.push_str(">\n");
        serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        push_indent(xml, indent);
        xml.push_str("</GraphicLine>\n");
        return Ok(());
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_polygon_with_indent(xml: &mut String, polygon: &Polygon, indent: usize) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<Polygon");
    if let Some(id) = &polygon.id {
        push_attr(xml, "Polygon", "Self", id)?;
    }
    if let Some(layer_id) = &polygon.layer_id {
        push_attr(xml, "Polygon", "ItemLayer", layer_id)?;
    }
    if let Some(bounds) = polygon.geometric_bounds {
        let bounds = format_geometric_bounds("Polygon", bounds)?;
        push_attr(xml, "Polygon", "GeometricBounds", &bounds)?;
    }
    if let Some(path_geometry) = &polygon.path_geometry {
        xml.push_str(">\n");
        serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        push_indent(xml, indent);
        xml.push_str("</Polygon>\n");
        return Ok(());
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_group(xml: &mut String, group: &Group, indent: usize) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<Group");
    if let Some(id) = &group.id {
        push_attr(xml, "Group", "Self", id)?;
    }
    if let Some(layer_id) = &group.layer_id {
        push_attr(xml, "Group", "ItemLayer", layer_id)?;
    }
    if let Some(bounds) = group.geometric_bounds {
        let bounds = format_geometric_bounds("Group", bounds)?;
        push_attr(xml, "Group", "GeometricBounds", &bounds)?;
    }
    if group.page_items.is_empty() {
        xml.push_str(" />\n");
        return Ok(());
    }
    xml.push_str(">\n");
    for child in &group.page_items {
        serialize_page_item(xml, child, indent + 2)?;
    }
    push_indent(xml, indent);
    xml.push_str("</Group>\n");
    Ok(())
}

fn push_indent(xml: &mut String, spaces: usize) {
    for _ in 0..spaces {
        xml.push(' ');
    }
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

fn validate_finite_point(
    element: &'static str,
    attribute: &'static str,
    point: Point,
) -> Result<()> {
    if !point.x.as_f64().is_finite() || !point.y.as_f64().is_finite() {
        return Err(non_finite_point_error(element, attribute));
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

fn format_point_pair(point: Point) -> String {
    format!("{} {}", format_point(point.x), format_point(point.y))
}

fn format_point_attr(
    element: &'static str,
    attribute: &'static str,
    point: Points,
) -> Result<String> {
    if !point.as_f64().is_finite() {
        return Err(non_finite_point_error(element, attribute));
    }
    Ok(format_point(point))
}

fn non_finite_point_error(element: &'static str, attribute: &'static str) -> IdmlError {
    IdmlError::InvalidAttribute {
        element: element.to_owned(),
        attribute,
        reason: "coordinate must be finite",
    }
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

fn push_optional_attr(xml: &mut String, name: &'static str, value: Option<&str>) {
    if let Some(value) = value {
        xml.push(' ');
        xml.push_str(name);
        xml.push_str("=\"");
        xml.push_str(escape(value).as_ref());
        xml.push('"');
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GeometryPath, GraphicLine, Group, Guide, Image, Oval, Page, PageItem, PathGeometry,
        PathPoint, Point, Polygon, Rect, Rectangle, Spread, SpreadParseOptions, TextFrame,
    };
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
    fn parses_spread_from_idpkg_wrapper() {
        let xml = r#"<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Spread Self="u10">
    <Page Self="p1" GeometricBounds="0 0 600 800" />
    <TextFrame Self="tf1" ParentStory="u2" />
  </Spread>
</idPkg:Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.id.as_deref(), Some("u10"));
        assert_eq!(spread.pages.len(), 1);
        assert_eq!(spread.text_frames.len(), 1);
        assert_eq!(spread.text_frames[0].parent_story.as_deref(), Some("u2"));
    }

    #[test]
    fn indexes_text_paths_as_text_bearing_items() {
        let xml = r#"<Spread Self="u10">
  <Polygon Self="poly1">
    <TextPath Self="tp1" ParentStory="u2" />
  </Polygon>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 1);
        assert_eq!(spread.text_frames[0].id.as_deref(), Some("tp1"));
        assert_eq!(spread.text_frames[0].parent_story.as_deref(), Some("u2"));
        assert!(spread.text_frames[0].geometric_bounds.is_none());
        assert_eq!(spread.page_items.len(), 1);
        assert!(matches!(
            &spread.page_items[0],
            PageItem::Polygon(Polygon { id, .. }) if id.as_deref() == Some("poly1")
        ));
    }

    #[test]
    fn validates_and_serializes_text_paths_not_represented_as_page_items() {
        let xml = r#"<Spread Self="u10">
  <TextFrame Self="tf1" ParentStory="u1" />
  <Polygon Self="poly1">
    <TextPath Self="tp1" ParentStory="u2" />
  </Polygon>
  <TextFrame Self="tf2" ParentStory="u3" />
  <Rectangle Self="r1" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 3);
        assert_eq!(spread.page_items.len(), 4);
        spread.validate().unwrap();

        let serialized = spread.to_xml().unwrap();
        let parsed = Spread::from_xml(&serialized).unwrap();
        let story_ids = parsed
            .text_frames
            .iter()
            .filter_map(|frame| frame.parent_story.as_deref())
            .collect::<Vec<_>>();

        assert_eq!(story_ids, ["u1", "u2", "u3"]);
    }

    #[test]
    fn rejects_idpkg_spread_wrapper_without_spread_model() {
        let err = Spread::from_xml(
            r#"<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" />"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "missing Spread element",
            } if element == "idPkg:Spread"
        ));
    }

    #[test]
    fn parses_empty_spread_root_id() {
        let spread = Spread::from_xml(r#"<Spread Self="u10" />"#).unwrap();

        assert_eq!(spread.id.as_deref(), Some("u10"));
        assert!(spread.pages.is_empty());
        assert!(spread.text_frames.is_empty());
    }

    #[test]
    fn rejects_wrong_spread_root_element() {
        let err = Spread::from_xml("<Wrapper><TextFrame Self=\"tf1\" /></Wrapper>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "expected Spread root element",
            } if element == "Wrapper"
        ));
    }

    #[test]
    fn rejects_trailing_spread_text_after_root() {
        let err = Spread::from_xml("<Spread />tail").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "non-whitespace character data outside root element",
            } if element == "#text"
        ));
    }

    #[test]
    fn rejects_trailing_spread_text_after_grouped_root() {
        let err = Spread::from_xml("<Spread><Group /></Spread>tail").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "non-whitespace character data outside root element",
            } if element == "#text"
        ));
    }

    #[test]
    fn rejects_entity_references_inside_spread_body() {
        let err = Spread::from_xml("<Spread>&amp;</Spread>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "entity references are not supported here",
            } if element == "#reference"
        ));
    }

    #[test]
    fn rejects_entity_references_inside_group_body() {
        let err = Spread::from_xml("<Spread><Group>&amp;</Group></Spread>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "entity references are not supported here",
            } if element == "#reference"
        ));
    }

    #[test]
    fn rejects_unclosed_group_before_eof() {
        let err = Spread::from_xml("<Spread><Group>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "unclosed Group element",
            } if element == "Group"
        ));
    }

    #[test]
    fn parses_pages_with_bounds_in_order() {
        let xml = r#"<Spread Self="u10">
  <Page Self="p1" GeometricBounds="0 0 792 612" />
  <Page Self="p2" GeometricBounds="0 612 792 1224" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.pages.len(), 2);
        assert_eq!(spread.pages[0].id.as_deref(), Some("p1"));
        assert_eq!(spread.pages[1].id.as_deref(), Some("p2"));
        assert_eq!(
            spread.pages[1].geometric_bounds.unwrap().left.as_f64(),
            612.0
        );
    }

    #[test]
    fn parses_guides_with_location_and_page_index() {
        let xml = r#"<Spread Self="u10">
  <Page Self="p1" GeometricBounds="0 0 792 612" />
  <Guide Self="g1" ItemLayer="layer1" Orientation="Horizontal" Location="51.25" PageIndex="-1">
    <Properties><GuideColor type="enumeration">LightBlue</GuideColor></Properties>
  </Guide>
  <TextFrame Self="tf1" ParentStory="u2" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.guides.len(), 1);
        assert_eq!(spread.guides[0].id.as_deref(), Some("g1"));
        assert_eq!(spread.guides[0].layer_id.as_deref(), Some("layer1"));
        assert_eq!(spread.guides[0].orientation.as_deref(), Some("Horizontal"));
        assert_eq!(spread.guides[0].location.unwrap().as_f64(), 51.25);
        assert_eq!(spread.guides[0].page_index, Some(-1));

        let serialized = spread.to_xml().unwrap();
        let page = serialized.find("<Page").unwrap();
        let guide = serialized.find("<Guide").unwrap();
        let frame = serialized.find("<TextFrame").unwrap();
        let parsed = Spread::from_xml(&serialized).unwrap();

        assert!(page < guide);
        assert!(guide < frame);
        assert_eq!(parsed.guides, spread.guides);
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
                geometric_bounds: Some(_),
                ..
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
                geometric_bounds: None,
                ..
            }) if id.as_deref() == Some("r2")
        ));
    }

    #[test]
    fn parses_rectangle_images_without_storing_embedded_contents() {
        let xml = r#"<Spread Self="u10">
  <Rectangle Self="r1" ItemLayer="layer1">
    <Image Self="img1" ImageTypeName="$ID/JPEG" ItemTransform="1 0 0 1 10 20">
      <Properties>
        <GraphicBounds Left="0" Top="1" Right="640" Bottom="481" />
        <Contents><![CDATA[large-payload-is-not-captured]]></Contents>
      </Properties>
      <Link Self="link1" LinkResourceURI="file:/tmp/image.jpg" LinkResourceFormat="$ID/JPEG" />
    </Image>
  </Rectangle>
  <TextFrame Self="tf1" ParentStory="u2" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        let PageItem::Rectangle(rectangle) = &spread.page_items[0] else {
            panic!("expected rectangle page item");
        };
        assert_eq!(rectangle.images.len(), 1);
        assert_eq!(rectangle.images[0].id.as_deref(), Some("img1"));
        assert_eq!(
            rectangle.images[0].image_type_name.as_deref(),
            Some("$ID/JPEG")
        );
        assert_eq!(
            rectangle.images[0].item_transform.as_deref(),
            Some("1 0 0 1 10 20")
        );
        assert_eq!(
            rectangle.images[0].graphic_bounds.unwrap(),
            Rect::new(
                Points::new(1.0),
                Points::new(0.0),
                Points::new(481.0),
                Points::new(640.0),
            )
        );
        assert_eq!(rectangle.images[0].links.len(), 1);
        assert_eq!(rectangle.images[0].links[0].id.as_deref(), Some("link1"));
        assert_eq!(
            rectangle.images[0].links[0].resource_uri.as_deref(),
            Some("file:/tmp/image.jpg")
        );

        let serialized = spread.to_xml().unwrap();
        let rectangle = serialized.find("<Rectangle").unwrap();
        let image = serialized.find("<Image").unwrap();
        let link = serialized.find("<Link").unwrap();
        let frame = serialized.find("<TextFrame").unwrap();
        let parsed = Spread::from_xml(&serialized).unwrap();

        assert!(rectangle < image);
        assert!(image < link);
        assert!(link < frame);
        assert!(!serialized.contains("large-payload-is-not-captured"));
        assert_eq!(parsed, spread);
    }

    #[test]
    fn parses_path_geometry_for_page_items_and_text_frames() {
        let xml = r#"<Spread Self="u10">
  <TextFrame Self="tf1" ParentStory="u2">
    <Properties>
      <PathGeometry>
        <GeometryPathType PathOpen="false">
          <PathPointArray>
            <PathPointType Anchor="0 1" LeftDirection="2 3" RightDirection="4 5" />
          </PathPointArray>
        </GeometryPathType>
      </PathGeometry>
    </Properties>
  </TextFrame>
  <Polygon Self="poly1">
    <Properties>
      <PathGeometry>
        <GeometryPathType PathOpen="true">
          <PathPointArray>
            <PathPointType Anchor="10 11" LeftDirection="12 13" RightDirection="14 15" />
          </PathPointArray>
        </GeometryPathType>
      </PathGeometry>
    </Properties>
  </Polygon>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 1);
        let frame_geometry = spread.text_frames[0].path_geometry.as_ref().unwrap();
        assert_eq!(frame_geometry.paths[0].path_open, Some(false));
        assert_eq!(
            frame_geometry.paths[0].points[0].anchor,
            Point::new(Points::new(0.0), Points::new(1.0))
        );

        let PageItem::Polygon(polygon) = &spread.page_items[1] else {
            panic!("expected polygon page item");
        };
        let polygon_geometry = polygon.path_geometry.as_ref().unwrap();
        assert_eq!(polygon_geometry.paths[0].path_open, Some(true));
        assert_eq!(
            polygon_geometry.paths[0].points[0].right_direction,
            Point::new(Points::new(14.0), Points::new(15.0))
        );

        let serialized = spread.to_xml().unwrap();
        let parsed = Spread::from_xml(&serialized).unwrap();
        assert_eq!(parsed, spread);
    }

    #[test]
    fn parses_ovals_and_preserves_page_item_order() {
        let xml = r#"<Spread Self="u10">
  <Oval Self="o1" ItemLayer="layer1" GeometricBounds="0 0 72 72" />
  <TextFrame Self="tf1" ParentStory="u2" GeometricBounds="72 0 144 72" />
  <Oval Self="o2" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 1);
        assert_eq!(spread.page_items.len(), 3);
        assert!(matches!(
            &spread.page_items[0],
            PageItem::Oval(Oval {
                id,
                layer_id,
                geometric_bounds: Some(_),
                ..
            }) if id.as_deref() == Some("o1") && layer_id.as_deref() == Some("layer1")
        ));
        assert!(matches!(
            &spread.page_items[1],
            PageItem::TextFrame(TextFrame { id, .. }) if id.as_deref() == Some("tf1")
        ));
        assert!(matches!(
            &spread.page_items[2],
            PageItem::Oval(Oval {
                id,
                geometric_bounds: None,
                ..
            }) if id.as_deref() == Some("o2")
        ));
    }

    #[test]
    fn parses_graphic_lines_and_preserves_page_item_order() {
        let xml = r#"<Spread Self="u10">
  <GraphicLine Self="l1" ItemLayer="layer1" GeometricBounds="0 0 72 72" />
  <TextFrame Self="tf1" ParentStory="u2" GeometricBounds="72 0 144 72" />
  <GraphicLine Self="l2" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 1);
        assert_eq!(spread.page_items.len(), 3);
        assert!(matches!(
            &spread.page_items[0],
            PageItem::GraphicLine(GraphicLine {
                id,
                layer_id,
                geometric_bounds: Some(_),
                ..
            }) if id.as_deref() == Some("l1") && layer_id.as_deref() == Some("layer1")
        ));
        assert!(matches!(
            &spread.page_items[1],
            PageItem::TextFrame(TextFrame { id, .. }) if id.as_deref() == Some("tf1")
        ));
        assert!(matches!(
            &spread.page_items[2],
            PageItem::GraphicLine(GraphicLine {
                id,
                geometric_bounds: None,
                ..
            }) if id.as_deref() == Some("l2")
        ));
    }

    #[test]
    fn parses_polygons_and_preserves_text_path_indexing() {
        let xml = r#"<Spread Self="u10">
  <Polygon Self="poly1" ItemLayer="layer1" GeometricBounds="0 0 72 72">
    <PathGeometry>
      <GeometryPathType>
        <PathPointArray>
          <PathPointType Anchor="0 0" LeftDirection="0 0" RightDirection="0 0" />
        </PathPointArray>
      </GeometryPathType>
    </PathGeometry>
    <TextPath Self="tp1" ParentStory="u2" />
  </Polygon>
  <TextFrame Self="tf1" ParentStory="u3" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 2);
        assert_eq!(spread.text_frames[0].id.as_deref(), Some("tp1"));
        assert_eq!(spread.text_frames[0].parent_story.as_deref(), Some("u2"));
        assert_eq!(spread.text_frames[1].id.as_deref(), Some("tf1"));
        assert_eq!(spread.page_items.len(), 2);
        assert!(matches!(
            &spread.page_items[0],
            PageItem::Polygon(Polygon {
                id,
                layer_id,
                geometric_bounds: Some(_),
                ..
            }) if id.as_deref() == Some("poly1") && layer_id.as_deref() == Some("layer1")
        ));

        let serialized = spread.to_xml().unwrap();
        let polygon = serialized.find("<Polygon").unwrap();
        let text_path_as_frame = serialized.find("Self=\"tp1\"").unwrap();

        assert!(polygon < text_path_as_frame);
    }

    #[test]
    fn parses_groups_recursively_and_indexes_nested_text_frames() {
        let xml = r#"<Spread Self="u10">
  <Rectangle Self="r1" />
  <Group Self="g1" GeometricBounds="0 0 200 200">
    <TextFrame Self="tf1" ParentStory="u2" GeometricBounds="10 10 20 20" />
    <Group Self="g2">
      <Rectangle Self="r2" />
      <TextFrame Self="tf2" ParentStory="u3" />
    </Group>
  </Group>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames.len(), 2);
        assert_eq!(spread.text_frames[0].id.as_deref(), Some("tf1"));
        assert_eq!(spread.text_frames[1].id.as_deref(), Some("tf2"));
        assert_eq!(spread.page_items.len(), 2);
        assert!(matches!(
            &spread.page_items[1],
            PageItem::Group(Group {
                id,
                geometric_bounds: Some(_),
                page_items,
                ..
            }) if id.as_deref() == Some("g1") && page_items.len() == 2
        ));

        let PageItem::Group(group) = &spread.page_items[1] else {
            panic!("expected group page item");
        };
        assert!(matches!(
            &group.page_items[0],
            PageItem::TextFrame(TextFrame { id, .. }) if id.as_deref() == Some("tf1")
        ));
        assert!(matches!(
            &group.page_items[1],
            PageItem::Group(Group { id, page_items, .. })
                if id.as_deref() == Some("g2") && page_items.len() == 2
        ));
    }

    #[test]
    fn parses_item_layer_references_on_page_items() {
        let xml = r#"<Spread Self="u10">
  <TextFrame Self="tf1" ItemLayer="layer1" ParentStory="u2" />
  <Rectangle Self="r1" ItemLayer="layer2" />
  <Group Self="g1" ItemLayer="layer3">
    <TextFrame Self="tf2" ItemLayer="layer3" ParentStory="u3" />
  </Group>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.text_frames[0].layer_id.as_deref(), Some("layer1"));
        assert!(matches!(
            &spread.page_items[1],
            PageItem::Rectangle(Rectangle { layer_id, .. })
                if layer_id.as_deref() == Some("layer2")
        ));
        let PageItem::Group(group) = &spread.page_items[2] else {
            panic!("expected group page item");
        };
        assert_eq!(group.layer_id.as_deref(), Some("layer3"));
        assert!(matches!(
            &group.page_items[0],
            PageItem::TextFrame(TextFrame { layer_id, .. })
                if layer_id.as_deref() == Some("layer3")
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
    fn rejects_invalid_guide_attributes() {
        let err = Spread::from_xml(r#"<Spread><Guide Location="NaN" /></Spread>"#).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Location",
                reason: "coordinate must be finite",
            } if element == "Guide"
        ));

        let err =
            Spread::from_xml(r#"<Spread><Guide PageIndex="page-one" /></Spread>"#).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "PageIndex",
                reason: "expected signed integer",
            } if element == "Guide"
        ));
    }

    #[test]
    fn rejects_invalid_image_graphic_bounds() {
        let err = Spread::from_xml(
            r#"<Spread><Rectangle><Image><Properties><GraphicBounds Left="0" Top="NaN" Right="1" Bottom="1" /></Properties></Image></Rectangle></Spread>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Top",
                reason: "coordinate must be finite",
            } if element == "GraphicBounds"
        ));
    }

    #[test]
    fn rejects_invalid_path_point_coordinates() {
        let err = Spread::from_xml(
            r#"<Spread><Polygon><Properties><PathGeometry><GeometryPathType><PathPointArray><PathPointType Anchor="0 NaN" LeftDirection="0 0" RightDirection="0 0" /></PathPointArray></GeometryPathType></PathGeometry></Properties></Polygon></Spread>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Anchor",
                reason: "coordinate must be finite",
            } if element == "PathPointType"
        ));
    }

    #[test]
    fn rejects_invalid_path_open_boolean() {
        let err = Spread::from_xml(
            r#"<Spread><Polygon><Properties><PathGeometry><GeometryPathType PathOpen="maybe" /></PathGeometry></Properties></Polygon></Spread>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "PathOpen",
                reason: "expected boolean",
            } if element == "GeometryPathType"
        ));
    }

    #[test]
    fn rejects_duplicate_text_frame_parent_story_attributes() {
        let err =
            Spread::from_xml(r#"<Spread><TextFrame ParentStory="u1" ParentStory="u2" /></Spread>"#)
                .unwrap_err();

        assert!(
            matches!(
                &err,
                IdmlError::InvalidAttribute {
                    element,
                    attribute: "ParentStory",
                    reason: "duplicate attribute",
                } if element == "TextFrame"
            ) || matches!(&err, IdmlError::XmlAttribute(_))
        );
    }

    #[test]
    fn rejects_xml_forbidden_parent_story_attribute_on_parse() {
        let err =
            Spread::from_xml("<Spread><TextFrame ParentStory=\"bad\0id\" /></Spread>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "ParentStory",
                reason: "contains an XML-forbidden character",
            } if element == "TextFrame"
        ));
    }

    #[test]
    fn rejects_xml_forbidden_empty_spread_id_attribute_on_parse() {
        let err = Spread::from_xml("<Spread Self=\"bad\0id\" />").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Spread"
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
    fn enforces_page_count_limit() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><Page Self="p1" /><Page Self="p2" /></Spread>"#,
            SpreadParseOptions {
                max_pages: 1,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread page count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn enforces_guide_count_limit() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><Guide Self="g1" /><Guide Self="g2" /></Spread>"#,
            SpreadParseOptions {
                max_guides: 1,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread guide count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn enforces_image_count_limit() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><Rectangle><Image Self="img1" /><Image Self="img2" /></Rectangle></Spread>"#,
            SpreadParseOptions {
                max_images: 1,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread image count",
                limit: 1,
                actual: 2,
            }
        ));
    }

    #[test]
    fn enforces_path_point_count_limit() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><Polygon><Properties><PathGeometry><GeometryPathType><PathPointArray><PathPointType Anchor="0 0" LeftDirection="0 0" RightDirection="0 0" /><PathPointType Anchor="1 1" LeftDirection="1 1" RightDirection="1 1" /></PathPointArray></GeometryPathType></PathGeometry></Properties></Polygon></Spread>"#,
            SpreadParseOptions {
                max_path_points: 1,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread path point count",
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
    fn enforces_page_item_count_limit_inside_groups() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><Group Self="g1"><Rectangle Self="r1" /><TextFrame Self="tf1" /></Group></Spread>"#,
            SpreadParseOptions {
                max_page_items: 2,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread page item count",
                limit: 2,
                actual: 3,
            }
        ));
    }

    #[test]
    fn enforces_attribute_byte_limit_before_decoding() {
        let err = Spread::from_xml_with_options(
            r#"<Spread><TextFrame Self="tf1234" /></Spread>"#,
            SpreadParseOptions {
                max_attribute_bytes: 5,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "Spread XML attribute bytes",
                limit: 5,
                actual: 6,
            }
        ));
    }

    #[test]
    fn enforces_xml_byte_limit() {
        let err = Spread::from_xml_with_options(
            "<Spread/>",
            SpreadParseOptions {
                max_xml_bytes: 4,
                ..SpreadParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "Spread XML bytes",
                limit: 4,
                actual: 9,
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
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![TextFrame {
                id: Some("tf&1".to_owned()),
                layer_id: None,
                parent_story: Some("u2".to_owned()),
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(72.5),
                    Points::new(144.0),
                    Points::new(216.25),
                )),
                path_geometry: None,
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
    fn serializer_writes_pages_before_page_items() {
        let text_frame = TextFrame {
            id: Some("tf1".to_owned()),
            layer_id: None,
            parent_story: Some("u2".to_owned()),
            geometric_bounds: None,
            path_geometry: None,
        };
        let spread = Spread {
            id: Some("u10".to_owned()),
            pages: vec![Page {
                id: Some("p1".to_owned()),
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(0.0),
                    Points::new(792.0),
                    Points::new(612.0),
                )),
            }],
            guides: Vec::new(),
            text_frames: vec![text_frame.clone()],
            page_items: vec![PageItem::TextFrame(text_frame)],
        };

        let xml = spread.to_xml().unwrap();
        let page = xml.find("<Page").unwrap();
        let frame = xml.find("<TextFrame").unwrap();
        let parsed = Spread::from_xml(&xml).unwrap();

        assert!(page < frame);
        assert_eq!(parsed, spread);
        assert_eq!(parsed.pages, spread.pages);
    }

    #[test]
    fn serializer_preserves_page_item_order() {
        let text_frame = TextFrame {
            id: Some("tf1".to_owned()),
            layer_id: None,
            parent_story: Some("u2".to_owned()),
            geometric_bounds: Some(Rect::new(
                Points::new(72.0),
                Points::new(0.0),
                Points::new(144.0),
                Points::new(72.0),
            )),
            path_geometry: None,
        };
        let spread = Spread {
            id: Some("u10".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![text_frame.clone()],
            page_items: vec![
                PageItem::Rectangle(Rectangle {
                    id: Some("r1".to_owned()),
                    layer_id: None,
                    geometric_bounds: Some(Rect::new(
                        Points::new(0.0),
                        Points::new(0.0),
                        Points::new(72.0),
                        Points::new(72.0),
                    )),
                    path_geometry: None,
                    images: Vec::new(),
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
    fn serializes_group_page_items_and_round_trips() {
        let text_frame = TextFrame {
            id: Some("tf1".to_owned()),
            layer_id: None,
            parent_story: Some("u2".to_owned()),
            geometric_bounds: None,
            path_geometry: None,
        };
        let spread = Spread {
            id: Some("u10".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![text_frame.clone()],
            page_items: vec![PageItem::Group(Group {
                id: Some("g1".to_owned()),
                layer_id: None,
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(0.0),
                    Points::new(200.0),
                    Points::new(200.0),
                )),
                page_items: vec![
                    PageItem::Rectangle(Rectangle {
                        id: Some("r1".to_owned()),
                        layer_id: None,
                        geometric_bounds: None,
                        path_geometry: None,
                        images: Vec::new(),
                    }),
                    PageItem::TextFrame(text_frame),
                ],
            })],
        };

        let xml = spread.to_xml().unwrap();
        let group = xml.find("<Group").unwrap();
        let rectangle = xml.find("<Rectangle").unwrap();
        let frame = xml.find("<TextFrame").unwrap();
        let parsed = Spread::from_xml(&xml).unwrap();

        assert!(group < rectangle);
        assert!(rectangle < frame);
        assert_eq!(parsed, spread);
    }

    #[test]
    fn serializes_item_layer_references_and_round_trips() {
        let text_frame = TextFrame {
            id: Some("tf1".to_owned()),
            layer_id: Some("layer1".to_owned()),
            parent_story: Some("u2".to_owned()),
            geometric_bounds: None,
            path_geometry: None,
        };
        let spread = Spread {
            id: Some("u10".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![text_frame.clone()],
            page_items: vec![
                PageItem::TextFrame(text_frame),
                PageItem::Rectangle(Rectangle {
                    id: Some("r1".to_owned()),
                    layer_id: Some("layer2".to_owned()),
                    geometric_bounds: None,
                    path_geometry: None,
                    images: Vec::new(),
                }),
                PageItem::Oval(Oval {
                    id: Some("o1".to_owned()),
                    layer_id: Some("layer4".to_owned()),
                    geometric_bounds: Some(Rect::new(
                        Points::new(0.0),
                        Points::new(0.0),
                        Points::new(20.0),
                        Points::new(20.0),
                    )),
                    path_geometry: None,
                }),
                PageItem::GraphicLine(GraphicLine {
                    id: Some("l1".to_owned()),
                    layer_id: Some("layer5".to_owned()),
                    geometric_bounds: Some(Rect::new(
                        Points::new(20.0),
                        Points::new(20.0),
                        Points::new(40.0),
                        Points::new(40.0),
                    )),
                    path_geometry: None,
                }),
                PageItem::Polygon(Polygon {
                    id: Some("poly1".to_owned()),
                    layer_id: Some("layer6".to_owned()),
                    geometric_bounds: Some(Rect::new(
                        Points::new(40.0),
                        Points::new(40.0),
                        Points::new(60.0),
                        Points::new(60.0),
                    )),
                    path_geometry: None,
                }),
                PageItem::Group(Group {
                    id: Some("g1".to_owned()),
                    layer_id: Some("layer3".to_owned()),
                    geometric_bounds: None,
                    page_items: Vec::new(),
                }),
            ],
        };

        let xml = spread.to_xml().unwrap();
        let parsed = Spread::from_xml(&xml).unwrap();

        assert!(xml.contains("ItemLayer=\"layer1\""));
        assert!(xml.contains("ItemLayer=\"layer2\""));
        assert!(xml.contains("ItemLayer=\"layer3\""));
        assert!(xml.contains("ItemLayer=\"layer4\""));
        assert!(xml.contains("<Oval"));
        assert!(xml.contains("ItemLayer=\"layer5\""));
        assert!(xml.contains("<GraphicLine"));
        assert!(xml.contains("ItemLayer=\"layer6\""));
        assert!(xml.contains("<Polygon"));
        assert_eq!(parsed, spread);
    }

    #[test]
    fn serializer_enforces_page_count_limit() {
        let spread = Spread {
            pages: (0..=super::DEFAULT_MAX_PAGES_PER_SPREAD)
                .map(|index| Page {
                    id: Some(format!("p{index}")),
                    geometric_bounds: None,
                })
                .collect(),
            ..Spread::default()
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread page count",
                limit: 10_000,
                actual: 10_001,
            }
        ));
    }

    #[test]
    fn serializer_enforces_guide_count_limit() {
        let spread = Spread {
            guides: (0..=super::DEFAULT_MAX_GUIDES_PER_SPREAD)
                .map(|index| Guide {
                    id: Some(format!("g{index}")),
                    layer_id: None,
                    orientation: Some("Horizontal".to_owned()),
                    location: None,
                    page_index: None,
                })
                .collect(),
            ..Spread::default()
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread guide count",
                limit: 100_000,
                actual: 100_001,
            }
        ));
    }

    #[test]
    fn serializer_enforces_image_count_limit() {
        let spread = Spread {
            page_items: vec![PageItem::Rectangle(Rectangle {
                id: Some("r1".to_owned()),
                layer_id: None,
                geometric_bounds: None,
                path_geometry: None,
                images: (0..=super::DEFAULT_MAX_IMAGES_PER_SPREAD)
                    .map(|index| Image {
                        id: Some(format!("img{index}")),
                        image_type_name: None,
                        item_transform: None,
                        graphic_bounds: None,
                        links: Vec::new(),
                    })
                    .collect(),
            })],
            ..Spread::default()
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread image count",
                limit: 100_000,
                actual: 100_001,
            }
        ));
    }

    #[test]
    fn serializer_enforces_path_point_count_limit() {
        let point = PathPoint {
            anchor: Point::new(Points::new(0.0), Points::new(0.0)),
            left_direction: Point::new(Points::new(0.0), Points::new(0.0)),
            right_direction: Point::new(Points::new(0.0), Points::new(0.0)),
        };
        let spread = Spread {
            page_items: vec![PageItem::Polygon(Polygon {
                id: Some("poly1".to_owned()),
                layer_id: None,
                geometric_bounds: None,
                path_geometry: Some(PathGeometry {
                    paths: vec![GeometryPath {
                        path_open: Some(false),
                        points: vec![point; super::DEFAULT_MAX_PATH_POINTS_PER_SPREAD + 1],
                    }],
                }),
            })],
            ..Spread::default()
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread path point count",
                limit: 100_000,
                actual: 100_001,
            }
        ));
    }

    #[test]
    fn serializer_enforces_text_frame_count_limit() {
        let spread = Spread {
            text_frames: (0..=super::DEFAULT_MAX_TEXT_FRAMES_PER_SPREAD)
                .map(|index| TextFrame {
                    id: Some(format!("tf{index}")),
                    layer_id: None,
                    parent_story: None,
                    geometric_bounds: None,
                    path_geometry: None,
                })
                .collect(),
            ..Spread::default()
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread text frame count",
                limit: 100_000,
                actual: 100_001,
            }
        ));
    }

    #[test]
    fn serializer_enforces_recursive_page_item_count_limit() {
        let spread = Spread {
            page_items: vec![PageItem::Group(Group {
                id: Some("g1".to_owned()),
                layer_id: None,
                geometric_bounds: None,
                page_items: (0..super::DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD)
                    .map(|index| {
                        PageItem::Rectangle(Rectangle {
                            id: Some(format!("r{index}")),
                            layer_id: None,
                            geometric_bounds: None,
                            path_geometry: None,
                            images: Vec::new(),
                        })
                    })
                    .collect(),
            })],
            ..Spread::default()
        };

        let err = spread.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "spread page item count",
                limit: 200_000,
                actual: 200_001,
            }
        ));
    }

    #[test]
    fn serializer_rejects_stale_page_item_index() {
        let spread = Spread {
            id: Some("u10".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![TextFrame {
                id: Some("tf1".to_owned()),
                layer_id: None,
                parent_story: Some("u2".to_owned()),
                geometric_bounds: None,
                path_geometry: None,
            }],
            page_items: vec![PageItem::TextFrame(TextFrame {
                id: Some("tf2".to_owned()),
                layer_id: None,
                parent_story: Some("u2".to_owned()),
                geometric_bounds: None,
                path_geometry: None,
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
    fn validate_rejects_stale_page_item_index() {
        let spread = Spread {
            id: Some("u10".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![TextFrame {
                id: Some("tf1".to_owned()),
                layer_id: None,
                parent_story: Some("u2".to_owned()),
                geometric_bounds: None,
                path_geometry: None,
            }],
            page_items: vec![PageItem::TextFrame(TextFrame {
                id: Some("tf2".to_owned()),
                layer_id: None,
                parent_story: Some("u2".to_owned()),
                geometric_bounds: None,
                path_geometry: None,
            })],
        };

        let err = spread.validate().unwrap_err();

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
    fn validate_rejects_non_finite_group_coordinates() {
        let spread = Spread {
            id: Some("u1".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: Vec::new(),
            page_items: vec![PageItem::Group(Group {
                id: Some("g1".to_owned()),
                layer_id: None,
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(f64::INFINITY),
                    Points::new(144.0),
                    Points::new(216.0),
                )),
                page_items: Vec::new(),
            })],
        };

        let err = spread.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "GeometricBounds",
                reason: "coordinate must be finite",
            } if element == "Group"
        ));
    }

    #[test]
    fn validate_rejects_xml_forbidden_group_attributes() {
        let spread = Spread {
            id: Some("u1".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: Vec::new(),
            page_items: vec![PageItem::Group(Group {
                id: Some("g\u{0}".to_owned()),
                layer_id: None,
                geometric_bounds: None,
                page_items: Vec::new(),
            })],
        };

        let err = spread.validate().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Group"
        ));
    }

    #[test]
    fn serializer_rejects_non_finite_coordinates() {
        let spread = Spread {
            id: Some("u1".to_owned()),
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![TextFrame {
                id: Some("tf1".to_owned()),
                layer_id: None,
                parent_story: Some("u1".to_owned()),
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(f64::NAN),
                    Points::new(144.0),
                    Points::new(216.0),
                )),
                path_geometry: None,
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
            pages: Vec::new(),
            guides: Vec::new(),
            text_frames: vec![TextFrame {
                id: Some("tf\u{0}".to_owned()),
                layer_id: None,
                parent_story: Some("u1".to_owned()),
                geometric_bounds: None,
                path_geometry: None,
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
