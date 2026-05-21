//! Internal XML value validation helpers.

use crate::error::{IdmlError, Result};

pub(crate) const DEFAULT_MAX_XML_DEPTH: usize = 1024;

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
