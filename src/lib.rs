#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]

//! Secure Rust primitives for working with Adobe InDesign IDML packages.
//!
//! The implementation will be added in small, validated milestones. The crate
//! starts with a locked verification gate so parsing and archive behavior can be
//! developed under audit, license, fuzzing, and feature-matrix checks from day
//! one.

/// Crate-wide result type.
pub type Result<T> = core::result::Result<T, IdmlError>;

/// Error type placeholder for the initial security scaffold.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdmlError {
    /// The requested feature or parser path has not been implemented yet.
    #[error("feature not implemented yet: {0}")]
    NotImplemented(&'static str),
}

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
