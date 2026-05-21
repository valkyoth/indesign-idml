//! Companion CLI for bounded IDML text extraction.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use indesign_idml::archive::{ArchiveLimits, IdmlPackage};
use indesign_idml::core::units::Points;
use indesign_idml::model::designmap::DesignMapParseOptions;
use indesign_idml::model::spread::{Rect, SpreadParseOptions};
use indesign_idml::model::story::StoryParseOptions;
use std::env;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("idml-text: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<(), CliError> {
    let mut input = None;
    let mut output = OutputTarget::Stdout;
    let mut archive_limits = ArchiveLimits::default();
    let mut designmap_options = DesignMapParseOptions::default();
    let mut spread_options = SpreadParseOptions::default();
    let mut story_options = StoryParseOptions::default();
    let mut spread_id = None;
    let mut all_spreads = false;
    let mut page_id = None;
    let mut rect_points = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            print_help()?;
            return Ok(());
        }
        if arg == "--output" || arg == "-o" {
            let Some(path) = args.next() else {
                return Err(CliError::Usage("missing path after --output"));
            };
            output = OutputTarget::File(PathBuf::from(path));
            continue;
        }
        if arg == "--max-story-text-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-story-text-bytes",
                ));
            };
            story_options.max_text_bytes = parse_usize(&value, "--max-story-text-bytes")?;
            continue;
        }
        if arg == "--max-story-attribute-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-story-attribute-bytes",
                ));
            };
            story_options.max_attribute_bytes = parse_usize(&value, "--max-story-attribute-bytes")?;
            continue;
        }
        if arg == "--max-story-xml-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage("missing value after --max-story-xml-bytes"));
            };
            story_options.max_xml_bytes = parse_usize(&value, "--max-story-xml-bytes")?;
            continue;
        }
        if arg == "--max-story-xml-depth" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage("missing value after --max-story-xml-depth"));
            };
            story_options.max_xml_depth = parse_usize(&value, "--max-story-xml-depth")?;
            continue;
        }
        if arg == "--spread-id" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage("missing value after --spread-id"));
            };
            spread_id = Some(parse_string(&value, "--spread-id")?);
            continue;
        }
        if arg == "--all-spreads" {
            all_spreads = true;
            continue;
        }
        if arg == "--page-id" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage("missing value after --page-id"));
            };
            page_id = Some(parse_string(&value, "--page-id")?);
            continue;
        }
        if arg == "--rect-points" {
            let Some(top) = args.next() else {
                return Err(CliError::Usage("missing top after --rect-points"));
            };
            let Some(left) = args.next() else {
                return Err(CliError::Usage("missing left after --rect-points"));
            };
            let Some(bottom) = args.next() else {
                return Err(CliError::Usage("missing bottom after --rect-points"));
            };
            let Some(right) = args.next() else {
                return Err(CliError::Usage("missing right after --rect-points"));
            };
            rect_points = Some(parse_rect_points([top, left, bottom, right])?);
            continue;
        }
        if arg == "--max-spread-pages" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage("missing value after --max-spread-pages"));
            };
            spread_options.max_pages = parse_usize(&value, "--max-spread-pages")?;
            continue;
        }
        if arg == "--max-spread-xml-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-spread-xml-bytes",
                ));
            };
            spread_options.max_xml_bytes = parse_usize(&value, "--max-spread-xml-bytes")?;
            continue;
        }
        if arg == "--max-spread-text-frames" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-spread-text-frames",
                ));
            };
            spread_options.max_text_frames = parse_usize(&value, "--max-spread-text-frames")?;
            continue;
        }
        if arg == "--max-spread-page-items" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-spread-page-items",
                ));
            };
            spread_options.max_page_items = parse_usize(&value, "--max-spread-page-items")?;
            continue;
        }
        if arg == "--max-spread-attribute-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-spread-attribute-bytes",
                ));
            };
            spread_options.max_attribute_bytes =
                parse_usize(&value, "--max-spread-attribute-bytes")?;
            continue;
        }
        if arg == "--max-spread-xml-depth" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-spread-xml-depth",
                ));
            };
            spread_options.max_xml_depth = parse_usize(&value, "--max-spread-xml-depth")?;
            continue;
        }
        if arg == "--max-designmap-package-refs" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-designmap-package-refs",
                ));
            };
            designmap_options.max_package_refs =
                parse_usize(&value, "--max-designmap-package-refs")?;
            continue;
        }
        if arg == "--max-designmap-package-src-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-designmap-package-src-bytes",
                ));
            };
            designmap_options.max_package_src_bytes =
                parse_usize(&value, "--max-designmap-package-src-bytes")?;
            continue;
        }
        if arg == "--max-designmap-attribute-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-designmap-attribute-bytes",
                ));
            };
            designmap_options.max_attribute_bytes =
                parse_usize(&value, "--max-designmap-attribute-bytes")?;
            continue;
        }
        if arg == "--max-designmap-xml-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-designmap-xml-bytes",
                ));
            };
            designmap_options.max_xml_bytes = parse_usize(&value, "--max-designmap-xml-bytes")?;
            continue;
        }
        if arg == "--max-designmap-xml-depth" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-designmap-xml-depth",
                ));
            };
            designmap_options.max_xml_depth = parse_usize(&value, "--max-designmap-xml-depth")?;
            continue;
        }
        if arg == "--max-archive-entries" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage("missing value after --max-archive-entries"));
            };
            archive_limits.max_entries = parse_usize(&value, "--max-archive-entries")?;
            continue;
        }
        if arg == "--max-entry-uncompressed-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-entry-uncompressed-bytes",
                ));
            };
            archive_limits.max_entry_uncompressed_size =
                parse_u64(&value, "--max-entry-uncompressed-bytes")?;
            continue;
        }
        if arg == "--max-total-uncompressed-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-total-uncompressed-bytes",
                ));
            };
            archive_limits.max_total_uncompressed_size =
                parse_u64(&value, "--max-total-uncompressed-bytes")?;
            continue;
        }
        if arg == "--max-entry-compression-ratio" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-entry-compression-ratio",
                ));
            };
            archive_limits.max_entry_compression_ratio =
                parse_u64(&value, "--max-entry-compression-ratio")?;
            continue;
        }
        if arg == "--max-archive-path-bytes" {
            let Some(value) = args.next() else {
                return Err(CliError::Usage(
                    "missing value after --max-archive-path-bytes",
                ));
            };
            archive_limits.max_path_bytes = parse_usize(&value, "--max-archive-path-bytes")?;
            continue;
        }
        if input.replace(PathBuf::from(arg)).is_some() {
            return Err(CliError::Usage("expected exactly one input IDML path"));
        }
    }

    let Some(input) = input else {
        return Err(CliError::Usage("missing input IDML path"));
    };
    if all_spreads && spread_id.is_some() {
        return Err(CliError::Usage(
            "--all-spreads cannot be combined with --spread-id",
        ));
    }
    if all_spreads && page_id.is_some() {
        return Err(CliError::Usage("--page-id requires --spread-id"));
    }
    if all_spreads && rect_points.is_some() {
        return Err(CliError::Usage("--rect-points requires --spread-id"));
    }
    if page_id.is_some() && spread_id.is_none() {
        return Err(CliError::Usage("--page-id requires --spread-id"));
    }
    if rect_points.is_some() && spread_id.is_none() {
        return Err(CliError::Usage("--rect-points requires --spread-id"));
    }
    if page_id.is_some() && rect_points.is_some() {
        return Err(CliError::Usage(
            "--page-id and --rect-points cannot be combined",
        ));
    }

    let file = File::open(input)?;
    let mut package = IdmlPackage::with_limits(file, archive_limits)?;
    let design_map = package.read_designmap_with_options(designmap_options)?;

    with_output_writer(output, |output| {
        let mut writer = TextWriter::new(output);
        if all_spreads {
            for spread_id in design_map.spread_ids() {
                let frames = package.resolve_spread_text_frames_with_options(
                    &design_map,
                    spread_id,
                    spread_options,
                    story_options,
                )?;
                for frame in &frames {
                    writer.write_text(&frame.text)?;
                }
            }
        } else {
            match (spread_id.as_deref(), page_id.as_deref(), rect_points) {
                (Some(spread_id), Some(page_id), None) => {
                    let frames = package.resolve_spread_text_on_page_with_options(
                        &design_map,
                        spread_id,
                        page_id,
                        spread_options,
                        story_options,
                    )?;
                    for frame in &frames {
                        writer.write_text(&frame.text)?;
                    }
                }
                (Some(spread_id), None, Some(query)) => {
                    let frames = package.resolve_spread_text_in_rect_with_options(
                        &design_map,
                        spread_id,
                        query,
                        spread_options,
                        story_options,
                    )?;
                    for frame in &frames {
                        writer.write_text(&frame.text)?;
                    }
                }
                (Some(spread_id), None, None) => {
                    let frames = package.resolve_spread_text_frames_with_options(
                        &design_map,
                        spread_id,
                        spread_options,
                        story_options,
                    )?;
                    for frame in &frames {
                        writer.write_text(&frame.text)?;
                    }
                }
                (None, None, None) => {
                    for path in design_map.story_srcs.values() {
                        let story = package.read_story_with_options(path, story_options)?;
                        writer.write_text(&story.text)?;
                    }
                }
                (None, Some(_), _) => {
                    return Err(CliError::Usage("--page-id requires --spread-id"));
                }
                (None, None, Some(_)) => {
                    return Err(CliError::Usage("--rect-points requires --spread-id"));
                }
                (Some(_), Some(_), Some(_)) => {
                    return Err(CliError::Usage(
                        "--page-id and --rect-points cannot be combined",
                    ));
                }
            }
        }
        Ok(())
    })?;

    Ok(())
}

