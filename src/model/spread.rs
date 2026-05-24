//! Parser for IDML spread geometry.

use crate::core::units::{Millimeters, Points};
use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::{
    AnyRootTracker, DEFAULT_MAX_XML_DEPTH, DEFAULT_MAX_XML_INPUT_BYTES, XmlDepthTracker,
    configure_idml_reader, enforce_xml_input_bytes, reject_xml_doctype,
    reject_xml_entity_reference, validate_xml_attribute, validate_xml_text,
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

/// Default maximum list items accepted in one spread property list.
pub const DEFAULT_MAX_SPREAD_LIST_ITEMS: usize = 100_000;

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
    /// Previous threaded text frame ID from `PreviousTextFrame`, when present.
    pub previous_text_frame: Option<String>,
    /// Next threaded text frame ID from `NextTextFrame`, when present.
    pub next_text_frame: Option<String>,
    /// Geometric bounds in points, when represented directly.
    pub geometric_bounds: Option<Rect>,
    /// Common page item metadata.
    pub metadata: PageItemMetadata,
    /// Text frame layout preferences, when present.
    pub preference: Option<TextFramePreference>,
    /// Text frame footnote options, when present.
    pub footnote_options: Option<TextFrameFootnoteOptions>,
    /// Vector path geometry in local point coordinates, when present.
    pub path_geometry: Option<PathGeometry>,
}

/// Footnote layout options for a text frame.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextFrameFootnoteOptions {
    /// Override flag, when present.
    pub enable_overrides: Option<bool>,
    /// Span footnotes across columns flag, when present.
    pub span_footnotes_across: Option<bool>,
    /// Minimum footnote spacing, preserved as an IDML string.
    pub minimum_spacing_option: Option<String>,
    /// Space between footnotes, preserved as an IDML string.
    pub space_between_footnotes: Option<String>,
}

/// Text frame layout preference metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextFramePreference {
    /// Number of text columns, preserved as an IDML string.
    pub text_column_count: Option<String>,
    /// Column gutter, preserved as an IDML string.
    pub text_column_gutter: Option<String>,
    /// Fixed column width, preserved as an IDML string.
    pub text_column_fixed_width: Option<String>,
    /// Maximum column width, preserved as an IDML string.
    pub text_column_max_width: Option<String>,
    /// Use fixed column width flag, when present.
    pub use_fixed_column_width: Option<bool>,
    /// First baseline offset mode, when present.
    pub first_baseline_offset: Option<String>,
    /// Minimum first baseline offset, preserved as an IDML string.
    pub minimum_first_baseline_offset: Option<String>,
    /// Vertical justification mode, when present.
    pub vertical_justification: Option<String>,
    /// Vertical threshold, preserved as an IDML string.
    pub vertical_threshold: Option<String>,
    /// Ignore text wrap flag, when present.
    pub ignore_wrap: Option<bool>,
    /// Use flexible column width flag, when present.
    pub use_flexible_column_width: Option<bool>,
    /// Auto-sizing type, when present.
    pub auto_sizing_type: Option<String>,
    /// Auto-sizing reference point, when present.
    pub auto_sizing_reference_point: Option<String>,
    /// Use minimum height for auto-sizing flag, when present.
    pub use_minimum_height_for_auto_sizing: Option<bool>,
    /// Minimum auto-sizing height, preserved as an IDML string.
    pub minimum_height_for_auto_sizing: Option<String>,
    /// Use minimum width for auto-sizing flag, when present.
    pub use_minimum_width_for_auto_sizing: Option<bool>,
    /// Minimum auto-sizing width, preserved as an IDML string.
    pub minimum_width_for_auto_sizing: Option<String>,
    /// Use no-line-breaks for auto-sizing flag, when present.
    pub use_no_line_breaks_for_auto_sizing: Option<bool>,
    /// Override footnote settings flag, when present.
    pub footnotes_enable_overrides: Option<bool>,
    /// Span footnotes across columns flag, when present.
    pub footnotes_span_across_columns: Option<bool>,
    /// Footnote minimum spacing, preserved as an IDML string.
    pub footnotes_minimum_spacing: Option<String>,
    /// Footnote space between, preserved as an IDML string.
    pub footnotes_space_between: Option<String>,
    /// Vertically balance columns flag, when present.
    pub vertical_balance_columns: Option<bool>,
    /// Column rule override flag, when present.
    pub column_rule_override: Option<bool>,
    /// Column rule offset, preserved as an IDML string.
    pub column_rule_offset: Option<String>,
    /// Column rule top inset, preserved as an IDML string.
    pub column_rule_top_inset: Option<String>,
    /// Column rule inset-chain override flag, when present.
    pub column_rule_inset_chain_override: Option<bool>,
    /// Column rule bottom inset, preserved as an IDML string.
    pub column_rule_bottom_inset: Option<String>,
    /// Column rule stroke width, preserved as an IDML string.
    pub column_rule_stroke_width: Option<String>,
    /// Column rule stroke color reference, when present.
    pub column_rule_stroke_color: Option<String>,
    /// Column rule stroke type reference, when present.
    pub column_rule_stroke_type: Option<String>,
    /// Column rule stroke tint, preserved as an IDML string.
    pub column_rule_stroke_tint: Option<String>,
    /// Column rule overprint override flag, when present.
    pub column_rule_overprint_override: Option<bool>,
    /// Inset spacing property, when present.
    pub inset_spacing: Option<SpreadListProperty>,
}

/// A typed IDML list property used by spread child properties.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SpreadListProperty {
    /// IDML `type` attribute, when present.
    pub value_type: Option<String>,
    /// Ordered list items.
    pub items: Vec<SpreadListItem>,
}

/// A scalar spread list item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SpreadListItem {
    /// IDML `type` attribute, when present.
    pub value_type: Option<String>,
    /// Item value.
    pub value: String,
}

/// Text wrap behavior for a page item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextWrapPreference {
    /// Inverse wrap flag, when present.
    pub inverse: Option<bool>,
    /// Restrict wrap application to master pages, when present.
    pub apply_to_master_page_only: Option<bool>,
    /// Wrap side mode, when present.
    pub text_wrap_side: Option<String>,
    /// Wrap mode, when present.
    pub text_wrap_mode: Option<String>,
    /// Wrap offsets, when present.
    pub text_wrap_offset: Option<TextWrapOffset>,
    /// Wrap contour option, when present.
    pub contour_option: Option<ContourOption>,
}

/// Text wrap offset values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextWrapOffset {
    /// Top offset, preserved as an IDML string.
    pub top: Option<String>,
    /// Left offset, preserved as an IDML string.
    pub left: Option<String>,
    /// Bottom offset, preserved as an IDML string.
    pub bottom: Option<String>,
    /// Right offset, preserved as an IDML string.
    pub right: Option<String>,
}

/// Contour behavior for text wrapping around a page item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContourOption {
    /// Contour type, when present.
    pub contour_type: Option<String>,
    /// Include inside edges flag, when present.
    pub include_inside_edges: Option<bool>,
    /// Named contour path, when present.
    pub contour_path_name: Option<String>,
}

/// Frame fitting behavior for a page item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FrameFittingOption {
    /// Auto-fit flag, when present.
    pub auto_fit: Option<bool>,
    /// Left crop, preserved as an IDML string.
    pub left_crop: Option<String>,
    /// Top crop, preserved as an IDML string.
    pub top_crop: Option<String>,
    /// Right crop, preserved as an IDML string.
    pub right_crop: Option<String>,
    /// Bottom crop, preserved as an IDML string.
    pub bottom_crop: Option<String>,
    /// Fitting behavior for empty frames, when present.
    pub fitting_on_empty_frame: Option<String>,
    /// Fitting alignment anchor, when present.
    pub fitting_alignment: Option<String>,
}

/// InCopy export options attached to a page item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PageItemInCopyExportOption {
    /// Include graphic proxies flag, when present.
    pub include_graphic_proxies: Option<bool>,
    /// Include all resources flag, when present.
    pub include_all_resources: Option<bool>,
}

/// Export behavior and accessibility metadata for a page item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObjectExportOption {
    /// EPUB type token, when present.
    pub epub_type: Option<String>,
    /// EPUB ARIA role token, when present.
    pub epub_aria_role: Option<String>,
    /// Export size mode, when present.
    pub size_type: Option<String>,
    /// Custom export size, preserved as an IDML string.
    pub custom_size: Option<String>,
    /// Layout appearance preservation mode, when present.
    pub preserve_appearance_from_layout: Option<String>,
    /// Source used for alternate text, when present.
    pub alt_text_source_type: Option<String>,
    /// Source used for actual text, when present.
    pub actual_text_source_type: Option<String>,
    /// Custom alternate text, when present.
    pub custom_alt_text: Option<String>,
    /// Custom actual text, when present.
    pub custom_actual_text: Option<String>,
    /// Tagging mode for export, when present.
    pub apply_tag_type: Option<String>,
    /// Whether custom image conversion is enabled.
    pub custom_image_conversion: Option<bool>,
    /// Image conversion format, when present.
    pub image_conversion_type: Option<String>,
    /// Custom image sizing mode, when present.
    pub custom_image_size_option: Option<String>,
    /// Image export resolution, when present.
    pub image_export_resolution: Option<String>,
    /// GIF palette option, when present.
    pub gif_options_palette: Option<String>,
    /// GIF interlace flag, when present.
    pub gif_options_interlaced: Option<bool>,
    /// JPEG quality option, when present.
    pub jpeg_options_quality: Option<String>,
    /// JPEG encoding option, when present.
    pub jpeg_options_format: Option<String>,
    /// Export image alignment, when present.
    pub image_alignment: Option<String>,
    /// Spacing before exported image, preserved as an IDML string.
    pub image_space_before: Option<String>,
    /// Spacing after exported image, preserved as an IDML string.
    pub image_space_after: Option<String>,
    /// Whether an image page break is applied.
    pub use_image_page_break: Option<bool>,
    /// Image page break mode, when present.
    pub image_page_break: Option<String>,
    /// Whether custom image alignment is enabled.
    pub custom_image_alignment: Option<bool>,
    /// Spacing unit for export values, when present.
    pub space_unit: Option<String>,
    /// Whether custom layout export settings are enabled.
    pub custom_layout: Option<bool>,
    /// Custom layout export mode, when present.
    pub custom_layout_type: Option<String>,
    /// Whether the original image is used for export.
    pub use_original_image: Option<bool>,
    /// Alternate-text metadata property, when present.
    pub alt_metadata_property: Option<ObjectExportMetadataProperty>,
    /// Actual-text metadata property, when present.
    pub actual_metadata_property: Option<ObjectExportMetadataProperty>,
}

/// Metadata property reference inside `ObjectExportOption`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObjectExportMetadataProperty {
    /// Metadata namespace prefix, when present.
    pub namespace_prefix: Option<String>,
    /// Metadata property path, when present.
    pub property_path: Option<String>,
}

/// Common page item metadata shared by frames, shapes, and groups.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageItemMetadata {
    /// Item content type, such as `TextType`, `GraphicType`, or `Unassigned`.
    pub content_type: Option<String>,
    /// Page item name, when present.
    pub name: Option<String>,
    /// Item transform matrix, preserved as an IDML string.
    pub item_transform: Option<String>,
    /// Applied object style reference, when present.
    pub applied_object_style: Option<String>,
    /// Fill color reference, when present.
    pub fill_color: Option<String>,
    /// Stroke color reference, when present.
    pub stroke_color: Option<String>,
    /// Stroke weight value, preserved as an IDML string.
    pub stroke_weight: Option<String>,
    /// Visibility flag, when present.
    pub visible: Option<bool>,
    /// Lock flag, when present.
    pub locked: Option<bool>,
    /// Fill tint value, preserved as an IDML string.
    pub fill_tint: Option<String>,
    /// Stroke tint value, preserved as an IDML string.
    pub stroke_tint: Option<String>,
    /// Gap color reference, when present.
    pub gap_color: Option<String>,
    /// Gap tint value, preserved as an IDML string.
    pub gap_tint: Option<String>,
    /// Stroke type reference, when present.
    pub stroke_type: Option<String>,
    /// Stroke alignment, preserved as an IDML string.
    pub stroke_alignment: Option<String>,
    /// Stroke miter limit, preserved as an IDML string.
    pub miter_limit: Option<String>,
    /// Stroke end cap style, preserved as an IDML string.
    pub end_cap: Option<String>,
    /// Stroke end join style, preserved as an IDML string.
    pub end_join: Option<String>,
    /// Left line end style, preserved as an IDML string.
    pub left_line_end: Option<String>,
    /// Right line end style, preserved as an IDML string.
    pub right_line_end: Option<String>,
    /// Legacy lock state field, preserved as an IDML string.
    pub lock_state: Option<String>,
    /// Story title metadata, preserved as an IDML string.
    pub story_title: Option<String>,
    /// Flex item width mode, preserved as an IDML string.
    pub flex_item_width_mode: Option<String>,
    /// Flex item height mode, preserved as an IDML string.
    pub flex_item_height_mode: Option<String>,
    /// Nonprinting flag, when present.
    pub nonprinting: Option<bool>,
    /// Fill overprint flag, when present.
    pub overprint_fill: Option<bool>,
    /// Stroke overprint flag, when present.
    pub overprint_stroke: Option<bool>,
    /// Shared corner radius, preserved as an IDML string.
    pub corner_radius: Option<String>,
    /// Shared corner option, preserved as an IDML string.
    pub corner_option: Option<String>,
    /// Top-left corner option, preserved as an IDML string.
    pub top_left_corner_option: Option<String>,
    /// Top-right corner option, preserved as an IDML string.
    pub top_right_corner_option: Option<String>,
    /// Bottom-left corner option, preserved as an IDML string.
    pub bottom_left_corner_option: Option<String>,
    /// Bottom-right corner option, preserved as an IDML string.
    pub bottom_right_corner_option: Option<String>,
    /// Top-left corner radius, preserved as an IDML string.
    pub top_left_corner_radius: Option<String>,
    /// Top-right corner radius, preserved as an IDML string.
    pub top_right_corner_radius: Option<String>,
    /// Bottom-left corner radius, preserved as an IDML string.
    pub bottom_left_corner_radius: Option<String>,
    /// Bottom-right corner radius, preserved as an IDML string.
    pub bottom_right_corner_radius: Option<String>,
    /// Adobe parent interface change counter, preserved as an IDML string.
    pub parent_interface_change_count: Option<String>,
    /// Adobe target interface change counter, preserved as an IDML string.
    pub target_interface_change_count: Option<String>,
    /// Adobe last-updated interface change counter, preserved as an IDML string.
    pub last_updated_interface_change_count: Option<String>,
    /// Overridden page item property list, preserved as an IDML string.
    pub overridden_page_item_props: Option<String>,
    /// Horizontal layout constraint triplet, preserved as an IDML string.
    pub horizontal_layout_constraints: Option<String>,
    /// Vertical layout constraint triplet, preserved as an IDML string.
    pub vertical_layout_constraints: Option<String>,
    /// Gradient fill start point, preserved as an IDML string.
    pub gradient_fill_start: Option<String>,
    /// Gradient fill length, preserved as an IDML string.
    pub gradient_fill_length: Option<String>,
    /// Gradient fill angle, preserved as an IDML string.
    pub gradient_fill_angle: Option<String>,
    /// Gradient stroke start point, preserved as an IDML string.
    pub gradient_stroke_start: Option<String>,
    /// Gradient stroke length, preserved as an IDML string.
    pub gradient_stroke_length: Option<String>,
    /// Gradient stroke angle, preserved as an IDML string.
    pub gradient_stroke_angle: Option<String>,
    /// Local display setting, preserved as an IDML string.
    pub local_display_setting: Option<String>,
    /// Gradient fill hilite length, preserved as an IDML string.
    pub gradient_fill_hilite_length: Option<String>,
    /// Gradient fill hilite angle, preserved as an IDML string.
    pub gradient_fill_hilite_angle: Option<String>,
    /// Gradient stroke hilite length, preserved as an IDML string.
    pub gradient_stroke_hilite_length: Option<String>,
    /// Gradient stroke hilite angle, preserved as an IDML string.
    pub gradient_stroke_hilite_angle: Option<String>,
    /// Text wrap preference child element, when present.
    pub text_wrap_preference: Option<TextWrapPreference>,
    /// Frame fitting option child element, when present.
    pub frame_fitting_option: Option<FrameFittingOption>,
    /// InCopy export option child element, when present.
    pub incopy_export_option: Option<PageItemInCopyExportOption>,
    /// Object export option child element, when present.
    pub object_export_option: Option<ObjectExportOption>,
}

