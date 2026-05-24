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
    story_count: usize,
    spread_count: usize,
    preserved_entry_count: usize,
    story_texts: Vec<(String, String)>,
    spread_summaries: Vec<SpreadSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct SpreadSummary {
    id: String,
    page_count: usize,
    text_frames: Vec<TextFrameSummary>,
}

#[derive(Debug, Eq, PartialEq)]
struct TextFrameSummary {
    id: Option<String>,
    parent_story: Option<String>,
}

impl Snapshot {
    fn from_document(document: &IdmlDocument) -> Result<Self> {
        let mut story_texts = Vec::new();
        document.try_for_each_story_text(|story_id, text| {
            story_texts.push((story_id.to_owned(), text.to_owned()));
            Ok::<(), IdmlError>(())
        })?;
        let spread_summaries =
            document
                .design_map
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
                        page_count: spread.pages.len(),
                        text_frames: spread
                            .text_frames
                            .iter()
                            .map(|frame| TextFrameSummary {
                                id: frame.id.clone(),
                                parent_story: frame.parent_story.clone(),
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            story_count: document.design_map.story_srcs.len(),
            spread_count: document.design_map.spread_srcs.len(),
            preserved_entry_count: document.preserved_entries.len(),
            story_texts,
            spread_summaries,
        })
    }
}
