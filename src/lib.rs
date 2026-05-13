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

pub use error::{IdmlError, Result};

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
}