impl PageItemMetadata {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        let element = String::from_utf8_lossy(event.name().as_ref()).into_owned();
        Ok(Self {
            content_type: optional_attr(event, "ContentType", reader, max_attribute_bytes)?,
            name: optional_attr(event, "Name", reader, max_attribute_bytes)?,
            item_transform: optional_attr(event, "ItemTransform", reader, max_attribute_bytes)?,
            applied_object_style: optional_attr(
                event,
                "AppliedObjectStyle",
                reader,
                max_attribute_bytes,
            )?,
            fill_color: optional_attr(event, "FillColor", reader, max_attribute_bytes)?,
            stroke_color: optional_attr(event, "StrokeColor", reader, max_attribute_bytes)?,
            stroke_weight: optional_attr(event, "StrokeWeight", reader, max_attribute_bytes)?,
            visible: optional_bool_attr(
                event,
                element.as_str(),
                "Visible",
                reader,
                max_attribute_bytes,
            )?,
            locked: optional_bool_attr(
                event,
                element.as_str(),
                "Locked",
                reader,
                max_attribute_bytes,
            )?,
            fill_tint: optional_attr(event, "FillTint", reader, max_attribute_bytes)?,
            stroke_tint: optional_attr(event, "StrokeTint", reader, max_attribute_bytes)?,
            gap_color: optional_attr(event, "GapColor", reader, max_attribute_bytes)?,
            gap_tint: optional_attr(event, "GapTint", reader, max_attribute_bytes)?,
            stroke_type: optional_attr(event, "StrokeType", reader, max_attribute_bytes)?,
            stroke_alignment: optional_attr(event, "StrokeAlignment", reader, max_attribute_bytes)?,
            miter_limit: optional_attr(event, "MiterLimit", reader, max_attribute_bytes)?,
            end_cap: optional_attr(event, "EndCap", reader, max_attribute_bytes)?,
            end_join: optional_attr(event, "EndJoin", reader, max_attribute_bytes)?,
            left_line_end: optional_attr(event, "LeftLineEnd", reader, max_attribute_bytes)?,
            right_line_end: optional_attr(event, "RightLineEnd", reader, max_attribute_bytes)?,
            lock_state: optional_attr(event, "LockState", reader, max_attribute_bytes)?,
            story_title: optional_attr(event, "StoryTitle", reader, max_attribute_bytes)?,
            flex_item_width_mode: optional_attr(
                event,
                "FlexItemWidthMode",
                reader,
                max_attribute_bytes,
            )?,
            flex_item_height_mode: optional_attr(
                event,
                "FlexItemHeightMode",
                reader,
                max_attribute_bytes,
            )?,
            nonprinting: optional_bool_attr(
                event,
                element.as_str(),
                "Nonprinting",
                reader,
                max_attribute_bytes,
            )?,
            overprint_fill: optional_bool_attr(
                event,
                element.as_str(),
                "OverprintFill",
                reader,
                max_attribute_bytes,
            )?,
            overprint_stroke: optional_bool_attr(
                event,
                element.as_str(),
                "OverprintStroke",
                reader,
                max_attribute_bytes,
            )?,
            corner_radius: optional_attr(event, "CornerRadius", reader, max_attribute_bytes)?,
            corner_option: optional_attr(event, "CornerOption", reader, max_attribute_bytes)?,
            top_left_corner_option: optional_attr(
                event,
                "TopLeftCornerOption",
                reader,
                max_attribute_bytes,
            )?,
            top_right_corner_option: optional_attr(
                event,
                "TopRightCornerOption",
                reader,
                max_attribute_bytes,
            )?,
            bottom_left_corner_option: optional_attr(
                event,
                "BottomLeftCornerOption",
                reader,
                max_attribute_bytes,
            )?,
            bottom_right_corner_option: optional_attr(
                event,
                "BottomRightCornerOption",
                reader,
                max_attribute_bytes,
            )?,
            top_left_corner_radius: optional_attr(
                event,
                "TopLeftCornerRadius",
                reader,
                max_attribute_bytes,
            )?,
            top_right_corner_radius: optional_attr(
                event,
                "TopRightCornerRadius",
                reader,
                max_attribute_bytes,
            )?,
            bottom_left_corner_radius: optional_attr(
                event,
                "BottomLeftCornerRadius",
                reader,
                max_attribute_bytes,
            )?,
            bottom_right_corner_radius: optional_attr(
                event,
                "BottomRightCornerRadius",
                reader,
                max_attribute_bytes,
            )?,
            parent_interface_change_count: optional_attr(
                event,
                "ParentInterfaceChangeCount",
                reader,
                max_attribute_bytes,
            )?,
            target_interface_change_count: optional_attr(
                event,
                "TargetInterfaceChangeCount",
                reader,
                max_attribute_bytes,
            )?,
            last_updated_interface_change_count: optional_attr(
                event,
                "LastUpdatedInterfaceChangeCount",
                reader,
                max_attribute_bytes,
            )?,
            overridden_page_item_props: optional_attr(
                event,
                "OverriddenPageItemProps",
                reader,
                max_attribute_bytes,
            )?,
            horizontal_layout_constraints: optional_attr(
                event,
                "HorizontalLayoutConstraints",
                reader,
                max_attribute_bytes,
            )?,
            vertical_layout_constraints: optional_attr(
                event,
                "VerticalLayoutConstraints",
                reader,
                max_attribute_bytes,
            )?,
            gradient_fill_start: optional_attr(
                event,
                "GradientFillStart",
                reader,
                max_attribute_bytes,
            )?,
            gradient_fill_length: optional_attr(
                event,
                "GradientFillLength",
                reader,
                max_attribute_bytes,
            )?,
            gradient_fill_angle: optional_attr(
                event,
                "GradientFillAngle",
                reader,
                max_attribute_bytes,
            )?,
            gradient_stroke_start: optional_attr(
                event,
                "GradientStrokeStart",
                reader,
                max_attribute_bytes,
            )?,
            gradient_stroke_length: optional_attr(
                event,
                "GradientStrokeLength",
                reader,
                max_attribute_bytes,
            )?,
            gradient_stroke_angle: optional_attr(
                event,
                "GradientStrokeAngle",
                reader,
                max_attribute_bytes,
            )?,
            local_display_setting: optional_attr(
                event,
                "LocalDisplaySetting",
                reader,
                max_attribute_bytes,
            )?,
            gradient_fill_hilite_length: optional_attr(
                event,
                "GradientFillHiliteLength",
                reader,
                max_attribute_bytes,
            )?,
            gradient_fill_hilite_angle: optional_attr(
                event,
                "GradientFillHiliteAngle",
                reader,
                max_attribute_bytes,
            )?,
            gradient_stroke_hilite_length: optional_attr(
                event,
                "GradientStrokeHiliteLength",
                reader,
                max_attribute_bytes,
            )?,
            gradient_stroke_hilite_angle: optional_attr(
                event,
                "GradientStrokeHiliteAngle",
                reader,
                max_attribute_bytes,
            )?,
            text_wrap_preference: None,
            frame_fitting_option: None,
            incopy_export_option: None,
            object_export_option: None,
        })
    }
}

impl TextWrapPreference {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            inverse: optional_bool_attr(
                event,
                "TextWrapPreference",
                "Inverse",
                reader,
                max_attribute_bytes,
            )?,
            apply_to_master_page_only: optional_bool_attr(
                event,
                "TextWrapPreference",
                "ApplyToMasterPageOnly",
                reader,
                max_attribute_bytes,
            )?,
            text_wrap_side: optional_attr(event, "TextWrapSide", reader, max_attribute_bytes)?,
            text_wrap_mode: optional_attr(event, "TextWrapMode", reader, max_attribute_bytes)?,
            text_wrap_offset: None,
            contour_option: None,
        })
    }
}

impl TextWrapOffset {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            top: optional_attr(event, "Top", reader, max_attribute_bytes)?,
            left: optional_attr(event, "Left", reader, max_attribute_bytes)?,
            bottom: optional_attr(event, "Bottom", reader, max_attribute_bytes)?,
            right: optional_attr(event, "Right", reader, max_attribute_bytes)?,
        })
    }
}

impl ContourOption {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            contour_type: optional_attr(event, "ContourType", reader, max_attribute_bytes)?,
            include_inside_edges: optional_bool_attr(
                event,
                "ContourOption",
                "IncludeInsideEdges",
                reader,
                max_attribute_bytes,
            )?,
            contour_path_name: optional_attr(
                event,
                "ContourPathName",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl FrameFittingOption {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            auto_fit: optional_bool_attr(
                event,
                "FrameFittingOption",
                "AutoFit",
                reader,
                max_attribute_bytes,
            )?,
            left_crop: optional_attr(event, "LeftCrop", reader, max_attribute_bytes)?,
            top_crop: optional_attr(event, "TopCrop", reader, max_attribute_bytes)?,
            right_crop: optional_attr(event, "RightCrop", reader, max_attribute_bytes)?,
            bottom_crop: optional_attr(event, "BottomCrop", reader, max_attribute_bytes)?,
            fitting_on_empty_frame: optional_attr(
                event,
                "FittingOnEmptyFrame",
                reader,
                max_attribute_bytes,
            )?,
            fitting_alignment: optional_attr(
                event,
                "FittingAlignment",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl PageItemInCopyExportOption {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            include_graphic_proxies: optional_bool_attr(
                event,
                "InCopyExportOption",
                "IncludeGraphicProxies",
                reader,
                max_attribute_bytes,
            )?,
            include_all_resources: optional_bool_attr(
                event,
                "InCopyExportOption",
                "IncludeAllResources",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl ObjectExportOption {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            epub_type: optional_attr(event, "EpubType", reader, max_attribute_bytes)?,
            epub_aria_role: optional_attr(event, "EpubAriaRole", reader, max_attribute_bytes)?,
            size_type: optional_attr(event, "SizeType", reader, max_attribute_bytes)?,
            custom_size: optional_attr(event, "CustomSize", reader, max_attribute_bytes)?,
            preserve_appearance_from_layout: optional_attr(
                event,
                "PreserveAppearanceFromLayout",
                reader,
                max_attribute_bytes,
            )?,
            alt_text_source_type: optional_attr(
                event,
                "AltTextSourceType",
                reader,
                max_attribute_bytes,
            )?,
            actual_text_source_type: optional_attr(
                event,
                "ActualTextSourceType",
                reader,
                max_attribute_bytes,
            )?,
            custom_alt_text: optional_attr(event, "CustomAltText", reader, max_attribute_bytes)?,
            custom_actual_text: optional_attr(
                event,
                "CustomActualText",
                reader,
                max_attribute_bytes,
            )?,
            apply_tag_type: optional_attr(event, "ApplyTagType", reader, max_attribute_bytes)?,
            custom_image_conversion: optional_bool_attr(
                event,
                "ObjectExportOption",
                "CustomImageConversion",
                reader,
                max_attribute_bytes,
            )?,
            image_conversion_type: optional_attr(
                event,
                "ImageConversionType",
                reader,
                max_attribute_bytes,
            )?,
            custom_image_size_option: optional_attr(
                event,
                "CustomImageSizeOption",
                reader,
                max_attribute_bytes,
            )?,
            image_export_resolution: optional_attr(
                event,
                "ImageExportResolution",
                reader,
                max_attribute_bytes,
            )?,
            gif_options_palette: optional_attr(
                event,
                "GIFOptionsPalette",
                reader,
                max_attribute_bytes,
            )?,
            gif_options_interlaced: optional_bool_attr(
                event,
                "ObjectExportOption",
                "GIFOptionsInterlaced",
                reader,
                max_attribute_bytes,
            )?,
            jpeg_options_quality: optional_attr(
                event,
                "JPEGOptionsQuality",
                reader,
                max_attribute_bytes,
            )?,
            jpeg_options_format: optional_attr(
                event,
                "JPEGOptionsFormat",
                reader,
                max_attribute_bytes,
            )?,
            image_alignment: optional_attr(event, "ImageAlignment", reader, max_attribute_bytes)?,
            image_space_before: optional_attr(
                event,
                "ImageSpaceBefore",
                reader,
                max_attribute_bytes,
            )?,
            image_space_after: optional_attr(
                event,
                "ImageSpaceAfter",
                reader,
                max_attribute_bytes,
            )?,
            use_image_page_break: optional_bool_attr(
                event,
                "ObjectExportOption",
                "UseImagePageBreak",
                reader,
                max_attribute_bytes,
            )?,
            image_page_break: optional_attr(event, "ImagePageBreak", reader, max_attribute_bytes)?,
            custom_image_alignment: optional_bool_attr(
                event,
                "ObjectExportOption",
                "CustomImageAlignment",
                reader,
                max_attribute_bytes,
            )?,
            space_unit: optional_attr(event, "SpaceUnit", reader, max_attribute_bytes)?,
            custom_layout: optional_bool_attr(
                event,
                "ObjectExportOption",
                "CustomLayout",
                reader,
                max_attribute_bytes,
            )?,
            custom_layout_type: optional_attr(
                event,
                "CustomLayoutType",
                reader,
                max_attribute_bytes,
            )?,
            use_original_image: optional_bool_attr(
                event,
                "ObjectExportOption",
                "UseOriginalImage",
                reader,
                max_attribute_bytes,
            )?,
            alt_metadata_property: None,
            actual_metadata_property: None,
        })
    }
}

impl ObjectExportMetadataProperty {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            namespace_prefix: optional_attr(event, "NamespacePrefix", reader, max_attribute_bytes)?,
            property_path: optional_attr(event, "PropertyPath", reader, max_attribute_bytes)?,
        })
    }
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
    /// Common page item metadata.
    pub metadata: PageItemMetadata,
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
            metadata: PageItemMetadata::from_xml_event(event, reader, max_attribute_bytes)?,
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
    /// Common page item metadata.
    pub metadata: PageItemMetadata,
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
            metadata: PageItemMetadata::from_xml_event(event, reader, max_attribute_bytes)?,
            path_geometry: None,
        })
    }
}

