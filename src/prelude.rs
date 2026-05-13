//! Common imports for applications using `indesign-idml`.

pub use crate::archive::{ArchiveLimits, IdmlPackage, IdmlPackageWriter, IdmlPath};
pub use crate::core::resolver::ResolvedTextFrameData;
pub use crate::core::units::{Inches, Millimeters, Points};
pub use crate::encoding::{Base64Mode, decode_standard, encode_standard};
pub use crate::error::{IdmlError, Result};
pub use crate::model::designmap::{
    DesignMap, MasterSpreadPointer, PackageResourcePointer, SpreadPointer, StoryPointer,
};
pub use crate::model::document::{
    IdmlDocument, IdmlDocumentReadOptions, IdmlIdAllocator, PreservedEntry,
};
pub use crate::model::resources::{ResourceInventory, ResourceKind, ResourceReference};
pub use crate::model::spread::{Rect, RectMm, Spread, TextFrame};
pub use crate::model::story::{Story, StoryParseOptions};
pub use crate::traits::{XmlLoadable, XmlSaveable};
