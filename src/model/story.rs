//! Parser for IDML story content XML.

use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::{
    AnyRootTracker, DEFAULT_MAX_XML_DEPTH, DEFAULT_MAX_XML_INPUT_BYTES, XmlDepthTracker,
    configure_idml_reader, enforce_xml_input_bytes, reject_xml_doctype,
    reject_xml_entity_reference, validate_xml_attribute, validate_xml_text,
};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::{EscapeError, escape, partial_escape, resolve_predefined_entity};
use quick_xml::events::{BytesStart, Event};

/// Default maximum extracted text bytes for one story.
pub const DEFAULT_MAX_STORY_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// Default maximum raw bytes accepted for one story XML attribute value.
pub const DEFAULT_MAX_STORY_ATTRIBUTE_BYTES: usize = 256 * 1024;

/// Story parser limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoryParseOptions {
    /// Maximum UTF-8 bytes accepted for the story XML document.
    pub max_xml_bytes: usize,
    /// Maximum UTF-8 bytes allowed in extracted story text.
    pub max_text_bytes: usize,
    /// Maximum raw bytes accepted for one XML attribute value.
    pub max_attribute_bytes: usize,
    /// Maximum XML element nesting depth accepted while parsing.
    pub max_xml_depth: usize,
}

impl Default for StoryParseOptions {
    fn default() -> Self {
        Self {
            max_xml_bytes: DEFAULT_MAX_XML_INPUT_BYTES,
            max_text_bytes: DEFAULT_MAX_STORY_TEXT_BYTES,
            max_attribute_bytes: DEFAULT_MAX_STORY_ATTRIBUTE_BYTES,
            max_xml_depth: DEFAULT_MAX_XML_DEPTH,
        }
    }
}

/// Extracted story text and metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Story {
    /// Story `Self` identifier, when present.
    pub id: Option<String>,
    /// Visible text extracted from `Content`, `Br`, and `Tab` elements.
    pub text: String,
    /// Paragraph and character style ranges in story order.
    pub paragraph_style_ranges: Vec<ParagraphStyleRange>,
    /// Story root metadata, when present.
    pub metadata: StoryMetadata,
    /// Story preference metadata, when present.
    pub preference: Option<StoryPreference>,
    /// InCopy export option metadata, when present.
    pub incopy_export_option: Option<InCopyExportOption>,
}

/// Common metadata on the `Story` root element.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryMetadata {
    /// Applied table-of-contents style reference, when present.
    pub applied_toc_style: Option<String>,
    /// Track changes flag, when present.
    pub track_changes: Option<bool>,
    /// Story title, when present.
    pub story_title: Option<String>,
    /// Applied named grid reference, when present.
    pub applied_named_grid: Option<String>,
    /// User text flag, when present.
    pub user_text: Option<bool>,
    /// Endnote story flag, when present.
    pub is_endnote_story: Option<bool>,
}

/// Metadata from the `StoryPreference` child element.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryPreference {
    /// Optical margin alignment flag, when present.
    pub optical_margin_alignment: Option<bool>,
    /// Optical margin size, preserved as an IDML string.
    pub optical_margin_size: Option<String>,
    /// Frame type, preserved as an IDML string.
    pub frame_type: Option<String>,
    /// Story orientation, preserved as an IDML string.
    pub story_orientation: Option<String>,
    /// Story direction, preserved as an IDML string.
    pub story_direction: Option<String>,
}

/// Metadata from the `InCopyExportOption` child element.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InCopyExportOption {
    /// Include graphic proxies flag, when present.
    pub include_graphic_proxies: Option<bool>,
    /// Include all resources flag, when present.
    pub include_all_resources: Option<bool>,
}

/// A paragraph style range and its contained character ranges.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParagraphStyleRange {
    /// Applied paragraph style reference, when present.
    pub applied_paragraph_style: Option<String>,
    /// Justification value, when present.
    pub justification: Option<String>,
    /// Hyphenation flag, when present.
    pub hyphenation: Option<bool>,
    /// Rule-above line weight, preserved as an IDML string.
    pub rule_above_line_weight: Option<String>,
    /// Rule-below line weight, preserved as an IDML string.
    pub rule_below_line_weight: Option<String>,
    /// Hyphenation zone, preserved as an IDML string.
    pub hyphenation_zone: Option<String>,
    /// Split-column inside gutter, preserved as an IDML string.
    pub split_column_inside_gutter: Option<String>,
    /// Space before, preserved as an IDML string.
    pub space_before: Option<String>,
    /// Space after, preserved as an IDML string.
    pub space_after: Option<String>,
    /// Paragraph style properties, when present.
    pub properties: ParagraphStyleProperties,
    /// Character ranges in this paragraph range.
    pub character_style_ranges: Vec<CharacterStyleRange>,
}

/// Typed child properties from a paragraph style range.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParagraphStyleProperties {
    /// Same-paragraph style spacing property, when present.
    pub same_para_style_spacing: Option<StoryProperty>,
    /// Rule-above gap color property, when present.
    pub rule_above_gap_color: Option<StoryProperty>,
    /// Rule-below gap color property, when present.
    pub rule_below_gap_color: Option<StoryProperty>,
}

/// A character style range with extracted visible text.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CharacterStyleRange {
    /// Applied character style reference, when present.
    pub applied_character_style: Option<String>,
    /// Point size value, preserved as an IDML string.
    pub point_size: Option<String>,
    /// Font style value, when present.
    pub font_style: Option<String>,
    /// Fill color reference, when present.
    pub fill_color: Option<String>,
    /// Tracking value, preserved as an IDML string.
    pub tracking: Option<String>,
    /// Stroke weight value, preserved as an IDML string.
    pub stroke_weight: Option<String>,
    /// Ruby font size value, preserved as an IDML string.
    pub ruby_font_size: Option<String>,
    /// Kenten font size value, preserved as an IDML string.
    pub kenten_font_size: Option<String>,
    /// Capitalization value, when present.
    pub capitalization: Option<String>,
    /// Miter limit value, preserved as an IDML string.
    pub miter_limit: Option<String>,
    /// Stroke alignment value, when present.
    pub stroke_alignment: Option<String>,
    /// Horizontal scale value, preserved as an IDML string.
    pub horizontal_scale: Option<String>,
    /// Vertical scale value, preserved as an IDML string.
    pub vertical_scale: Option<String>,
    /// Character style properties, when present.
    pub properties: CharacterStyleProperties,
    /// Table metadata directly anchored in this character range.
    pub tables: Vec<StoryTable>,
    /// Visible text carried by this character range.
    pub text: String,
}

/// Metadata skeleton for a story table.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryTable {
    /// Table `Self` identifier, when present.
    pub id: Option<String>,
    /// Header row count, preserved as an IDML string.
    pub header_row_count: Option<String>,
    /// Footer row count, preserved as an IDML string.
    pub footer_row_count: Option<String>,
    /// Body row count, preserved as an IDML string.
    pub body_row_count: Option<String>,
    /// Column count, preserved as an IDML string.
    pub column_count: Option<String>,
    /// Applied table style reference, when present.
    pub applied_table_style: Option<String>,
    /// Table direction, when present.
    pub table_direction: Option<String>,
    /// Table text insets and clipping metadata.
    pub text_frame: TableTextInsets,
    /// Row descriptors in table order.
    pub rows: Vec<StoryTableRow>,
    /// Column descriptors in table order.
    pub columns: Vec<StoryTableColumn>,
    /// Cell descriptors in table order.
    pub cells: Vec<StoryTableCell>,
}

/// Shared table text inset metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableTextInsets {
    /// Top text inset, preserved as an IDML string.
    pub text_top_inset: Option<String>,
    /// Left text inset, preserved as an IDML string.
    pub text_left_inset: Option<String>,
    /// Bottom text inset, preserved as an IDML string.
    pub text_bottom_inset: Option<String>,
    /// Right text inset, preserved as an IDML string.
    pub text_right_inset: Option<String>,
    /// Clip-content flag, when present.
    pub clip_content_to_text_cell: Option<bool>,
}

/// Metadata skeleton for one IDML table row.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryTableRow {
    /// Row `Self` identifier, when present.
    pub id: Option<String>,
    /// Row name, when present.
    pub name: Option<String>,
    /// Row text insets and clipping metadata.
    pub text_frame: TableTextInsets,
    /// Row height, preserved as an IDML string.
    pub single_row_height: Option<String>,
    /// Minimum height, preserved as an IDML string.
    pub minimum_height: Option<String>,
    /// Auto-grow flag, when present.
    pub auto_grow: Option<bool>,
}

/// Metadata skeleton for one IDML table column.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryTableColumn {
    /// Column `Self` identifier, when present.
    pub id: Option<String>,
    /// Column name, when present.
    pub name: Option<String>,
    /// Column text insets and clipping metadata.
    pub text_frame: TableTextInsets,
    /// Column width, preserved as an IDML string.
    pub single_column_width: Option<String>,
}

/// Metadata skeleton for one IDML table cell.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryTableCell {
    /// Cell `Self` identifier, when present.
    pub id: Option<String>,
    /// Cell name, when present.
    pub name: Option<String>,
    /// Row span, preserved as an IDML string.
    pub row_span: Option<String>,
    /// Column span, preserved as an IDML string.
    pub column_span: Option<String>,
    /// Cell type, when present.
    pub cell_type: Option<String>,
    /// Applied cell style reference, when present.
    pub applied_cell_style: Option<String>,
    /// Applied cell style priority, preserved as an IDML string.
    pub applied_cell_style_priority: Option<String>,
    /// Cell text insets and clipping metadata.
    pub text_frame: TableTextInsets,
    /// Left cell inset, preserved as an IDML string.
    pub left_inset: Option<String>,
    /// Top cell inset, preserved as an IDML string.
    pub top_inset: Option<String>,
    /// Right cell inset, preserved as an IDML string.
    pub right_inset: Option<String>,
    /// Bottom cell inset, preserved as an IDML string.
    pub bottom_inset: Option<String>,
    /// Fill color reference, when present.
    pub fill_color: Option<String>,
}

/// Typed child properties from a character style range.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CharacterStyleProperties {
    /// Applied font property, when present.
    pub applied_font: Option<StoryProperty>,
    /// Leading property, when present.
    pub leading: Option<StoryProperty>,
    /// Underline gap color property, when present.
    pub underline_gap_color: Option<StoryProperty>,
    /// Strike-through gap color property, when present.
    pub strike_through_gap_color: Option<StoryProperty>,
}

/// A typed IDML property value from a style range `Properties` element.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryProperty {
    /// IDML `type` attribute, when present.
    pub value_type: Option<String>,
    /// Property text value.
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoryPropertyTarget {
    ParagraphSameParaStyleSpacing(usize),
    ParagraphRuleAboveGapColor(usize),
    ParagraphRuleBelowGapColor(usize),
    CharacterAppliedFont(usize, usize),
    CharacterLeading(usize, usize),
    CharacterUnderlineGapColor(usize, usize),
    CharacterStrikeThroughGapColor(usize, usize),
}

#[derive(Debug)]
struct StoryPropertyCapture {
    element: &'static str,
    target: StoryPropertyTarget,
    property: StoryProperty,
}

impl Story {
    /// Parses a story XML document with default limits.
    pub fn from_xml(xml_content: &str) -> Result<Self> {
        Self::from_xml_with_options(xml_content, StoryParseOptions::default())
    }