fn with_output_writer(
    output: OutputTarget,
    action: impl FnOnce(&mut dyn Write) -> Result<(), CliError>,
) -> Result<(), CliError> {
    match output {
        OutputTarget::Stdout => {
            let stdout = io::stdout();
            let mut writer = BufWriter::new(stdout.lock());
            action(&mut writer)?;
            writer.flush()?;
        }
        OutputTarget::File(path) => {
            let file = File::create(path)?;
            let mut writer = BufWriter::new(file);
            action(&mut writer)?;
            writer.flush()?;
        }
    }
    Ok(())
}

struct TextWriter<'a, W: Write + ?Sized> {
    writer: &'a mut W,
    first: bool,
}

impl<'a, W: Write + ?Sized> TextWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            first: true,
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), CliError> {
        if !self.first {
            self.writer.write_all(b"\n")?;
        }
        self.first = false;
        self.writer.write_all(text.as_bytes())?;
        if !text.ends_with('\n') {
            self.writer.write_all(b"\n")?;
        }
        Ok(())
    }
}

fn print_help() -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    writer.write_all(
        b"Usage: idml-text [--output PATH] [--all-spreads | --spread-id ID [--page-id ID | --rect-points T L B R]] [limits] INPUT.idml\n\nExtracts story text in designmap.xml order, all layout-resolved spread text, or layout-resolved text for one spread/page/rectangle.\n\nLayout:\n  --all-spreads\n  --spread-id ID\n  --page-id ID\n  --rect-points TOP LEFT BOTTOM RIGHT\n\nLimits:\n  --max-archive-entries COUNT\n  --max-entry-uncompressed-bytes BYTES\n  --max-total-uncompressed-bytes BYTES\n  --max-entry-compression-ratio RATIO\n  --max-archive-path-bytes BYTES\n  --max-designmap-package-refs COUNT\n  --max-designmap-package-src-bytes BYTES\n  --max-designmap-attribute-bytes BYTES\n  --max-designmap-xml-bytes BYTES\n  --max-designmap-xml-depth DEPTH\n  --max-story-text-bytes BYTES\n  --max-story-attribute-bytes BYTES\n  --max-story-xml-bytes BYTES\n  --max-story-xml-depth DEPTH\n  --max-spread-pages COUNT\n  --max-spread-xml-bytes BYTES\n  --max-spread-text-frames COUNT\n  --max-spread-page-items COUNT\n  --max-spread-attribute-bytes BYTES\n  --max-spread-xml-depth DEPTH\n",
    )?;
    writer.flush()?;
    Ok(())
}

