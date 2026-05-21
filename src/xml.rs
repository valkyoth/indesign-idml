//! Internal XML value validation helpers.

use crate::error::{IdmlError, Result};
use quick_xml::reader::Reader;

pub(crate) const DEFAULT_MAX_XML_DEPTH: usize = 1024;
pub(crate) const DEFAULT_MAX_XML_INPUT_BYTES: usize = 512 * 1024 * 1024;

pub(crate) fn configure_idml_reader(reader: &mut Reader<&[u8]>, trim_text: bool) {
    let config = reader.config_mut();
    config.trim_text(trim_text);
    config.check_comments = true;
    config.check_end_names = true;
}

pub(crate) fn reject_xml_doctype() -> Result<()> {
    Err(IdmlError::InvalidElement {
        element: "!DOCTYPE".to_owned(),
        reason: "DOCTYPE declarations are not supported",
    })
}

pub(crate) fn reject_xml_entity_reference() -> Result<()> {
    Err(IdmlError::InvalidElement {
        element: "#reference".to_owned(),
        reason: "entity references are not supported here",
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootElementTracker {
    expected_name: &'static [u8],
    expected_root_reason: &'static str,
    seen: bool,
    depth: usize,
    closed: bool,
}

impl RootElementTracker {
    pub(crate) const fn new(
        expected_name: &'static [u8],
        expected_root_reason: &'static str,
    ) -> Self {
        Self {
            expected_name,
            expected_root_reason,
            seen: false,
            depth: 0,
            closed: false,
        }
    }

    pub(crate) fn enter(&mut self, name: &[u8]) -> Result<()> {
        if self.closed {
            return Err(self.invalid_element(name, "multiple root elements"));
        }
        if !self.seen {
            self.validate_root(name)?;
            self.seen = true;
        }
        self.depth = self
            .depth
            .checked_add(1)
            .ok_or_else(|| self.invalid_element(name, "root depth overflow"))?;
        Ok(())
    }

    pub(crate) fn visit_empty(&mut self, name: &[u8]) -> Result<()> {
        if self.closed {
            return Err(self.invalid_element(name, "multiple root elements"));
        }
        if !self.seen {
            self.validate_root(name)?;
            self.seen = true;
            self.closed = true;
        }
        Ok(())
    }

    pub(crate) fn exit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        if self.seen && self.depth == 0 {
            self.closed = true;
        }
    }

    pub(crate) fn visit_text(&self, text: &[u8]) -> Result<()> {
        if self.is_outside_root() && !is_xml_space(text) {
            return Err(IdmlError::InvalidElement {
                element: "#text".to_owned(),
                reason: "non-whitespace character data outside root element",
            });
        }
        Ok(())
    }

    pub(crate) fn visit_cdata(&self) -> Result<()> {
        if self.is_outside_root() {
            return Err(IdmlError::InvalidElement {
                element: "#cdata".to_owned(),
                reason: "CDATA outside root element",
            });
        }
        Ok(())
    }

    pub(crate) fn visit_reference(&self) -> Result<()> {
        if self.is_outside_root() {
            return Err(IdmlError::InvalidElement {
                element: "#reference".to_owned(),
                reason: "entity reference outside root element",
            });
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<()> {
        if !self.seen {
            return Err(IdmlError::InvalidElement {
                element: String::new(),
                reason: "missing root element",
            });
        }
        if !self.closed {
            return Err(IdmlError::InvalidElement {
                element: String::from_utf8_lossy(self.expected_name).into_owned(),
                reason: "unclosed root element",
            });
        }
        Ok(())
    }

    fn validate_root(&self, name: &[u8]) -> Result<()> {
        if name == self.expected_name {
            return Ok(());
        }
        Err(self.invalid_element(name, self.expected_root_reason))
    }

    fn invalid_element(&self, name: &[u8], reason: &'static str) -> IdmlError {
        IdmlError::InvalidElement {
            element: String::from_utf8_lossy(name).into_owned(),
            reason,
        }
    }

    const fn is_outside_root(&self) -> bool {
        !self.seen || self.closed
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AnyRootTracker {
    root_name: Option<String>,
    depth: usize,
    closed: bool,
}

impl AnyRootTracker {
    pub(crate) fn enter(&mut self, name: &[u8]) -> Result<()> {
        if self.closed {
            return Err(invalid_element(name, "multiple root elements"));
        }
        if self.root_name.is_none() {
            self.root_name = Some(String::from_utf8_lossy(name).into_owned());
        }
        self.depth = self
            .depth
            .checked_add(1)
            .ok_or_else(|| IdmlError::InvalidElement {
                element: String::from_utf8_lossy(name).into_owned(),
                reason: "root depth overflow",
            })?;
        Ok(())
    }

    pub(crate) fn visit_empty(&mut self, name: &[u8]) -> Result<()> {
        if self.closed {
            return Err(invalid_element(name, "multiple root elements"));
        }
        if self.root_name.is_none() {
            self.root_name = Some(String::from_utf8_lossy(name).into_owned());
            self.closed = true;
        }
        Ok(())
    }

    pub(crate) fn exit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        if self.root_name.is_some() && self.depth == 0 {
            self.closed = true;
        }
    }

    pub(crate) fn visit_text(&self, text: &[u8]) -> Result<()> {
        if self.is_outside_root() && !is_xml_space(text) {
            return Err(IdmlError::InvalidElement {
                element: "#text".to_owned(),
                reason: "non-whitespace character data outside root element",
            });
        }
        Ok(())
    }

    pub(crate) fn visit_cdata(&self) -> Result<()> {
        if self.is_outside_root() {
            return Err(IdmlError::InvalidElement {
                element: "#cdata".to_owned(),
                reason: "CDATA outside root element",
            });
        }
        Ok(())
    }

    pub(crate) fn visit_reference(&self) -> Result<()> {
        if self.is_outside_root() {
            return Err(IdmlError::InvalidElement {
                element: "#reference".to_owned(),
                reason: "entity reference outside root element",
            });
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<()> {
        let Some(root_name) = self.root_name else {
            return Err(IdmlError::InvalidElement {
                element: String::new(),
                reason: "missing root element",
            });
        };
        if !self.closed {
            return Err(IdmlError::InvalidElement {
                element: root_name,
                reason: "unclosed root element",
            });
        }
        Ok(())
    }

    fn is_outside_root(&self) -> bool {
        self.root_name.is_none() || self.closed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct XmlDepthTracker {
    current: usize,
    max: usize,
    what: &'static str,
}

impl XmlDepthTracker {
    pub(crate) const fn new(max: usize, what: &'static str) -> Self {
        Self {
            current: 0,
            max,
            what,
        }
    }

    pub(crate) fn enter(&mut self) -> Result<()> {
        let next = self
            .current
            .checked_add(1)
            .ok_or(IdmlError::LimitExceeded {
                what: self.what,
                limit: self.max as u64,
                actual: u64::MAX,
            })?;

        if next > self.max {
            return Err(IdmlError::LimitExceeded {
                what: self.what,
                limit: self.max as u64,
                actual: next as u64,
            });
        }

        self.current = next;
        Ok(())
    }

    pub(crate) fn exit(&mut self) {
        self.current = self.current.saturating_sub(1);
    }

    pub(crate) fn visit_empty(&mut self) -> Result<()> {
        self.enter()?;
        self.exit();
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<()> {
        if self.current == 0 {
            return Ok(());
        }

        Err(IdmlError::InvalidElement {
            element: "#document".to_owned(),
            reason: "unclosed XML element",
        })
    }
}

pub(crate) fn validate_xml_text(what: &'static str, text: &str) -> Result<()> {
    if text.chars().any(|ch| !is_xml_char(ch)) {
        return Err(IdmlError::InvalidText {
            what,
            reason: "contains an XML-forbidden character",
        });
    }
    Ok(())
}

pub(crate) fn enforce_xml_input_bytes(
    what: &'static str,
    xml_content: &str,
    max_xml_bytes: usize,
) -> Result<()> {
    if xml_content.len() > max_xml_bytes {
        return Err(IdmlError::LimitExceeded {
            what,
            limit: max_xml_bytes as u64,
            actual: xml_content.len() as u64,
        });
    }
    Ok(())
}

pub(crate) fn validate_xml_attribute(
    element: impl Into<String>,
    attribute: &'static str,
    value: &str,
) -> Result<()> {
    if value.chars().any(|ch| !is_xml_char(ch)) {
        return Err(IdmlError::InvalidAttribute {
            element: element.into(),
            attribute,
            reason: "contains an XML-forbidden character",
        });
    }
    Ok(())
}

fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&ch)
        || ('\u{E000}'..='\u{FFFD}').contains(&ch)
        || ('\u{10000}'..='\u{10FFFF}').contains(&ch)
}

fn is_xml_space(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|byte| matches!(byte, b'\t' | b'\n' | b'\r' | b' '))
}

fn invalid_element(name: &[u8], reason: &'static str) -> IdmlError {
    IdmlError::InvalidElement {
        element: String::from_utf8_lossy(name).into_owned(),
        reason,
    }
}
