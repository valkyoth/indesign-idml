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
                    exit_story_depth(&mut story_depth)?;
                    xml_depth.exit()?;
                    root.exit(e.name().as_ref())?;
                }
                Event::Text(e) => {
                    root.visit_text(e.as_ref())?;
                    if content_depth > 0 {
                        let decoded = e.decode()?;
                        push_limited(&mut text, decoded.as_ref(), options.max_text_bytes)?;
                    }
                }
                Event::CData(e) => {
                    root.visit_cdata()?;
                    if content_depth > 0 {
                        push_limited(
                            &mut text,
                            e.xml_content(XmlVersion::Implicit1_0)?.as_ref(),
                            options.max_text_bytes,
                        )?;
                    }
                }
                Event::GeneralRef(e) => {
                    root.visit_reference()?;
                    if content_depth > 0 {
                        if let Some(ch) = e.resolve_char_ref()? {
                            push_char_limited(&mut text, ch, options.max_text_bytes)?;
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
                    } else if story_depth > 0 && e.name().as_ref() == b"Br" {
                        push_limited(&mut text, "\n", options.max_text_bytes)?;
                    } else if story_depth > 0 && e.name().as_ref() == b"Tab" {
                        push_limited(&mut text, "\t", options.max_text_bytes)?;
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
        Ok(Self {
            id,
            text,
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
    xml.push_str(">\n  <ParagraphStyleRange>\n    <CharacterStyleRange>");

    let mut content = String::new();
    for ch in story.text.chars() {
        match ch {
            '\n' => {
                push_content_if_needed(&mut xml, &mut content);
                xml.push_str("<Br/>");
            }
            '\t' => {
                push_content_if_needed(&mut xml, &mut content);
                xml.push_str("<Tab/>");
            }
            _ => content.push(ch),
        }
    }
    push_content_if_needed(&mut xml, &mut content);

    xml.push_str("</CharacterStyleRange>\n  </ParagraphStyleRange>\n");
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
    use super::{InCopyExportOption, Story, StoryMetadata, StoryParseOptions, StoryPreference};
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
    fn serializer_enforces_text_byte_limit() {
        let story = Story {
            id: None,
            text: "a".repeat(super::DEFAULT_MAX_STORY_TEXT_BYTES + 1),
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
