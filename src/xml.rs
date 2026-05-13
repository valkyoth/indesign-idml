//! Internal XML value validation helpers.

use crate::error::{IdmlError, Result};

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
