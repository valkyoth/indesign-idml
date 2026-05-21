//! Parser for IDML story content XML.

use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::{
    DEFAULT_MAX_XML_DEPTH, DEFAULT_MAX_XML_INPUT_BYTES, RootElementTracker, XmlDepthTracker,
    configure_idml_reader, enforce_xml_input_bytes, reject_xml_doctype,
    reject_xml_entity_reference, validate_xml_attribute, validate_xml_text,
};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::{EscapeError, escape, partial_escape, resolve_predefined_entity};
use quick_xml::events::{BytesStart, Event};

/// Default maximum extracted text bytes for one story.
pub const DEFAULT_MAX_STORY_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// Story parser limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoryParseOptions {
    /// Maximum UTF-8 bytes accepted for the story XML document.
    pub max_xml_bytes: usize,
    /// Maximum UTF-8 bytes allowed in extracted story text.
    pub max_text_bytes: usize,
    /// Maximum XML element nesting depth accepted while parsing.
    pub max_xml_depth: usize,
}

impl Default for StoryParseOptions {
    fn default() -> Self {
        Self {
            max_xml_bytes: DEFAULT_MAX_XML_INPUT_BYTES,
            max_text_bytes: DEFAULT_MAX_STORY_TEXT_BYTES,
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
        let mut content_depth = 0usize;
        let mut xml_depth = XmlDepthTracker::new(options.max_xml_depth, "Story XML depth");
        let mut root = RootElementTracker::new(b"Story", "expected Story root element");

        loop {
            match reader.read_event()? {
                Event::Start(e) => {
                    root.enter(e.name().as_ref())?;
                    xml_depth.enter()?;
                    if content_depth > 0 {
                        return Err(IdmlError::InvalidElement {
                            element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                            reason: "element inside story Content",
                        });
                    }
                    if e.name().as_ref() == b"Story" && id.is_none() {
                        id = optional_attr(&e, "Self", &reader)?;
                    } else if e.name().as_ref() == b"Content" {
                        content_depth = 1;
                    }
                }
                Event::End(e) if e.name().as_ref() == b"Content" => {
                    content_depth = content_depth.saturating_sub(1);
                    xml_depth.exit()?;
                    root.exit(e.name().as_ref())?;
                }
                Event::End(e) => {
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
                    root.visit_empty(e.name().as_ref())?;
                    xml_depth.visit_empty()?;
                    if content_depth > 0 {
                        return Err(IdmlError::InvalidElement {
                            element: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                            reason: "element inside story Content",
                        });
                    }
                    if e.name().as_ref() == b"Story" && id.is_none() {
                        id = optional_attr(&e, "Self", &reader)?;
                    } else if e.name().as_ref() == b"Br" {
                        push_limited(&mut text, "\n", options.max_text_bytes)?;
                    } else if e.name().as_ref() == b"Tab" {
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
        Ok(Self { id, text })
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

fn optional_attr(
    event: &BytesStart<'_>,
    attribute: &'static str,
    reader: &Reader<&[u8]>,
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

fn serialize_story(story: &Story) -> Result<String> {
    validate_xml_text("story text", &story.text)?;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Story");
    if let Some(id) = &story.id {
        push_attr(&mut xml, "Story", "Self", id)?;
    }
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

    xml.push_str("</CharacterStyleRange>\n  </ParagraphStyleRange>\n</Story>\n");
    Ok(xml)
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
    use super::{Story, StoryParseOptions};
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
        };

        let xml = <Story as crate::XmlSaveable>::to_xml(&story).unwrap();
        let parsed = <Story as crate::XmlLoadable>::from_xml(&xml).unwrap();

        assert!(xml.contains("Self=\"u&amp;&quot;2\""));
        assert!(xml.contains("<Content>Hello &amp; &lt;world&gt;</Content><Br/>"));
        assert!(xml.contains("<Tab/><Content>Cell</Content>"));
        assert_eq!(parsed, story);
    }

    #[test]
    fn serializer_rejects_xml_forbidden_control_characters() {
        let story = Story {
            id: Some("u1".to_owned()),
            text: "bad\u{0}".to_owned(),
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
