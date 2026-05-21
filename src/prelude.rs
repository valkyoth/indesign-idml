//! Common imports for applications using `indesign-idml`.

pub use crate::archive::{ArchiveLimits, IdmlPackage, IdmlPackageWriter, IdmlPath};
pub use crate::core::resolver::ResolvedTextFrameData;
pub use crate::core::units::{Inches, Millimeters, Points};
pub use crate::encoding::{Base64Mode, decode_standard, encode_standard};
pub use crate::error::{IdmlError, Result};
pub use crate::model::designmap::{
    DesignMap, DesignMapPackageRef, DesignMapParseOptions, MasterSpreadPointer,
    PackageResourcePointer, SpreadPointer, StoryPointer,
};
pub use crate::model::document::{
    IdmlDocument, IdmlDocumentReadOptions, IdmlIdAllocator, PreservedEntry,
};
pub use crate::model::resources::{
    EmbeddedAsset, EmbeddedAssetParseOptions, ResourceArchiveMetadata, ResourceIntegrityOptions,
    ResourceInventory, ResourceKind, ResourceReference, extract_embedded_assets,
    extract_embedded_assets_with_options,
};
pub use crate::model::spread::{
    Page, PageItem, Rect, RectMm, Rectangle, Spread, SpreadParseOptions, TextFrame,
};
pub use crate::model::story::{Story, StoryParseOptions};
pub use crate::traits::{XmlLoadable, XmlSaveable};
