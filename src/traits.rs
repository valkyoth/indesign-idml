//! Shared XML loading and saving traits for IDML object models.

use crate::Result;

/// Parses an IDML XML fragment into a typed model.
pub trait XmlLoadable: Sized {
    /// Parses `xml` into `Self`.
    fn from_xml(xml: &str) -> Result<Self>;
}

/// Serializes a typed IDML model into XML.
pub trait XmlSaveable {
    /// Serializes `self` into an owned UTF-8 XML string.
    fn to_xml(&self) -> Result<String>;
}
