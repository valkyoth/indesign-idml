//! Parser for IDML story content XML.

use crate::error::{IdmlError, Result};
use crate::traits::{XmlLoadable, XmlSaveable};
use crate::xml::{validate_xml_attribute, validate_xml_text};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::{EscapeError, escape, partial_escape, resolve_predefined_entity};
use quick_xml::events::{BytesStart, Event};

/// Default maximum extracted text bytes for one story.
pub const DEFAULT_MAX_STORY_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// Story parser limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoryParseOptions {
    /// Maximum UTF-8 bytes allowed in extracted story text.
    pub max_text_bytes: usize,
}

impl Default for StoryParseOptions {
    fn default() -> Self {
        Self {
            max_text_bytes: DEFAULT_MAX_STORY_TEXT_BYTES,
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
        let mut reader = Reader::from_str(xml_content);
        reader.config_mut().trim_text(false);

        let mut id = None;
        let mut text = String::new();
        let mut content_depth = 0usize;

        loop {
            match reader.read_event()? {
                Event::Start(e) if e.name().as_ref() == b"Story" && id.is_none() => {
                    id = optional_attr(&e, "Self", &reader)?;
                }
                Event::Start(e) if e.name().as_ref() == b"Content" => {
                    content_depth = content_depth.saturating_add(1);
                }
                Event::End(e) if e.name().as_ref() == b"Content" => {
                    content_depth = content_depth.saturating_sub(1);
                }
                Event::Text(e) if content_depth > 0 => {
                    let decoded = e.decode()?;
                    push_limited(&mut text, decoded.as_ref(), options.max_text_bytes)?;
                }
                Event::CData(e) if content_depth > 0 => {
                    push_limited(
                        &mut text,
                        e.xml_content(XmlVersion::Implicit1_0)?.as_ref(),
                        options.max_text_bytes,
                    )?;
                }
                Event::GeneralRef(e) if content_depth > 0 => {
                    if let Some(ch) = e.resolve_char_ref()? {
                        push_char_limited(&mut text, ch, options.max_text_bytes)?;
                    } else {
                        let entity = e.decode()?;
                        let replacement =
                            resolve_predefined_entity(entity.as_ref()).ok_or_else(|| {
                                IdmlError::XmlEscape(EscapeError::UnrecognizedEntity(
                                    0..entity.len(),
                                    entity.to_string(),
                                ))
                            })?;
                        push_limited(&mut text, replacement, options.max_text_bytes)?;
                    }
                }
                Event::Empty(e) if e.name().as_ref() == b"Br" => {
                    push_limited(&mut text, "\n", options.max_text_bytes)?;
                }
                Event::Empty(e) if e.name().as_ref() == b"Tab" => {
                    push_limited(&mut text, "\t", options.max_text_bytes)?;
                }
                Event::Eof => break,
                _ => {}
            }
        }

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
    fn enforces_story_text_limit() {
        let err = Story::from_xml_with_options(
            "<Story><Content>hello</Content></Story>",
            StoryParseOptions { max_text_bytes: 4 },
        )
        .unwrap_err();

        assert!(matches!(err, IdmlError::LimitExceeded { what, .. } if what == "story text bytes"));
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