fn parse_string(value: &std::ffi::OsStr, flag: &'static str) -> Result<String, CliError> {
    let Some(value) = value.to_str() else {
        return Err(CliError::Usage("ID option must be valid UTF-8"));
    };
    if value.is_empty() {
        return Err(CliError::Usage(match flag {
            "--spread-id" => "--spread-id cannot be empty",
            "--page-id" => "--page-id cannot be empty",
            _ => "ID option cannot be empty",
        }));
    }
    Ok(value.to_owned())
}

fn parse_rect_points(values: [std::ffi::OsString; 4]) -> Result<Rect, CliError> {
    let [top, left, bottom, right] = values.map(|value| parse_f64(&value, "--rect-points"));
    let (top, left, bottom, right) = (top?, left?, bottom?, right?);
    if top > bottom || left > right {
        return Err(CliError::Usage(
            "--rect-points must satisfy top <= bottom and left <= right",
        ));
    }
    Ok(Rect::new(
        Points::new(top),
        Points::new(left),
        Points::new(bottom),
        Points::new(right),
    ))
}

fn parse_usize(value: &std::ffi::OsStr, flag: &'static str) -> Result<usize, CliError> {
    let Some(value) = value.to_str() else {
        return Err(CliError::Usage("numeric option must be valid UTF-8"));
    };
    value.parse::<usize>().map_err(|_| CliError::InvalidNumber {
        flag,
        value: value.to_owned(),
    })
}

fn parse_u64(value: &std::ffi::OsStr, flag: &'static str) -> Result<u64, CliError> {
    let Some(value) = value.to_str() else {
        return Err(CliError::Usage("numeric option must be valid UTF-8"));
    };
    value.parse::<u64>().map_err(|_| CliError::InvalidNumber {
        flag,
        value: value.to_owned(),
    })
}

fn parse_f64(value: &std::ffi::OsStr, flag: &'static str) -> Result<f64, CliError> {
    let Some(value) = value.to_str() else {
        return Err(CliError::Usage("numeric option must be valid UTF-8"));
    };
    let parsed = value.parse::<f64>().map_err(|_| CliError::InvalidNumber {
        flag,
        value: value.to_owned(),
    })?;
    if !parsed.is_finite() {
        return Err(CliError::InvalidNumber {
            flag,
            value: value.to_owned(),
        });
    }
    Ok(parsed)
}

enum OutputTarget {
    Stdout,
    File(PathBuf),
}

#[derive(Debug)]
enum CliError {
    Idml(indesign_idml::IdmlError),
    Io(io::Error),
    InvalidNumber { flag: &'static str, value: String },
    Usage(&'static str),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idml(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::InvalidNumber { flag, value } => {
                write!(formatter, "invalid value `{value}` for {flag}")
            }
            Self::Usage(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<indesign_idml::IdmlError> for CliError {
    fn from(error: indesign_idml::IdmlError) -> Self {
        Self::Idml(error)
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