    /// Parses a story XML document with explicit limits.
    pub fn from_xml_with_options(xml_content: &str, options: StoryParseOptions) -> Result<Self> {
        enforce_xml_input_bytes("Story XML bytes", xml_content, options.max_xml_bytes)?;

        let mut reader = Reader::from_str(xml_content);
        configure_idml_reader(&mut reader, false);

        let mut id = None;
        let mut text = String::new();
        let mut paragraph_style_ranges = Vec::new();
        let mut current_paragraph_range = None;
        let mut current_character_range = None;
        let mut current_property = None;
        let mut current_table = None;
        let mut metadata = StoryMetadata::default();
        let mut preference = None;
        let mut incopy_export_option = None;
        let mut content_depth = 0usize;
        let mut story_depth = 0usize;
        let mut seen_story = false;
        let mut root_name = None;
        let mut xml_depth = XmlDepthTracker::new(options.max_xml_depth, "Story XML depth");
        let mut root = AnyRootTracker::default();

        loop {
            match reader.read_event()? {
                Event::Start(e) => {
                    validate_story_root(&mut root_name, e.name().as_ref())?;
                    root.enter(e.name().as_ref())?;
                    xml_depth.enter()?;
                    if content_depth > 0 {
                        return Err(IdmlError::InvalidElement {
                            element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                            reason: "element inside story Content",
                        });
                    }
                    if current_property.is_some() {
                        return Err(IdmlError::InvalidElement {
                            element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                            reason: "element inside story property",
                        });
                    }
                    if e.name().as_ref() == b"Story" {
                        enter_story_element(&mut seen_story, &mut story_depth)?;
                        id = optional_attr(&e, "Self", &reader, options.max_attribute_bytes)?;
                        metadata = StoryMetadata::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 {
                        story_depth = increment_story_depth(story_depth)?;
                    }
                    if story_depth > 0 && e.name().as_ref() == b"StoryPreference" {
                        preference = Some(StoryPreference::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?);
                    } else if story_depth > 0 && e.name().as_ref() == b"InCopyExportOption" {
                        incopy_export_option = Some(InCopyExportOption::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?);
                    } else if story_depth > 0 && e.name().as_ref() == b"ParagraphStyleRange" {
                        paragraph_style_ranges.push(ParagraphStyleRange::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?);
                        current_paragraph_range = Some(paragraph_style_ranges.len() - 1);
                    } else if story_depth > 0 && e.name().as_ref() == b"CharacterStyleRange" {
                        let paragraph_index = ensure_current_paragraph_range(
                            &mut paragraph_style_ranges,
                            &mut current_paragraph_range,
                        );
                        let paragraph = &mut paragraph_style_ranges[paragraph_index];
                        paragraph
                            .character_style_ranges
                            .push(CharacterStyleRange::from_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?);
                        current_character_range = Some(paragraph.character_style_ranges.len() - 1);
                    } else if story_depth > 0 && e.name().as_ref() == b"Table" {
                        current_table = Some(start_story_table(
                            &mut paragraph_style_ranges,
                            current_paragraph_range,
                            current_character_range,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?);
                    } else if story_depth > 0 && e.name().as_ref() == b"Row" {
                        push_story_table_row(
                            &mut paragraph_style_ranges,
                            current_table,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 && e.name().as_ref() == b"Column" {
                        push_story_table_column(
                            &mut paragraph_style_ranges,
                            current_table,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 && e.name().as_ref() == b"Cell" {
                        push_story_table_cell(
                            &mut paragraph_style_ranges,
                            current_table,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 {
                        maybe_start_story_property(
                            &mut current_property,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                            current_paragraph_range,
                            current_character_range,
                        )?;
                    }
                    if story_depth > 0 && e.name().as_ref() == b"Content" {
                        content_depth = 1;
                    }
                }
                Event::End(e) if e.name().as_ref() == b"Content" => {
                    exit_content_depth(&mut content_depth)?;
                    exit_story_depth(&mut story_depth)?;
                    xml_depth.exit()?;
                    root.exit(e.name().as_ref())?;
                }
                Event::End(e) => {
                    if property_end_matches(&current_property, e.name().as_ref()) {
                        finish_story_property(&mut paragraph_style_ranges, &mut current_property)?;
                    } else if e.name().as_ref() == b"Table" {
                        current_table = None;
                    } else if e.name().as_ref() == b"CharacterStyleRange" {
                        current_character_range = None;
                    } else if e.name().as_ref() == b"ParagraphStyleRange" {
                        current_character_range = None;
                        current_paragraph_range = None;
                    }
                    exit_story_depth(&mut story_depth)?;
                    xml_depth.exit()?;
                    root.exit(e.name().as_ref())?;
                }
                Event::Text(e) => {
                    root.visit_text(e.as_ref())?;
                    if content_depth > 0 {
                        let decoded = e.decode()?;
                        push_limited(&mut text, decoded.as_ref(), options.max_text_bytes)?;
                        push_range_limited(
                            &mut paragraph_style_ranges,
                            current_paragraph_range,
                            current_character_range,
                            decoded.as_ref(),
                            options.max_text_bytes,
                        )?;
                    } else if let Some(property) = &mut current_property {
                        let decoded = e.decode()?;
                        push_property_value_limited(
                            &mut property.property.value,
                            decoded.as_ref(),
                            options.max_attribute_bytes,
                        )?;
                    }
                }
                Event::CData(e) => {
                    root.visit_cdata()?;
                    if content_depth > 0 {
                        let decoded = e.xml_content(XmlVersion::Implicit1_0)?;
                        push_limited(&mut text, decoded.as_ref(), options.max_text_bytes)?;
                        push_range_limited(
                            &mut paragraph_style_ranges,
                            current_paragraph_range,
                            current_character_range,
                            decoded.as_ref(),
                            options.max_text_bytes,
                        )?;
                    } else if let Some(property) = &mut current_property {
                        let decoded = e.xml_content(XmlVersion::Implicit1_0)?;
                        push_property_value_limited(
                            &mut property.property.value,
                            decoded.as_ref(),
                            options.max_attribute_bytes,
                        )?;
                    }
                }
                Event::GeneralRef(e) => {
                    root.visit_reference()?;
                    if content_depth > 0 {
                        if let Some(ch) = e.resolve_char_ref()? {
                            push_char_limited(&mut text, ch, options.max_text_bytes)?;
                            push_range_char_limited(
                                &mut paragraph_style_ranges,
                                current_paragraph_range,
                                current_character_range,
                                ch,
                                options.max_text_bytes,
                            )?;
                        } else {
                            let entity = e.decode()?;
                            let replacement = resolve_predefined_entity(entity.as_ref())
                                .ok_or_else(|| {
                                    IdmlError::XmlEscape(EscapeError::UnrecognizedEntity(
                                        0..entity.len(),
                                        entity.to_string(),
                                    ))
                                })?;
                            push_limited(&mut text, replacement, options.max_text_bytes)?;
                            push_range_limited(
                                &mut paragraph_style_ranges,
                                current_paragraph_range,
                                current_character_range,
                                replacement,
                                options.max_text_bytes,
                            )?;
                        }
                    } else if let Some(property) = &mut current_property {
                        if let Some(ch) = e.resolve_char_ref()? {
                            push_property_char_limited(
                                &mut property.property.value,
                                ch,
                                options.max_attribute_bytes,
                            )?;
                        } else {
                            let entity = e.decode()?;
                            let replacement = resolve_predefined_entity(entity.as_ref())
                                .ok_or_else(|| {
                                    IdmlError::XmlEscape(EscapeError::UnrecognizedEntity(
                                        0..entity.len(),
                                        entity.to_string(),
                                    ))
                                })?;
                            push_property_value_limited(
                                &mut property.property.value,
                                replacement,
                                options.max_attribute_bytes,
                            )?;
                        }
                    } else {
                        reject_xml_entity_reference()?;
                    }
                }
                Event::Empty(e) => {
                    validate_story_root(&mut root_name, e.name().as_ref())?;
                    root.visit_empty(e.name().as_ref())?;
                    xml_depth.visit_empty()?;
                    if content_depth > 0 {
                        return Err(IdmlError::InvalidElement {
                            element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                            reason: "element inside story Content",
                        });
                    }
                    if current_property.is_some() {
                        return Err(IdmlError::InvalidElement {
                            element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                            reason: "element inside story property",
                        });
                    }
                    if e.name().as_ref() == b"Story" {
                        enter_empty_story_element(&mut seen_story)?;
                        id = optional_attr(&e, "Self", &reader, options.max_attribute_bytes)?;
                        metadata = StoryMetadata::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 && e.name().as_ref() == b"StoryPreference" {
                        preference = Some(StoryPreference::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?);
                    } else if story_depth > 0 && e.name().as_ref() == b"InCopyExportOption" {
                        incopy_export_option = Some(InCopyExportOption::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?);
                    } else if story_depth > 0 && e.name().as_ref() == b"ParagraphStyleRange" {
                        paragraph_style_ranges.push(ParagraphStyleRange::from_xml_event(
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?);
                    } else if story_depth > 0 && e.name().as_ref() == b"CharacterStyleRange" {
                        let paragraph_index = ensure_current_paragraph_range(
                            &mut paragraph_style_ranges,
                            &mut current_paragraph_range,
                        );
                        paragraph_style_ranges[paragraph_index]
                            .character_style_ranges
                            .push(CharacterStyleRange::from_xml_event(
                                &e,
                                &reader,
                                options.max_attribute_bytes,
                            )?);
                    } else if story_depth > 0 && e.name().as_ref() == b"Table" {
                        start_story_table(
                            &mut paragraph_style_ranges,
                            current_paragraph_range,
                            current_character_range,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 && e.name().as_ref() == b"Row" {
                        push_story_table_row(
                            &mut paragraph_style_ranges,
                            current_table,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 && e.name().as_ref() == b"Column" {
                        push_story_table_column(
                            &mut paragraph_style_ranges,
                            current_table,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0 && e.name().as_ref() == b"Cell" {
                        push_story_table_cell(
                            &mut paragraph_style_ranges,
                            current_table,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                        )?;
                    } else if story_depth > 0
                        && maybe_insert_empty_story_property(
                            &mut paragraph_style_ranges,
                            &e,
                            &reader,
                            options.max_attribute_bytes,
                            current_paragraph_range,
                            current_character_range,
                        )?
                    {
                    } else if story_depth > 0 && e.name().as_ref() == b"Br" {
                        push_limited(&mut text, "\n", options.max_text_bytes)?;
                        push_range_limited(
                            &mut paragraph_style_ranges,
                            current_paragraph_range,
                            current_character_range,
                            "\n",
                            options.max_text_bytes,
                        )?;
                    } else if story_depth > 0 && e.name().as_ref() == b"Tab" {
                        push_limited(&mut text, "\t", options.max_text_bytes)?;
                        push_range_limited(
                            &mut paragraph_style_ranges,
                            current_paragraph_range,
                            current_character_range,
                            "\t",
                            options.max_text_bytes,
                        )?;
                    }
                }
                Event::DocType(_) => reject_xml_doctype()?,
                Event::Eof => break,
                _ => {}
            }
        }

        root.finish()?;
        xml_depth.finish()?;
        if !seen_story {
            return Err(IdmlError::InvalidElement {
                element: root_name
                    .map(|name| String::from_utf8_lossy(&name).into_owned())
                    .unwrap_or_default(),
                reason: "missing Story element",
            });
        }
        normalize_default_style_ranges(&mut paragraph_style_ranges, &text);
        Ok(Self {
            id,
            text,
            paragraph_style_ranges,
            metadata,
            preference,
            incopy_export_option,
        })
    }

    /// Serializes this story into a minimal standalone story XML document.
    pub fn to_xml(&self) -> Result<String> {
        serialize_story(self)
    }
}

impl XmlLoadable for Story {
    fn from_xml(xml: &str) -> Result<Self> {
        Story::from_xml(xml)
    }
}

impl XmlSaveable for Story {
    fn to_xml(&self) -> Result<String> {
        serialize_story(self)
    }
}

impl StoryMetadata {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            applied_toc_style: optional_attr(
                event,
                "AppliedTOCStyle",
                reader,
                max_attribute_bytes,
            )?,
            track_changes: optional_bool_attr(event, "TrackChanges", reader, max_attribute_bytes)?,
            story_title: optional_attr(event, "StoryTitle", reader, max_attribute_bytes)?,
            applied_named_grid: optional_attr(
                event,
                "AppliedNamedGrid",
                reader,
                max_attribute_bytes,
            )?,
            user_text: optional_bool_attr(event, "UserText", reader, max_attribute_bytes)?,
            is_endnote_story: optional_bool_attr(
                event,
                "IsEndnoteStory",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl StoryPreference {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            optical_margin_alignment: optional_bool_attr(
                event,
                "OpticalMarginAlignment",
                reader,
                max_attribute_bytes,
            )?,
            optical_margin_size: optional_attr(
                event,
                "OpticalMarginSize",
                reader,
                max_attribute_bytes,
            )?,
            frame_type: optional_attr(event, "FrameType", reader, max_attribute_bytes)?,
            story_orientation: optional_attr(
                event,
                "StoryOrientation",
                reader,
                max_attribute_bytes,
            )?,
            story_direction: optional_attr(event, "StoryDirection", reader, max_attribute_bytes)?,
        })
    }
}

impl InCopyExportOption {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            include_graphic_proxies: optional_bool_attr(
                event,
                "IncludeGraphicProxies",
                reader,
                max_attribute_bytes,
            )?,
            include_all_resources: optional_bool_attr(
                event,
                "IncludeAllResources",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl ParagraphStyleRange {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            applied_paragraph_style: optional_attr(
                event,
                "AppliedParagraphStyle",
                reader,
                max_attribute_bytes,
            )?,
            justification: optional_attr(event, "Justification", reader, max_attribute_bytes)?,
            hyphenation: optional_bool_attr(event, "Hyphenation", reader, max_attribute_bytes)?,
            rule_above_line_weight: optional_attr(
                event,
                "RuleAboveLineWeight",
                reader,
                max_attribute_bytes,
            )?,
            rule_below_line_weight: optional_attr(
                event,
                "RuleBelowLineWeight",
                reader,
                max_attribute_bytes,
            )?,
            hyphenation_zone: optional_attr(event, "HyphenationZone", reader, max_attribute_bytes)?,
            split_column_inside_gutter: optional_attr(
                event,
                "SplitColumnInsideGutter",
                reader,
                max_attribute_bytes,
            )?,
            space_before: optional_attr(event, "SpaceBefore", reader, max_attribute_bytes)?,
            space_after: optional_attr(event, "SpaceAfter", reader, max_attribute_bytes)?,
            properties: ParagraphStyleProperties::default(),
            character_style_ranges: Vec::new(),
        })
    }
}

impl CharacterStyleRange {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            applied_character_style: optional_attr(
                event,
                "AppliedCharacterStyle",
                reader,
                max_attribute_bytes,
            )?,
            point_size: optional_attr(event, "PointSize", reader, max_attribute_bytes)?,
            font_style: optional_attr(event, "FontStyle", reader, max_attribute_bytes)?,
            fill_color: optional_attr(event, "FillColor", reader, max_attribute_bytes)?,
            tracking: optional_attr(event, "Tracking", reader, max_attribute_bytes)?,
            stroke_weight: optional_attr(event, "StrokeWeight", reader, max_attribute_bytes)?,
            ruby_font_size: optional_attr(event, "RubyFontSize", reader, max_attribute_bytes)?,
            kenten_font_size: optional_attr(event, "KentenFontSize", reader, max_attribute_bytes)?,
            capitalization: optional_attr(event, "Capitalization", reader, max_attribute_bytes)?,
            miter_limit: optional_attr(event, "MiterLimit", reader, max_attribute_bytes)?,
            stroke_alignment: optional_attr(event, "StrokeAlignment", reader, max_attribute_bytes)?,
            horizontal_scale: optional_attr(event, "HorizontalScale", reader, max_attribute_bytes)?,
            vertical_scale: optional_attr(event, "VerticalScale", reader, max_attribute_bytes)?,
            properties: CharacterStyleProperties::default(),
            tables: Vec::new(),
            text: String::new(),
        })
    }
}

impl StoryTable {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            id: optional_attr(event, "Self", reader, max_attribute_bytes)?,
            header_row_count: optional_attr(event, "HeaderRowCount", reader, max_attribute_bytes)?,
            footer_row_count: optional_attr(event, "FooterRowCount", reader, max_attribute_bytes)?,
            body_row_count: optional_attr(event, "BodyRowCount", reader, max_attribute_bytes)?,
            column_count: optional_attr(event, "ColumnCount", reader, max_attribute_bytes)?,
            applied_table_style: optional_attr(
                event,
                "AppliedTableStyle",
                reader,
                max_attribute_bytes,
            )?,
            table_direction: optional_attr(event, "TableDirection", reader, max_attribute_bytes)?,
            text_frame: TableTextInsets::from_xml_event(event, reader, max_attribute_bytes)?,
            rows: Vec::new(),
            columns: Vec::new(),
            cells: Vec::new(),
        })
    }
}

impl TableTextInsets {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            text_top_inset: optional_attr(event, "TextTopInset", reader, max_attribute_bytes)?,
            text_left_inset: optional_attr(event, "TextLeftInset", reader, max_attribute_bytes)?,
            text_bottom_inset: optional_attr(
                event,
                "TextBottomInset",
                reader,
                max_attribute_bytes,
            )?,
            text_right_inset: optional_attr(event, "TextRightInset", reader, max_attribute_bytes)?,
            clip_content_to_text_cell: optional_bool_attr(
                event,
                "ClipContentToTextCell",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl StoryTableRow {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            id: optional_attr(event, "Self", reader, max_attribute_bytes)?,
            name: optional_attr(event, "Name", reader, max_attribute_bytes)?,
            text_frame: TableTextInsets::from_xml_event(event, reader, max_attribute_bytes)?,
            single_row_height: optional_attr(
                event,
                "SingleRowHeight",
                reader,
                max_attribute_bytes,
            )?,
            minimum_height: optional_attr(event, "MinimumHeight", reader, max_attribute_bytes)?,
            auto_grow: optional_bool_attr(event, "AutoGrow", reader, max_attribute_bytes)?,
        })
    }
}

impl StoryTableColumn {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            id: optional_attr(event, "Self", reader, max_attribute_bytes)?,
            name: optional_attr(event, "Name", reader, max_attribute_bytes)?,
            text_frame: TableTextInsets::from_xml_event(event, reader, max_attribute_bytes)?,
            single_column_width: optional_attr(
                event,
                "SingleColumnWidth",
                reader,
                max_attribute_bytes,
            )?,
        })
    }
}

impl StoryTableCell {
    fn from_xml_event(
        event: &BytesStart<'_>,
        reader: &Reader<&[u8]>,
        max_attribute_bytes: usize,
    ) -> Result<Self> {
        Ok(Self {
            id: optional_attr(event, "Self", reader, max_attribute_bytes)?,
            name: optional_attr(event, "Name", reader, max_attribute_bytes)?,
            row_span: optional_attr(event, "RowSpan", reader, max_attribute_bytes)?,
            column_span: optional_attr(event, "ColumnSpan", reader, max_attribute_bytes)?,
            cell_type: optional_attr(event, "CellType", reader, max_attribute_bytes)?,
            applied_cell_style: optional_attr(
                event,
                "AppliedCellStyle",
                reader,
                max_attribute_bytes,
            )?,
            applied_cell_style_priority: optional_attr(
                event,
                "AppliedCellStylePriority",
                reader,
                max_attribute_bytes,
            )?,
            text_frame: TableTextInsets::from_xml_event(event, reader, max_attribute_bytes)?,
            left_inset: optional_attr(event, "LeftInset", reader, max_attribute_bytes)?,
            top_inset: optional_attr(event, "TopInset", reader, max_attribute_bytes)?,
            right_inset: optional_attr(event, "RightInset", reader, max_attribute_bytes)?,
            bottom_inset: optional_attr(event, "BottomInset", reader, max_attribute_bytes)?,
            fill_color: optional_attr(event, "FillColor", reader, max_attribute_bytes)?,
        })
    }
}

impl StoryProperty {
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
            enforce_story_attribute_bytes(attr.value.len(), max_attribute_bytes)?;
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
    attribute: &'static str,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<Option<bool>> {
    optional_attr(event, attribute, reader, max_attribute_bytes)?
        .map(|value| parse_bool_attr(event, attribute, &value))
        .transpose()
}

fn parse_bool_attr(event: &BytesStart<'_>, attribute: &'static str, value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(IdmlError::InvalidAttribute {
            element: String::from_utf8_lossy(event.name().as_ref()).into_owned(),
            attribute,
            reason: "expected boolean",
        }),
    }
}

fn enforce_story_attribute_bytes(actual: usize, max_attribute_bytes: usize) -> Result<()> {
    if actual > max_attribute_bytes {
        return Err(IdmlError::LimitExceeded {
            what: "Story XML attribute bytes",
            limit: max_attribute_bytes as u64,
            actual: actual as u64,
        });
    }
    Ok(())
}

fn validate_story_root(root_name: &mut Option<Vec<u8>>, name: &[u8]) -> Result<()> {
    if root_name.is_some() {
        return Ok(());
    }

    if matches!(name, b"Story" | b"idPkg:Story") {
        *root_name = Some(name.to_vec());
        return Ok(());
    }

    Err(IdmlError::InvalidElement {
        element: String::from_utf8_lossy(name).into_owned(),
        reason: "expected Story root element",
    })
}

fn enter_story_element(seen_story: &mut bool, story_depth: &mut usize) -> Result<()> {
    if *story_depth > 0 {
        return Err(IdmlError::InvalidElement {
            element: "Story".to_owned(),
            reason: "nested Story element",
        });
    }
    if *seen_story {
        return Err(IdmlError::InvalidElement {
            element: "Story".to_owned(),
            reason: "multiple Story elements",
        });
    }

    *seen_story = true;
    *story_depth = 1;
    Ok(())
}

fn enter_empty_story_element(seen_story: &mut bool) -> Result<()> {
    if *seen_story {
        return Err(IdmlError::InvalidElement {
            element: "Story".to_owned(),
            reason: "multiple Story elements",
        });
    }

    *seen_story = true;
    Ok(())
}

fn increment_story_depth(story_depth: usize) -> Result<usize> {
    story_depth
        .checked_add(1)
        .ok_or_else(|| IdmlError::InvalidElement {
            element: "Story".to_owned(),
            reason: "story depth overflow",
        })
}

fn exit_story_depth(story_depth: &mut usize) -> Result<()> {
    if *story_depth > 0 {
        *story_depth -= 1;
    }
    Ok(())
}

fn push_limited(output: &mut String, text: &str, max_text_bytes: usize) -> Result<()> {
    let next_len = output
        .len()
        .checked_add(text.len())
        .ok_or(IdmlError::LimitExceeded {
            what: "story text bytes",
            limit: max_text_bytes as u64,
            actual: u64::MAX,
        })?;

    if next_len > max_text_bytes {
        return Err(IdmlError::LimitExceeded {
            what: "story text bytes",
            limit: max_text_bytes as u64,
            actual: next_len as u64,
        });
    }

    output.push_str(text);
    Ok(())
}

fn push_char_limited(output: &mut String, ch: char, max_text_bytes: usize) -> Result<()> {
    let mut buffer = [0u8; 4];
    push_limited(output, ch.encode_utf8(&mut buffer), max_text_bytes)
}

fn push_property_value_limited(
    output: &mut String,
    text: &str,
    max_property_bytes: usize,
) -> Result<()> {
    let next_len = output
        .len()
        .checked_add(text.len())
        .ok_or(IdmlError::LimitExceeded {
            what: "Story XML property bytes",
            limit: max_property_bytes as u64,
            actual: u64::MAX,
        })?;

    if next_len > max_property_bytes {
        return Err(IdmlError::LimitExceeded {
            what: "Story XML property bytes",
            limit: max_property_bytes as u64,
            actual: next_len as u64,
        });
    }

    output.push_str(text);
    Ok(())
}

fn push_property_char_limited(
    output: &mut String,
    ch: char,
    max_property_bytes: usize,
) -> Result<()> {
    let mut buffer = [0u8; 4];
    push_property_value_limited(output, ch.encode_utf8(&mut buffer), max_property_bytes)
}

fn ensure_current_paragraph_range(
    paragraph_style_ranges: &mut Vec<ParagraphStyleRange>,
    current_paragraph_range: &mut Option<usize>,
) -> usize {
    if let Some(index) = *current_paragraph_range {
        return index;
    }

    paragraph_style_ranges.push(ParagraphStyleRange::default());
    let index = paragraph_style_ranges.len() - 1;
    *current_paragraph_range = Some(index);
    index
}

fn push_range_limited(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    current_paragraph_range: Option<usize>,
    current_character_range: Option<usize>,
    text: &str,
    max_text_bytes: usize,
) -> Result<()> {
    let Some(paragraph_index) = current_paragraph_range else {
        return Ok(());
    };
    let Some(character_index) = current_character_range else {
        return Ok(());
    };
    let Some(range) = paragraph_style_ranges
        .get_mut(paragraph_index)
        .and_then(|paragraph| paragraph.character_style_ranges.get_mut(character_index))
    else {
        return Err(IdmlError::InvalidElement {
            element: "CharacterStyleRange".to_owned(),
            reason: "style range parser state is inconsistent",
        });
    };
    push_limited(&mut range.text, text, max_text_bytes)
}

fn push_range_char_limited(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    current_paragraph_range: Option<usize>,
    current_character_range: Option<usize>,
    ch: char,
    max_text_bytes: usize,
) -> Result<()> {
    let mut buffer = [0u8; 4];
    push_range_limited(
        paragraph_style_ranges,
        current_paragraph_range,
        current_character_range,
        ch.encode_utf8(&mut buffer),
        max_text_bytes,
    )
}

fn start_story_table(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    current_paragraph_range: Option<usize>,
    current_character_range: Option<usize>,
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<(usize, usize, usize)> {
    let Some(paragraph_index) = current_paragraph_range else {
        return Err(IdmlError::InvalidElement {
            element: "Table".to_owned(),
            reason: "table outside paragraph style range",
        });
    };
    let Some(character_index) = current_character_range else {
        return Err(IdmlError::InvalidElement {
            element: "Table".to_owned(),
            reason: "table outside character style range",
        });
    };
    let Some(character) = paragraph_style_ranges
        .get_mut(paragraph_index)
        .and_then(|paragraph| paragraph.character_style_ranges.get_mut(character_index))
    else {
        return Err(IdmlError::InvalidElement {
            element: "Table".to_owned(),
            reason: "table parser state is inconsistent",
        });
    };

    character.tables.push(StoryTable::from_xml_event(
        event,
        reader,
        max_attribute_bytes,
    )?);
    let table_index = character.tables.len() - 1;
    Ok((paragraph_index, character_index, table_index))
}

fn push_story_table_row(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    current_table: Option<(usize, usize, usize)>,
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<()> {
    let table = current_story_table_mut(paragraph_style_ranges, current_table, "Row")?;
    table.rows.push(StoryTableRow::from_xml_event(
        event,
        reader,
        max_attribute_bytes,
    )?);
    Ok(())
}

fn push_story_table_column(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    current_table: Option<(usize, usize, usize)>,
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<()> {
    let table = current_story_table_mut(paragraph_style_ranges, current_table, "Column")?;
    table.columns.push(StoryTableColumn::from_xml_event(
        event,
        reader,
        max_attribute_bytes,
    )?);
    Ok(())
}

fn push_story_table_cell(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    current_table: Option<(usize, usize, usize)>,
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
) -> Result<()> {
    let table = current_story_table_mut(paragraph_style_ranges, current_table, "Cell")?;
    table.cells.push(StoryTableCell::from_xml_event(
        event,
        reader,
        max_attribute_bytes,
    )?);
    Ok(())
}

fn current_story_table_mut<'a>(
    paragraph_style_ranges: &'a mut [ParagraphStyleRange],
    current_table: Option<(usize, usize, usize)>,
    element: &'static str,
) -> Result<&'a mut StoryTable> {
    let Some((paragraph_index, character_index, table_index)) = current_table else {
        return Err(IdmlError::InvalidElement {
            element: element.to_owned(),
            reason: "table child outside table",
        });
    };
    paragraph_style_ranges
        .get_mut(paragraph_index)
        .and_then(|paragraph| paragraph.character_style_ranges.get_mut(character_index))
        .and_then(|character| character.tables.get_mut(table_index))
        .ok_or_else(|| IdmlError::InvalidElement {
            element: element.to_owned(),
            reason: "table parser state is inconsistent",
        })
}

fn maybe_start_story_property(
    current_property: &mut Option<StoryPropertyCapture>,
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
    current_paragraph_range: Option<usize>,
    current_character_range: Option<usize>,
) -> Result<()> {
    let Some((element, target)) = story_property_target(
        event.name().as_ref(),
        current_paragraph_range,
        current_character_range,
    ) else {
        return Ok(());
    };

    if current_property.is_some() {
        return Err(IdmlError::InvalidElement {
            element: element.to_owned(),
            reason: "nested story property",
        });
    }

    *current_property = Some(StoryPropertyCapture {
        element,
        target,
        property: StoryProperty::from_xml_event(event, reader, max_attribute_bytes)?,
    });
    Ok(())
}

fn maybe_insert_empty_story_property(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    event: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    max_attribute_bytes: usize,
    current_paragraph_range: Option<usize>,
    current_character_range: Option<usize>,
) -> Result<bool> {
    let Some((_, target)) = story_property_target(
        event.name().as_ref(),
        current_paragraph_range,
        current_character_range,
    ) else {
        return Ok(false);
    };

    insert_story_property(
        paragraph_style_ranges,
        target,
        StoryProperty::from_xml_event(event, reader, max_attribute_bytes)?,
    )?;
    Ok(true)
}

fn story_property_target(
    element: &[u8],
    current_paragraph_range: Option<usize>,
    current_character_range: Option<usize>,
) -> Option<(&'static str, StoryPropertyTarget)> {
    match element {
        b"SameParaStyleSpacing" => current_paragraph_range.map(|paragraph| {
            (
                "SameParaStyleSpacing",
                StoryPropertyTarget::ParagraphSameParaStyleSpacing(paragraph),
            )
        }),
        b"RuleAboveGapColor" => current_paragraph_range.map(|paragraph| {
            (
                "RuleAboveGapColor",
                StoryPropertyTarget::ParagraphRuleAboveGapColor(paragraph),
            )
        }),
        b"RuleBelowGapColor" => current_paragraph_range.map(|paragraph| {
            (
                "RuleBelowGapColor",
                StoryPropertyTarget::ParagraphRuleBelowGapColor(paragraph),
            )
        }),
        b"AppliedFont" => {
            current_paragraph_range
                .zip(current_character_range)
                .map(|(paragraph, character)| {
                    (
                        "AppliedFont",
                        StoryPropertyTarget::CharacterAppliedFont(paragraph, character),
                    )
                })
        }
        b"Leading" => {
            current_paragraph_range
                .zip(current_character_range)
                .map(|(paragraph, character)| {
                    (
                        "Leading",
                        StoryPropertyTarget::CharacterLeading(paragraph, character),
                    )
                })
        }
        b"UnderlineGapColor" => {
            current_paragraph_range
                .zip(current_character_range)
                .map(|(paragraph, character)| {
                    (
                        "UnderlineGapColor",
                        StoryPropertyTarget::CharacterUnderlineGapColor(paragraph, character),
                    )
                })
        }
        b"StrikeThroughGapColor" => {
            current_paragraph_range
                .zip(current_character_range)
                .map(|(paragraph, character)| {
                    (
                        "StrikeThroughGapColor",
                        StoryPropertyTarget::CharacterStrikeThroughGapColor(paragraph, character),
                    )
                })
        }
        _ => None,
    }
}

fn property_end_matches(current_property: &Option<StoryPropertyCapture>, element: &[u8]) -> bool {
    current_property
        .as_ref()
        .is_some_and(|property| property.element.as_bytes() == element)
}

fn finish_story_property(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    current_property: &mut Option<StoryPropertyCapture>,
) -> Result<()> {
    let Some(capture) = current_property.take() else {
        return Ok(());
    };
    insert_story_property(paragraph_style_ranges, capture.target, capture.property)
}

fn insert_story_property(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    target: StoryPropertyTarget,
    property: StoryProperty,
) -> Result<()> {
    match target {
        StoryPropertyTarget::ParagraphSameParaStyleSpacing(paragraph) => set_paragraph_property(
            paragraph_style_ranges,
            paragraph,
            "SameParaStyleSpacing",
            |properties| &mut properties.same_para_style_spacing,
            property,
        ),
        StoryPropertyTarget::ParagraphRuleAboveGapColor(paragraph) => set_paragraph_property(
            paragraph_style_ranges,
            paragraph,
            "RuleAboveGapColor",
            |properties| &mut properties.rule_above_gap_color,
            property,
        ),
        StoryPropertyTarget::ParagraphRuleBelowGapColor(paragraph) => set_paragraph_property(
            paragraph_style_ranges,
            paragraph,
            "RuleBelowGapColor",
            |properties| &mut properties.rule_below_gap_color,
            property,
        ),
        StoryPropertyTarget::CharacterAppliedFont(paragraph, character) => set_character_property(
            paragraph_style_ranges,
            paragraph,
            character,
            "AppliedFont",
            |properties| &mut properties.applied_font,
            property,
        ),
        StoryPropertyTarget::CharacterLeading(paragraph, character) => set_character_property(
            paragraph_style_ranges,
            paragraph,
            character,
            "Leading",
            |properties| &mut properties.leading,
            property,
        ),
        StoryPropertyTarget::CharacterUnderlineGapColor(paragraph, character) => {
            set_character_property(
                paragraph_style_ranges,
                paragraph,
                character,
                "UnderlineGapColor",
                |properties| &mut properties.underline_gap_color,
                property,
            )
        }
        StoryPropertyTarget::CharacterStrikeThroughGapColor(paragraph, character) => {
            set_character_property(
                paragraph_style_ranges,
                paragraph,
                character,
                "StrikeThroughGapColor",
                |properties| &mut properties.strike_through_gap_color,
                property,
            )
        }
    }
}

fn set_paragraph_property(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    paragraph: usize,
    element: &'static str,
    selector: impl FnOnce(&mut ParagraphStyleProperties) -> &mut Option<StoryProperty>,
    property: StoryProperty,
) -> Result<()> {
    let Some(paragraph) = paragraph_style_ranges.get_mut(paragraph) else {
        return Err(inconsistent_property_state(element));
    };
    set_story_property_slot(selector(&mut paragraph.properties), element, property)
}

fn set_character_property(
    paragraph_style_ranges: &mut [ParagraphStyleRange],
    paragraph: usize,
    character: usize,
    element: &'static str,
    selector: impl FnOnce(&mut CharacterStyleProperties) -> &mut Option<StoryProperty>,
    property: StoryProperty,
) -> Result<()> {
    let Some(character) = paragraph_style_ranges
        .get_mut(paragraph)
        .and_then(|paragraph| paragraph.character_style_ranges.get_mut(character))
    else {
        return Err(inconsistent_property_state(element));
    };
    set_story_property_slot(selector(&mut character.properties), element, property)
}

fn set_story_property_slot(
    slot: &mut Option<StoryProperty>,
    element: &'static str,
    property: StoryProperty,
) -> Result<()> {
    if slot.is_some() {
        return Err(IdmlError::InvalidElement {
            element: element.to_owned(),
            reason: "duplicate story property",
        });
    }
    *slot = Some(property);
    Ok(())
}

fn inconsistent_property_state(element: &'static str) -> IdmlError {
    IdmlError::InvalidElement {
        element: element.to_owned(),
        reason: "story property parser state is inconsistent",
    }
}

fn normalize_default_style_ranges(ranges: &mut Vec<ParagraphStyleRange>, text: &str) {
    let [paragraph] = ranges.as_mut_slice() else {
        return;
    };
    if paragraph.applied_paragraph_style.is_some()
        || paragraph.justification.is_some()
        || paragraph.hyphenation.is_some()
        || paragraph.rule_above_line_weight.is_some()
        || paragraph.rule_below_line_weight.is_some()
        || paragraph.hyphenation_zone.is_some()
        || paragraph.split_column_inside_gutter.is_some()
        || paragraph.space_before.is_some()
        || paragraph.space_after.is_some()
        || paragraph.properties != ParagraphStyleProperties::default()
    {
        return;
    }

    let [character] = paragraph.character_style_ranges.as_slice() else {
        return;
    };
    if character.applied_character_style.is_none()
        && character.point_size.is_none()
        && character.font_style.is_none()
        && character.fill_color.is_none()
        && character.tracking.is_none()
        && character.stroke_weight.is_none()
        && character.ruby_font_size.is_none()
        && character.kenten_font_size.is_none()
        && character.capitalization.is_none()
        && character.miter_limit.is_none()
        && character.stroke_alignment.is_none()
        && character.horizontal_scale.is_none()
        && character.vertical_scale.is_none()
        && character.properties == CharacterStyleProperties::default()
        && character.tables.is_empty()
        && character.text == text
    {
        ranges.clear();
    }
}

fn exit_content_depth(content_depth: &mut usize) -> Result<()> {
    if *content_depth == 0 {
        return Err(IdmlError::InvalidElement {
            element: "Content".to_owned(),
            reason: "unexpected closing Content element",
        });
    }
    *content_depth -= 1;
    Ok(())
}

fn serialize_story(story: &Story) -> Result<String> {
    validate_serialized_story_limits(story)?;
    validate_xml_text("story text", &story.text)?;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Story");
    if let Some(id) = &story.id {
        push_attr(&mut xml, "Story", "Self", id)?;
    }
    push_story_metadata(&mut xml, &story.metadata)?;
    xml.push_str(">\n");
    if story.paragraph_style_ranges.is_empty() {
        xml.push_str("  <ParagraphStyleRange>\n    <CharacterStyleRange>");
        push_story_text_content(&mut xml, &story.text);
        xml.push_str("</CharacterStyleRange>\n  </ParagraphStyleRange>\n");
    } else {
        serialize_paragraph_style_ranges(&mut xml, &story.paragraph_style_ranges)?;
    }
    if let Some(preference) = &story.preference {
        serialize_story_preference(&mut xml, preference)?;
    }
    if let Some(option) = &story.incopy_export_option {
        serialize_incopy_export_option(&mut xml, option)?;
    }
    xml.push_str("</Story>\n");
    Ok(xml)
}

fn validate_serialized_story_limits(story: &Story) -> Result<()> {
    if let Some(id) = &story.id {
        enforce_story_attribute_bytes(id.len(), DEFAULT_MAX_STORY_ATTRIBUTE_BYTES)?;
    }
    validate_story_metadata(&story.metadata)?;
    if let Some(preference) = &story.preference {
        validate_story_preference(preference)?;
    }
    if let Some(option) = &story.incopy_export_option {
        validate_incopy_export_option(option)?;
    }
    enforce_serialized_story_text_bytes(story.text.len())?;
    validate_story_style_ranges(story)?;
    Ok(())
}

fn validate_story_metadata(metadata: &StoryMetadata) -> Result<()> {
    validate_optional_story_attr(
        "Story",
        "AppliedTOCStyle",
        metadata.applied_toc_style.as_deref(),
    )?;
    validate_optional_story_attr("Story", "StoryTitle", metadata.story_title.as_deref())?;
    validate_optional_story_attr(
        "Story",
        "AppliedNamedGrid",
        metadata.applied_named_grid.as_deref(),
    )
}

fn validate_story_preference(preference: &StoryPreference) -> Result<()> {
    validate_optional_story_attr(
        "StoryPreference",
        "OpticalMarginSize",
        preference.optical_margin_size.as_deref(),
    )?;
    validate_optional_story_attr(
        "StoryPreference",
        "FrameType",
        preference.frame_type.as_deref(),
    )?;
    validate_optional_story_attr(
        "StoryPreference",
        "StoryOrientation",
        preference.story_orientation.as_deref(),
    )?;
    validate_optional_story_attr(
        "StoryPreference",
        "StoryDirection",
        preference.story_direction.as_deref(),
    )
}

fn validate_incopy_export_option(_: &InCopyExportOption) -> Result<()> {
    Ok(())
}

fn validate_story_style_ranges(story: &Story) -> Result<()> {
    if story.paragraph_style_ranges.is_empty() {
        return Ok(());
    }

    let mut range_text = String::new();
    for paragraph in &story.paragraph_style_ranges {
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "AppliedParagraphStyle",
            paragraph.applied_paragraph_style.as_deref(),
        )?;
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "Justification",
            paragraph.justification.as_deref(),
        )?;
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "RuleAboveLineWeight",
            paragraph.rule_above_line_weight.as_deref(),
        )?;
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "RuleBelowLineWeight",
            paragraph.rule_below_line_weight.as_deref(),
        )?;
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "HyphenationZone",
            paragraph.hyphenation_zone.as_deref(),
        )?;
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "SplitColumnInsideGutter",
            paragraph.split_column_inside_gutter.as_deref(),
        )?;
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "SpaceBefore",
            paragraph.space_before.as_deref(),
        )?;
        validate_optional_story_attr(
            "ParagraphStyleRange",
            "SpaceAfter",
            paragraph.space_after.as_deref(),
        )?;
        validate_paragraph_style_properties(&paragraph.properties)?;

        for character in &paragraph.character_style_ranges {
            validate_optional_story_attr(
                "CharacterStyleRange",
                "AppliedCharacterStyle",
                character.applied_character_style.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "PointSize",
                character.point_size.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "FontStyle",
                character.font_style.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "FillColor",
                character.fill_color.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "Tracking",
                character.tracking.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "StrokeWeight",
                character.stroke_weight.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "RubyFontSize",
                character.ruby_font_size.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "KentenFontSize",
                character.kenten_font_size.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "Capitalization",
                character.capitalization.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "MiterLimit",
                character.miter_limit.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "StrokeAlignment",
                character.stroke_alignment.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "HorizontalScale",
                character.horizontal_scale.as_deref(),
            )?;
            validate_optional_story_attr(
                "CharacterStyleRange",
                "VerticalScale",
                character.vertical_scale.as_deref(),
            )?;
            validate_character_style_properties(&character.properties)?;
            validate_story_tables(&character.tables)?;
            validate_xml_text("story style range text", &character.text)?;
            push_limited(
                &mut range_text,
                &character.text,
                DEFAULT_MAX_STORY_TEXT_BYTES,
            )?;
        }
    }

    if range_text != story.text {
        return Err(IdmlError::InvalidText {
            what: "story style range text",
            reason: "does not match story text",
        });
    }
    Ok(())
}