impl TextFrameFootnoteOptions {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            enable_overrides: optional_bool_attr(
                event,
                "TextFrameFootnoteOptionsObject",
                "EnableOverrides",
                reader,
                max_attribute_bytes,
            )?,
            span_footnotes_across: optional_bool_attr(
                event,
                "TextFrameFootnoteOptionsObject",
                "SpanFootnotesAcross",
                reader,
                max_attribute_bytes,
            )?,
            minimum_spacing_option: optional_attr(
                event,
                "MinimumSpacingOption",
                reader,
                max_attribute_bytes,
            )?,
            space_between_footnotes: optional_attr(
                event,
                "SpaceBetweenFootnotes",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl TextFramePreference {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            text_column_count: optional_attr(
                event,
                "TextColumnCount",
                reader,
                max_attribute_bytes,
            )?,
            text_column_gutter: optional_attr(
                event,
                "TextColumnGutter",
                reader,
                max_attribute_bytes,
            )?,
            text_column_fixed_width: optional_attr(
                event,
                "TextColumnFixedWidth",
                reader,
                max_attribute_bytes,
            )?,
            text_column_max_width: optional_attr(
                event,
                "TextColumnMaxWidth",
                reader,
                max_attribute_bytes,
            )?,
            use_fixed_column_width: optional_bool_attr(
                event,
                "TextFramePreference",
                "UseFixedColumnWidth",
                reader,
                max_attribute_bytes,
            )?,
            first_baseline_offset: optional_attr(
                event,
                "FirstBaselineOffset",
                reader,
                max_attribute_bytes,
            )?,
            minimum_first_baseline_offset: optional_attr(
                event,
                "MinimumFirstBaselineOffset",
                reader,
                max_attribute_bytes,
            )?,
            vertical_justification: optional_attr(
                event,
                "VerticalJustification",
                reader,
                max_attribute_bytes,
            )?,
            vertical_threshold: optional_attr(
                event,
                "VerticalThreshold",
                reader,
                max_attribute_bytes,
            )?,
            ignore_wrap: optional_bool_attr(
                event,
                "TextFramePreference",
                "IgnoreWrap",
                reader,
                max_attribute_bytes,
            )?,
            use_flexible_column_width: optional_bool_attr(
                event,
                "TextFramePreference",
                "UseFlexibleColumnWidth",
                reader,
                max_attribute_bytes,
            )?,
            auto_sizing_type: optional_attr(event, "AutoSizingType", reader, max_attribute_bytes)?,
            auto_sizing_reference_point: optional_attr(
                event,
                "AutoSizingReferencePoint",
                reader,
                max_attribute_bytes,
            )?,
            use_minimum_height_for_auto_sizing: optional_bool_attr(
                event,
                "TextFramePreference",
                "UseMinimumHeightForAutoSizing",
                reader,
                max_attribute_bytes,
            )?,
            minimum_height_for_auto_sizing: optional_attr(
                event,
                "MinimumHeightForAutoSizing",
                reader,
                max_attribute_bytes,
            )?,
            use_minimum_width_for_auto_sizing: optional_bool_attr(
                event,
                "TextFramePreference",
                "UseMinimumWidthForAutoSizing",
                reader,
                max_attribute_bytes,
            )?,
            minimum_width_for_auto_sizing: optional_attr(
                event,
                "MinimumWidthForAutoSizing",
                reader,
                max_attribute_bytes,
            )?,
            use_no_line_breaks_for_auto_sizing: optional_bool_attr(
                event,
                "TextFramePreference",
                "UseNoLineBreaksForAutoSizing",
                reader,
                max_attribute_bytes,
            )?,
            footnotes_enable_overrides: optional_bool_attr(
                event,
                "TextFramePreference",
                "FootnotesEnableOverrides",
                reader,
                max_attribute_bytes,
            )?,
            footnotes_span_across_columns: optional_bool_attr(
                event,
                "TextFramePreference",
                "FootnotesSpanAcrossColumns",
                reader,
                max_attribute_bytes,
            )?,
            footnotes_minimum_spacing: optional_attr(
                event,
                "FootnotesMinimumSpacing",
                reader,
                max_attribute_bytes,
            )?,
            footnotes_space_between: optional_attr(
                event,
                "FootnotesSpaceBetween",
                reader,
                max_attribute_bytes,
            )?,
            vertical_balance_columns: optional_bool_attr(
                event,
                "TextFramePreference",
                "VerticalBalanceColumns",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_override: optional_bool_attr(
                event,
                "TextFramePreference",
                "ColumnRuleOverride",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_offset: optional_attr(
                event,
                "ColumnRuleOffset",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_top_inset: optional_attr(
                event,
                "ColumnRuleTopInset",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_inset_chain_override: optional_bool_attr(
                event,
                "TextFramePreference",
                "ColumnRuleInsetChainOverride",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_bottom_inset: optional_attr(
                event,
                "ColumnRuleBottomInset",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_stroke_width: optional_attr(
                event,
                "ColumnRuleStrokeWidth",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_stroke_color: optional_attr(
                event,
                "ColumnRuleStrokeColor",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_stroke_type: optional_attr(
                event,
                "ColumnRuleStrokeType",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_stroke_tint: optional_attr(
                event,
                "ColumnRuleStrokeTint",
                reader,
                max_attribute_bytes,
            )?,
            column_rule_overprint_override: optional_bool_attr(
                event,
                "TextFramePreference",
                "ColumnRuleOverprintOverride",
                reader,
                max_attribute_bytes,
            )?,
            inset_spacing: None,
        })
    }
}

impl SpreadListProperty {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            value_type: optional_attr(event, "type", reader, max_attribute_bytes)?,
            items: Vec::new(),
        })
    }
}

impl SpreadListItem {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            value_type: optional_attr(event, "type", reader, max_attribute_bytes)?,
            value: String::new(),
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
    /// Common page item metadata.
    pub metadata: PageItemMetadata,
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
            metadata: PageItemMetadata::from_xml_event(event, reader, max_attribute_bytes)?,
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
    /// Common page item metadata.
    pub metadata: PageItemMetadata,
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
            metadata: PageItemMetadata::from_xml_event(event, reader, max_attribute_bytes)?,
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
    /// Common page item metadata.
    pub metadata: PageItemMetadata,
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
            metadata: PageItemMetadata::from_xml_event(event, reader, max_attribute_bytes)?,
            page_items: Vec::new(),
        })
    }
}

/// Supported spread page item in z-order.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
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
        let previous_text_frame =
            optional_attr(event, "PreviousTextFrame", reader, max_attribute_bytes)?;
        let next_text_frame = optional_attr(event, "NextTextFrame", reader, max_attribute_bytes)?;
        let geometric_bounds =
            optional_attr(event, "GeometricBounds", reader, max_attribute_bytes)?
                .map(|value| parse_geometric_bounds(element, &value))
                .transpose()?;

        Ok(Self {
            id,
            layer_id,
            parent_story,
            previous_text_frame,
            next_text_frame,
            geometric_bounds,
            metadata: PageItemMetadata::from_xml_event(event, reader, max_attribute_bytes)?,
            preference: None,
            footnote_options: None,
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

fn optional_bool_attr(
    event: &BytesStart<'_>,
    element: &str,
    attribute: &'static str,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<Option<bool>> {
    optional_attr(event, attribute, reader, max_attribute_bytes)?
        .map(|value| parse_bool_attribute(element, attribute, &value))
        .transpose()
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

fn parse_bool_attribute(element: &str, attribute: &'static str, value: &str) -> Result<bool> {
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
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"PathGeometry" {
                    assign_path_geometry(
                        &mut frame.path_geometry,
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
                } else if e.name().as_ref() == b"TextFramePreference" {
                    let preference = TextFramePreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    frame.preference = Some(parse_text_frame_preference_contents(
                        reader, preference, state,
                    )?);
                } else if e.name().as_ref() == b"TextFrameFootnoteOptionsObject" {
                    frame.footnote_options = Some(TextFrameFootnoteOptions::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                    parse_text_frame_footnote_options_contents(reader, state)?;
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    let preference = TextWrapPreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    frame.metadata.text_wrap_preference = Some(
                        parse_text_wrap_preference_contents(reader, preference, state)?,
                    );
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    frame.metadata.frame_fitting_option = Some(FrameFittingOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                    parse_frame_fitting_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    frame.metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                    parse_incopy_export_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    let option = ObjectExportOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    frame.metadata.object_export_option =
                        Some(parse_object_export_option_contents(reader, option, state)?);
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"PathGeometry" {
                    assign_path_geometry(
                        &mut frame.path_geometry,
                        PathGeometry { paths: Vec::new() },
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
                } else if e.name().as_ref() == b"TextFramePreference" {
                    frame.preference = Some(TextFramePreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"TextFrameFootnoteOptionsObject" {
                    frame.footnote_options = Some(TextFrameFootnoteOptions::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    frame.metadata.text_wrap_preference = Some(TextWrapPreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    frame.metadata.frame_fitting_option = Some(FrameFittingOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    frame.metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    frame.metadata.object_export_option = Some(ObjectExportOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"TextFrame" {
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
                    element: "TextFrame".to_owned(),
                    reason: "unclosed page item element",
                });
            }
            _ => {}
        }
    }
    Ok(frame)
}

fn parse_text_frame_preference_contents(
    reader: &mut Reader<&[u8]>,
    mut preference: TextFramePreference,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<TextFramePreference> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"InsetSpacing" {
                    preference.inset_spacing = Some(parse_spread_list_property_contents(
                        reader,
                        SpreadListProperty::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?,
                        "InsetSpacing",
                        state,
                    )?);
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"InsetSpacing" {
                    preference.inset_spacing = Some(SpreadListProperty::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"TextFramePreference" {
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
                    element: "TextFramePreference".to_owned(),
                    reason: "unclosed TextFramePreference element",
                });
            }
            _ => {}
        }
    }
    Ok(preference)
}

fn parse_text_frame_footnote_options_contents(
    reader: &mut Reader<&[u8]>,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<()> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"TextFrameFootnoteOptionsObject" {
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
                    element: "TextFrameFootnoteOptionsObject".to_owned(),
                    reason: "unclosed TextFrameFootnoteOptionsObject element",
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_text_wrap_preference_contents(
    reader: &mut Reader<&[u8]>,
    mut preference: TextWrapPreference,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<TextWrapPreference> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"TextWrapOffset" {
                    preference.text_wrap_offset = Some(TextWrapOffset::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"ContourOption" {
                    preference.contour_option = Some(ContourOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                    parse_contour_option_contents(reader, state)?;
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"TextWrapOffset" {
                    preference.text_wrap_offset = Some(TextWrapOffset::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"ContourOption" {
                    preference.contour_option = Some(ContourOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                }
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"TextWrapPreference" {
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
                    element: "TextWrapPreference".to_owned(),
                    reason: "unclosed TextWrapPreference element",
                });
            }
            _ => {}
        }
    }
    Ok(preference)
}

fn parse_contour_option_contents(
    reader: &mut Reader<&[u8]>,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<()> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"ContourOption" {
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
                    element: "ContourOption".to_owned(),
                    reason: "unclosed ContourOption element",
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_frame_fitting_option_contents(
    reader: &mut Reader<&[u8]>,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<()> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"FrameFittingOption" {
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
                    element: "FrameFittingOption".to_owned(),
                    reason: "unclosed FrameFittingOption element",
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_incopy_export_option_contents(
    reader: &mut Reader<&[u8]>,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<()> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"InCopyExportOption" {
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
                    element: "InCopyExportOption".to_owned(),
                    reason: "unclosed InCopyExportOption element",
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_object_export_option_contents(
    reader: &mut Reader<&[u8]>,
    mut option: ObjectExportOption,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<ObjectExportOption> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                parse_object_export_property(&mut option, &e, reader, state)?;
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                parse_object_export_property(&mut option, &e, reader, state)?;
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"ObjectExportOption" {
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
                    element: "ObjectExportOption".to_owned(),
                    reason: "unclosed ObjectExportOption element",
                });
            }
            _ => {}
        }
    }
    Ok(option)
}

fn parse_object_export_property(
    option: &mut ObjectExportOption,
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    state: &NestedSpreadParseState<'_>,
) -> Result<()> {
    if event.name().as_ref() == b"AltMetadataProperty" {
        option.alt_metadata_property = Some(ObjectExportMetadataProperty::from_xml_event(
            event,
            reader,
            state.options.max_attribute_bytes,
        )?);
    } else if event.name().as_ref() == b"ActualMetadataProperty" {
        option.actual_metadata_property = Some(ObjectExportMetadataProperty::from_xml_event(
            event,
            reader,
            state.options.max_attribute_bytes,
        )?);
    }
    Ok(())
}

fn parse_spread_list_property_contents(
    reader: &mut Reader<&[u8]>,
    mut property: SpreadListProperty,
    end_tag: &'static str,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<SpreadListProperty> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                if e.name().as_ref() == b"ListItem" {
                    let item = SpreadListItem::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    push_spread_list_item_limited(
                        &mut property.items,
                        parse_spread_list_item_contents(reader, item, state)?,
                    )?;
                }
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                if e.name().as_ref() == b"ListItem" {
                    push_spread_list_item_limited(
                        &mut property.items,
                        SpreadListItem::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?,
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
                    reason: "unclosed spread list property",
                });
            }
            _ => {}
        }
    }
    Ok(property)
}

fn parse_spread_list_item_contents(
    reader: &mut Reader<&[u8]>,
    mut item: SpreadListItem,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<SpreadListItem> {
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                state.root.enter(e.name().as_ref())?;
                state.xml_depth.enter()?;
                return Err(IdmlError::InvalidElement {
                    element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    reason: "nested spread list item",
                });
            }
            Event::Empty(e) => {
                state.root.visit_empty(e.name().as_ref())?;
                state.xml_depth.visit_empty()?;
                return Err(IdmlError::InvalidElement {
                    element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    reason: "nested spread list item",
                });
            }
            Event::End(e) => {
                state.xml_depth.exit()?;
                state.root.exit(e.name().as_ref())?;
                if e.name().as_ref() == b"ListItem" {
                    break;
                }
            }
            Event::Text(e) => {
                state.root.visit_text(e.as_ref())?;
                let decoded = e.decode()?;
                push_spread_list_item_value_limited(
                    &mut item.value,
                    decoded.as_ref(),
                    state.options.max_attribute_bytes,
                )?;
            }
            Event::CData(e) => {
                state.root.visit_cdata()?;
                let decoded = e.xml_content(XmlVersion::Implicit1_0)?;
                push_spread_list_item_value_limited(
                    &mut item.value,
                    decoded.as_ref(),
                    state.options.max_attribute_bytes,
                )?;
            }
            Event::GeneralRef(_) => {
                state.root.visit_reference()?;
                reject_xml_entity_reference()?;
            }
            Event::DocType(_) => reject_xml_doctype()?,
            Event::Eof => {
                return Err(IdmlError::InvalidElement {
                    element: "ListItem".to_owned(),
                    reason: "unclosed spread list item",
                });
            }
            _ => {}
        }
    }
    Ok(item)
}

fn push_spread_list_item_limited(
    items: &mut Vec<SpreadListItem>,
    item: SpreadListItem,
) -> Result<()> {
    let next_len = items.len().checked_add(1).ok_or(IdmlError::LimitExceeded {
        what: "spread list item count",
        limit: DEFAULT_MAX_SPREAD_LIST_ITEMS as u64,
        actual: u64::MAX,
    })?;
    if next_len > DEFAULT_MAX_SPREAD_LIST_ITEMS {
        return Err(IdmlError::LimitExceeded {
            what: "spread list item count",
            limit: DEFAULT_MAX_SPREAD_LIST_ITEMS as u64,
            actual: next_len as u64,
        });
    }
    items.push(item);
    Ok(())
}

fn push_spread_list_item_value_limited(
    output: &mut String,
    text: &str,
    max_attribute_bytes: usize,
) -> Result<()> {
    let next_len = output
        .len()
        .checked_add(text.len())
        .ok_or(IdmlError::LimitExceeded {
            what: "Spread XML attribute bytes",
            limit: max_attribute_bytes as u64,
            actual: u64::MAX,
        })?;
    if next_len > max_attribute_bytes {
        return Err(IdmlError::LimitExceeded {
            what: "Spread XML attribute bytes",
            limit: max_attribute_bytes as u64,
            actual: next_len as u64,
        });
    }
    output.push_str(text);
    Ok(())
}

fn parse_oval_contents(
    reader: &mut Reader<&[u8]>,
    mut oval: Oval,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<Oval> {
    parse_page_item_path_contents(
        reader,
        "Oval",
        state,
        &mut oval.path_geometry,
        &mut oval.metadata,
    )?;
    Ok(oval)
}

fn parse_graphic_line_contents(
    reader: &mut Reader<&[u8]>,
    mut line: GraphicLine,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<GraphicLine> {
    parse_page_item_path_contents(
        reader,
        "GraphicLine",
        state,
        &mut line.path_geometry,
        &mut line.metadata,
    )?;
    Ok(line)
}

fn parse_polygon_contents(
    reader: &mut Reader<&[u8]>,
    mut polygon: Polygon,
    state: &mut NestedSpreadParseState<'_>,
) -> Result<Polygon> {
    parse_page_item_path_contents(
        reader,
        "Polygon",
        state,
        &mut polygon.path_geometry,
        &mut polygon.metadata,
    )?;
    Ok(polygon)
}

fn parse_page_item_path_contents(
    reader: &mut Reader<&[u8]>,
    end_tag: &'static str,
    state: &mut NestedSpreadParseState<'_>,
    path_geometry: &mut Option<PathGeometry>,
    metadata: &mut PageItemMetadata,
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
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    let preference = TextWrapPreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    metadata.text_wrap_preference = Some(parse_text_wrap_preference_contents(
                        reader, preference, state,
                    )?);
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    metadata.frame_fitting_option = Some(FrameFittingOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                    parse_frame_fitting_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                    parse_incopy_export_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    let option = ObjectExportOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    metadata.object_export_option =
                        Some(parse_object_export_option_contents(reader, option, state)?);
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
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    metadata.text_wrap_preference = Some(TextWrapPreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    metadata.frame_fitting_option = Some(FrameFittingOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    metadata.object_export_option = Some(ObjectExportOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
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
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    let preference = TextWrapPreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    rectangle.metadata.text_wrap_preference = Some(
                        parse_text_wrap_preference_contents(reader, preference, state)?,
                    );
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    rectangle.metadata.frame_fitting_option =
                        Some(FrameFittingOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                    parse_frame_fitting_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    rectangle.metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                    parse_incopy_export_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    let option = ObjectExportOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    rectangle.metadata.object_export_option =
                        Some(parse_object_export_option_contents(reader, option, state)?);
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
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    rectangle.metadata.text_wrap_preference =
                        Some(TextWrapPreference::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    rectangle.metadata.frame_fitting_option =
                        Some(FrameFittingOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    rectangle.metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    rectangle.metadata.object_export_option =
                        Some(ObjectExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
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
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    let preference = TextWrapPreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    group.metadata.text_wrap_preference = Some(
                        parse_text_wrap_preference_contents(reader, preference, state)?,
                    );
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    group.metadata.frame_fitting_option = Some(FrameFittingOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                    parse_frame_fitting_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    group.metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                    parse_incopy_export_option_contents(reader, state)?;
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    let option = ObjectExportOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?;
                    group.metadata.object_export_option =
                        Some(parse_object_export_option_contents(reader, option, state)?);
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
                } else if e.name().as_ref() == b"TextWrapPreference" {
                    group.metadata.text_wrap_preference = Some(TextWrapPreference::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"FrameFittingOption" {
                    group.metadata.frame_fitting_option = Some(FrameFittingOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
                } else if e.name().as_ref() == b"InCopyExportOption" {
                    group.metadata.incopy_export_option =
                        Some(PageItemInCopyExportOption::from_xml_event(
                            &e,
                            reader,
                            state.options.max_attribute_bytes,
                        )?);
                } else if e.name().as_ref() == b"ObjectExportOption" {
                    group.metadata.object_export_option = Some(ObjectExportOption::from_xml_event(
                        &e,
                        reader,
                        state.options.max_attribute_bytes,
                    )?);
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
    validate_optional_attr(
        "TextFrame",
        "PreviousTextFrame",
        frame.previous_text_frame.as_deref(),
    )?;
    validate_optional_attr(
        "TextFrame",
        "NextTextFrame",
        frame.next_text_frame.as_deref(),
    )?;
    validate_page_item_metadata("TextFrame", &frame.metadata)?;
    if let Some(bounds) = frame.geometric_bounds {
        validate_finite_bounds("TextFrame", bounds)?;
    }
    if let Some(path_geometry) = &frame.path_geometry {
        validate_path_geometry(path_geometry)?;
    }
    if let Some(preference) = &frame.preference {
        validate_text_frame_preference(preference)?;
    }
    if let Some(options) = &frame.footnote_options {
        validate_text_frame_footnote_options(options)?;
    }
    Ok(())
}

fn validate_text_frame_footnote_options(options: &TextFrameFootnoteOptions) -> Result<()> {
    validate_optional_attr(
        "TextFrameFootnoteOptionsObject",
        "MinimumSpacingOption",
        options.minimum_spacing_option.as_deref(),
    )?;
    validate_optional_attr(
        "TextFrameFootnoteOptionsObject",
        "SpaceBetweenFootnotes",
        options.space_between_footnotes.as_deref(),
    )?;
    Ok(())
}

fn validate_text_frame_preference(preference: &TextFramePreference) -> Result<()> {
    for (name, value) in [
        ("TextColumnCount", preference.text_column_count.as_deref()),
        ("TextColumnGutter", preference.text_column_gutter.as_deref()),
        (
            "TextColumnFixedWidth",
            preference.text_column_fixed_width.as_deref(),
        ),
        (
            "TextColumnMaxWidth",
            preference.text_column_max_width.as_deref(),
        ),
        (
            "FirstBaselineOffset",
            preference.first_baseline_offset.as_deref(),
        ),
        (
            "MinimumFirstBaselineOffset",
            preference.minimum_first_baseline_offset.as_deref(),
        ),
        (
            "VerticalJustification",
            preference.vertical_justification.as_deref(),
        ),
        (
            "VerticalThreshold",
            preference.vertical_threshold.as_deref(),
        ),
        ("AutoSizingType", preference.auto_sizing_type.as_deref()),
        (
            "AutoSizingReferencePoint",
            preference.auto_sizing_reference_point.as_deref(),
        ),
        (
            "MinimumHeightForAutoSizing",
            preference.minimum_height_for_auto_sizing.as_deref(),
        ),
        (
            "MinimumWidthForAutoSizing",
            preference.minimum_width_for_auto_sizing.as_deref(),
        ),
        (
            "FootnotesMinimumSpacing",
            preference.footnotes_minimum_spacing.as_deref(),
        ),
        (
            "FootnotesSpaceBetween",
            preference.footnotes_space_between.as_deref(),
        ),
        ("ColumnRuleOffset", preference.column_rule_offset.as_deref()),
        (
            "ColumnRuleTopInset",
            preference.column_rule_top_inset.as_deref(),
        ),
        (
            "ColumnRuleBottomInset",
            preference.column_rule_bottom_inset.as_deref(),
        ),
        (
            "ColumnRuleStrokeWidth",
            preference.column_rule_stroke_width.as_deref(),
        ),
        (
            "ColumnRuleStrokeColor",
            preference.column_rule_stroke_color.as_deref(),
        ),
        (
            "ColumnRuleStrokeType",
            preference.column_rule_stroke_type.as_deref(),
        ),
        (
            "ColumnRuleStrokeTint",
            preference.column_rule_stroke_tint.as_deref(),
        ),
    ] {
        validate_optional_attr("TextFramePreference", name, value)?;
    }
    if let Some(inset_spacing) = &preference.inset_spacing {
        validate_spread_list_property("InsetSpacing", inset_spacing)?;
    }
    Ok(())
}

fn validate_spread_list_property(
    element: &'static str,
    property: &SpreadListProperty,
) -> Result<()> {
    validate_optional_attr(element, "type", property.value_type.as_deref())?;
    enforce_serialized_spread_count(
        property.items.len(),
        DEFAULT_MAX_SPREAD_LIST_ITEMS,
        "spread list item count",
    )?;
    for item in &property.items {
        validate_spread_list_item(item)?;
    }
    Ok(())
}

fn validate_spread_list_item(item: &SpreadListItem) -> Result<()> {
    validate_optional_attr("ListItem", "type", item.value_type.as_deref())?;
    enforce_spread_attribute_bytes(item.value.len(), DEFAULT_MAX_SPREAD_ATTRIBUTE_BYTES)?;
    validate_xml_text("spread list item value", &item.value)?;
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
    validate_page_item_metadata("Rectangle", &rectangle.metadata)?;
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
    validate_page_item_metadata("Oval", &oval.metadata)?;
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
    validate_page_item_metadata("GraphicLine", &line.metadata)?;
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
    validate_page_item_metadata("Polygon", &polygon.metadata)?;
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
    validate_page_item_metadata("Group", &group.metadata)?;
    if let Some(bounds) = group.geometric_bounds {
        validate_finite_bounds("Group", bounds)?;
    }
    for child in &group.page_items {
        validate_page_item(child)?;
    }
    Ok(())
}

fn validate_page_item_metadata(element: &'static str, metadata: &PageItemMetadata) -> Result<()> {
    validate_optional_attr(element, "ContentType", metadata.content_type.as_deref())?;
    validate_optional_attr(element, "Name", metadata.name.as_deref())?;
    validate_optional_attr(element, "ItemTransform", metadata.item_transform.as_deref())?;
    validate_optional_attr(
        element,
        "AppliedObjectStyle",
        metadata.applied_object_style.as_deref(),
    )?;
    validate_optional_attr(element, "FillColor", metadata.fill_color.as_deref())?;
    validate_optional_attr(element, "StrokeColor", metadata.stroke_color.as_deref())?;
    validate_optional_attr(element, "StrokeWeight", metadata.stroke_weight.as_deref())?;
    validate_optional_attr(element, "FillTint", metadata.fill_tint.as_deref())?;
    validate_optional_attr(element, "StrokeTint", metadata.stroke_tint.as_deref())?;
    validate_optional_attr(element, "GapColor", metadata.gap_color.as_deref())?;
    validate_optional_attr(element, "GapTint", metadata.gap_tint.as_deref())?;
    validate_optional_attr(element, "StrokeType", metadata.stroke_type.as_deref())?;
    validate_optional_attr(
        element,
        "StrokeAlignment",
        metadata.stroke_alignment.as_deref(),
    )?;
    validate_optional_attr(element, "MiterLimit", metadata.miter_limit.as_deref())?;
    validate_optional_attr(element, "EndCap", metadata.end_cap.as_deref())?;
    validate_optional_attr(element, "EndJoin", metadata.end_join.as_deref())?;
    validate_optional_attr(element, "LeftLineEnd", metadata.left_line_end.as_deref())?;
    validate_optional_attr(element, "RightLineEnd", metadata.right_line_end.as_deref())?;
    validate_optional_attr(element, "LockState", metadata.lock_state.as_deref())?;
    validate_optional_attr(element, "StoryTitle", metadata.story_title.as_deref())?;
    validate_optional_attr(
        element,
        "FlexItemWidthMode",
        metadata.flex_item_width_mode.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "FlexItemHeightMode",
        metadata.flex_item_height_mode.as_deref(),
    )?;
    validate_optional_attr(element, "CornerRadius", metadata.corner_radius.as_deref())?;
    validate_optional_attr(element, "CornerOption", metadata.corner_option.as_deref())?;
    validate_optional_attr(
        element,
        "TopLeftCornerOption",
        metadata.top_left_corner_option.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "TopRightCornerOption",
        metadata.top_right_corner_option.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "BottomLeftCornerOption",
        metadata.bottom_left_corner_option.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "BottomRightCornerOption",
        metadata.bottom_right_corner_option.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "TopLeftCornerRadius",
        metadata.top_left_corner_radius.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "TopRightCornerRadius",
        metadata.top_right_corner_radius.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "BottomLeftCornerRadius",
        metadata.bottom_left_corner_radius.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "BottomRightCornerRadius",
        metadata.bottom_right_corner_radius.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "ParentInterfaceChangeCount",
        metadata.parent_interface_change_count.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "TargetInterfaceChangeCount",
        metadata.target_interface_change_count.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "LastUpdatedInterfaceChangeCount",
        metadata.last_updated_interface_change_count.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "OverriddenPageItemProps",
        metadata.overridden_page_item_props.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "HorizontalLayoutConstraints",
        metadata.horizontal_layout_constraints.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "VerticalLayoutConstraints",
        metadata.vertical_layout_constraints.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientFillStart",
        metadata.gradient_fill_start.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientFillLength",
        metadata.gradient_fill_length.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientFillAngle",
        metadata.gradient_fill_angle.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientStrokeStart",
        metadata.gradient_stroke_start.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientStrokeLength",
        metadata.gradient_stroke_length.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientStrokeAngle",
        metadata.gradient_stroke_angle.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "LocalDisplaySetting",
        metadata.local_display_setting.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientFillHiliteLength",
        metadata.gradient_fill_hilite_length.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientFillHiliteAngle",
        metadata.gradient_fill_hilite_angle.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientStrokeHiliteLength",
        metadata.gradient_stroke_hilite_length.as_deref(),
    )?;
    validate_optional_attr(
        element,
        "GradientStrokeHiliteAngle",
        metadata.gradient_stroke_hilite_angle.as_deref(),
    )?;
    if let Some(preference) = &metadata.text_wrap_preference {
        validate_text_wrap_preference(preference)?;
    }
    if let Some(option) = &metadata.frame_fitting_option {
        validate_frame_fitting_option(option)?;
    }
    if let Some(option) = &metadata.object_export_option {
        validate_object_export_option(option)?;
    }
    Ok(())
}

fn validate_text_wrap_preference(preference: &TextWrapPreference) -> Result<()> {
    validate_optional_attr(
        "TextWrapPreference",
        "TextWrapSide",
        preference.text_wrap_side.as_deref(),
    )?;
    validate_optional_attr(
        "TextWrapPreference",
        "TextWrapMode",
        preference.text_wrap_mode.as_deref(),
    )?;
    if let Some(offset) = &preference.text_wrap_offset {
        validate_text_wrap_offset(offset)?;
    }
    if let Some(option) = &preference.contour_option {
        validate_contour_option(option)?;
    }
    Ok(())
}

fn validate_text_wrap_offset(offset: &TextWrapOffset) -> Result<()> {
    validate_optional_attr("TextWrapOffset", "Top", offset.top.as_deref())?;
    validate_optional_attr("TextWrapOffset", "Left", offset.left.as_deref())?;
    validate_optional_attr("TextWrapOffset", "Bottom", offset.bottom.as_deref())?;
    validate_optional_attr("TextWrapOffset", "Right", offset.right.as_deref())?;
    Ok(())
}

fn validate_contour_option(option: &ContourOption) -> Result<()> {
    validate_optional_attr(
        "ContourOption",
        "ContourType",
        option.contour_type.as_deref(),
    )?;
    validate_optional_attr(
        "ContourOption",
        "ContourPathName",
        option.contour_path_name.as_deref(),
    )?;
    Ok(())
}

fn validate_frame_fitting_option(option: &FrameFittingOption) -> Result<()> {
    validate_optional_attr(
        "FrameFittingOption",
        "LeftCrop",
        option.left_crop.as_deref(),
    )?;
    validate_optional_attr("FrameFittingOption", "TopCrop", option.top_crop.as_deref())?;
    validate_optional_attr(
        "FrameFittingOption",
        "RightCrop",
        option.right_crop.as_deref(),
    )?;
    validate_optional_attr(
        "FrameFittingOption",
        "BottomCrop",
        option.bottom_crop.as_deref(),
    )?;
    validate_optional_attr(
        "FrameFittingOption",
        "FittingOnEmptyFrame",
        option.fitting_on_empty_frame.as_deref(),
    )?;
    validate_optional_attr(
        "FrameFittingOption",
        "FittingAlignment",
        option.fitting_alignment.as_deref(),
    )?;
    Ok(())
}

fn validate_object_export_option(option: &ObjectExportOption) -> Result<()> {
    for (name, value) in [
        ("EpubType", option.epub_type.as_deref()),
        ("EpubAriaRole", option.epub_aria_role.as_deref()),
        ("SizeType", option.size_type.as_deref()),
        ("CustomSize", option.custom_size.as_deref()),
        (
            "PreserveAppearanceFromLayout",
            option.preserve_appearance_from_layout.as_deref(),
        ),
        ("AltTextSourceType", option.alt_text_source_type.as_deref()),
        (
            "ActualTextSourceType",
            option.actual_text_source_type.as_deref(),
        ),
        ("CustomAltText", option.custom_alt_text.as_deref()),
        ("CustomActualText", option.custom_actual_text.as_deref()),
        ("ApplyTagType", option.apply_tag_type.as_deref()),
        (
            "ImageConversionType",
            option.image_conversion_type.as_deref(),
        ),
        (
            "CustomImageSizeOption",
            option.custom_image_size_option.as_deref(),
        ),
        (
            "ImageExportResolution",
            option.image_export_resolution.as_deref(),
        ),
        ("GIFOptionsPalette", option.gif_options_palette.as_deref()),
        ("JPEGOptionsQuality", option.jpeg_options_quality.as_deref()),
        ("JPEGOptionsFormat", option.jpeg_options_format.as_deref()),
        ("ImageAlignment", option.image_alignment.as_deref()),
        ("ImageSpaceBefore", option.image_space_before.as_deref()),
        ("ImageSpaceAfter", option.image_space_after.as_deref()),
        ("ImagePageBreak", option.image_page_break.as_deref()),
        ("SpaceUnit", option.space_unit.as_deref()),
        ("CustomLayoutType", option.custom_layout_type.as_deref()),
    ] {
        validate_optional_attr("ObjectExportOption", name, value)?;
    }
    if let Some(property) = &option.alt_metadata_property {
        validate_object_export_metadata_property("AltMetadataProperty", property)?;
    }
    if let Some(property) = &option.actual_metadata_property {
        validate_object_export_metadata_property("ActualMetadataProperty", property)?;
    }
    Ok(())
}

fn validate_object_export_metadata_property(
    element: &'static str,
    property: &ObjectExportMetadataProperty,
) -> Result<()> {
    validate_optional_attr(
        element,
        "NamespacePrefix",
        property.namespace_prefix.as_deref(),
    )?;
    validate_optional_attr(element, "PropertyPath", property.property_path.as_deref())?;
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
    if let Some(previous_text_frame) = &frame.previous_text_frame {
        push_attr(xml, "TextFrame", "PreviousTextFrame", previous_text_frame)?;
    }
    if let Some(next_text_frame) = &frame.next_text_frame {
        push_attr(xml, "TextFrame", "NextTextFrame", next_text_frame)?;
    }
    push_page_item_metadata(xml, "TextFrame", &frame.metadata)?;
    if let Some(bounds) = frame.geometric_bounds {
        let bounds = format_geometric_bounds("TextFrame", bounds)?;
        push_attr(xml, "TextFrame", "GeometricBounds", &bounds)?;
    }
    if frame.path_geometry.is_some()
        || frame.preference.is_some()
        || frame.footnote_options.is_some()
        || frame.metadata.text_wrap_preference.is_some()
        || frame.metadata.frame_fitting_option.is_some()
        || frame.metadata.incopy_export_option.is_some()
        || frame.metadata.object_export_option.is_some()
    {
        xml.push_str(">\n");
        if let Some(path_geometry) = &frame.path_geometry {
            serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        }
        if let Some(preference) = &frame.preference {
            serialize_text_frame_preference(xml, preference, indent + 2)?;
        }
        if let Some(options) = &frame.footnote_options {
            serialize_text_frame_footnote_options(xml, options, indent + 2)?;
        }
        serialize_page_item_metadata_children(xml, &frame.metadata, indent + 2)?;
        push_indent(xml, indent);
        xml.push_str("</TextFrame>\n");
        return Ok(());
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_text_frame_footnote_options(
    xml: &mut String,
    options: &TextFrameFootnoteOptions,
    indent: usize,
) -> Result<()> {
    validate_text_frame_footnote_options(options)?;
    push_indent(xml, indent);
    xml.push_str("<TextFrameFootnoteOptionsObject");
    if let Some(enable_overrides) = options.enable_overrides {
        push_attr(
            xml,
            "TextFrameFootnoteOptionsObject",
            "EnableOverrides",
            bool_str(enable_overrides),
        )?;
    }
    if let Some(span_footnotes_across) = options.span_footnotes_across {
        push_attr(
            xml,
            "TextFrameFootnoteOptionsObject",
            "SpanFootnotesAcross",
            bool_str(span_footnotes_across),
        )?;
    }
    push_optional_serialized_attr(
        xml,
        "TextFrameFootnoteOptionsObject",
        "MinimumSpacingOption",
        options.minimum_spacing_option.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFrameFootnoteOptionsObject",
        "SpaceBetweenFootnotes",
        options.space_between_footnotes.as_deref(),
    )?;
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_text_frame_preference(
    xml: &mut String,
    preference: &TextFramePreference,
    indent: usize,
) -> Result<()> {
    validate_text_frame_preference(preference)?;
    push_indent(xml, indent);
    xml.push_str("<TextFramePreference");
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "TextColumnCount",
        preference.text_column_count.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "TextColumnGutter",
        preference.text_column_gutter.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "TextColumnFixedWidth",
        preference.text_column_fixed_width.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "TextColumnMaxWidth",
        preference.text_column_max_width.as_deref(),
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "UseFixedColumnWidth",
        preference.use_fixed_column_width,
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "FirstBaselineOffset",
        preference.first_baseline_offset.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "MinimumFirstBaselineOffset",
        preference.minimum_first_baseline_offset.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "VerticalJustification",
        preference.vertical_justification.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "VerticalThreshold",
        preference.vertical_threshold.as_deref(),
    )?;
    push_optional_bool_serialized_attr(xml, "IgnoreWrap", preference.ignore_wrap)?;
    push_optional_bool_serialized_attr(
        xml,
        "UseFlexibleColumnWidth",
        preference.use_flexible_column_width,
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "AutoSizingType",
        preference.auto_sizing_type.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "AutoSizingReferencePoint",
        preference.auto_sizing_reference_point.as_deref(),
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "UseMinimumHeightForAutoSizing",
        preference.use_minimum_height_for_auto_sizing,
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "MinimumHeightForAutoSizing",
        preference.minimum_height_for_auto_sizing.as_deref(),
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "UseMinimumWidthForAutoSizing",
        preference.use_minimum_width_for_auto_sizing,
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "MinimumWidthForAutoSizing",
        preference.minimum_width_for_auto_sizing.as_deref(),
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "UseNoLineBreaksForAutoSizing",
        preference.use_no_line_breaks_for_auto_sizing,
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "FootnotesEnableOverrides",
        preference.footnotes_enable_overrides,
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "FootnotesSpanAcrossColumns",
        preference.footnotes_span_across_columns,
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "FootnotesMinimumSpacing",
        preference.footnotes_minimum_spacing.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "FootnotesSpaceBetween",
        preference.footnotes_space_between.as_deref(),
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "VerticalBalanceColumns",
        preference.vertical_balance_columns,
    )?;
    push_optional_bool_serialized_attr(xml, "ColumnRuleOverride", preference.column_rule_override)?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "ColumnRuleOffset",
        preference.column_rule_offset.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "ColumnRuleTopInset",
        preference.column_rule_top_inset.as_deref(),
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "ColumnRuleInsetChainOverride",
        preference.column_rule_inset_chain_override,
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "ColumnRuleBottomInset",
        preference.column_rule_bottom_inset.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "ColumnRuleStrokeWidth",
        preference.column_rule_stroke_width.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "ColumnRuleStrokeColor",
        preference.column_rule_stroke_color.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "ColumnRuleStrokeType",
        preference.column_rule_stroke_type.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextFramePreference",
        "ColumnRuleStrokeTint",
        preference.column_rule_stroke_tint.as_deref(),
    )?;
    push_optional_bool_serialized_attr(
        xml,
        "ColumnRuleOverprintOverride",
        preference.column_rule_overprint_override,
    )?;

    let Some(inset_spacing) = &preference.inset_spacing else {
        xml.push_str(" />\n");
        return Ok(());
    };

    xml.push_str(">\n");
    push_indent(xml, indent + 2);
    xml.push_str("<Properties>\n");
    serialize_spread_list_property(xml, "InsetSpacing", inset_spacing, indent + 4)?;
    push_indent(xml, indent + 2);
    xml.push_str("</Properties>\n");
    push_indent(xml, indent);
    xml.push_str("</TextFramePreference>\n");
    Ok(())
}

fn serialize_spread_list_property(
    xml: &mut String,
    element: &'static str,
    property: &SpreadListProperty,
    indent: usize,
) -> Result<()> {
    validate_spread_list_property(element, property)?;
    push_indent(xml, indent);
    xml.push('<');
    xml.push_str(element);
    push_optional_serialized_attr(xml, element, "type", property.value_type.as_deref())?;
    if property.items.is_empty() {
        xml.push_str(" />\n");
        return Ok(());
    }
    xml.push_str(">\n");
    for item in &property.items {
        serialize_spread_list_item(xml, item, indent + 2)?;
    }
    push_indent(xml, indent);
    xml.push_str("</");
    xml.push_str(element);
    xml.push_str(">\n");
    Ok(())
}

fn serialize_spread_list_item(
    xml: &mut String,
    item: &SpreadListItem,
    indent: usize,
) -> Result<()> {
    validate_spread_list_item(item)?;
    push_indent(xml, indent);
    xml.push_str("<ListItem");
    push_optional_serialized_attr(xml, "ListItem", "type", item.value_type.as_deref())?;
    if item.value.is_empty() {
        xml.push_str(" />\n");
        return Ok(());
    }
    xml.push('>');
    xml.push_str(escape(&item.value).as_ref());
    xml.push_str("</ListItem>\n");
    Ok(())
}

fn push_optional_serialized_attr(
    xml: &mut String,
    element: &'static str,
    name: &'static str,
    value: Option<&str>,
) -> Result<()> {
    if let Some(value) = value {
        push_attr(xml, element, name, value)?;
    }
    Ok(())
}

fn push_optional_bool_serialized_attr(
    xml: &mut String,
    name: &'static str,
    value: Option<bool>,
) -> Result<()> {
    if let Some(value) = value {
        push_attr(xml, "TextFramePreference", name, bool_str(value))?;
    }
    Ok(())
}

fn serialize_page_item_metadata_children(
    xml: &mut String,
    metadata: &PageItemMetadata,
    indent: usize,
) -> Result<()> {
    if let Some(preference) = &metadata.text_wrap_preference {
        serialize_text_wrap_preference(xml, preference, indent)?;
    }
    if let Some(option) = &metadata.frame_fitting_option {
        serialize_frame_fitting_option(xml, option, indent)?;
    }
    if let Some(option) = &metadata.incopy_export_option {
        serialize_incopy_export_option(xml, option, indent)?;
    }
    if let Some(option) = &metadata.object_export_option {
        serialize_object_export_option(xml, option, indent)?;
    }
    Ok(())
}

fn serialize_text_wrap_preference(
    xml: &mut String,
    preference: &TextWrapPreference,
    indent: usize,
) -> Result<()> {
    validate_text_wrap_preference(preference)?;
    push_indent(xml, indent);
    xml.push_str("<TextWrapPreference");
    if let Some(inverse) = preference.inverse {
        push_attr(xml, "TextWrapPreference", "Inverse", bool_str(inverse))?;
    }
    if let Some(apply_to_master_page_only) = preference.apply_to_master_page_only {
        push_attr(
            xml,
            "TextWrapPreference",
            "ApplyToMasterPageOnly",
            bool_str(apply_to_master_page_only),
        )?;
    }
    push_optional_serialized_attr(
        xml,
        "TextWrapPreference",
        "TextWrapSide",
        preference.text_wrap_side.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "TextWrapPreference",
        "TextWrapMode",
        preference.text_wrap_mode.as_deref(),
    )?;

    if preference.text_wrap_offset.is_none() && preference.contour_option.is_none() {
        xml.push_str(" />\n");
        return Ok(());
    }

    xml.push_str(">\n");
    if let Some(offset) = &preference.text_wrap_offset {
        push_indent(xml, indent + 2);
        xml.push_str("<Properties>\n");
        serialize_text_wrap_offset(xml, offset, indent + 4)?;
        push_indent(xml, indent + 2);
        xml.push_str("</Properties>\n");
    }
    if let Some(option) = &preference.contour_option {
        serialize_contour_option(xml, option, indent + 2)?;
    }
    push_indent(xml, indent);
    xml.push_str("</TextWrapPreference>\n");
    Ok(())
}

fn serialize_text_wrap_offset(
    xml: &mut String,
    offset: &TextWrapOffset,
    indent: usize,
) -> Result<()> {
    validate_text_wrap_offset(offset)?;
    push_indent(xml, indent);
    xml.push_str("<TextWrapOffset");
    push_optional_serialized_attr(xml, "TextWrapOffset", "Top", offset.top.as_deref())?;
    push_optional_serialized_attr(xml, "TextWrapOffset", "Left", offset.left.as_deref())?;
    push_optional_serialized_attr(xml, "TextWrapOffset", "Bottom", offset.bottom.as_deref())?;
    push_optional_serialized_attr(xml, "TextWrapOffset", "Right", offset.right.as_deref())?;
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_contour_option(xml: &mut String, option: &ContourOption, indent: usize) -> Result<()> {
    validate_contour_option(option)?;
    push_indent(xml, indent);
    xml.push_str("<ContourOption");
    push_optional_serialized_attr(
        xml,
        "ContourOption",
        "ContourType",
        option.contour_type.as_deref(),
    )?;
    if let Some(include_inside_edges) = option.include_inside_edges {
        push_attr(
            xml,
            "ContourOption",
            "IncludeInsideEdges",
            bool_str(include_inside_edges),
        )?;
    }
    push_optional_serialized_attr(
        xml,
        "ContourOption",
        "ContourPathName",
        option.contour_path_name.as_deref(),
    )?;
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_frame_fitting_option(
    xml: &mut String,
    option: &FrameFittingOption,
    indent: usize,
) -> Result<()> {
    validate_frame_fitting_option(option)?;
    push_indent(xml, indent);
    xml.push_str("<FrameFittingOption");
    if let Some(auto_fit) = option.auto_fit {
        push_attr(xml, "FrameFittingOption", "AutoFit", bool_str(auto_fit))?;
    }
    push_optional_serialized_attr(
        xml,
        "FrameFittingOption",
        "LeftCrop",
        option.left_crop.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "FrameFittingOption",
        "TopCrop",
        option.top_crop.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "FrameFittingOption",
        "RightCrop",
        option.right_crop.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "FrameFittingOption",
        "BottomCrop",
        option.bottom_crop.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "FrameFittingOption",
        "FittingOnEmptyFrame",
        option.fitting_on_empty_frame.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "FrameFittingOption",
        "FittingAlignment",
        option.fitting_alignment.as_deref(),
    )?;
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_incopy_export_option(
    xml: &mut String,
    option: &PageItemInCopyExportOption,
    indent: usize,
) -> Result<()> {
    push_indent(xml, indent);
    xml.push_str("<InCopyExportOption");
    if let Some(include_graphic_proxies) = option.include_graphic_proxies {
        push_attr(
            xml,
            "InCopyExportOption",
            "IncludeGraphicProxies",
            bool_str(include_graphic_proxies),
        )?;
    }
    if let Some(include_all_resources) = option.include_all_resources {
        push_attr(
            xml,
            "InCopyExportOption",
            "IncludeAllResources",
            bool_str(include_all_resources),
        )?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_object_export_option(
    xml: &mut String,
    option: &ObjectExportOption,
    indent: usize,
) -> Result<()> {
    validate_object_export_option(option)?;
    push_indent(xml, indent);
    xml.push_str("<ObjectExportOption");
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "EpubType",
        option.epub_type.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "EpubAriaRole",
        option.epub_aria_role.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "SizeType",
        option.size_type.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "CustomSize",
        option.custom_size.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "PreserveAppearanceFromLayout",
        option.preserve_appearance_from_layout.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "AltTextSourceType",
        option.alt_text_source_type.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ActualTextSourceType",
        option.actual_text_source_type.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "CustomAltText",
        option.custom_alt_text.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "CustomActualText",
        option.custom_actual_text.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ApplyTagType",
        option.apply_tag_type.as_deref(),
    )?;
    push_optional_object_export_bool_attr(
        xml,
        "CustomImageConversion",
        option.custom_image_conversion,
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ImageConversionType",
        option.image_conversion_type.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "CustomImageSizeOption",
        option.custom_image_size_option.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ImageExportResolution",
        option.image_export_resolution.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "GIFOptionsPalette",
        option.gif_options_palette.as_deref(),
    )?;
    push_optional_object_export_bool_attr(
        xml,
        "GIFOptionsInterlaced",
        option.gif_options_interlaced,
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "JPEGOptionsQuality",
        option.jpeg_options_quality.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "JPEGOptionsFormat",
        option.jpeg_options_format.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ImageAlignment",
        option.image_alignment.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ImageSpaceBefore",
        option.image_space_before.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ImageSpaceAfter",
        option.image_space_after.as_deref(),
    )?;
    push_optional_object_export_bool_attr(xml, "UseImagePageBreak", option.use_image_page_break)?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "ImagePageBreak",
        option.image_page_break.as_deref(),
    )?;
    push_optional_object_export_bool_attr(
        xml,
        "CustomImageAlignment",
        option.custom_image_alignment,
    )?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "SpaceUnit",
        option.space_unit.as_deref(),
    )?;
    push_optional_object_export_bool_attr(xml, "CustomLayout", option.custom_layout)?;
    push_optional_serialized_attr(
        xml,
        "ObjectExportOption",
        "CustomLayoutType",
        option.custom_layout_type.as_deref(),
    )?;
    push_optional_object_export_bool_attr(xml, "UseOriginalImage", option.use_original_image)?;

    if option.alt_metadata_property.is_none() && option.actual_metadata_property.is_none() {
        xml.push_str(" />\n");
        return Ok(());
    }

    xml.push_str(">\n");
    push_indent(xml, indent + 2);
    xml.push_str("<Properties>\n");
    if let Some(property) = &option.alt_metadata_property {
        serialize_object_export_metadata_property(
            xml,
            "AltMetadataProperty",
            property,
            indent + 4,
        )?;
    }
    if let Some(property) = &option.actual_metadata_property {
        serialize_object_export_metadata_property(
            xml,
            "ActualMetadataProperty",
            property,
            indent + 4,
        )?;
    }
    push_indent(xml, indent + 2);
    xml.push_str("</Properties>\n");
    push_indent(xml, indent);
    xml.push_str("</ObjectExportOption>\n");
    Ok(())
}

fn push_optional_object_export_bool_attr(
    xml: &mut String,
    name: &'static str,
    value: Option<bool>,
) -> Result<()> {
    if let Some(value) = value {
        push_attr(xml, "ObjectExportOption", name, bool_str(value))?;
    }
    Ok(())
}

fn serialize_object_export_metadata_property(
    xml: &mut String,
    element: &'static str,
    property: &ObjectExportMetadataProperty,
    indent: usize,
) -> Result<()> {
    validate_object_export_metadata_property(element, property)?;
    push_indent(xml, indent);
    xml.push('<');
    xml.push_str(element);
    push_optional_serialized_attr(
        xml,
        element,
        "NamespacePrefix",
        property.namespace_prefix.as_deref(),
    )?;
    push_optional_serialized_attr(
        xml,
        element,
        "PropertyPath",
        property.property_path.as_deref(),
    )?;
    xml.push_str(" />\n");
    Ok(())
}

fn push_page_item_metadata(
    xml: &mut String,
    element: &'static str,
    metadata: &PageItemMetadata,
) -> Result<()> {
    if let Some(content_type) = &metadata.content_type {
        push_attr(xml, element, "ContentType", content_type)?;
    }
    if let Some(name) = &metadata.name {
        push_attr(xml, element, "Name", name)?;
    }
    if let Some(item_transform) = &metadata.item_transform {
        push_attr(xml, element, "ItemTransform", item_transform)?;
    }
    if let Some(applied_object_style) = &metadata.applied_object_style {
        push_attr(xml, element, "AppliedObjectStyle", applied_object_style)?;
    }
    if let Some(fill_color) = &metadata.fill_color {
        push_attr(xml, element, "FillColor", fill_color)?;
    }
    if let Some(stroke_color) = &metadata.stroke_color {
        push_attr(xml, element, "StrokeColor", stroke_color)?;
    }
    if let Some(stroke_weight) = &metadata.stroke_weight {
        push_attr(xml, element, "StrokeWeight", stroke_weight)?;
    }
    if let Some(visible) = metadata.visible {
        push_attr(xml, element, "Visible", bool_str(visible))?;
    }
    if let Some(locked) = metadata.locked {
        push_attr(xml, element, "Locked", bool_str(locked))?;
    }
    if let Some(fill_tint) = &metadata.fill_tint {
        push_attr(xml, element, "FillTint", fill_tint)?;
    }
    if let Some(stroke_tint) = &metadata.stroke_tint {
        push_attr(xml, element, "StrokeTint", stroke_tint)?;
    }
    if let Some(gap_color) = &metadata.gap_color {
        push_attr(xml, element, "GapColor", gap_color)?;
    }
    if let Some(gap_tint) = &metadata.gap_tint {
        push_attr(xml, element, "GapTint", gap_tint)?;
    }
    if let Some(stroke_type) = &metadata.stroke_type {
        push_attr(xml, element, "StrokeType", stroke_type)?;
    }
    if let Some(stroke_alignment) = &metadata.stroke_alignment {
        push_attr(xml, element, "StrokeAlignment", stroke_alignment)?;
    }
    if let Some(miter_limit) = &metadata.miter_limit {
        push_attr(xml, element, "MiterLimit", miter_limit)?;
    }
    if let Some(end_cap) = &metadata.end_cap {
        push_attr(xml, element, "EndCap", end_cap)?;
    }
    if let Some(end_join) = &metadata.end_join {
        push_attr(xml, element, "EndJoin", end_join)?;
    }
    if let Some(left_line_end) = &metadata.left_line_end {
        push_attr(xml, element, "LeftLineEnd", left_line_end)?;
    }
    if let Some(right_line_end) = &metadata.right_line_end {
        push_attr(xml, element, "RightLineEnd", right_line_end)?;
    }
    if let Some(lock_state) = &metadata.lock_state {
        push_attr(xml, element, "LockState", lock_state)?;
    }
    if let Some(story_title) = &metadata.story_title {
        push_attr(xml, element, "StoryTitle", story_title)?;
    }
    if let Some(flex_item_width_mode) = &metadata.flex_item_width_mode {
        push_attr(xml, element, "FlexItemWidthMode", flex_item_width_mode)?;
    }
    if let Some(flex_item_height_mode) = &metadata.flex_item_height_mode {
        push_attr(xml, element, "FlexItemHeightMode", flex_item_height_mode)?;
    }
    if let Some(nonprinting) = metadata.nonprinting {
        push_attr(xml, element, "Nonprinting", bool_str(nonprinting))?;
    }
    if let Some(overprint_fill) = metadata.overprint_fill {
        push_attr(xml, element, "OverprintFill", bool_str(overprint_fill))?;
    }
    if let Some(overprint_stroke) = metadata.overprint_stroke {
        push_attr(xml, element, "OverprintStroke", bool_str(overprint_stroke))?;
    }
    if let Some(corner_radius) = &metadata.corner_radius {
        push_attr(xml, element, "CornerRadius", corner_radius)?;
    }
    if let Some(corner_option) = &metadata.corner_option {
        push_attr(xml, element, "CornerOption", corner_option)?;
    }
    if let Some(top_left_corner_option) = &metadata.top_left_corner_option {
        push_attr(xml, element, "TopLeftCornerOption", top_left_corner_option)?;
    }
    if let Some(top_right_corner_option) = &metadata.top_right_corner_option {
        push_attr(
            xml,
            element,
            "TopRightCornerOption",
            top_right_corner_option,
        )?;
    }
    if let Some(bottom_left_corner_option) = &metadata.bottom_left_corner_option {
        push_attr(
            xml,
            element,
            "BottomLeftCornerOption",
            bottom_left_corner_option,
        )?;
    }
    if let Some(bottom_right_corner_option) = &metadata.bottom_right_corner_option {
        push_attr(
            xml,
            element,
            "BottomRightCornerOption",
            bottom_right_corner_option,
        )?;
    }
    if let Some(top_left_corner_radius) = &metadata.top_left_corner_radius {
        push_attr(xml, element, "TopLeftCornerRadius", top_left_corner_radius)?;
    }
    if let Some(top_right_corner_radius) = &metadata.top_right_corner_radius {
        push_attr(
            xml,
            element,
            "TopRightCornerRadius",
            top_right_corner_radius,
        )?;
    }
    if let Some(bottom_left_corner_radius) = &metadata.bottom_left_corner_radius {
        push_attr(
            xml,
            element,
            "BottomLeftCornerRadius",
            bottom_left_corner_radius,
        )?;
    }
    if let Some(bottom_right_corner_radius) = &metadata.bottom_right_corner_radius {
        push_attr(
            xml,
            element,
            "BottomRightCornerRadius",
            bottom_right_corner_radius,
        )?;
    }
    if let Some(parent_interface_change_count) = &metadata.parent_interface_change_count {
        push_attr(
            xml,
            element,
            "ParentInterfaceChangeCount",
            parent_interface_change_count,
        )?;
    }
    if let Some(target_interface_change_count) = &metadata.target_interface_change_count {
        push_attr(
            xml,
            element,
            "TargetInterfaceChangeCount",
            target_interface_change_count,
        )?;
    }
    if let Some(last_updated_interface_change_count) = &metadata.last_updated_interface_change_count
    {
        push_attr(
            xml,
            element,
            "LastUpdatedInterfaceChangeCount",
            last_updated_interface_change_count,
        )?;
    }
    if let Some(overridden_page_item_props) = &metadata.overridden_page_item_props {
        push_attr(
            xml,
            element,
            "OverriddenPageItemProps",
            overridden_page_item_props,
        )?;
    }
    if let Some(horizontal_layout_constraints) = &metadata.horizontal_layout_constraints {
        push_attr(
            xml,
            element,
            "HorizontalLayoutConstraints",
            horizontal_layout_constraints,
        )?;
    }
    if let Some(vertical_layout_constraints) = &metadata.vertical_layout_constraints {
        push_attr(
            xml,
            element,
            "VerticalLayoutConstraints",
            vertical_layout_constraints,
        )?;
    }
    if let Some(gradient_fill_start) = &metadata.gradient_fill_start {
        push_attr(xml, element, "GradientFillStart", gradient_fill_start)?;
    }
    if let Some(gradient_fill_length) = &metadata.gradient_fill_length {
        push_attr(xml, element, "GradientFillLength", gradient_fill_length)?;
    }
    if let Some(gradient_fill_angle) = &metadata.gradient_fill_angle {
        push_attr(xml, element, "GradientFillAngle", gradient_fill_angle)?;
    }
    if let Some(gradient_stroke_start) = &metadata.gradient_stroke_start {
        push_attr(xml, element, "GradientStrokeStart", gradient_stroke_start)?;
    }
    if let Some(gradient_stroke_length) = &metadata.gradient_stroke_length {
        push_attr(xml, element, "GradientStrokeLength", gradient_stroke_length)?;
    }
    if let Some(gradient_stroke_angle) = &metadata.gradient_stroke_angle {
        push_attr(xml, element, "GradientStrokeAngle", gradient_stroke_angle)?;
    }
    if let Some(local_display_setting) = &metadata.local_display_setting {
        push_attr(xml, element, "LocalDisplaySetting", local_display_setting)?;
    }
    if let Some(gradient_fill_hilite_length) = &metadata.gradient_fill_hilite_length {
        push_attr(
            xml,
            element,
            "GradientFillHiliteLength",
            gradient_fill_hilite_length,
        )?;
    }
    if let Some(gradient_fill_hilite_angle) = &metadata.gradient_fill_hilite_angle {
        push_attr(
            xml,
            element,
            "GradientFillHiliteAngle",
            gradient_fill_hilite_angle,
        )?;
    }
    if let Some(gradient_stroke_hilite_length) = &metadata.gradient_stroke_hilite_length {
        push_attr(
            xml,
            element,
            "GradientStrokeHiliteLength",
            gradient_stroke_hilite_length,
        )?;
    }
    if let Some(gradient_stroke_hilite_angle) = &metadata.gradient_stroke_hilite_angle {
        push_attr(
            xml,
            element,
            "GradientStrokeHiliteAngle",
            gradient_stroke_hilite_angle,
        )?;
    }
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
    push_page_item_metadata(xml, "Rectangle", &rectangle.metadata)?;
    if let Some(bounds) = rectangle.geometric_bounds {
        let bounds = format_geometric_bounds("Rectangle", bounds)?;
        push_attr(xml, "Rectangle", "GeometricBounds", &bounds)?;
    }
    if rectangle.path_geometry.is_none()
        && rectangle.images.is_empty()
        && rectangle.metadata.text_wrap_preference.is_none()
        && rectangle.metadata.frame_fitting_option.is_none()
        && rectangle.metadata.incopy_export_option.is_none()
        && rectangle.metadata.object_export_option.is_none()
    {
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
    serialize_page_item_metadata_children(xml, &rectangle.metadata, indent + 2)?;
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
    push_page_item_metadata(xml, "Oval", &oval.metadata)?;
    if let Some(bounds) = oval.geometric_bounds {
        let bounds = format_geometric_bounds("Oval", bounds)?;
        push_attr(xml, "Oval", "GeometricBounds", &bounds)?;
    }
    if oval.path_geometry.is_some()
        || oval.metadata.text_wrap_preference.is_some()
        || oval.metadata.frame_fitting_option.is_some()
        || oval.metadata.incopy_export_option.is_some()
        || oval.metadata.object_export_option.is_some()
    {
        xml.push_str(">\n");
        if let Some(path_geometry) = &oval.path_geometry {
            serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        }
        serialize_page_item_metadata_children(xml, &oval.metadata, indent + 2)?;
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
    push_page_item_metadata(xml, "GraphicLine", &line.metadata)?;
    if let Some(bounds) = line.geometric_bounds {
        let bounds = format_geometric_bounds("GraphicLine", bounds)?;
        push_attr(xml, "GraphicLine", "GeometricBounds", &bounds)?;
    }
    if line.path_geometry.is_some()
        || line.metadata.text_wrap_preference.is_some()
        || line.metadata.frame_fitting_option.is_some()
        || line.metadata.incopy_export_option.is_some()
        || line.metadata.object_export_option.is_some()
    {
        xml.push_str(">\n");
        if let Some(path_geometry) = &line.path_geometry {
            serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        }
        serialize_page_item_metadata_children(xml, &line.metadata, indent + 2)?;
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
    push_page_item_metadata(xml, "Polygon", &polygon.metadata)?;
    if let Some(bounds) = polygon.geometric_bounds {
        let bounds = format_geometric_bounds("Polygon", bounds)?;
        push_attr(xml, "Polygon", "GeometricBounds", &bounds)?;
    }
    if polygon.path_geometry.is_some()
        || polygon.metadata.text_wrap_preference.is_some()
        || polygon.metadata.frame_fitting_option.is_some()
        || polygon.metadata.incopy_export_option.is_some()
        || polygon.metadata.object_export_option.is_some()
    {
        xml.push_str(">\n");
        if let Some(path_geometry) = &polygon.path_geometry {
            serialize_path_geometry_properties(xml, path_geometry, indent + 2)?;
        }
        serialize_page_item_metadata_children(xml, &polygon.metadata, indent + 2)?;
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
    push_page_item_metadata(xml, "Group", &group.metadata)?;
    if let Some(bounds) = group.geometric_bounds {
        let bounds = format_geometric_bounds("Group", bounds)?;
        push_attr(xml, "Group", "GeometricBounds", &bounds)?;
    }
    if group.page_items.is_empty()
        && group.metadata.text_wrap_preference.is_none()
        && group.metadata.frame_fitting_option.is_none()
        && group.metadata.incopy_export_option.is_none()
        && group.metadata.object_export_option.is_none()
    {
        xml.push_str(" />\n");
        return Ok(());
    }
    xml.push_str(">\n");
    serialize_page_item_metadata_children(xml, &group.metadata, indent + 2)?;
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

fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
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
        PathPoint, Point, Polygon, Rect, Rectangle, Spread, SpreadListItem, SpreadListProperty,
        SpreadParseOptions, TextFrame,
    };
    use crate::IdmlError;
    use crate::core::units::Points;

    #[test]
    fn parses_text_frames_with_bounds_and_parent_story() {
        let xml = r#"<Spread Self="u10">
  <TextFrame Self="tf1" ParentStory="u2" NextTextFrame="tf2" GeometricBounds="0 72 144 216" />
  <TextFrame Self="tf2" ParentStory="u3" PreviousTextFrame="tf1" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(spread.id.as_deref(), Some("u10"));
        assert_eq!(spread.text_frames.len(), 2);
        assert_eq!(spread.page_items.len(), 2);
        assert!(matches!(spread.page_items[0], PageItem::TextFrame(_)));
        assert_eq!(spread.text_frames[0].id.as_deref(), Some("tf1"));
        assert_eq!(spread.text_frames[0].parent_story.as_deref(), Some("u2"));
        assert_eq!(
            spread.text_frames[0].next_text_frame.as_deref(),
            Some("tf2")
        );
        let bounds = spread.text_frames[0].geometric_bounds.unwrap();
        assert_eq!(bounds.width().as_f64(), 144.0);
        assert_eq!(bounds.height().as_f64(), 144.0);
        assert_eq!(bounds.to_millimeters().left.as_f64(), 25.4);
        assert_eq!(
            spread.text_frames[1].previous_text_frame.as_deref(),
            Some("tf1")
        );
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
    fn parses_and_serializes_common_page_item_metadata() {
        let xml = r#"<Spread Self="u10">
  <TextFrame Self="tf1" ParentStory="u2" ContentType="TextType" Name="Headline" ItemTransform="1 0 0 1 10 20" AppliedObjectStyle="ObjectStyle/body" FillColor="Color/Black" StrokeColor="Swatch/None" StrokeWeight="0.5" Visible="true" Locked="false" ParentInterfaceChangeCount="1 2" TargetInterfaceChangeCount="" LastUpdatedInterfaceChangeCount="" OverriddenPageItemProps="" HorizontalLayoutConstraints="FlexibleDimension FixedDimension FlexibleDimension" VerticalLayoutConstraints="FlexibleDimension FixedDimension FlexibleDimension" GradientFillStart="0 0" GradientFillLength="0" GradientFillAngle="0" GradientStrokeStart="0 0" GradientStrokeLength="0" GradientStrokeAngle="0" LocalDisplaySetting="Default" GradientFillHiliteLength="0" GradientFillHiliteAngle="0" GradientStrokeHiliteLength="0" GradientStrokeHiliteAngle="0" />
  <Rectangle Self="r1" ContentType="GraphicType" Name="Logo" ItemTransform="1 0 0 1 30 40" AppliedObjectStyle="ObjectStyle/image" FillColor="Color/Paper" FillTint="-1" StrokeColor="Color/Black" StrokeWeight="1" StrokeTint="-1" GapColor="Swatch/None" GapTint="-1" StrokeType="StrokeStyle/$ID/Solid" StrokeAlignment="CenterAlignment" MiterLimit="4" EndCap="ButtEndCap" EndJoin="MiterEndJoin" LeftLineEnd="None" RightLineEnd="None" Nonprinting="false" OverprintFill="false" OverprintStroke="true" CornerRadius="12" CornerOption="None" TopLeftCornerOption="None" TopRightCornerOption="None" BottomLeftCornerOption="None" BottomRightCornerOption="None" TopLeftCornerRadius="12" TopRightCornerRadius="12" BottomLeftCornerRadius="12" BottomRightCornerRadius="12" StoryTitle="$ID/" FlexItemWidthMode="FlexFixed" FlexItemHeightMode="FlexFixed" Visible="false" Locked="true" />
  <GraphicLine Self="gl1" LockState="None" />
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();

        assert_eq!(
            spread.text_frames[0].metadata.content_type.as_deref(),
            Some("TextType")
        );
        assert_eq!(
            spread.text_frames[0].metadata.name.as_deref(),
            Some("Headline")
        );
        assert_eq!(
            spread.text_frames[0].metadata.item_transform.as_deref(),
            Some("1 0 0 1 10 20")
        );
        assert_eq!(spread.text_frames[0].metadata.visible, Some(true));
        assert_eq!(spread.text_frames[0].metadata.locked, Some(false));
        assert_eq!(
            spread.text_frames[0]
                .metadata
                .horizontal_layout_constraints
                .as_deref(),
            Some("FlexibleDimension FixedDimension FlexibleDimension")
        );
        assert_eq!(
            spread.text_frames[0]
                .metadata
                .gradient_fill_start
                .as_deref(),
            Some("0 0")
        );
        assert_eq!(
            spread.text_frames[0]
                .metadata
                .local_display_setting
                .as_deref(),
            Some("Default")
        );
        assert_eq!(
            spread.text_frames[0]
                .metadata
                .gradient_stroke_hilite_angle
                .as_deref(),
            Some("0")
        );

        let PageItem::Rectangle(rectangle) = &spread.page_items[1] else {
            panic!("expected rectangle page item");
        };
        assert_eq!(rectangle.metadata.name.as_deref(), Some("Logo"));
        assert_eq!(
            rectangle.metadata.applied_object_style.as_deref(),
            Some("ObjectStyle/image")
        );
        assert_eq!(
            rectangle.metadata.fill_color.as_deref(),
            Some("Color/Paper")
        );
        assert_eq!(
            rectangle.metadata.stroke_color.as_deref(),
            Some("Color/Black")
        );
        assert_eq!(rectangle.metadata.stroke_weight.as_deref(), Some("1"));
        assert_eq!(rectangle.metadata.fill_tint.as_deref(), Some("-1"));
        assert_eq!(
            rectangle.metadata.stroke_type.as_deref(),
            Some("StrokeStyle/$ID/Solid")
        );
        assert_eq!(
            rectangle.metadata.stroke_alignment.as_deref(),
            Some("CenterAlignment")
        );
        assert_eq!(rectangle.metadata.nonprinting, Some(false));
        assert_eq!(rectangle.metadata.overprint_stroke, Some(true));
        assert_eq!(rectangle.metadata.corner_radius.as_deref(), Some("12"));
        assert_eq!(
            rectangle.metadata.top_left_corner_option.as_deref(),
            Some("None")
        );
        assert_eq!(
            rectangle.metadata.bottom_right_corner_radius.as_deref(),
            Some("12")
        );
        assert_eq!(rectangle.metadata.story_title.as_deref(), Some("$ID/"));
        assert_eq!(
            rectangle.metadata.flex_item_width_mode.as_deref(),
            Some("FlexFixed")
        );
        assert_eq!(rectangle.metadata.visible, Some(false));
        assert_eq!(rectangle.metadata.locked, Some(true));

        let PageItem::GraphicLine(line) = &spread.page_items[2] else {
            panic!("expected graphic line page item");
        };
        assert_eq!(line.metadata.lock_state.as_deref(), Some("None"));

        let serialized = spread.to_xml().unwrap();
        assert!(serialized.contains("ContentType=\"TextType\""));
        assert!(serialized.contains("AppliedObjectStyle=\"ObjectStyle/image\""));
        assert!(serialized.contains("Visible=\"false\""));
        assert!(serialized.contains("Locked=\"true\""));
        assert!(serialized.contains("StrokeType=\"StrokeStyle/$ID/Solid\""));
        assert!(serialized.contains("StrokeAlignment=\"CenterAlignment\""));
        assert!(serialized.contains("OverprintStroke=\"true\""));
        assert!(serialized.contains("TopLeftCornerOption=\"None\""));
        assert!(serialized.contains("BottomRightCornerRadius=\"12\""));
        assert!(serialized.contains("StoryTitle=\"$ID/\""));
        assert!(serialized.contains("FlexItemWidthMode=\"FlexFixed\""));
        assert!(serialized.contains("LockState=\"None\""));
        assert!(serialized.contains(
            "HorizontalLayoutConstraints=\"FlexibleDimension FixedDimension FlexibleDimension\""
        ));
        assert!(serialized.contains("GradientFillStart=\"0 0\""));
        assert!(serialized.contains("LocalDisplaySetting=\"Default\""));
        assert!(serialized.contains("GradientStrokeHiliteAngle=\"0\""));
        assert_eq!(Spread::from_xml(&serialized).unwrap(), spread);
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
    fn rejects_invalid_page_item_metadata_boolean() {
        let err =
            Spread::from_xml(r#"<Spread><TextFrame Visible="maybe" /></Spread>"#).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Visible",
                reason: "expected boolean",
            } if element == "TextFrame"
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
    fn rejects_xml_forbidden_text_frame_thread_attributes_on_parse() {
        let err = Spread::from_xml("<Spread><TextFrame NextTextFrame=\"bad\0id\" /></Spread>")
            .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "NextTextFrame",
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
                previous_text_frame: Some("tf0".to_owned()),
                next_text_frame: Some("tf2".to_owned()),
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(72.5),
                    Points::new(144.0),
                    Points::new(216.25),
                )),
                metadata: Default::default(),
                preference: None,
                footnote_options: None,
                path_geometry: None,
            }],
            page_items: Vec::new(),
        };

        let xml = <Spread as crate::XmlSaveable>::to_xml(&spread).unwrap();
        let parsed = <Spread as crate::XmlLoadable>::from_xml(&xml).unwrap();

        assert!(xml.contains("Self=\"u&amp;&quot;10\""));
        assert!(xml.contains("Self=\"tf&amp;1\""));
        assert!(xml.contains("PreviousTextFrame=\"tf0\""));
        assert!(xml.contains("NextTextFrame=\"tf2\""));
        assert!(xml.contains("GeometricBounds=\"0 72.5 144 216.25\""));
        assert_eq!(parsed, spread);
    }

    #[test]
    fn preserves_text_frame_preferences_and_inset_spacing() {
        let xml = r#"<Spread Self="s1">
  <TextFrame Self="tf1" ParentStory="u1">
    <TextFramePreference TextColumnCount="2" TextColumnGutter="12" TextColumnFixedWidth="100" TextColumnMaxWidth="0" UseFixedColumnWidth="false" FirstBaselineOffset="AscentOffset" MinimumFirstBaselineOffset="0" VerticalJustification="TopAlign" VerticalThreshold="0" IgnoreWrap="false" UseFlexibleColumnWidth="false" AutoSizingType="Off" AutoSizingReferencePoint="CenterPoint" UseMinimumHeightForAutoSizing="false" MinimumHeightForAutoSizing="0" UseMinimumWidthForAutoSizing="false" MinimumWidthForAutoSizing="0" UseNoLineBreaksForAutoSizing="false" FootnotesEnableOverrides="false" FootnotesSpanAcrossColumns="false" FootnotesMinimumSpacing="12" FootnotesSpaceBetween="6" VerticalBalanceColumns="false" ColumnRuleOverride="false" ColumnRuleOffset="0" ColumnRuleTopInset="0" ColumnRuleInsetChainOverride="true" ColumnRuleBottomInset="0" ColumnRuleStrokeWidth="1" ColumnRuleStrokeColor="Color/Black" ColumnRuleStrokeType="StrokeStyle/$ID/Solid" ColumnRuleStrokeTint="100" ColumnRuleOverprintOverride="false">
      <Properties>
        <InsetSpacing type="list">
          <ListItem type="unit">1</ListItem>
          <ListItem type="unit">2</ListItem>
          <ListItem type="unit">3</ListItem>
          <ListItem type="unit">4</ListItem>
        </InsetSpacing>
      </Properties>
    </TextFramePreference>
    <TextFrameFootnoteOptionsObject EnableOverrides="true" SpanFootnotesAcross="false" MinimumSpacingOption="12" SpaceBetweenFootnotes="6" />
  </TextFrame>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();
        let frame = &spread.text_frames[0];
        let preference = frame.preference.as_ref().unwrap();
        let footnote_options = frame.footnote_options.as_ref().unwrap();
        let inset_spacing = preference.inset_spacing.as_ref().unwrap();

        assert_eq!(preference.text_column_count.as_deref(), Some("2"));
        assert_eq!(preference.use_fixed_column_width, Some(false));
        assert_eq!(
            preference.column_rule_stroke_type.as_deref(),
            Some("StrokeStyle/$ID/Solid")
        );
        assert_eq!(
            inset_spacing,
            &SpreadListProperty {
                value_type: Some("list".to_owned()),
                items: vec![
                    SpreadListItem {
                        value_type: Some("unit".to_owned()),
                        value: "1".to_owned(),
                    },
                    SpreadListItem {
                        value_type: Some("unit".to_owned()),
                        value: "2".to_owned(),
                    },
                    SpreadListItem {
                        value_type: Some("unit".to_owned()),
                        value: "3".to_owned(),
                    },
                    SpreadListItem {
                        value_type: Some("unit".to_owned()),
                        value: "4".to_owned(),
                    },
                ],
            }
        );
        assert_eq!(footnote_options.enable_overrides, Some(true));
        assert_eq!(footnote_options.span_footnotes_across, Some(false));
        assert_eq!(
            footnote_options.minimum_spacing_option.as_deref(),
            Some("12")
        );
        assert_eq!(
            footnote_options.space_between_footnotes.as_deref(),
            Some("6")
        );

        let serialized = spread.to_xml().unwrap();
        assert!(serialized.contains("<TextFramePreference"));
        assert!(serialized.contains("<TextFrameFootnoteOptionsObject"));
        assert!(serialized.contains("<InsetSpacing type=\"list\">"));
        assert!(serialized.contains("<ListItem type=\"unit\">1</ListItem>"));
        assert_eq!(Spread::from_xml(&serialized).unwrap(), spread);
    }

    #[test]
    fn preserves_text_wrap_preferences_on_page_items() {
        let xml = r#"<Spread Self="s1">
  <Rectangle Self="r1">
    <TextWrapPreference Inverse="false" ApplyToMasterPageOnly="true" TextWrapSide="BothSides" TextWrapMode="BoundingBoxTextWrap">
      <Properties>
        <TextWrapOffset Top="1" Left="2" Bottom="3" Right="4" />
      </Properties>
      <ContourOption ContourType="SameAsClipping" IncludeInsideEdges="false" ContourPathName="$ID/" />
    </TextWrapPreference>
  </Rectangle>
  <TextFrame Self="tf1" ParentStory="u1">
    <TextWrapPreference Inverse="true" ApplyToMasterPageOnly="false" TextWrapSide="LeftSide" TextWrapMode="None" />
  </TextFrame>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();
        let PageItem::Rectangle(rectangle) = &spread.page_items[0] else {
            panic!("expected rectangle page item");
        };
        let rectangle_wrap = rectangle.metadata.text_wrap_preference.as_ref().unwrap();
        let rectangle_offset = rectangle_wrap.text_wrap_offset.as_ref().unwrap();
        let rectangle_contour = rectangle_wrap.contour_option.as_ref().unwrap();
        let frame_wrap = spread.text_frames[0]
            .metadata
            .text_wrap_preference
            .as_ref()
            .unwrap();

        assert_eq!(rectangle_wrap.inverse, Some(false));
        assert_eq!(rectangle_wrap.apply_to_master_page_only, Some(true));
        assert_eq!(
            rectangle_wrap.text_wrap_mode.as_deref(),
            Some("BoundingBoxTextWrap")
        );
        assert_eq!(rectangle_offset.top.as_deref(), Some("1"));
        assert_eq!(rectangle_offset.left.as_deref(), Some("2"));
        assert_eq!(rectangle_offset.bottom.as_deref(), Some("3"));
        assert_eq!(rectangle_offset.right.as_deref(), Some("4"));
        assert_eq!(
            rectangle_contour.contour_type.as_deref(),
            Some("SameAsClipping")
        );
        assert_eq!(rectangle_contour.include_inside_edges, Some(false));
        assert_eq!(rectangle_contour.contour_path_name.as_deref(), Some("$ID/"));
        assert_eq!(frame_wrap.inverse, Some(true));
        assert_eq!(frame_wrap.text_wrap_side.as_deref(), Some("LeftSide"));

        let serialized = spread.to_xml().unwrap();
        assert!(serialized.contains("<TextWrapPreference"));
        assert!(
            serialized.contains("<TextWrapOffset Top=\"1\" Left=\"2\" Bottom=\"3\" Right=\"4\" />")
        );
        assert!(serialized.contains("<ContourOption ContourType=\"SameAsClipping\""));
        assert_eq!(Spread::from_xml(&serialized).unwrap(), spread);
    }

    #[test]
    fn preserves_frame_fitting_options_on_page_items() {
        let xml = r#"<Spread Self="s1">
  <Rectangle Self="r1">
    <FrameFittingOption AutoFit="true" LeftCrop="1" TopCrop="2" RightCrop="3" BottomCrop="4" FittingOnEmptyFrame="FillProportionally" FittingAlignment="CenterAnchor" />
  </Rectangle>
  <TextFrame Self="tf1" ParentStory="u1">
    <FrameFittingOption FittingOnEmptyFrame="ContentToFrame" />
  </TextFrame>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();
        let PageItem::Rectangle(rectangle) = &spread.page_items[0] else {
            panic!("expected rectangle page item");
        };
        let rectangle_option = rectangle.metadata.frame_fitting_option.as_ref().unwrap();
        let frame_option = spread.text_frames[0]
            .metadata
            .frame_fitting_option
            .as_ref()
            .unwrap();

        assert_eq!(rectangle_option.auto_fit, Some(true));
        assert_eq!(rectangle_option.left_crop.as_deref(), Some("1"));
        assert_eq!(rectangle_option.top_crop.as_deref(), Some("2"));
        assert_eq!(rectangle_option.right_crop.as_deref(), Some("3"));
        assert_eq!(rectangle_option.bottom_crop.as_deref(), Some("4"));
        assert_eq!(
            rectangle_option.fitting_on_empty_frame.as_deref(),
            Some("FillProportionally")
        );
        assert_eq!(
            rectangle_option.fitting_alignment.as_deref(),
            Some("CenterAnchor")
        );
        assert_eq!(
            frame_option.fitting_on_empty_frame.as_deref(),
            Some("ContentToFrame")
        );

        let serialized = spread.to_xml().unwrap();
        assert!(serialized.contains("<FrameFittingOption"));
        assert!(serialized.contains("FittingOnEmptyFrame=\"FillProportionally\""));
        assert!(serialized.contains("FittingOnEmptyFrame=\"ContentToFrame\""));
        assert_eq!(Spread::from_xml(&serialized).unwrap(), spread);
    }

    #[test]
    fn preserves_page_item_incopy_export_options() {
        let xml = r#"<Spread Self="s1">
  <Rectangle Self="r1">
    <InCopyExportOption IncludeGraphicProxies="true" IncludeAllResources="false" />
  </Rectangle>
  <TextFrame Self="tf1" ParentStory="u1">
    <InCopyExportOption IncludeGraphicProxies="false" IncludeAllResources="true" />
  </TextFrame>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();
        let PageItem::Rectangle(rectangle) = &spread.page_items[0] else {
            panic!("expected rectangle page item");
        };
        let rectangle_option = rectangle.metadata.incopy_export_option.as_ref().unwrap();
        let frame_option = spread.text_frames[0]
            .metadata
            .incopy_export_option
            .as_ref()
            .unwrap();

        assert_eq!(rectangle_option.include_graphic_proxies, Some(true));
        assert_eq!(rectangle_option.include_all_resources, Some(false));
        assert_eq!(frame_option.include_graphic_proxies, Some(false));
        assert_eq!(frame_option.include_all_resources, Some(true));

        let serialized = spread.to_xml().unwrap();
        assert!(serialized.contains("<InCopyExportOption"));
        assert!(serialized.contains("IncludeGraphicProxies=\"true\""));
        assert!(serialized.contains("IncludeAllResources=\"true\""));
        assert_eq!(Spread::from_xml(&serialized).unwrap(), spread);
    }

    #[test]
    fn preserves_object_export_options_on_page_items() {
        let xml = r#"<Spread Self="s1">
  <Rectangle Self="r1">
    <ObjectExportOption EpubType="Figure" EpubAriaRole="doc-cover" SizeType="CustomSize" CustomSize="100 200" PreserveAppearanceFromLayout="PreserveAppearanceRasterize" AltTextSourceType="Custom" ActualTextSourceType="Custom" CustomAltText="alt" CustomActualText="actual" ApplyTagType="TagArtifact" CustomImageConversion="true" ImageConversionType="PNG" CustomImageSizeOption="SizeRelativeToPageWidth" ImageExportResolution="Ppi300" GIFOptionsPalette="AdaptivePalette" GIFOptionsInterlaced="true" JPEGOptionsQuality="High" JPEGOptionsFormat="BaselineEncoding" ImageAlignment="AlignCenter" ImageSpaceBefore="1" ImageSpaceAfter="2" UseImagePageBreak="true" ImagePageBreak="PageBreakBefore" CustomImageAlignment="false" SpaceUnit="CssPixel" CustomLayout="true" CustomLayoutType="AlignmentAndSpacing" UseOriginalImage="false">
      <Properties>
        <AltMetadataProperty NamespacePrefix="dc" PropertyPath="title" />
        <ActualMetadataProperty NamespacePrefix="xmp" PropertyPath="description" />
      </Properties>
    </ObjectExportOption>
  </Rectangle>
  <TextFrame Self="tf1" ParentStory="u1">
    <ObjectExportOption AltTextSourceType="SourceXMLStructure" />
  </TextFrame>
</Spread>"#;

        let spread = Spread::from_xml(xml).unwrap();
        let PageItem::Rectangle(rectangle) = &spread.page_items[0] else {
            panic!("expected rectangle page item");
        };
        let rectangle_option = rectangle.metadata.object_export_option.as_ref().unwrap();
        let frame_option = spread.text_frames[0]
            .metadata
            .object_export_option
            .as_ref()
            .unwrap();

        assert_eq!(rectangle_option.epub_type.as_deref(), Some("Figure"));
        assert_eq!(
            rectangle_option.epub_aria_role.as_deref(),
            Some("doc-cover")
        );
        assert_eq!(
            rectangle_option.preserve_appearance_from_layout.as_deref(),
            Some("PreserveAppearanceRasterize")
        );
        assert_eq!(rectangle_option.custom_image_conversion, Some(true));
        assert_eq!(rectangle_option.gif_options_interlaced, Some(true));
        assert_eq!(rectangle_option.use_image_page_break, Some(true));
        assert_eq!(rectangle_option.custom_image_alignment, Some(false));
        assert_eq!(rectangle_option.custom_layout, Some(true));
        assert_eq!(rectangle_option.use_original_image, Some(false));
        assert_eq!(
            rectangle_option
                .alt_metadata_property
                .as_ref()
                .and_then(|property| property.namespace_prefix.as_deref()),
            Some("dc")
        );
        assert_eq!(
            rectangle_option
                .actual_metadata_property
                .as_ref()
                .and_then(|property| property.property_path.as_deref()),
            Some("description")
        );
        assert_eq!(
            frame_option.alt_text_source_type.as_deref(),
            Some("SourceXMLStructure")
        );

        let serialized = spread.to_xml().unwrap();
        assert!(serialized.contains("<ObjectExportOption"));
        assert!(serialized.contains("EpubAriaRole=\"doc-cover\""));
        assert!(serialized.contains("<AltMetadataProperty NamespacePrefix=\"dc\""));
        assert_eq!(Spread::from_xml(&serialized).unwrap(), spread);
    }

    #[test]
    fn serializer_writes_pages_before_page_items() {
        let text_frame = TextFrame {
            id: Some("tf1".to_owned()),
            layer_id: None,
            parent_story: Some("u2".to_owned()),
            previous_text_frame: None,
            next_text_frame: None,
            geometric_bounds: None,
            metadata: Default::default(),
            preference: None,
            footnote_options: None,
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
            previous_text_frame: None,
            next_text_frame: None,
            geometric_bounds: Some(Rect::new(
                Points::new(72.0),
                Points::new(0.0),
                Points::new(144.0),
                Points::new(72.0),
            )),
            metadata: Default::default(),
            preference: None,
            footnote_options: None,
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
                    metadata: Default::default(),
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
            previous_text_frame: None,
            next_text_frame: None,
            geometric_bounds: None,
            metadata: Default::default(),
            preference: None,
            footnote_options: None,
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
                metadata: Default::default(),
                page_items: vec![
                    PageItem::Rectangle(Rectangle {
                        id: Some("r1".to_owned()),
                        layer_id: None,
                        geometric_bounds: None,
                        metadata: Default::default(),
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
            previous_text_frame: None,
            next_text_frame: None,
            geometric_bounds: None,
            metadata: Default::default(),
            preference: None,
            footnote_options: None,
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
                    metadata: Default::default(),
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
                    metadata: Default::default(),
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
                    metadata: Default::default(),
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
                    metadata: Default::default(),
                    path_geometry: None,
                }),
                PageItem::Group(Group {
                    id: Some("g1".to_owned()),
                    layer_id: Some("layer3".to_owned()),
                    geometric_bounds: None,
                    metadata: Default::default(),
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
                metadata: Default::default(),
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
                metadata: Default::default(),
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
                    previous_text_frame: None,
                    next_text_frame: None,
                    geometric_bounds: None,
                    metadata: Default::default(),
                    preference: None,
                    footnote_options: None,
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
                metadata: Default::default(),
                page_items: (0..super::DEFAULT_MAX_PAGE_ITEMS_PER_SPREAD)
                    .map(|index| {
                        PageItem::Rectangle(Rectangle {
                            id: Some(format!("r{index}")),
                            layer_id: None,
                            geometric_bounds: None,
                            metadata: Default::default(),
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
                previous_text_frame: None,
                next_text_frame: None,
                geometric_bounds: None,
                metadata: Default::default(),
                preference: None,
                footnote_options: None,
                path_geometry: None,
            }],
            page_items: vec![PageItem::TextFrame(TextFrame {
                id: Some("tf2".to_owned()),
                layer_id: None,
                parent_story: Some("u2".to_owned()),
                previous_text_frame: None,
                next_text_frame: None,
                geometric_bounds: None,
                metadata: Default::default(),
                preference: None,
                footnote_options: None,
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
                previous_text_frame: None,
                next_text_frame: None,
                geometric_bounds: None,
                metadata: Default::default(),
                preference: None,
                footnote_options: None,
                path_geometry: None,
            }],
            page_items: vec![PageItem::TextFrame(TextFrame {
                id: Some("tf2".to_owned()),
                layer_id: None,
                parent_story: Some("u2".to_owned()),
                previous_text_frame: None,
                next_text_frame: None,
                geometric_bounds: None,
                metadata: Default::default(),
                preference: None,
                footnote_options: None,
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
                metadata: Default::default(),
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
                metadata: Default::default(),
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
                previous_text_frame: None,
                next_text_frame: None,
                geometric_bounds: Some(Rect::new(
                    Points::new(0.0),
                    Points::new(f64::NAN),
                    Points::new(144.0),
                    Points::new(216.0),
                )),
                metadata: Default::default(),
                preference: None,
                footnote_options: None,
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
                previous_text_frame: None,
                next_text_frame: None,
                geometric_bounds: None,
                metadata: Default::default(),
                preference: None,
                footnote_options: None,
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
