//! Integration tests for the `idml-text` companion CLI.

#![cfg(feature = "cli")]

use assert_cmd::Command;
use std::fs;
use std::io::{Cursor, Write};
use tempfile::tempdir;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[test]
fn idml_text_extracts_stories_to_stdout() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg(&input);

    command.assert().success().stdout("Hello\n\nWorld\n");
}

#[test]
fn idml_text_extracts_spread_text_to_stdout() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--spread-id").arg("u10").arg(&input);

    command.assert().success().stdout("Page hit\n\nPage miss\n");
}

#[test]
fn idml_text_extracts_page_text_to_stdout() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--spread-id")
        .arg("u10")
        .arg("--page-id")
        .arg("p1")
        .arg(&input);

    command.assert().success().stdout("Page hit\n");
}

#[test]
fn idml_text_extracts_rect_text_to_stdout() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--spread-id")
        .arg("u10")
        .arg("--rect-points")
        .args(["0", "0", "100", "100"])
        .arg(&input);

    command.assert().success().stdout("Page hit\n");
}

#[test]
fn idml_text_extracts_stories_to_file() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    let output = temp.path().join("out.txt");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--output").arg(&output).arg(&input);

    command.assert().success();
    assert_eq!(fs::read_to_string(output).unwrap(), "Hello\n\nWorld\n");
}

#[test]
fn idml_text_rejects_missing_input() {
    let mut command = Command::cargo_bin("idml-text").unwrap();

    command
        .assert()
        .failure()
        .stderr("idml-text: missing input IDML path\n");
}

#[test]
fn idml_text_rejects_page_without_spread() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--page-id").arg("p1").arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: --page-id requires --spread-id\n");
}

#[test]
fn idml_text_rejects_rect_without_spread() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--rect-points")
        .args(["0", "0", "100", "100"])
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: --rect-points requires --spread-id\n");
}

#[test]
fn idml_text_rejects_conflicting_layout_scopes() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--spread-id")
        .arg("u10")
        .arg("--page-id")
        .arg("p1")
        .arg("--rect-points")
        .args(["0", "0", "100", "100"])
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: --page-id and --rect-points cannot be combined\n");
}

#[test]
fn idml_text_rejects_invalid_rect_points() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--spread-id")
        .arg("u10")
        .arg("--rect-points")
        .args(["0", "0", "nan", "100"])
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: invalid value `nan` for --rect-points\n");
}

#[test]
fn idml_text_enforces_story_text_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--max-story-text-bytes").arg("4").arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for story text bytes: limit 4, actual 5\n");
}

#[test]
fn idml_text_rejects_invalid_story_text_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--max-story-text-bytes")
        .arg("not-a-number")
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: invalid value `not-a-number` for --max-story-text-bytes\n");
}

#[test]
fn idml_text_enforces_archive_entry_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--max-archive-entries").arg("1").arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for archive entry count: limit 1, actual 4\n");
}

#[test]
fn idml_text_enforces_entry_compression_ratio_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("compressed.idml");
    fs::write(&input, compressed_entry_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--max-entry-compression-ratio")
        .arg("1")
        .arg(&input);

    command.assert().failure().stderr(
        "idml-text: limit exceeded for archive entry compression ratio: limit 1, actual 196\n",
    );
}

#[test]
fn idml_text_enforces_archive_path_byte_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--max-archive-path-bytes").arg("8").arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for archive path bytes: limit 8, actual 13\n");
}

#[test]
fn idml_text_enforces_designmap_ref_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--max-designmap-package-refs")
        .arg("1")
        .arg(&input);

    command.assert().failure().stderr(
        "idml-text: limit exceeded for DesignMap package reference count: limit 1, actual 2\n",
    );
}

#[test]
fn idml_text_enforces_designmap_package_src_byte_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--max-designmap-package-src-bytes")
        .arg("8")
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for DesignMap package src bytes: limit 8, actual 20\n");
}

#[test]
fn idml_text_enforces_designmap_xml_byte_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--max-designmap-xml-bytes")
        .arg("8")
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for DesignMap XML bytes: limit 8, actual 179\n");
}

#[test]
fn idml_text_enforces_story_xml_byte_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--max-story-xml-bytes").arg("8").arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for Story XML bytes: limit 8, actual 49\n");
}