fn validate_paragraph_style_properties(properties: &ParagraphStyleProperties) -> Result<()> {
    validate_story_property(
        "SameParaStyleSpacing",
        properties.same_para_style_spacing.as_ref(),
    )?;
    validate_story_property(
        "RuleAboveGapColor",
        properties.rule_above_gap_color.as_ref(),
    )?;
    validate_story_property(
        "RuleBelowGapColor",
        properties.rule_below_gap_color.as_ref(),
    )
}

fn validate_character_style_properties(properties: &CharacterStyleProperties) -> Result<()> {
    validate_story_property("AppliedFont", properties.applied_font.as_ref())?;
    validate_story_property("Leading", properties.leading.as_ref())?;
    validate_story_property("UnderlineGapColor", properties.underline_gap_color.as_ref())?;
    validate_story_property(
        "StrikeThroughGapColor",
        properties.strike_through_gap_color.as_ref(),
    )
}

fn validate_story_property(element: &'static str, property: Option<&StoryProperty>) -> Result<()> {
    let Some(property) = property else {
        return Ok(());
    };
    validate_optional_story_attr(element, "type", property.value_type.as_deref())?;
    enforce_story_attribute_bytes(property.value.len(), DEFAULT_MAX_STORY_ATTRIBUTE_BYTES)?;
    validate_xml_text("story property value", &property.value)
}

