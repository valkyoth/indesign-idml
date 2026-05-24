//! Common imports for applications using `indesign-idml`.

pub use crate::archive::{ArchiveLimits, IdmlPackage, IdmlPackageWriter, IdmlPath};
pub use crate::core::resolver::{ResolvedTextFrame, ResolvedTextFrameData};
pub use crate::core::units::{Inches, Millimeters, Points};
pub use crate::encoding::{Base64Mode, decode_standard, encode_standard};
pub use crate::error::{IdmlError, Result};
pub use crate::model::designmap::{
    DesignMap, DesignMapPackageRef, DesignMapParseOptions, Layer, MasterSpreadPointer,
    PackageResourcePointer, SpreadPointer, StoryPointer,
};
pub use crate::model::document::{
    IdmlDocument, IdmlDocumentReadOptions, IdmlIdAllocator, PreservedEntry, PreservedEntryRef,
};
pub use crate::model::resources::{
    EmbeddedAsset, EmbeddedAssetParseOptions, ResourceArchiveMetadata, ResourceIntegrityOptions,
    ResourceInventory, ResourceItem, ResourceItemGroup, ResourceItemParseOptions, ResourceKind,
    ResourceReference, extract_embedded_assets, extract_embedded_assets_with_options,
    extract_resource_items, extract_resource_items_with_options,
};
pub use crate::model::spread::{
    GeometryPath, GraphicLine, Group, Guide, Image, ImageLink, Oval, Page, PageItem, PathGeometry,
    PathPoint, Point, Polygon, Rect, RectMm, Rectangle, Spread, SpreadParseOptions, TextFrame,
};
pub use crate::model::story::{Story, StoryParseOptions};
pub use crate::traits::{XmlLoadable, XmlSaveable};
