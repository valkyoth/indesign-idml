#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]

//! Secure Rust primitives for working with Adobe InDesign IDML packages.
//!
//! The crate treats IDML files as untrusted ZIP archives containing relational
//! XML. The first implemented layers enforce archive path policy, bounded entry
//! reads, strict Base64 decoding, and DesignMap reference inventory.

pub mod archive;
pub mod core;
pub mod encoding;
pub mod error;
pub mod model;
pub mod prelude;
pub mod traits;
mod xml;

pub use error::{IdmlError, Result};
pub use traits::{XmlLoadable, XmlSaveable};

/// Returns the package name as compiled into the crate.
#[must_use]
pub const fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn crate_name_matches_package() {
        assert_eq!(crate_name(), "indesign-idml");
    }

    #[test]
    fn prelude_exports_core_document_types() {
        let _: Option<crate::prelude::IdmlDocument> = None;
        let _: Option<crate::prelude::DesignMap> = None;
        let _: Option<crate::prelude::Story> = None;
        let _: Option<crate::prelude::Spread> = None;
        let _: Option<crate::prelude::IdmlError> = None;
        let _: Option<crate::prelude::ResolvedTextFrameData> = None;
        let _: Option<crate::prelude::IdmlIdAllocator> = None;
        let _: Option<crate::prelude::PreservedEntry> = None;
        let _: Option<crate::prelude::StoryPointer<'_>> = None;
        let _: Option<crate::prelude::SpreadPointer<'_>> = None;
        let _: Option<crate::prelude::MasterSpreadPointer<'_>> = None;
        let _: Option<crate::prelude::PackageResourcePointer<'_>> = None;
    }
}