fn validate_story_tables(tables: &[StoryTable]) -> Result<()> {
    for table in tables {
        validate_optional_story_attr("Table", "Self", table.id.as_deref())?;
        validate_optional_story_attr("Table", "HeaderRowCount", table.header_row_count.as_deref())?;
        validate_optional_story_attr("Table", "FooterRowCount", table.footer_row_count.as_deref())?;
        validate_optional_story_attr("Table", "BodyRowCount", table.body_row_count.as_deref())?;
        validate_optional_story_attr("Table", "ColumnCount", table.column_count.as_deref())?;
        validate_optional_story_attr(
            "Table",
            "AppliedTableStyle",
            table.applied_table_style.as_deref(),
        )?;
        validate_optional_story_attr("Table", "TableDirection", table.table_direction.as_deref())?;
        validate_table_text_insets("Table", &table.text_frame)?;

        for row in &table.rows {
            validate_optional_story_attr("Row", "Self", row.id.as_deref())?;
            validate_optional_story_attr("Row", "Name", row.name.as_deref())?;
            validate_table_text_insets("Row", &row.text_frame)?;
            validate_optional_story_attr(
                "Row",
                "SingleRowHeight",
                row.single_row_height.as_deref(),
            )?;
            validate_optional_story_attr("Row", "MinimumHeight", row.minimum_height.as_deref())?;
        }
        for column in &table.columns {
            validate_optional_story_attr("Column", "Self", column.id.as_deref())?;
            validate_optional_story_attr("Column", "Name", column.name.as_deref())?;
            validate_table_text_insets("Column", &column.text_frame)?;
            validate_optional_story_attr(
                "Column",
                "SingleColumnWidth",
                column.single_column_width.as_deref(),
            )?;
        }
        for cell in &table.cells {
            validate_optional_story_attr("Cell", "Self", cell.id.as_deref())?;
            validate_optional_story_attr("Cell", "Name", cell.name.as_deref())?;
            validate_optional_story_attr("Cell", "RowSpan", cell.row_span.as_deref())?;
            validate_optional_story_attr("Cell", "ColumnSpan", cell.column_span.as_deref())?;
            validate_optional_story_attr("Cell", "CellType", cell.cell_type.as_deref())?;
            validate_optional_story_attr(
                "Cell",
                "AppliedCellStyle",
                cell.applied_cell_style.as_deref(),
            )?;
            validate_optional_story_attr(
                "Cell",
                "AppliedCellStylePriority",
                cell.applied_cell_style_priority.as_deref(),
            )?;
            validate_table_text_insets("Cell", &cell.text_frame)?;
            validate_optional_story_attr("Cell", "LeftInset", cell.left_inset.as_deref())?;
            validate_optional_story_attr("Cell", "TopInset", cell.top_inset.as_deref())?;
            validate_optional_story_attr("Cell", "RightInset", cell.right_inset.as_deref())?;
            validate_optional_story_attr("Cell", "BottomInset", cell.bottom_inset.as_deref())?;
            validate_optional_story_attr("Cell", "FillColor", cell.fill_color.as_deref())?;
        }
    }
    Ok(())
}

