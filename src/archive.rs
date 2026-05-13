//! Secure ZIP archive inventory and bounded read support.

use crate::error::{IdmlError, Result};
use crate::model::designmap::DesignMap;
use indexmap::IndexMap;
use std::fmt;
use std::io::{Read, Seek};
use zip::{CompressionMethod, ZipArchive};

/// Default maximum number of entries accepted in an IDML archive.
pub const DEFAULT_MAX_ENTRIES: usize = 20_000;

/// Default maximum uncompressed bytes accepted for one entry.
pub const DEFAULT_MAX_ENTRY_UNCOMPRESSED_SIZE: u64 = 256 * 1024 * 1024;

/// Default maximum aggregate uncompressed bytes accepted for an archive.
pub const DEFAULT_MAX_TOTAL_UNCOMPRESSED_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// Logical, normalized path inside an IDML ZIP archive.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdmlPath(String);

impl IdmlPath {
    /// Validates and stores a logical ZIP path.
    pub fn new(path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        validate_archive_path(&path)?;
        Ok(Self(path))
    }

    /// Returns the path as a ZIP entry name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdmlPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Size and count limits enforced before entry content is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchiveLimits {
    /// Maximum number of ZIP entries.
    pub max_entries: usize,
    /// Maximum uncompressed size for one entry.
    pub max_entry_uncompressed_size: u64,
    /// Maximum aggregate uncompressed size across all entries.
    pub max_total_uncompressed_size: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_entry_uncompressed_size: DEFAULT_MAX_ENTRY_UNCOMPRESSED_SIZE,
            max_total_uncompressed_size: DEFAULT_MAX_TOTAL_UNCOMPRESSED_SIZE,
        }
    }
}

/// Metadata for one archive entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEntry {
    /// Logical entry path.
    pub path: IdmlPath,
    /// Compressed byte size from ZIP metadata.
    pub compressed_size: u64,
    /// Uncompressed byte size from ZIP metadata.
    pub uncompressed_size: u64,
    /// ZIP compression method.
    pub compression: CompressionMethod,
}

/// Open IDML package with a validated entry inventory.
#[derive(Debug)]
pub struct IdmlPackage<R>
where
    R: Read + Seek,
{
    archive: ZipArchive<R>,
    entries: IndexMap<IdmlPath, ArchiveEntry>,
    limits: ArchiveLimits,
}

impl<R> IdmlPackage<R>
where
    R: Read + Seek,
{
    /// Opens an IDML ZIP archive with default limits.
    pub fn new(reader: R) -> Result<Self> {
        Self::with_limits(reader, ArchiveLimits::default())
    }

    /// Opens an IDML ZIP archive with explicit limits.
    pub fn with_limits(reader: R, limits: ArchiveLimits) -> Result<Self> {
        let archive = ZipArchive::new(reader)?;
        Self::from_archive(archive, limits)
    }

    /// Returns the ordered archive inventory.
    #[must_use]
    pub fn entries(&self) -> &IndexMap<IdmlPath, ArchiveEntry> {
        &self.entries
    }

    /// Returns true when the archive contains the requested entry.
    #[must_use]
    pub fn contains(&self, path: &IdmlPath) -> bool {
        self.entries.contains_key(path)
    }

    /// Reads a complete entry into memory after enforcing configured limits.
    pub fn read_entry(&mut self, path: &IdmlPath) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(path)
            .ok_or_else(|| IdmlError::MissingArchiveEntry(path.to_string()))?;
        enforce_entry_size(
            entry.uncompressed_size,
            self.limits.max_entry_uncompressed_size,
        )?;

        let capacity =
            usize::try_from(entry.uncompressed_size).map_err(|_| IdmlError::LimitExceeded {
                what: "entry uncompressed size",
                limit: usize::MAX as u64,
                actual: entry.uncompressed_size,
            })?;
        let mut file = self.archive.by_name(path.as_str())?;
        let mut data = Vec::with_capacity(capacity);
        file.read_to_end(&mut data)?;

        if u64::try_from(data.len()).unwrap_or(u64::MAX) != entry.uncompressed_size {
            return Err(IdmlError::LimitExceeded {
                what: "entry read size mismatch",
                limit: entry.uncompressed_size,
                actual: u64::try_from(data.len()).unwrap_or(u64::MAX),
            });
        }

        Ok(data)
    }

    /// Reads and validates the package root `designmap.xml`.
    pub fn read_designmap(&mut self) -> Result<DesignMap> {
        let path = IdmlPath::new("designmap.xml")?;
        let bytes = self.read_entry(&path)?;
        let xml = std::str::from_utf8(&bytes)?;
        let design_map = DesignMap::from_xml(xml)?;
        self.validate_designmap_entries(&design_map)?;
        Ok(design_map)
    }

    fn from_archive(mut archive: ZipArchive<R>, limits: ArchiveLimits) -> Result<Self> {
        enforce_entry_count(archive.len(), limits.max_entries)?;

        let mut entries = IndexMap::with_capacity(archive.len());
        let mut total_uncompressed = 0u64;

        for index in 0..archive.len() {
            let file = archive.by_index(index)?;
            let path = IdmlPath::new(file.name().to_owned())?;
            let uncompressed_size = file.size();
            enforce_entry_size(uncompressed_size, limits.max_entry_uncompressed_size)?;
            total_uncompressed = total_uncompressed.checked_add(uncompressed_size).ok_or(
                IdmlError::LimitExceeded {
                    what: "archive total uncompressed size",
                    limit: limits.max_total_uncompressed_size,
                    actual: u64::MAX,
                },
            )?;
            if total_uncompressed > limits.max_total_uncompressed_size {
                return Err(IdmlError::LimitExceeded {
                    what: "archive total uncompressed size",
                    limit: limits.max_total_uncompressed_size,
                    actual: total_uncompressed,
                });
            }

            let entry = ArchiveEntry {
                path: path.clone(),
                compressed_size: file.compressed_size(),
                uncompressed_size,
                compression: file.compression(),
            };
            if entries.insert(path.clone(), entry).is_some() {
                return Err(IdmlError::DuplicateArchiveEntry(path.to_string()));
            }
        }

        Ok(Self {
            archive,
            entries,
            limits,
        })
    }

    fn validate_designmap_entries(&self, design_map: &DesignMap) -> Result<()> {
        for path in design_map
            .spread_srcs
            .values()
            .chain(design_map.story_srcs.values())
            .chain(design_map.master_spread_srcs.values())
            .chain(design_map.other_package_srcs.values().flatten())
        {
            if !self.contains(path) {
                return Err(IdmlError::MissingArchiveEntry(path.to_string()));
            }
        }
        Ok(())
    }
}

