//! Read, write, and reopen an IDML package while checking supported model data.

use indesign_idml::prelude::*;
use std::env;
use std::fs::File;
use std::io::Cursor;

fn main() -> Result<()> {
    let Some(path) = env::args_os().nth(1) else {
        eprintln!("Usage: cargo run --example roundtrip_file -- INPUT.idml");
        return Ok(());
    };

    let mut package = IdmlPackage::new(File::open(&path)?)?;
    let document = package.read_document()?;
    let expected = Snapshot::from_document(&document)?;

    let roundtripped = document.write_to(Cursor::new(Vec::new()))?.into_inner();
    let mut package = IdmlPackage::new(Cursor::new(roundtripped))?;
    let reparsed = package.read_document()?;
    let actual = Snapshot::from_document(&reparsed)?;

    if actual != expected {
        return Err(IdmlError::InvalidPackage(
            "round-trip changed supported document data",
        ));
    }

    println!(
        "round-trip ok: {} stories, {} spreads, {} preserved entries",
        expected.story_count, expected.spread_count, expected.preserved_entry_count
    );
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct Snapshot {
    document_id: String,
    layers: Vec<LayerSummary>,
    package_refs: Vec<PackageRefSummary>,
    story_count: usize,
    spread_count: usize,
    preserved_entry_count: usize,
    preserved_entries: Vec<PreservedEntrySummary>,
    story_texts: Vec<(String, String)>,
    spread_summaries: Vec<SpreadSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct LayerSummary {
    id: Option<String>,
    name: Option<String>,
    visible: Option<bool>,
    locked: Option<bool>,
}

#[derive(Debug, Eq, PartialEq)]
struct PackageRefSummary {
    element: String,
    id: Option<String>,
    path: String,
}

#[derive(Debug, Eq, PartialEq)]
struct PreservedEntrySummary {
    element: String,
    id: Option<String>,
    path: String,
    compression: String,
    byte_len: usize,
    byte_hash: u64,
}

#[derive(Debug, Eq, PartialEq)]
struct SpreadSummary {
    id: String,
    pages: Vec<PageSummary>,
    guides: Vec<GuideSummary>,
    text_frames: Vec<TextFrameSummary>,
    page_items: Vec<PageItemSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct PageSummary {
    id: Option<String>,
    bounds: Option<[u64; 4]>,
}

#[derive(Debug, Eq, PartialEq)]
struct GuideSummary {
    id: Option<String>,
    layer_id: Option<String>,
    orientation: Option<String>,
    location: Option<u64>,
    page_index: Option<i32>,
}

#[derive(Debug, Eq, PartialEq)]
struct TextFrameSummary {
    id: Option<String>,
    layer_id: Option<String>,
    parent_story: Option<String>,
    bounds: Option<[u64; 4]>,
    path_geometry: Option<PathGeometrySummary>,
}

#[derive(Debug, Eq, PartialEq)]
enum PageItemSummary {
    Rectangle {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        path_geometry: Option<PathGeometrySummary>,
        images: Vec<ImageSummary>,
    },
    Oval {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        path_geometry: Option<PathGeometrySummary>,
    },
    GraphicLine {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        path_geometry: Option<PathGeometrySummary>,
    },
    Polygon {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        path_geometry: Option<PathGeometrySummary>,
    },
    Group {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        page_items: Vec<PageItemSummary>,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct PathGeometrySummary {
    paths: Vec<GeometryPathSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct GeometryPathSummary {
    path_open: Option<bool>,
    points: Vec<PathPointSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct PathPointSummary {
    anchor: [u64; 2],
    left_direction: [u64; 2],
    right_direction: [u64; 2],
}

#[derive(Debug, Eq, PartialEq)]
struct ImageSummary {
    id: Option<String>,
    image_type_name: Option<String>,
    item_transform: Option<String>,
    graphic_bounds: Option<[u64; 4]>,
    links: Vec<ImageLinkSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct ImageLinkSummary {
    id: Option<String>,
    resource_uri: Option<String>,
    resource_format: Option<String>,
}

impl Snapshot {
    fn from_document(document: &IdmlDocument) -> Result<Self> {
        let design_map = &document.design_map;
        let mut story_texts = Vec::new();
        document.try_for_each_story_text(|story_id, text| {
            story_texts.push((story_id.to_owned(), text.to_owned()));
            Ok::<(), IdmlError>(())
        })?;
        let layers = design_map
            .layers
            .iter()
            .map(|layer| LayerSummary {
                id: layer.id.clone(),
                name: layer.name.clone(),
                visible: layer.visible,
                locked: layer.locked,
            })
            .collect();
        let package_refs = design_map
            .package_refs
            .iter()
            .map(|reference| PackageRefSummary {
                element: reference.element.clone(),
                id: reference.id.clone(),
                path: reference.path.as_str().to_owned(),
            })
            .collect();
        let mut preserved_entries = Vec::new();
        document.try_for_each_preserved_entry(|preserved| {
            preserved_entries.push(PreservedEntrySummary {
                element: preserved.element.to_owned(),
                id: preserved.id.map(str::to_owned),
                path: preserved.path.as_str().to_owned(),
                compression: format!("{:?}", preserved.entry.compression),
                byte_len: preserved.entry.data.len(),
                byte_hash: fnv1a64(&preserved.entry.data),
            });
            Ok::<(), IdmlError>(())
        })?;
        let spread_summaries =
            design_map
                .spread_ids()
                .map(|spread_id| {
                    let spread = document.spreads.get(spread_id).ok_or_else(|| {
                        IdmlError::MissingReference {
                            kind: "spread model",
                            id: spread_id.to_owned(),
                        }
                    })?;
                    Ok(SpreadSummary {
                        id: spread_id.to_owned(),
                        pages: spread
                            .pages
                            .iter()
                            .map(|page| PageSummary {
                                id: page.id.clone(),
                                bounds: page.geometric_bounds.map(rect_bits),
                            })
                            .collect(),
                        guides: spread
                            .guides
                            .iter()
                            .map(|guide| GuideSummary {
                                id: guide.id.clone(),
                                layer_id: guide.layer_id.clone(),
                                orientation: guide.orientation.clone(),
                                location: guide.location.map(|point| point.as_f64().to_bits()),
                                page_index: guide.page_index,
                            })
                            .collect(),
                        text_frames: spread
                            .text_frames
                            .iter()
                            .map(|frame| TextFrameSummary {
                                id: frame.id.clone(),
                                layer_id: frame.layer_id.clone(),
                                parent_story: frame.parent_story.clone(),
                                bounds: frame.geometric_bounds.map(rect_bits),
                                path_geometry: frame
                                    .path_geometry
                                    .as_ref()
                                    .map(summarize_path_geometry),
                            })
                            .collect(),
                        page_items: summarize_page_items(&spread.page_items),
                    })
                })
                .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            document_id: design_map.id.clone(),
            layers,
            package_refs,
            story_count: design_map.story_srcs.len(),
            spread_count: design_map.spread_srcs.len(),
            preserved_entry_count: document.preserved_entries.len(),
            preserved_entries,
            story_texts,
            spread_summaries,
        })
    }
}

fn summarize_page_items(page_items: &[PageItem]) -> Vec<PageItemSummary> {
    page_items
        .iter()
        .filter_map(|page_item| match page_item {
            PageItem::TextFrame(_) => None,
            PageItem::Rectangle(rectangle) => Some(PageItemSummary::Rectangle {
                id: rectangle.id.clone(),
                layer_id: rectangle.layer_id.clone(),
                bounds: rectangle.geometric_bounds.map(rect_bits),
                path_geometry: rectangle
                    .path_geometry
                    .as_ref()
                    .map(summarize_path_geometry),
                images: rectangle
                    .images
                    .iter()
                    .map(|image| ImageSummary {
                        id: image.id.clone(),
                        image_type_name: image.image_type_name.clone(),
                        item_transform: image.item_transform.clone(),
                        graphic_bounds: image.graphic_bounds.map(rect_bits),
                        links: image
                            .links
                            .iter()
                            .map(|link| ImageLinkSummary {
                                id: link.id.clone(),
                                resource_uri: link.resource_uri.clone(),
                                resource_format: link.resource_format.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            }),
            PageItem::Oval(oval) => Some(PageItemSummary::Oval {
                id: oval.id.clone(),
                layer_id: oval.layer_id.clone(),
                bounds: oval.geometric_bounds.map(rect_bits),
                path_geometry: oval.path_geometry.as_ref().map(summarize_path_geometry),
            }),
            PageItem::GraphicLine(line) => Some(PageItemSummary::GraphicLine {
                id: line.id.clone(),
                layer_id: line.layer_id.clone(),
                bounds: line.geometric_bounds.map(rect_bits),
                path_geometry: line.path_geometry.as_ref().map(summarize_path_geometry),
            }),
            PageItem::Polygon(polygon) => Some(PageItemSummary::Polygon {
                id: polygon.id.clone(),
                layer_id: polygon.layer_id.clone(),
                bounds: polygon.geometric_bounds.map(rect_bits),
                path_geometry: polygon.path_geometry.as_ref().map(summarize_path_geometry),
            }),
            PageItem::Group(group) => Some(PageItemSummary::Group {
                id: group.id.clone(),
                layer_id: group.layer_id.clone(),
                bounds: group.geometric_bounds.map(rect_bits),
                page_items: summarize_page_items(&group.page_items),
            }),
        })
        .collect()
}

fn summarize_path_geometry(path_geometry: &PathGeometry) -> PathGeometrySummary {
    PathGeometrySummary {
        paths: path_geometry
            .paths
            .iter()
            .map(|path| GeometryPathSummary {
                path_open: path.path_open,
                points: path
                    .points
                    .iter()
                    .map(|point| PathPointSummary {
                        anchor: point_bits(point.anchor),
                        left_direction: point_bits(point.left_direction),
                        right_direction: point_bits(point.right_direction),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn rect_bits(rect: Rect) -> [u64; 4] {
    [
        rect.top.as_f64().to_bits(),
        rect.left.as_f64().to_bits(),
        rect.bottom.as_f64().to_bits(),
        rect.right.as_f64().to_bits(),
    ]
}

fn point_bits(point: Point) -> [u64; 2] {
    [point.x.as_f64().to_bits(), point.y.as_f64().to_bits()]
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