fn validate_table_text_insets(element: &'static str, insets: &TableTextInsets) -> Result<()> {
    validate_optional_story_attr(element, "TextTopInset", insets.text_top_inset.as_deref())?;
    validate_optional_story_attr(element, "TextLeftInset", insets.text_left_inset.as_deref())?;
    validate_optional_story_attr(
        element,
        "TextBottomInset",
        insets.text_bottom_inset.as_deref(),
    )?;
    validate_optional_story_attr(
        element,
        "TextRightInset",
        insets.text_right_inset.as_deref(),
    )
}

fn validate_optional_story_attr(
    element: &'static str,
    attribute: &'static str,
    value: Option<&str>,
) -> Result<()> {
    if let Some(value) = value {
        enforce_story_attribute_bytes(value.len(), DEFAULT_MAX_STORY_ATTRIBUTE_BYTES)?;
        validate_xml_attribute(element, attribute, value)?;
    }
    Ok(())
}

fn enforce_serialized_story_text_bytes(actual: usize) -> Result<()> {
    if actual > DEFAULT_MAX_STORY_TEXT_BYTES {
        return Err(IdmlError::LimitExceeded {
            what: "story text bytes",
            limit: DEFAULT_MAX_STORY_TEXT_BYTES as u64,
            actual: actual as u64,
        });
    }
    Ok(())
}

fn push_content_if_needed(xml: &mut String, content: &mut String) {
    if content.is_empty() {
        return;
    }
    xml.push_str("<Content>");
    xml.push_str(partial_escape(content.as_str()).as_ref());
    xml.push_str("</Content>");
    content.clear();
}

fn push_story_text_content(xml: &mut String, text: &str) {
    let mut content = String::new();
    for ch in text.chars() {
        match ch {
            '\n' => {
                push_content_if_needed(xml, &mut content);
                xml.push_str("<Br/>");
            }
            '\t' => {
                push_content_if_needed(xml, &mut content);
                xml.push_str("<Tab/>");
            }
            _ => content.push(ch),
        }
    }
    push_content_if_needed(xml, &mut content);
}

fn push_story_metadata(xml: &mut String, metadata: &StoryMetadata) -> Result<()> {
    if let Some(applied_toc_style) = &metadata.applied_toc_style {
        push_attr(xml, "Story", "AppliedTOCStyle", applied_toc_style)?;
    }
    if let Some(track_changes) = metadata.track_changes {
        push_attr(xml, "Story", "TrackChanges", bool_str(track_changes))?;
    }
    if let Some(story_title) = &metadata.story_title {
        push_attr(xml, "Story", "StoryTitle", story_title)?;
    }
    if let Some(applied_named_grid) = &metadata.applied_named_grid {
        push_attr(xml, "Story", "AppliedNamedGrid", applied_named_grid)?;
    }
    if let Some(user_text) = metadata.user_text {
        push_attr(xml, "Story", "UserText", bool_str(user_text))?;
    }
    if let Some(is_endnote_story) = metadata.is_endnote_story {
        push_attr(xml, "Story", "IsEndnoteStory", bool_str(is_endnote_story))?;
    }
    Ok(())
}

fn serialize_paragraph_style_ranges(
    xml: &mut String,
    ranges: &[ParagraphStyleRange],
) -> Result<()> {
    for paragraph in ranges {
        xml.push_str("  <ParagraphStyleRange");
        if let Some(applied_paragraph_style) = &paragraph.applied_paragraph_style {
            push_attr(
                xml,
                "ParagraphStyleRange",
                "AppliedParagraphStyle",
                applied_paragraph_style,
            )?;
        }
        if let Some(justification) = &paragraph.justification {
            push_attr(xml, "ParagraphStyleRange", "Justification", justification)?;
        }
        if let Some(hyphenation) = paragraph.hyphenation {
            push_attr(
                xml,
                "ParagraphStyleRange",
                "Hyphenation",
                bool_str(hyphenation),
            )?;
        }
        if let Some(rule_above_line_weight) = &paragraph.rule_above_line_weight {
            push_attr(
                xml,
                "ParagraphStyleRange",
                "RuleAboveLineWeight",
                rule_above_line_weight,
            )?;
        }
        if let Some(rule_below_line_weight) = &paragraph.rule_below_line_weight {
            push_attr(
                xml,
                "ParagraphStyleRange",
                "RuleBelowLineWeight",
                rule_below_line_weight,
            )?;
        }
        if let Some(hyphenation_zone) = &paragraph.hyphenation_zone {
            push_attr(
                xml,
                "ParagraphStyleRange",
                "HyphenationZone",
                hyphenation_zone,
            )?;
        }
        if let Some(split_column_inside_gutter) = &paragraph.split_column_inside_gutter {
            push_attr(
                xml,
                "ParagraphStyleRange",
                "SplitColumnInsideGutter",
                split_column_inside_gutter,
            )?;
        }
        if let Some(space_before) = &paragraph.space_before {
            push_attr(xml, "ParagraphStyleRange", "SpaceBefore", space_before)?;
        }
        if let Some(space_after) = &paragraph.space_after {
            push_attr(xml, "ParagraphStyleRange", "SpaceAfter", space_after)?;
        }
        if paragraph.character_style_ranges.is_empty()
            && paragraph.properties == ParagraphStyleProperties::default()
        {
            xml.push_str(" />\n");
            continue;
        }

        xml.push_str(">\n");
        serialize_paragraph_style_properties(xml, &paragraph.properties)?;
        for character in &paragraph.character_style_ranges {
            serialize_character_style_range(xml, character)?;
        }
        xml.push_str("  </ParagraphStyleRange>\n");
    }
    Ok(())
}

