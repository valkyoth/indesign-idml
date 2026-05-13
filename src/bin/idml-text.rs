#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use indesign_idml::archive::IdmlPackage;
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
        if input.replace(PathBuf::from(arg)).is_some() {
            return Err(CliError::Usage("expected exactly one input IDML path"));
        }
    }

    let Some(input) = input else {
        return Err(CliError::Usage("missing input IDML path"));
    };

    let file = File::open(input)?;
    let mut package = IdmlPackage::new(file)?;
    let design_map = package.read_designmap()?;
    let texts = package.extract_story_texts(&design_map)?;

    match output {
        OutputTarget::Stdout => {
            let stdout = io::stdout();
            write_texts(BufWriter::new(stdout.lock()), texts.values())?;
        }
        OutputTarget::File(path) => {
            let file = File::create(path)?;
            write_texts(BufWriter::new(file), texts.values())?;
        }
    }

    Ok(())
}

fn write_texts<'a>(
    mut writer: impl Write,
    texts: impl IntoIterator<Item = &'a String>,
) -> Result<(), CliError> {
    let mut first = true;
    for text in texts {
        if !first {
            writer.write_all(b"\n")?;
        }
        first = false;
        writer.write_all(text.as_bytes())?;
        if !text.ends_with('\n') {
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn print_help() -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    writer.write_all(
        b"Usage: idml-text [--output PATH] INPUT.idml\n\nExtracts story text in designmap.xml order.\n",
    )?;
    writer.flush()?;
    Ok(())
}

enum OutputTarget {
    Stdout,
    File(PathBuf),
}

#[derive(Debug)]
enum CliError {
    Idml(indesign_idml::IdmlError),
    Io(io::Error),
    Usage(&'static str),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idml(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
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
