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