fn serialize_character_style_range(xml: &mut String, range: &CharacterStyleRange) -> Result<()> {
    xml.push_str("    <CharacterStyleRange");
    if let Some(applied_character_style) = &range.applied_character_style {
        push_attr(
            xml,
            "CharacterStyleRange",
            "AppliedCharacterStyle",
            applied_character_style,
        )?;
    }
    if let Some(point_size) = &range.point_size {
        push_attr(xml, "CharacterStyleRange", "PointSize", point_size)?;
    }
    if let Some(font_style) = &range.font_style {
        push_attr(xml, "CharacterStyleRange", "FontStyle", font_style)?;
    }
    if let Some(fill_color) = &range.fill_color {
        push_attr(xml, "CharacterStyleRange", "FillColor", fill_color)?;
    }
    if let Some(tracking) = &range.tracking {
        push_attr(xml, "CharacterStyleRange", "Tracking", tracking)?;
    }
    if let Some(stroke_weight) = &range.stroke_weight {
        push_attr(xml, "CharacterStyleRange", "StrokeWeight", stroke_weight)?;
    }
    if let Some(ruby_font_size) = &range.ruby_font_size {
        push_attr(xml, "CharacterStyleRange", "RubyFontSize", ruby_font_size)?;
    }
    if let Some(kenten_font_size) = &range.kenten_font_size {
        push_attr(
            xml,
            "CharacterStyleRange",
            "KentenFontSize",
            kenten_font_size,
        )?;
    }
    if let Some(capitalization) = &range.capitalization {
        push_attr(xml, "CharacterStyleRange", "Capitalization", capitalization)?;
    }
    if let Some(miter_limit) = &range.miter_limit {
        push_attr(xml, "CharacterStyleRange", "MiterLimit", miter_limit)?;
    }
    if let Some(stroke_alignment) = &range.stroke_alignment {
        push_attr(
            xml,
            "CharacterStyleRange",
            "StrokeAlignment",
            stroke_alignment,
        )?;
    }
    if let Some(horizontal_scale) = &range.horizontal_scale {
        push_attr(
            xml,
            "CharacterStyleRange",
            "HorizontalScale",
            horizontal_scale,
        )?;
    }
    if let Some(vertical_scale) = &range.vertical_scale {
        push_attr(xml, "CharacterStyleRange", "VerticalScale", vertical_scale)?;
    }
    xml.push('>');
    serialize_character_style_properties(xml, &range.properties)?;
    push_story_text_content(xml, &range.text);
    serialize_story_tables(xml, &range.tables)?;
    xml.push_str("</CharacterStyleRange>\n");
    Ok(())
}

fn serialize_story_tables(xml: &mut String, tables: &[StoryTable]) -> Result<()> {
    for table in tables {
        xml.push_str("\n      <Table");
        push_story_table_attrs(xml, table)?;
        if table.rows.is_empty() && table.columns.is_empty() && table.cells.is_empty() {
            xml.push_str(" />\n      ");
            continue;
        }

        xml.push_str(">\n");
        for row in &table.rows {
            xml.push_str("        <Row");
            push_story_table_row_attrs(xml, row)?;
            xml.push_str(" />\n");
        }
        for column in &table.columns {
            xml.push_str("        <Column");
            push_story_table_column_attrs(xml, column)?;
            xml.push_str(" />\n");
        }
        for cell in &table.cells {
            xml.push_str("        <Cell");
            push_story_table_cell_attrs(xml, cell)?;
            xml.push_str(" />\n");
        }
        xml.push_str("      </Table>\n      ");
    }
    Ok(())
}

fn push_story_table_attrs(xml: &mut String, table: &StoryTable) -> Result<()> {
    if let Some(id) = &table.id {
        push_attr(xml, "Table", "Self", id)?;
    }
    if let Some(header_row_count) = &table.header_row_count {
        push_attr(xml, "Table", "HeaderRowCount", header_row_count)?;
    }
    if let Some(footer_row_count) = &table.footer_row_count {
        push_attr(xml, "Table", "FooterRowCount", footer_row_count)?;
    }
    push_table_text_insets(xml, "Table", &table.text_frame)?;
    if let Some(body_row_count) = &table.body_row_count {
        push_attr(xml, "Table", "BodyRowCount", body_row_count)?;
    }
    if let Some(column_count) = &table.column_count {
        push_attr(xml, "Table", "ColumnCount", column_count)?;
    }
    if let Some(applied_table_style) = &table.applied_table_style {
        push_attr(xml, "Table", "AppliedTableStyle", applied_table_style)?;
    }
    if let Some(table_direction) = &table.table_direction {
        push_attr(xml, "Table", "TableDirection", table_direction)?;
    }
    Ok(())
}

fn push_story_table_row_attrs(xml: &mut String, row: &StoryTableRow) -> Result<()> {
    if let Some(id) = &row.id {
        push_attr(xml, "Row", "Self", id)?;
    }
    if let Some(name) = &row.name {
        push_attr(xml, "Row", "Name", name)?;
    }
    push_table_text_insets(xml, "Row", &row.text_frame)?;
    if let Some(single_row_height) = &row.single_row_height {
        push_attr(xml, "Row", "SingleRowHeight", single_row_height)?;
    }
    if let Some(minimum_height) = &row.minimum_height {
        push_attr(xml, "Row", "MinimumHeight", minimum_height)?;
    }
    if let Some(auto_grow) = row.auto_grow {
        push_attr(xml, "Row", "AutoGrow", bool_str(auto_grow))?;
    }
    Ok(())
}

fn push_story_table_column_attrs(xml: &mut String, column: &StoryTableColumn) -> Result<()> {
    if let Some(id) = &column.id {
        push_attr(xml, "Column", "Self", id)?;
    }
    if let Some(name) = &column.name {
        push_attr(xml, "Column", "Name", name)?;
    }
    push_table_text_insets(xml, "Column", &column.text_frame)?;
    if let Some(single_column_width) = &column.single_column_width {
        push_attr(xml, "Column", "SingleColumnWidth", single_column_width)?;
    }
    Ok(())
}

fn push_story_table_cell_attrs(xml: &mut String, cell: &StoryTableCell) -> Result<()> {
    if let Some(id) = &cell.id {
        push_attr(xml, "Cell", "Self", id)?;
    }
    if let Some(name) = &cell.name {
        push_attr(xml, "Cell", "Name", name)?;
    }
    if let Some(row_span) = &cell.row_span {
        push_attr(xml, "Cell", "RowSpan", row_span)?;
    }
    if let Some(column_span) = &cell.column_span {
        push_attr(xml, "Cell", "ColumnSpan", column_span)?;
    }
    if let Some(cell_type) = &cell.cell_type {
        push_attr(xml, "Cell", "CellType", cell_type)?;
    }
    push_table_text_insets(xml, "Cell", &cell.text_frame)?;
    if let Some(applied_cell_style) = &cell.applied_cell_style {
        push_attr(xml, "Cell", "AppliedCellStyle", applied_cell_style)?;
    }
    if let Some(applied_cell_style_priority) = &cell.applied_cell_style_priority {
        push_attr(
            xml,
            "Cell",
            "AppliedCellStylePriority",
            applied_cell_style_priority,
        )?;
    }
    if let Some(left_inset) = &cell.left_inset {
        push_attr(xml, "Cell", "LeftInset", left_inset)?;
    }
    if let Some(top_inset) = &cell.top_inset {
        push_attr(xml, "Cell", "TopInset", top_inset)?;
    }
    if let Some(right_inset) = &cell.right_inset {
        push_attr(xml, "Cell", "RightInset", right_inset)?;
    }
    if let Some(bottom_inset) = &cell.bottom_inset {
        push_attr(xml, "Cell", "BottomInset", bottom_inset)?;
    }
    if let Some(fill_color) = &cell.fill_color {
        push_attr(xml, "Cell", "FillColor", fill_color)?;
    }
    Ok(())
}

fn push_table_text_insets(
    xml: &mut String,
    element: &'static str,
    insets: &TableTextInsets,
) -> Result<()> {
    if let Some(text_top_inset) = &insets.text_top_inset {
        push_attr(xml, element, "TextTopInset", text_top_inset)?;
    }
    if let Some(text_left_inset) = &insets.text_left_inset {
        push_attr(xml, element, "TextLeftInset", text_left_inset)?;
    }
    if let Some(text_bottom_inset) = &insets.text_bottom_inset {
        push_attr(xml, element, "TextBottomInset", text_bottom_inset)?;
    }
    if let Some(text_right_inset) = &insets.text_right_inset {
        push_attr(xml, element, "TextRightInset", text_right_inset)?;
    }
    if let Some(clip_content_to_text_cell) = insets.clip_content_to_text_cell {
        push_attr(
            xml,
            element,
            "ClipContentToTextCell",
            bool_str(clip_content_to_text_cell),
        )?;
    }
    Ok(())
}

fn serialize_paragraph_style_properties(
    xml: &mut String,
    properties: &ParagraphStyleProperties,
) -> Result<()> {
    if properties == &ParagraphStyleProperties::default() {
        return Ok(());
    }

    xml.push_str("    <Properties>\n");
    serialize_story_property(
        xml,
        "SameParaStyleSpacing",
        properties.same_para_style_spacing.as_ref(),
        6,
    )?;
    serialize_story_property(
        xml,
        "RuleAboveGapColor",
        properties.rule_above_gap_color.as_ref(),
        6,
    )?;
    serialize_story_property(
        xml,
        "RuleBelowGapColor",
        properties.rule_below_gap_color.as_ref(),
        6,
    )?;
    xml.push_str("    </Properties>\n");
    Ok(())
}

fn serialize_character_style_properties(
    xml: &mut String,
    properties: &CharacterStyleProperties,
) -> Result<()> {
    if properties == &CharacterStyleProperties::default() {
        return Ok(());
    }

    xml.push_str("\n      <Properties>\n");
    serialize_story_property(xml, "Leading", properties.leading.as_ref(), 8)?;
    serialize_story_property(xml, "AppliedFont", properties.applied_font.as_ref(), 8)?;
    serialize_story_property(
        xml,
        "UnderlineGapColor",
        properties.underline_gap_color.as_ref(),
        8,
    )?;
    serialize_story_property(
        xml,
        "StrikeThroughGapColor",
        properties.strike_through_gap_color.as_ref(),
        8,
    )?;
    xml.push_str("      </Properties>\n      ");
    Ok(())
}

fn serialize_story_property(
    xml: &mut String,
    element: &'static str,
    property: Option<&StoryProperty>,
    indent: usize,
) -> Result<()> {
    let Some(property) = property else {
        return Ok(());
    };

    for _ in 0..indent {
        xml.push(' ');
    }
    xml.push('<');
    xml.push_str(element);
    if let Some(value_type) = &property.value_type {
        push_attr(xml, element, "type", value_type)?;
    }
    xml.push('>');
    xml.push_str(partial_escape(property.value.as_str()).as_ref());
    xml.push_str("</");
    xml.push_str(element);
    xml.push_str(">\n");
    Ok(())
}

fn serialize_story_preference(xml: &mut String, preference: &StoryPreference) -> Result<()> {
    xml.push_str("  <StoryPreference");
    if let Some(optical_margin_alignment) = preference.optical_margin_alignment {
        push_attr(
            xml,
            "StoryPreference",
            "OpticalMarginAlignment",
            bool_str(optical_margin_alignment),
        )?;
    }
    if let Some(optical_margin_size) = &preference.optical_margin_size {
        push_attr(
            xml,
            "StoryPreference",
            "OpticalMarginSize",
            optical_margin_size,
        )?;
    }
    if let Some(frame_type) = &preference.frame_type {
        push_attr(xml, "StoryPreference", "FrameType", frame_type)?;
    }
    if let Some(story_orientation) = &preference.story_orientation {
        push_attr(
            xml,
            "StoryPreference",
            "StoryOrientation",
            story_orientation,
        )?;
    }
    if let Some(story_direction) = &preference.story_direction {
        push_attr(xml, "StoryPreference", "StoryDirection", story_direction)?;
    }
    xml.push_str(" />\n");
    Ok(())
}