fn enforce_entry_count(actual: usize, limit: usize) -> Result<()> {
    if actual > limit {
        return Err(IdmlError::LimitExceeded {
            what: "archive entry count",
            limit: limit as u64,
            actual: actual as u64,
        });
    }
    Ok(())
}

fn enforce_entry_size(actual: u64, limit: u64) -> Result<()> {
    if actual > limit {
        return Err(IdmlError::LimitExceeded {
            what: "entry uncompressed size",
            limit,
            actual,
        });
    }
    Ok(())
}

fn validate_archive_path(path: &str) -> Result<()> {
    let reject = |reason| IdmlError::InvalidArchivePath {
        path: path.to_owned(),
        reason,
    };

    if path.is_empty() {
        return Err(reject("empty path"));
    }
    if path.as_bytes().contains(&0) {
        return Err(reject("NUL byte"));
    }
    if path.starts_with('/') {
        return Err(reject("absolute path"));
    }
    if path.contains('\\') {
        return Err(reject("backslash separator"));
    }
    if path.contains(':') {
        return Err(reject("drive or scheme separator"));
    }
    if path.ends_with('/') {
        return Err(reject("directory entry"));
    }

    for component in path.split('/') {
        if component.is_empty() {
            return Err(reject("empty path component"));
        }
        if component == "." || component == ".." {
            return Err(reject("relative path component"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ArchiveLimits, IdmlPackage, IdmlPath};
    use crate::IdmlError;
    use std::io::{Cursor, Write};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    #[test]
    fn rejects_dangerous_archive_paths() {
        for path in [
            "",
            "/designmap.xml",
            "../designmap.xml",
            "Stories/../Story.xml",
            "Stories\\Story.xml",
            "C:/Story.xml",
            "Stories/",
            "Stories//Story.xml",
            "bad\0name",
        ] {
            assert!(IdmlPath::new(path).is_err(), "{path:?} should fail");
        }
    }

    #[test]
    fn inventories_and_reads_valid_entries() {
        let zip = make_zip(&[("mimetype", b"application/vnd.adobe.indesign-idml-package")]);
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();
        let path = IdmlPath::new("mimetype").unwrap();

        assert!(package.contains(&path));
        assert_eq!(package.entries().len(), 1);
        assert_eq!(
            package.read_entry(&path).unwrap(),
            b"application/vnd.adobe.indesign-idml-package"
        );
    }

    #[test]
    fn reads_designmap_and_validates_referenced_entries() {
        let designmap = br#"<Document Self="d1">
  <idPkg:Spread src="Spreads/Spread_u1.xml" />
  <idPkg:Story src="Stories/Story_u2.xml" />
</Document>"#;
        let zip = make_zip(&[
            ("mimetype", b"application/vnd.adobe.indesign-idml-package"),
            ("designmap.xml", designmap),
            ("Spreads/Spread_u1.xml", b"<Spread />"),
            ("Stories/Story_u2.xml", b"<Story />"),
        ]);
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let design_map = package.read_designmap().unwrap();

        assert_eq!(design_map.id, "d1");
        assert!(design_map.spread_srcs.contains_key("u1"));
        assert!(design_map.story_srcs.contains_key("u2"));
    }

    #[test]
    fn read_designmap_rejects_missing_referenced_entries() {
        let designmap = br#"<Document><idPkg:Story src="Stories/Story_u2.xml" /></Document>"#;
        let zip = make_zip(&[("designmap.xml", designmap)]);
        let mut package = IdmlPackage::new(Cursor::new(zip)).unwrap();

        let err = package.read_designmap().unwrap_err();

        assert!(
            matches!(err, IdmlError::MissingArchiveEntry(path) if path == "Stories/Story_u2.xml")
        );
    }

    #[test]
    fn enforces_entry_count_limit() {
        let zip = make_zip(&[("mimetype", b"idml")]);
        let err = IdmlPackage::with_limits(
            Cursor::new(zip),
            ArchiveLimits {
                max_entries: 0,
                ..ArchiveLimits::default()
            },
        )
        .unwrap_err();

        assert!(
            matches!(err, IdmlError::LimitExceeded { what, .. } if what == "archive entry count")
        );
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
}
