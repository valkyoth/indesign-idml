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
    story_summaries: Vec<StorySummary>,
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
struct StorySummary {
    id: String,
    self_id: Option<String>,
    text: String,
    paragraph_style_ranges: Vec<ParagraphStyleRangeSummary>,
    metadata: StoryMetadataSummary,
    preference: Option<StoryPreferenceSummary>,
    incopy_export_option: Option<InCopyExportOptionSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct ParagraphStyleRangeSummary {
    applied_paragraph_style: Option<String>,
    justification: Option<String>,
    hyphenation: Option<bool>,
    rule_above_line_weight: Option<String>,
    rule_below_line_weight: Option<String>,
    hyphenation_zone: Option<String>,
    split_column_inside_gutter: Option<String>,
    space_before: Option<String>,
    space_after: Option<String>,
    properties: ParagraphStylePropertiesSummary,
    character_style_ranges: Vec<CharacterStyleRangeSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct CharacterStyleRangeSummary {
    applied_character_style: Option<String>,
    point_size: Option<String>,
    font_style: Option<String>,
    fill_color: Option<String>,
    tracking: Option<String>,
    stroke_weight: Option<String>,
    ruby_font_size: Option<String>,
    kenten_font_size: Option<String>,
    capitalization: Option<String>,
    miter_limit: Option<String>,
    stroke_alignment: Option<String>,
    horizontal_scale: Option<String>,
    vertical_scale: Option<String>,
    properties: CharacterStylePropertiesSummary,
    tables: Vec<StoryTableSummary>,
    text: String,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryTableSummary {
    id: Option<String>,
    header_row_count: Option<String>,
    footer_row_count: Option<String>,
    body_row_count: Option<String>,
    column_count: Option<String>,
    applied_table_style: Option<String>,
    table_direction: Option<String>,
    text_frame: TableTextInsetsSummary,
    rows: Vec<StoryTableRowSummary>,
    columns: Vec<StoryTableColumnSummary>,
    cells: Vec<StoryTableCellSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct TableTextInsetsSummary {
    text_top_inset: Option<String>,
    text_left_inset: Option<String>,
    text_bottom_inset: Option<String>,
    text_right_inset: Option<String>,
    clip_content_to_text_cell: Option<bool>,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryTableRowSummary {
    id: Option<String>,
    name: Option<String>,
    text_frame: TableTextInsetsSummary,
    single_row_height: Option<String>,
    minimum_height: Option<String>,
    auto_grow: Option<bool>,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryTableColumnSummary {
    id: Option<String>,
    name: Option<String>,
    text_frame: TableTextInsetsSummary,
    single_column_width: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryTableCellSummary {
    id: Option<String>,
    name: Option<String>,
    row_span: Option<String>,
    column_span: Option<String>,
    cell_type: Option<String>,
    applied_cell_style: Option<String>,
    applied_cell_style_priority: Option<String>,
    text_frame: TableTextInsetsSummary,
    left_inset: Option<String>,
    top_inset: Option<String>,
    right_inset: Option<String>,
    bottom_inset: Option<String>,
    fill_color: Option<String>,
    paragraph_style_ranges: Vec<ParagraphStyleRangeSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct ParagraphStylePropertiesSummary {
    same_para_style_spacing: Option<StoryPropertySummary>,
    rule_above_gap_color: Option<StoryPropertySummary>,
    rule_below_gap_color: Option<StoryPropertySummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct CharacterStylePropertiesSummary {
    applied_font: Option<StoryPropertySummary>,
    leading: Option<StoryPropertySummary>,
    underline_gap_color: Option<StoryPropertySummary>,
    strike_through_gap_color: Option<StoryPropertySummary>,
    open_type_features: Option<StoryListPropertySummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryPropertySummary {
    value_type: Option<String>,
    value: String,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryListPropertySummary {
    value_type: Option<String>,
    items: Vec<StoryListItemSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryListItemSummary {
    value_type: Option<String>,
    value: String,
    items: Vec<StoryListItemSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryMetadataSummary {
    applied_toc_style: Option<String>,
    track_changes: Option<bool>,
    story_title: Option<String>,
    applied_named_grid: Option<String>,
    user_text: Option<bool>,
    is_endnote_story: Option<bool>,
}

#[derive(Debug, Eq, PartialEq)]
struct StoryPreferenceSummary {
    optical_margin_alignment: Option<bool>,
    optical_margin_size: Option<String>,
    frame_type: Option<String>,
    story_orientation: Option<String>,
    story_direction: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct InCopyExportOptionSummary {
    include_graphic_proxies: Option<bool>,
    include_all_resources: Option<bool>,
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
    previous_text_frame: Option<String>,
    next_text_frame: Option<String>,
    bounds: Option<[u64; 4]>,
    metadata: MetadataSummary,
    preference: Option<TextFramePreferenceSummary>,
    path_geometry: Option<PathGeometrySummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct TextFramePreferenceSummary {
    text_column_count: Option<String>,
    text_column_gutter: Option<String>,
    text_column_fixed_width: Option<String>,
    text_column_max_width: Option<String>,
    use_fixed_column_width: Option<bool>,
    first_baseline_offset: Option<String>,
    minimum_first_baseline_offset: Option<String>,
    vertical_justification: Option<String>,
    vertical_threshold: Option<String>,
    ignore_wrap: Option<bool>,
    use_flexible_column_width: Option<bool>,
    auto_sizing_type: Option<String>,
    auto_sizing_reference_point: Option<String>,
    use_minimum_height_for_auto_sizing: Option<bool>,
    minimum_height_for_auto_sizing: Option<String>,
    use_minimum_width_for_auto_sizing: Option<bool>,
    minimum_width_for_auto_sizing: Option<String>,
    use_no_line_breaks_for_auto_sizing: Option<bool>,
    footnotes_enable_overrides: Option<bool>,
    footnotes_span_across_columns: Option<bool>,
    footnotes_minimum_spacing: Option<String>,
    footnotes_space_between: Option<String>,
    vertical_balance_columns: Option<bool>,
    column_rule_override: Option<bool>,
    column_rule_offset: Option<String>,
    column_rule_top_inset: Option<String>,
    column_rule_inset_chain_override: Option<bool>,
    column_rule_bottom_inset: Option<String>,
    column_rule_stroke_width: Option<String>,
    column_rule_stroke_color: Option<String>,
    column_rule_stroke_type: Option<String>,
    column_rule_stroke_tint: Option<String>,
    column_rule_overprint_override: Option<bool>,
    inset_spacing: Option<SpreadListPropertySummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct SpreadListPropertySummary {
    value_type: Option<String>,
    items: Vec<SpreadListItemSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct SpreadListItemSummary {
    value_type: Option<String>,
    value: String,
}

#[derive(Debug, Eq, PartialEq)]
enum PageItemSummary {
    Rectangle {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        metadata: MetadataSummary,
        path_geometry: Option<PathGeometrySummary>,
        images: Vec<ImageSummary>,
    },
    Oval {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        metadata: MetadataSummary,
        path_geometry: Option<PathGeometrySummary>,
    },
    GraphicLine {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        metadata: MetadataSummary,
        path_geometry: Option<PathGeometrySummary>,
    },
    Polygon {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        metadata: MetadataSummary,
        path_geometry: Option<PathGeometrySummary>,
    },
    Group {
        id: Option<String>,
        layer_id: Option<String>,
        bounds: Option<[u64; 4]>,
        metadata: MetadataSummary,
        page_items: Vec<PageItemSummary>,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct MetadataSummary {
    content_type: Option<String>,
    name: Option<String>,
    item_transform: Option<String>,
    applied_object_style: Option<String>,
    fill_color: Option<String>,
    stroke_color: Option<String>,
    stroke_weight: Option<String>,
    visible: Option<bool>,
    locked: Option<bool>,
    fill_tint: Option<String>,
    stroke_tint: Option<String>,
    gap_color: Option<String>,
    gap_tint: Option<String>,
    stroke_type: Option<String>,
    stroke_alignment: Option<String>,
    miter_limit: Option<String>,
    end_cap: Option<String>,
    end_join: Option<String>,
    left_line_end: Option<String>,
    right_line_end: Option<String>,
    lock_state: Option<String>,
    story_title: Option<String>,
    flex_item_width_mode: Option<String>,
    flex_item_height_mode: Option<String>,
    nonprinting: Option<bool>,
    overprint_fill: Option<bool>,
    overprint_stroke: Option<bool>,
    corner_radius: Option<String>,
    corner_option: Option<String>,
    top_left_corner_option: Option<String>,
    top_right_corner_option: Option<String>,
    bottom_left_corner_option: Option<String>,
    bottom_right_corner_option: Option<String>,
    top_left_corner_radius: Option<String>,
    top_right_corner_radius: Option<String>,
    bottom_left_corner_radius: Option<String>,
    bottom_right_corner_radius: Option<String>,
    parent_interface_change_count: Option<String>,
    target_interface_change_count: Option<String>,
    last_updated_interface_change_count: Option<String>,
    overridden_page_item_props: Option<String>,
    horizontal_layout_constraints: Option<String>,
    vertical_layout_constraints: Option<String>,
    gradient_fill_start: Option<String>,
    gradient_fill_length: Option<String>,
    gradient_fill_angle: Option<String>,
    gradient_stroke_start: Option<String>,
    gradient_stroke_length: Option<String>,
    gradient_stroke_angle: Option<String>,
    local_display_setting: Option<String>,
    gradient_fill_hilite_length: Option<String>,
    gradient_fill_hilite_angle: Option<String>,
    gradient_stroke_hilite_length: Option<String>,
    gradient_stroke_hilite_angle: Option<String>,
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
        let story_summaries =
            design_map
                .story_ids()
                .map(|story_id| {
                    let story = document.stories.get(story_id).ok_or_else(|| {
                        IdmlError::MissingReference {
                            kind: "story model",
                            id: story_id.to_owned(),
                        }
                    })?;
                    Ok(StorySummary {
                        id: story_id.to_owned(),
                        self_id: story.id.clone(),
                        text: story.text.clone(),
                        paragraph_style_ranges: summarize_paragraph_style_ranges(
                            &story.paragraph_style_ranges,
                        ),
                        metadata: summarize_story_metadata(&story.metadata),
                        preference: story.preference.as_ref().map(summarize_story_preference),
                        incopy_export_option: story
                            .incopy_export_option
                            .as_ref()
                            .map(summarize_incopy_export_option),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
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
                                previous_text_frame: frame.previous_text_frame.clone(),
                                next_text_frame: frame.next_text_frame.clone(),
                                bounds: frame.geometric_bounds.map(rect_bits),
                                metadata: summarize_metadata(&frame.metadata),
                                preference: frame
                                    .preference
                                    .as_ref()
                                    .map(summarize_text_frame_preference),
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
            story_summaries,
            spread_summaries,
        })
    }
}

fn summarize_paragraph_style_ranges(
    ranges: &[ParagraphStyleRange],
) -> Vec<ParagraphStyleRangeSummary> {
    ranges
        .iter()
        .map(|paragraph| ParagraphStyleRangeSummary {
            applied_paragraph_style: paragraph.applied_paragraph_style.clone(),
            justification: paragraph.justification.clone(),
            hyphenation: paragraph.hyphenation,
            rule_above_line_weight: paragraph.rule_above_line_weight.clone(),
            rule_below_line_weight: paragraph.rule_below_line_weight.clone(),
            hyphenation_zone: paragraph.hyphenation_zone.clone(),
            split_column_inside_gutter: paragraph.split_column_inside_gutter.clone(),
            space_before: paragraph.space_before.clone(),
            space_after: paragraph.space_after.clone(),
            properties: summarize_paragraph_style_properties(&paragraph.properties),
            character_style_ranges: paragraph
                .character_style_ranges
                .iter()
                .map(|character| CharacterStyleRangeSummary {
                    applied_character_style: character.applied_character_style.clone(),
                    point_size: character.point_size.clone(),
                    font_style: character.font_style.clone(),
                    fill_color: character.fill_color.clone(),
                    tracking: character.tracking.clone(),
                    stroke_weight: character.stroke_weight.clone(),
                    ruby_font_size: character.ruby_font_size.clone(),
                    kenten_font_size: character.kenten_font_size.clone(),
                    capitalization: character.capitalization.clone(),
                    miter_limit: character.miter_limit.clone(),
                    stroke_alignment: character.stroke_alignment.clone(),
                    horizontal_scale: character.horizontal_scale.clone(),
                    vertical_scale: character.vertical_scale.clone(),
                    properties: summarize_character_style_properties(&character.properties),
                    tables: summarize_story_tables(&character.tables),
                    text: character.text.clone(),
                })
                .collect(),
        })
        .collect()
}

fn summarize_story_tables(tables: &[StoryTable]) -> Vec<StoryTableSummary> {
    tables
        .iter()
        .map(|table| StoryTableSummary {
            id: table.id.clone(),
            header_row_count: table.header_row_count.clone(),
            footer_row_count: table.footer_row_count.clone(),
            body_row_count: table.body_row_count.clone(),
            column_count: table.column_count.clone(),
            applied_table_style: table.applied_table_style.clone(),
            table_direction: table.table_direction.clone(),
            text_frame: summarize_table_text_insets(&table.text_frame),
            rows: table
                .rows
                .iter()
                .map(|row| StoryTableRowSummary {
                    id: row.id.clone(),
                    name: row.name.clone(),
                    text_frame: summarize_table_text_insets(&row.text_frame),
                    single_row_height: row.single_row_height.clone(),
                    minimum_height: row.minimum_height.clone(),
                    auto_grow: row.auto_grow,
                })
                .collect(),
            columns: table
                .columns
                .iter()
                .map(|column| StoryTableColumnSummary {
                    id: column.id.clone(),
                    name: column.name.clone(),
                    text_frame: summarize_table_text_insets(&column.text_frame),
                    single_column_width: column.single_column_width.clone(),
                })
                .collect(),
            cells: table
                .cells
                .iter()
                .map(|cell| StoryTableCellSummary {
                    id: cell.id.clone(),
                    name: cell.name.clone(),
                    row_span: cell.row_span.clone(),
                    column_span: cell.column_span.clone(),
                    cell_type: cell.cell_type.clone(),
                    applied_cell_style: cell.applied_cell_style.clone(),
                    applied_cell_style_priority: cell.applied_cell_style_priority.clone(),
                    text_frame: summarize_table_text_insets(&cell.text_frame),
                    left_inset: cell.left_inset.clone(),
                    top_inset: cell.top_inset.clone(),
                    right_inset: cell.right_inset.clone(),
                    bottom_inset: cell.bottom_inset.clone(),
                    fill_color: cell.fill_color.clone(),
                    paragraph_style_ranges: summarize_paragraph_style_ranges(
                        &cell.paragraph_style_ranges,
                    ),
                })
                .collect(),
        })
        .collect()
}

fn summarize_table_text_insets(insets: &TableTextInsets) -> TableTextInsetsSummary {
    TableTextInsetsSummary {
        text_top_inset: insets.text_top_inset.clone(),
        text_left_inset: insets.text_left_inset.clone(),
        text_bottom_inset: insets.text_bottom_inset.clone(),
        text_right_inset: insets.text_right_inset.clone(),
        clip_content_to_text_cell: insets.clip_content_to_text_cell,
    }
}

fn summarize_paragraph_style_properties(
    properties: &ParagraphStyleProperties,
) -> ParagraphStylePropertiesSummary {
    ParagraphStylePropertiesSummary {
        same_para_style_spacing: properties
            .same_para_style_spacing
            .as_ref()
            .map(summarize_story_property),
        rule_above_gap_color: properties
            .rule_above_gap_color
            .as_ref()
            .map(summarize_story_property),
        rule_below_gap_color: properties
            .rule_below_gap_color
            .as_ref()
            .map(summarize_story_property),
    }
}

fn summarize_character_style_properties(
    properties: &CharacterStyleProperties,
) -> CharacterStylePropertiesSummary {
    CharacterStylePropertiesSummary {
        applied_font: properties
            .applied_font
            .as_ref()
            .map(summarize_story_property),
        leading: properties.leading.as_ref().map(summarize_story_property),
        underline_gap_color: properties
            .underline_gap_color
            .as_ref()
            .map(summarize_story_property),
        strike_through_gap_color: properties
            .strike_through_gap_color
            .as_ref()
            .map(summarize_story_property),
        open_type_features: properties
            .open_type_features
            .as_ref()
            .map(summarize_story_list_property),
    }
}

fn summarize_story_property(property: &StoryProperty) -> StoryPropertySummary {
    StoryPropertySummary {
        value_type: property.value_type.clone(),
        value: property.value.clone(),
    }
}

fn summarize_story_list_property(property: &StoryListProperty) -> StoryListPropertySummary {
    StoryListPropertySummary {
        value_type: property.value_type.clone(),
        items: property
            .items
            .iter()
            .map(summarize_story_list_item)
            .collect(),
    }
}

fn summarize_story_list_item(item: &StoryListItem) -> StoryListItemSummary {
    StoryListItemSummary {
        value_type: item.value_type.clone(),
        value: item.value.clone(),
        items: item.items.iter().map(summarize_story_list_item).collect(),
    }
}

fn summarize_story_metadata(metadata: &StoryMetadata) -> StoryMetadataSummary {
    StoryMetadataSummary {
        applied_toc_style: metadata.applied_toc_style.clone(),
        track_changes: metadata.track_changes,
        story_title: metadata.story_title.clone(),
        applied_named_grid: metadata.applied_named_grid.clone(),
        user_text: metadata.user_text,
        is_endnote_story: metadata.is_endnote_story,
    }
}

fn summarize_story_preference(preference: &StoryPreference) -> StoryPreferenceSummary {
    StoryPreferenceSummary {
        optical_margin_alignment: preference.optical_margin_alignment,
        optical_margin_size: preference.optical_margin_size.clone(),
        frame_type: preference.frame_type.clone(),
        story_orientation: preference.story_orientation.clone(),
        story_direction: preference.story_direction.clone(),
    }
}

fn summarize_incopy_export_option(option: &InCopyExportOption) -> InCopyExportOptionSummary {
    InCopyExportOptionSummary {
        include_graphic_proxies: option.include_graphic_proxies,
        include_all_resources: option.include_all_resources,
    }
}

fn summarize_text_frame_preference(preference: &TextFramePreference) -> TextFramePreferenceSummary {
    TextFramePreferenceSummary {
        text_column_count: preference.text_column_count.clone(),
        text_column_gutter: preference.text_column_gutter.clone(),
        text_column_fixed_width: preference.text_column_fixed_width.clone(),
        text_column_max_width: preference.text_column_max_width.clone(),
        use_fixed_column_width: preference.use_fixed_column_width,
        first_baseline_offset: preference.first_baseline_offset.clone(),
        minimum_first_baseline_offset: preference.minimum_first_baseline_offset.clone(),
        vertical_justification: preference.vertical_justification.clone(),
        vertical_threshold: preference.vertical_threshold.clone(),
        ignore_wrap: preference.ignore_wrap,
        use_flexible_column_width: preference.use_flexible_column_width,
        auto_sizing_type: preference.auto_sizing_type.clone(),
        auto_sizing_reference_point: preference.auto_sizing_reference_point.clone(),
        use_minimum_height_for_auto_sizing: preference.use_minimum_height_for_auto_sizing,
        minimum_height_for_auto_sizing: preference.minimum_height_for_auto_sizing.clone(),
        use_minimum_width_for_auto_sizing: preference.use_minimum_width_for_auto_sizing,
        minimum_width_for_auto_sizing: preference.minimum_width_for_auto_sizing.clone(),
        use_no_line_breaks_for_auto_sizing: preference.use_no_line_breaks_for_auto_sizing,
        footnotes_enable_overrides: preference.footnotes_enable_overrides,
        footnotes_span_across_columns: preference.footnotes_span_across_columns,
        footnotes_minimum_spacing: preference.footnotes_minimum_spacing.clone(),
        footnotes_space_between: preference.footnotes_space_between.clone(),
        vertical_balance_columns: preference.vertical_balance_columns,
        column_rule_override: preference.column_rule_override,
        column_rule_offset: preference.column_rule_offset.clone(),
        column_rule_top_inset: preference.column_rule_top_inset.clone(),
        column_rule_inset_chain_override: preference.column_rule_inset_chain_override,
        column_rule_bottom_inset: preference.column_rule_bottom_inset.clone(),
        column_rule_stroke_width: preference.column_rule_stroke_width.clone(),
        column_rule_stroke_color: preference.column_rule_stroke_color.clone(),
        column_rule_stroke_type: preference.column_rule_stroke_type.clone(),
        column_rule_stroke_tint: preference.column_rule_stroke_tint.clone(),
        column_rule_overprint_override: preference.column_rule_overprint_override,
        inset_spacing: preference
            .inset_spacing
            .as_ref()
            .map(summarize_spread_list_property),
    }
}

fn summarize_spread_list_property(property: &SpreadListProperty) -> SpreadListPropertySummary {
    SpreadListPropertySummary {
        value_type: property.value_type.clone(),
        items: property
            .items
            .iter()
            .map(|item| SpreadListItemSummary {
                value_type: item.value_type.clone(),
                value: item.value.clone(),
            })
            .collect(),
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
                metadata: summarize_metadata(&rectangle.metadata),
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
                metadata: summarize_metadata(&oval.metadata),
                path_geometry: oval.path_geometry.as_ref().map(summarize_path_geometry),
            }),
            PageItem::GraphicLine(line) => Some(PageItemSummary::GraphicLine {
                id: line.id.clone(),
                layer_id: line.layer_id.clone(),
                bounds: line.geometric_bounds.map(rect_bits),
                metadata: summarize_metadata(&line.metadata),
                path_geometry: line.path_geometry.as_ref().map(summarize_path_geometry),
            }),
            PageItem::Polygon(polygon) => Some(PageItemSummary::Polygon {
                id: polygon.id.clone(),
                layer_id: polygon.layer_id.clone(),
                bounds: polygon.geometric_bounds.map(rect_bits),
                metadata: summarize_metadata(&polygon.metadata),
                path_geometry: polygon.path_geometry.as_ref().map(summarize_path_geometry),
            }),
            PageItem::Group(group) => Some(PageItemSummary::Group {
                id: group.id.clone(),
                layer_id: group.layer_id.clone(),
                bounds: group.geometric_bounds.map(rect_bits),
                metadata: summarize_metadata(&group.metadata),
                page_items: summarize_page_items(&group.page_items),
            }),
        })
        .collect()
}

fn summarize_metadata(metadata: &PageItemMetadata) -> MetadataSummary {
    MetadataSummary {
        content_type: metadata.content_type.clone(),
        name: metadata.name.clone(),
        item_transform: metadata.item_transform.clone(),
        applied_object_style: metadata.applied_object_style.clone(),
        fill_color: metadata.fill_color.clone(),
        stroke_color: metadata.stroke_color.clone(),
        stroke_weight: metadata.stroke_weight.clone(),
        visible: metadata.visible,
        locked: metadata.locked,
        fill_tint: metadata.fill_tint.clone(),
        stroke_tint: metadata.stroke_tint.clone(),
        gap_color: metadata.gap_color.clone(),
        gap_tint: metadata.gap_tint.clone(),
        stroke_type: metadata.stroke_type.clone(),
        stroke_alignment: metadata.stroke_alignment.clone(),
        miter_limit: metadata.miter_limit.clone(),
        end_cap: metadata.end_cap.clone(),
        end_join: metadata.end_join.clone(),
        left_line_end: metadata.left_line_end.clone(),
        right_line_end: metadata.right_line_end.clone(),
        lock_state: metadata.lock_state.clone(),
        story_title: metadata.story_title.clone(),
        flex_item_width_mode: metadata.flex_item_width_mode.clone(),
        flex_item_height_mode: metadata.flex_item_height_mode.clone(),
        nonprinting: metadata.nonprinting,
        overprint_fill: metadata.overprint_fill,
        overprint_stroke: metadata.overprint_stroke,
        corner_radius: metadata.corner_radius.clone(),
        corner_option: metadata.corner_option.clone(),
        top_left_corner_option: metadata.top_left_corner_option.clone(),
        top_right_corner_option: metadata.top_right_corner_option.clone(),
        bottom_left_corner_option: metadata.bottom_left_corner_option.clone(),
        bottom_right_corner_option: metadata.bottom_right_corner_option.clone(),
        top_left_corner_radius: metadata.top_left_corner_radius.clone(),
        top_right_corner_radius: metadata.top_right_corner_radius.clone(),
        bottom_left_corner_radius: metadata.bottom_left_corner_radius.clone(),
        bottom_right_corner_radius: metadata.bottom_right_corner_radius.clone(),
        parent_interface_change_count: metadata.parent_interface_change_count.clone(),
        target_interface_change_count: metadata.target_interface_change_count.clone(),
        last_updated_interface_change_count: metadata.last_updated_interface_change_count.clone(),
        overridden_page_item_props: metadata.overridden_page_item_props.clone(),
        horizontal_layout_constraints: metadata.horizontal_layout_constraints.clone(),
        vertical_layout_constraints: metadata.vertical_layout_constraints.clone(),
        gradient_fill_start: metadata.gradient_fill_start.clone(),
        gradient_fill_length: metadata.gradient_fill_length.clone(),
        gradient_fill_angle: metadata.gradient_fill_angle.clone(),
        gradient_stroke_start: metadata.gradient_stroke_start.clone(),
        gradient_stroke_length: metadata.gradient_stroke_length.clone(),
        gradient_stroke_angle: metadata.gradient_stroke_angle.clone(),
        local_display_setting: metadata.local_display_setting.clone(),
        gradient_fill_hilite_length: metadata.gradient_fill_hilite_length.clone(),
        gradient_fill_hilite_angle: metadata.gradient_fill_hilite_angle.clone(),
        gradient_stroke_hilite_length: metadata.gradient_stroke_hilite_length.clone(),
        gradient_stroke_hilite_angle: metadata.gradient_stroke_hilite_angle.clone(),
    }
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