fn serialize_incopy_export_option(xml: &mut String, option: &InCopyExportOption) -> Result<()> {
    xml.push_str("  <InCopyExportOption");
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

fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
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
    use super::{
        CharacterStyleProperties, CharacterStyleRange, InCopyExportOption,
        ParagraphStyleProperties, ParagraphStyleRange, Story, StoryMetadata, StoryParseOptions,
        StoryPreference, StoryProperty,
    };
    use crate::IdmlError;

    #[test]
    fn extracts_story_content_in_order() {
        let xml = r#"<Story Self="u2">
  <ParagraphStyleRange>
    <CharacterStyleRange>
      <Content>Hello &amp; welcome &#x21;</Content><Br/><Content>Next</Content><Tab/><Content>Cell</Content>
    </CharacterStyleRange>
  </ParagraphStyleRange>
</Story>"#;

        let story = Story::from_xml(xml).unwrap();

        assert_eq!(story.id.as_deref(), Some("u2"));
        assert_eq!(story.text, "Hello & welcome !\nNext\tCell");
    }

    #[test]
    fn extracts_story_from_idpkg_wrapper() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Story Self="u2">
    <ParagraphStyleRange>
      <CharacterStyleRange>
        <Content>Wrapped</Content><Br/><Content>Story</Content>
      </CharacterStyleRange>
    </ParagraphStyleRange>
  </Story>
</idPkg:Story>"#;

        let story = Story::from_xml(xml).unwrap();

        assert_eq!(story.id.as_deref(), Some("u2"));
        assert_eq!(story.text, "Wrapped\nStory");
    }

    #[test]
    fn parses_and_serializes_story_metadata() {
        let xml = r#"<Story Self="u2" AppliedTOCStyle="n" TrackChanges="false" StoryTitle="$ID/" AppliedNamedGrid="n" UserText="true" IsEndnoteStory="false">
  <StoryPreference OpticalMarginAlignment="true" OpticalMarginSize="12" FrameType="TextFrameType" StoryOrientation="Horizontal" StoryDirection="LeftToRightDirection" />
  <InCopyExportOption IncludeGraphicProxies="true" IncludeAllResources="false" />
  <ParagraphStyleRange>
    <CharacterStyleRange>
      <Content>Styled metadata</Content>
    </CharacterStyleRange>
  </ParagraphStyleRange>
</Story>"#;

        let story = Story::from_xml(xml).unwrap();

        assert_eq!(story.metadata.applied_toc_style.as_deref(), Some("n"));
        assert_eq!(story.metadata.track_changes, Some(false));
        assert_eq!(story.metadata.story_title.as_deref(), Some("$ID/"));
        assert_eq!(story.metadata.applied_named_grid.as_deref(), Some("n"));
        assert_eq!(story.metadata.user_text, Some(true));
        assert_eq!(story.metadata.is_endnote_story, Some(false));
        let preference = story.preference.as_ref().unwrap();
        assert_eq!(preference.optical_margin_alignment, Some(true));
        assert_eq!(preference.optical_margin_size.as_deref(), Some("12"));
        assert_eq!(preference.frame_type.as_deref(), Some("TextFrameType"));
        assert_eq!(preference.story_orientation.as_deref(), Some("Horizontal"));
        assert_eq!(
            preference.story_direction.as_deref(),
            Some("LeftToRightDirection")
        );
        let incopy = story.incopy_export_option.as_ref().unwrap();
        assert_eq!(incopy.include_graphic_proxies, Some(true));
        assert_eq!(incopy.include_all_resources, Some(false));

        let serialized = story.to_xml().unwrap();
        assert!(serialized.contains("AppliedTOCStyle=\"n\""));
        assert!(serialized.contains("TrackChanges=\"false\""));
        assert!(serialized.contains("StoryTitle=\"$ID/\""));
        assert!(serialized.contains("OpticalMarginAlignment=\"true\""));
        assert!(serialized.contains("StoryDirection=\"LeftToRightDirection\""));
        assert!(serialized.contains("IncludeGraphicProxies=\"true\""));
        assert_eq!(Story::from_xml(&serialized).unwrap(), story);
    }

    #[test]
    fn rejects_idpkg_story_wrapper_without_story_model() {
        let err = Story::from_xml(
            r#"<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" />"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "missing Story element",
            } if element == "idPkg:Story"
        ));
    }

    #[test]
    fn ignores_non_content_text() {
        let xml = r#"<Story Self="u2">metadata<Content>visible</Content></Story>"#;

        let story = Story::from_xml(xml).unwrap();

        assert_eq!(story.text, "visible");
    }

    #[test]
    fn parses_empty_story_root_id() {
        let story = Story::from_xml(r#"<Story Self="u2" />"#).unwrap();

        assert_eq!(story.id.as_deref(), Some("u2"));
        assert_eq!(story.text, "");
    }

    #[test]
    fn rejects_wrong_story_root_element() {
        let err = Story::from_xml("<Wrapper><Content>hidden</Content></Wrapper>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "expected Story root element",
            } if element == "Wrapper"
        ));
    }

    #[test]
    fn rejects_multiple_story_roots() {
        let err = Story::from_xml("<Story /><Story />").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "multiple root elements",
            } if element == "Story"
        ));
    }

    #[test]
    fn rejects_trailing_story_text_after_root() {
        let err = Story::from_xml("<Story />tail").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "non-whitespace character data outside root element",
            } if element == "#text"
        ));
    }

    #[test]
    fn allows_boundary_whitespace_around_story_root() {
        let story = Story::from_xml(" \n\t<Story Self=\"u2\" />\r\n ").unwrap();

        assert_eq!(story.id.as_deref(), Some("u2"));
    }

    #[test]
    fn rejects_malformed_xml_comments() {
        let err = Story::from_xml("<Story><!-- invalid -- comment --><Content>x</Content></Story>")
            .unwrap_err();

        assert!(matches!(err, IdmlError::Xml(_)));
    }

    #[test]
    fn rejects_mismatched_xml_end_tags() {
        let err = Story::from_xml("<Story><Content>x</Story></Content>").unwrap_err();

        assert!(matches!(err, IdmlError::Xml(_)));
    }

    #[test]
    fn rejects_xml_doctype_declarations() {
        let err =
            Story::from_xml("<!DOCTYPE Story><Story><Content>x</Content></Story>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "DOCTYPE declarations are not supported",
            } if element == "!DOCTYPE"
        ));
    }

    #[test]
    fn rejects_entity_references_outside_content() {
        let err = Story::from_xml("<Story>&amp;<Content>x</Content></Story>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "entity references are not supported here",
            } if element == "#reference"
        ));
    }

    #[test]
    fn rejects_child_elements_inside_content() {
        let err =
            Story::from_xml("<Story><Content>a<Span>b</Span>c</Content></Story>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "element inside story Content",
            } if element == "Span"
        ));
    }

    #[test]
    fn rejects_empty_child_elements_inside_content() {
        let err = Story::from_xml("<Story><Content>a<Br/>c</Content></Story>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "element inside story Content",
            } if element == "Br"
        ));
    }

    #[test]
    fn rejects_nested_content_elements() {
        let err =
            Story::from_xml("<Story><Content><Content>x</Content></Content></Story>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "element inside story Content",
            } if element == "Content"
        ));
    }

    #[test]
    fn content_depth_rejects_unexpected_close() {
        let mut content_depth = 0;
        let err = super::exit_content_depth(&mut content_depth).unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidElement {
                element,
                reason: "unexpected closing Content element",
            } if element == "Content"
        ));
    }

    #[test]
    fn enforces_story_text_limit() {
        let err = Story::from_xml_with_options(
            "<Story><Content>hello</Content></Story>",
            StoryParseOptions {
                max_text_bytes: 4,
                ..StoryParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(err, IdmlError::LimitExceeded { what, .. } if what == "story text bytes"));
    }

    #[test]
    fn enforces_attribute_byte_limit_before_decoding() {
        let err = Story::from_xml_with_options(
            r#"<Story Self="u12345" />"#,
            StoryParseOptions {
                max_attribute_bytes: 5,
                ..StoryParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "Story XML attribute bytes",
                limit: 5,
                actual: 6,
            }
        ));
    }

    #[test]
    fn enforces_xml_byte_limit() {
        let err = Story::from_xml_with_options(
            "<Story/>",
            StoryParseOptions {
                max_xml_bytes: 4,
                ..StoryParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "Story XML bytes",
                limit: 4,
                actual: 8,
            }
        ));
    }

    #[test]
    fn rejects_duplicate_story_id_attributes() {
        let err = Story::from_xml(r#"<Story Self="u1" Self="u2"><Content>x</Content></Story>"#)
            .unwrap_err();

        assert!(
            matches!(
                &err,
                IdmlError::InvalidAttribute {
                    element,
                    attribute: "Self",
                    reason: "duplicate attribute",
                } if element == "Story"
            ) || matches!(&err, IdmlError::XmlAttribute(_))
        );
    }

    #[test]
    fn rejects_xml_forbidden_story_id_attribute_on_parse() {
        let err =
            Story::from_xml("<Story Self=\"bad\0id\"><Content>x</Content></Story>").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Story"
        ));
    }

    #[test]
    fn rejects_xml_forbidden_empty_story_id_attribute_on_parse() {
        let err = Story::from_xml("<Story Self=\"bad\0id\" />").unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Story"
        ));
    }

    #[test]
    fn enforces_xml_depth_limit() {
        let err = Story::from_xml_with_options(
            "<Story><Wrapper><Content>hello</Content></Wrapper></Story>",
            StoryParseOptions {
                max_xml_depth: 2,
                ..StoryParseOptions::default()
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "Story XML depth",
                limit: 2,
                actual: 3,
            }
        ));
    }

    #[test]
    fn serializes_story_text_with_markers_and_escaping() {
        let story = Story {
            id: Some("u&\"2".to_owned()),
            text: "Hello & <world>\nNext\tCell".to_owned(),
            paragraph_style_ranges: Vec::new(),
            metadata: StoryMetadata {
                applied_toc_style: Some("n".to_owned()),
                track_changes: Some(false),
                story_title: Some("$ID/".to_owned()),
                applied_named_grid: Some("n".to_owned()),
                user_text: Some(true),
                is_endnote_story: Some(false),
            },
            preference: Some(StoryPreference {
                optical_margin_alignment: Some(false),
                optical_margin_size: Some("12".to_owned()),
                frame_type: Some("TextFrameType".to_owned()),
                story_orientation: Some("Horizontal".to_owned()),
                story_direction: Some("LeftToRightDirection".to_owned()),
            }),
            incopy_export_option: Some(InCopyExportOption {
                include_graphic_proxies: Some(true),
                include_all_resources: Some(false),
            }),
        };

        let xml = <Story as crate::XmlSaveable>::to_xml(&story).unwrap();
        let parsed = <Story as crate::XmlLoadable>::from_xml(&xml).unwrap();

        assert!(xml.contains("Self=\"u&amp;&quot;2\""));
        assert!(xml.contains("AppliedTOCStyle=\"n\""));
        assert!(xml.contains("StoryPreference"));
        assert!(xml.contains("InCopyExportOption"));
        assert!(xml.contains("<Content>Hello &amp; &lt;world&gt;</Content><Br/>"));
        assert!(xml.contains("<Tab/><Content>Cell</Content>"));
        assert_eq!(parsed, story);
    }

    #[test]
    fn parses_and_serializes_story_style_ranges() {
        let xml = r##"<Story Self="u2">
  <ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle" Justification="CenterAlign" Hyphenation="false" RuleAboveLineWeight="1" RuleBelowLineWeight="2" HyphenationZone="36" SplitColumnInsideGutter="8" SpaceBefore="4" SpaceAfter="6">
    <CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PointSize="12" FontStyle="Bold" FillColor="Color/Black" Tracking="15" StrokeWeight="0.5" RubyFontSize="6" KentenFontSize="7" Capitalization="AllCaps" MiterLimit="4" StrokeAlignment="CenterAlignment" HorizontalScale="98" VerticalScale="102">
      <Properties>
        <Leading type="unit">14</Leading>
        <AppliedFont type="string">Lato &amp; Sans</AppliedFont>
      </Properties>
      <Content>Hello</Content><Br/><Content>Next</Content>
    </CharacterStyleRange>
    <CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Emphasis" PointSize="10">
      <Tab/><Content>Cell</Content>
    </CharacterStyleRange>
  </ParagraphStyleRange>
  <ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Caption">
    <CharacterStyleRange FillColor="Color/Red"><Content>Tail</Content></CharacterStyleRange>
  </ParagraphStyleRange>
</Story>"##;

        let story = Story::from_xml(xml).unwrap();

        assert_eq!(story.text, "Hello\nNext\tCellTail");
        assert_eq!(story.paragraph_style_ranges.len(), 2);
        let paragraph = &story.paragraph_style_ranges[0];
        assert_eq!(
            paragraph.applied_paragraph_style.as_deref(),
            Some("ParagraphStyle/$ID/NormalParagraphStyle")
        );
        assert_eq!(paragraph.justification.as_deref(), Some("CenterAlign"));
        assert_eq!(paragraph.hyphenation, Some(false));
        assert_eq!(paragraph.rule_above_line_weight.as_deref(), Some("1"));
        assert_eq!(paragraph.rule_below_line_weight.as_deref(), Some("2"));
        assert_eq!(paragraph.hyphenation_zone.as_deref(), Some("36"));
        assert_eq!(paragraph.split_column_inside_gutter.as_deref(), Some("8"));
        assert_eq!(paragraph.space_before.as_deref(), Some("4"));
        assert_eq!(paragraph.space_after.as_deref(), Some("6"));
        assert_eq!(paragraph.character_style_ranges.len(), 2);
        let character = &paragraph.character_style_ranges[0];
        assert_eq!(
            character.applied_character_style.as_deref(),
            Some("CharacterStyle/$ID/[No character style]")
        );
        assert_eq!(character.point_size.as_deref(), Some("12"));
        assert_eq!(character.font_style.as_deref(), Some("Bold"));
        assert_eq!(character.fill_color.as_deref(), Some("Color/Black"));
        assert_eq!(character.tracking.as_deref(), Some("15"));
        assert_eq!(character.stroke_weight.as_deref(), Some("0.5"));
        assert_eq!(character.ruby_font_size.as_deref(), Some("6"));
        assert_eq!(character.kenten_font_size.as_deref(), Some("7"));
        assert_eq!(character.capitalization.as_deref(), Some("AllCaps"));
        assert_eq!(character.miter_limit.as_deref(), Some("4"));
        assert_eq!(
            character.stroke_alignment.as_deref(),
            Some("CenterAlignment")
        );
        assert_eq!(character.horizontal_scale.as_deref(), Some("98"));
        assert_eq!(character.vertical_scale.as_deref(), Some("102"));
        assert_eq!(
            character.properties.leading.as_ref(),
            Some(&StoryProperty {
                value_type: Some("unit".to_owned()),
                value: "14".to_owned()
            })
        );
        assert_eq!(
            character.properties.applied_font.as_ref(),
            Some(&StoryProperty {
                value_type: Some("string".to_owned()),
                value: "Lato & Sans".to_owned()
            })
        );
        assert_eq!(character.text, "Hello\nNext");
        assert_eq!(paragraph.character_style_ranges[1].text, "\tCell");
        assert_eq!(
            story.paragraph_style_ranges[1].character_style_ranges[0].text,
            "Tail"
        );

        let serialized = story.to_xml().unwrap();
        assert!(
            serialized
                .contains("AppliedParagraphStyle=\"ParagraphStyle/$ID/NormalParagraphStyle\"")
        );
        assert!(
            serialized
                .contains("AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\"")
        );
        assert!(serialized.contains("StrokeWeight=\"0.5\""));
        assert!(serialized.contains("RuleAboveLineWeight=\"1\""));
        assert!(serialized.contains("SplitColumnInsideGutter=\"8\""));
        assert!(serialized.contains("RubyFontSize=\"6\""));
        assert!(serialized.contains("HorizontalScale=\"98\""));
        assert!(serialized.contains("<AppliedFont type=\"string\">Lato &amp; Sans</AppliedFont>"));
        assert!(serialized.contains("<Leading type=\"unit\">14</Leading>"));
        assert_eq!(Story::from_xml(&serialized).unwrap(), story);
    }

    #[test]
    fn serializes_paragraph_and_character_style_properties() {
        let story = Story {
            id: Some("u1".to_owned()),
            text: "Styled".to_owned(),
            paragraph_style_ranges: vec![ParagraphStyleRange {
                applied_paragraph_style: Some("ParagraphStyle/Body".to_owned()),
                properties: ParagraphStyleProperties {
                    same_para_style_spacing: Some(StoryProperty {
                        value_type: Some("unit".to_owned()),
                        value: "3.5".to_owned(),
                    }),
                    rule_above_gap_color: Some(StoryProperty {
                        value_type: Some("string".to_owned()),
                        value: "Color/None".to_owned(),
                    }),
                    rule_below_gap_color: None,
                },
                character_style_ranges: vec![CharacterStyleRange {
                    applied_character_style: Some(
                        "CharacterStyle/$ID/[No character style]".to_owned(),
                    ),
                    properties: CharacterStyleProperties {
                        applied_font: Some(StoryProperty {
                            value_type: Some("string".to_owned()),
                            value: "Lato".to_owned(),
                        }),
                        leading: Some(StoryProperty {
                            value_type: Some("unit".to_owned()),
                            value: "12".to_owned(),
                        }),
                        underline_gap_color: Some(StoryProperty {
                            value_type: Some("string".to_owned()),
                            value: "Color/Black".to_owned(),
                        }),
                        strike_through_gap_color: None,
                    },
                    text: "Styled".to_owned(),
                    ..CharacterStyleRange::default()
                }],
                ..ParagraphStyleRange::default()
            }],
            metadata: Default::default(),
            preference: None,
            incopy_export_option: None,
        };

        let xml = story.to_xml().unwrap();
        let parsed = Story::from_xml(&xml).unwrap();

        assert!(xml.contains("<SameParaStyleSpacing type=\"unit\">3.5</SameParaStyleSpacing>"));
        assert!(xml.contains("<RuleAboveGapColor type=\"string\">Color/None</RuleAboveGapColor>"));
        assert!(xml.contains("<UnderlineGapColor type=\"string\">Color/Black</UnderlineGapColor>"));
        assert_eq!(parsed, story);
    }

    #[test]
    fn parses_and_serializes_story_table_metadata() {
        let xml = r#"<Story Self="u1">
  <ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Table Anchor">
    <CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
      <Table Self="t1" HeaderRowCount="1" FooterRowCount="0" TextTopInset="4" TextLeftInset="5" TextBottomInset="6" TextRightInset="7" ClipContentToTextCell="false" BodyRowCount="1" ColumnCount="1" AppliedTableStyle="TableStyle/$ID/[Basic Table]" TableDirection="LeftToRightDirection">
        <Row Self="r1" Name="0" TextTopInset="1" TextLeftInset="2" TextBottomInset="3" TextRightInset="4" ClipContentToTextCell="false" SingleRowHeight="10" MinimumHeight="8" AutoGrow="false" />
        <Column Self="c1" Name="0" TextTopInset="1" TextLeftInset="2" TextBottomInset="3" TextRightInset="4" ClipContentToTextCell="false" SingleColumnWidth="20" />
        <Cell Self="cell1" Name="0:0" RowSpan="1" ColumnSpan="1" CellType="TextTypeCell" TextTopInset="1" TextLeftInset="2" TextBottomInset="3" TextRightInset="4" ClipContentToTextCell="false" AppliedCellStyle="CellStyle/$ID/[None]" AppliedCellStylePriority="0" LeftInset="2" TopInset="1" RightInset="2" BottomInset="1" FillColor="Swatch/None">
          <ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Cell">
            <CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Cell">
              <Content>Cell text</Content>
            </CharacterStyleRange>
          </ParagraphStyleRange>
        </Cell>
      </Table>
    </CharacterStyleRange>
  </ParagraphStyleRange>
</Story>"#;

        let story = Story::from_xml(xml).unwrap();

        assert_eq!(story.text, "Cell text");
        assert_eq!(story.paragraph_style_ranges.len(), 2);
        let table = &story.paragraph_style_ranges[0].character_style_ranges[0].tables[0];
        assert_eq!(table.id.as_deref(), Some("t1"));
        assert_eq!(table.header_row_count.as_deref(), Some("1"));
        assert_eq!(table.body_row_count.as_deref(), Some("1"));
        assert_eq!(table.column_count.as_deref(), Some("1"));
        assert_eq!(
            table.applied_table_style.as_deref(),
            Some("TableStyle/$ID/[Basic Table]")
        );
        assert_eq!(table.text_frame.text_top_inset.as_deref(), Some("4"));
        assert_eq!(table.text_frame.clip_content_to_text_cell, Some(false));
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].single_row_height.as_deref(), Some("10"));
        assert_eq!(table.rows[0].auto_grow, Some(false));
        assert_eq!(table.columns.len(), 1);
        assert_eq!(table.columns[0].single_column_width.as_deref(), Some("20"));
        assert_eq!(table.cells.len(), 1);
        assert_eq!(table.cells[0].name.as_deref(), Some("0:0"));
        assert_eq!(table.cells[0].fill_color.as_deref(), Some("Swatch/None"));

        let serialized = story.to_xml().unwrap();
        assert!(serialized.contains("<Table Self=\"t1\""));
        assert!(serialized.contains("<Row Self=\"r1\""));
        assert!(serialized.contains("<Column Self=\"c1\""));
        assert!(serialized.contains("<Cell Self=\"cell1\""));
        assert_eq!(Story::from_xml(&serialized).unwrap(), story);
    }

    #[test]
    fn rejects_invalid_story_style_range_boolean() {
        let err = Story::from_xml(
            r#"<Story><ParagraphStyleRange Hyphenation="maybe"><CharacterStyleRange><Content>x</Content></CharacterStyleRange></ParagraphStyleRange></Story>"#,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Hyphenation",
                reason: "expected boolean",
            } if element == "ParagraphStyleRange"
        ));
    }

    #[test]
    fn serializer_rejects_mismatched_story_style_range_text() {
        let story = Story {
            id: Some("u1".to_owned()),
            text: "expected".to_owned(),
            paragraph_style_ranges: vec![ParagraphStyleRange {
                character_style_ranges: vec![CharacterStyleRange {
                    text: "actual".to_owned(),
                    ..CharacterStyleRange::default()
                }],
                ..ParagraphStyleRange::default()
            }],
            metadata: Default::default(),
            preference: None,
            incopy_export_option: None,
        };

        let err = story.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidText {
                what: "story style range text",
                reason: "does not match story text",
            }
        ));
    }

    #[test]
    fn serializer_enforces_text_byte_limit() {
        let story = Story {
            id: None,
            text: "a".repeat(super::DEFAULT_MAX_STORY_TEXT_BYTES + 1),
            paragraph_style_ranges: Vec::new(),
            metadata: Default::default(),
            preference: None,
            incopy_export_option: None,
        };

        let err = story.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "story text bytes",
                limit: 67_108_864,
                actual: 67_108_865,
            }
        ));
    }

    #[test]
    fn serializer_enforces_id_attribute_byte_limit() {
        let story = Story {
            id: Some("u".repeat(super::DEFAULT_MAX_STORY_ATTRIBUTE_BYTES + 1)),
            text: "valid".to_owned(),
            paragraph_style_ranges: Vec::new(),
            metadata: Default::default(),
            preference: None,
            incopy_export_option: None,
        };

        let err = story.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::LimitExceeded {
                what: "Story XML attribute bytes",
                limit: 262_144,
                actual: 262_145,
            }
        ));
    }

    #[test]
    fn serializer_rejects_xml_forbidden_control_characters() {
        let story = Story {
            id: Some("u1".to_owned()),
            text: "bad\u{0}".to_owned(),
            paragraph_style_ranges: Vec::new(),
            metadata: Default::default(),
            preference: None,
            incopy_export_option: None,
        };

        let err = story.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidText {
                what: "story text",
                reason: "contains an XML-forbidden character",
            }
        ));
    }

    #[test]
    fn serializer_rejects_xml_forbidden_id_attribute() {
        let story = Story {
            id: Some("u\u{0}".to_owned()),
            text: "valid".to_owned(),
            paragraph_style_ranges: Vec::new(),
            metadata: Default::default(),
            preference: None,
            incopy_export_option: None,
        };

        let err = story.to_xml().unwrap_err();

        assert!(matches!(
            err,
            IdmlError::InvalidAttribute {
                element,
                attribute: "Self",
                reason: "contains an XML-forbidden character",
            } if element == "Story"
        ));
    }
}
