//! Error types for IDML archive, XML, and encoding operations.

/// Crate-wide result type.
pub type Result<T> = ::core::result::Result<T, IdmlError>;

/// Error type used by all public fallible APIs.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdmlError {
    /// An I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A ZIP operation failed.
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    /// XML parsing failed.
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),

    /// XML attribute parsing failed.
    #[error("XML attribute error: {0}")]
    XmlAttribute(#[from] quick_xml::events::attributes::AttrError),

    /// XML text decoding failed.
    #[error("XML encoding error: {0}")]
    XmlEncoding(#[from] quick_xml::encoding::EncodingError),

    /// XML entity unescaping failed.
    #[error("XML escape error: {0}")]
    XmlEscape(#[from] quick_xml::escape::EscapeError),

    /// UTF-8 decoding failed for an XML entry.
    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    /// Numeric parsing failed.
    #[error("number parse error: {0}")]
    ParseFloat(#[from] std::num::ParseFloatError),

    /// A required XML attribute was not present.
    #[error("missing required attribute `{attribute}` on `{element}`")]
    MissingAttribute {
        /// Element name.
        element: String,
        /// Attribute name.
        attribute: &'static str,
    },

    /// An XML attribute value was malformed.
    #[error("invalid attribute `{attribute}` on `{element}`: {reason}")]
    InvalidAttribute {
        /// Element name.
        element: String,
        /// Attribute name.
        attribute: &'static str,
        /// Rejection reason.
        reason: &'static str,
    },

    /// An archive entry path violates the crate path policy.
    #[error("invalid archive path `{path}`: {reason}")]
    InvalidArchivePath {
        /// Raw archive path.
        path: String,
        /// Reason the path was rejected.
        reason: &'static str,
    },

    /// The archive contains the same logical entry more than once.
    #[error("duplicate archive entry `{0}`")]
    DuplicateArchiveEntry(String),

    /// The requested archive entry is absent.
    #[error("missing archive entry `{0}`")]
    MissingArchiveEntry(String),

    /// A requested ID was not present in the parsed package manifest.
    #[error("missing {kind} reference `{id}`")]
    MissingReference {
        /// Reference kind.
        kind: &'static str,
        /// Requested ID.
        id: String,
    },

    /// A configured parser or archive size limit was exceeded.
    #[error("limit exceeded for {what}: limit {limit}, actual {actual}")]
    LimitExceeded {
        /// What was limited.
        what: &'static str,
        /// Configured limit.
        limit: u64,
        /// Observed value.
        actual: u64,
    },

    /// Base64 decoding failed.
    #[error("base64 decode error: {0}")]
    Base64Decode(base64_ng::DecodeError),

    /// Base64 encoding failed.
    #[error("base64 encode error: {0}")]
    Base64Encode(base64_ng::EncodeError),
}

impl From<base64_ng::DecodeError> for IdmlError {
    fn from(error: base64_ng::DecodeError) -> Self {
        Self::Base64Decode(error)
    }
}

impl From<base64_ng::EncodeError> for IdmlError {
    fn from(error: base64_ng::EncodeError) -> Self {
        Self::Base64Encode(error)
    }
}