#[test]
fn idml_text_enforces_story_attribute_byte_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--max-story-attribute-bytes")
        .arg("1")
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for Story XML attribute bytes: limit 1, actual 2\n");
}

#[test]
fn idml_text_enforces_story_xml_depth_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("sample.idml");
    fs::write(&input, sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command.arg("--max-story-xml-depth").arg("1").arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for Story XML depth: limit 1, actual 2\n");
}

#[test]
fn idml_text_enforces_spread_xml_byte_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--spread-id")
        .arg("u10")
        .arg("--max-spread-xml-bytes")
        .arg("8")
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for Spread XML bytes: limit 8, actual 233\n");
}

#[test]
fn idml_text_enforces_spread_text_frame_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--spread-id")
        .arg("u10")
        .arg("--max-spread-text-frames")
        .arg("1")
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for spread text frame count: limit 1, actual 2\n");
}

#[test]
fn idml_text_enforces_spread_attribute_byte_limit() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("layout.idml");
    fs::write(&input, layout_sample_idml()).unwrap();

    let mut command = Command::cargo_bin("idml-text").unwrap();
    command
        .arg("--spread-id")
        .arg("u10")
        .arg("--max-spread-attribute-bytes")
        .arg("2")
        .arg(&input);

    command
        .assert()
        .failure()
        .stderr("idml-text: limit exceeded for Spread XML attribute bytes: limit 2, actual 3\n");
}

fn sample_idml() -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut buffer);
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::DEFAULT);
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::DEFAULT);

    writer.start_file("mimetype", stored).unwrap();
    writer
        .write_all(b"application/vnd.adobe.indesign-idml-package")
        .unwrap();
    writer.start_file("designmap.xml", deflated).unwrap();
    writer
        .write_all(
            br#"<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <idPkg:Story src="Stories/Story_u1.xml" />
  <idPkg:Story src="Stories/Story_u2.xml" />
</Document>"#,
        )
        .unwrap();
    writer.start_file("Stories/Story_u1.xml", deflated).unwrap();
    writer
        .write_all(br#"<Story Self="u1"><Content>Hello</Content></Story>"#)
        .unwrap();
    writer.start_file("Stories/Story_u2.xml", deflated).unwrap();
    writer
        .write_all(br#"<Story Self="u2"><Content>World</Content></Story>"#)
        .unwrap();
    writer.finish().unwrap();

    buffer.into_inner()
}

fn compressed_entry_idml() -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut buffer);
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::DEFAULT);
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::DEFAULT);

    writer.start_file("mimetype", stored).unwrap();
    writer
        .write_all(b"application/vnd.adobe.indesign-idml-package")
        .unwrap();
    writer
        .start_file("Resources/Compressed.bin", deflated)
        .unwrap();
    writer.write_all(&vec![0u8; 4096]).unwrap();
    writer.finish().unwrap();
    buffer.into_inner()
}

fn layout_sample_idml() -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut buffer);
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::DEFAULT);
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::DEFAULT);

    writer.start_file("mimetype", stored).unwrap();
    writer
        .write_all(b"application/vnd.adobe.indesign-idml-package")
        .unwrap();
    writer.start_file("designmap.xml", deflated).unwrap();
    writer
        .write_all(
            br#"<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <idPkg:Spread src="Spreads/Spread_u10.xml" />
  <idPkg:Story src="Stories/Story_u1.xml" />
  <idPkg:Story src="Stories/Story_u2.xml" />
</Document>"#,
        )
        .unwrap();
    writer
        .start_file("Spreads/Spread_u10.xml", deflated)
        .unwrap();
    writer
        .write_all(
            br#"<Spread Self="u10">
  <Page Self="p1" GeometricBounds="0 0 100 100" />
  <TextFrame Self="hit" ParentStory="u1" GeometricBounds="10 10 20 20" />
  <TextFrame Self="miss" ParentStory="u2" GeometricBounds="200 200 300 300" />
</Spread>"#,
        )
        .unwrap();
    writer.start_file("Stories/Story_u1.xml", deflated).unwrap();
    writer
        .write_all(br#"<Story Self="u1"><Content>Page hit</Content></Story>"#)
        .unwrap();
    writer.start_file("Stories/Story_u2.xml", deflated).unwrap();
    writer
        .write_all(br#"<Story Self="u2"><Content>Page miss</Content></Story>"#)
        .unwrap();
    writer.finish().unwrap();

    buffer.into_inner()
}
