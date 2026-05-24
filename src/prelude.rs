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
    ContourOption, FrameFittingOption, GeometryPath, GraphicLine, Group, Guide, Image, ImageLink,
    ObjectExportMetadataProperty, ObjectExportOption, Oval, Page, PageItem,
    PageItemInCopyExportOption, PageItemMetadata, PathGeometry, PathPoint, Point, Polygon, Rect,
    RectMm, Rectangle, Spread, SpreadListItem, SpreadListProperty, SpreadParseOptions, TextFrame,
    TextFrameFootnoteOptions, TextFramePreference, TextWrapOffset, TextWrapPreference,
};
pub use crate::model::story::{
    CharacterStyleProperties, CharacterStyleRange, InCopyExportOption, ParagraphStyleProperties,
    ParagraphStyleRange, Story, StoryListItem, StoryListProperty, StoryMetadata, StoryParseOptions,
    StoryPreference, StoryProperty, StoryTable, StoryTableCell, StoryTableColumn, StoryTableRow,
    TableTextInsets,
};
pub use crate::traits::{XmlLoadable, XmlSaveable};
