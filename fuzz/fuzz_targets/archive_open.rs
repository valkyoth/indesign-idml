#![no_main]

use indesign_idml::archive::{ArchiveLimits, IdmlPackage};
use indesign_idml::model::designmap::DesignMapParseOptions;
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let limits = ArchiveLimits {
        max_entries: 128,
        max_entry_uncompressed_size: 1024 * 1024,
        max_total_uncompressed_size: 4 * 1024 * 1024,
    };

    let Ok(mut package) = IdmlPackage::with_limits(Cursor::new(data), limits) else {
        return;
    };

    let _ = package.read_designmap_with_options(DesignMapParseOptions {
        max_layers: 1024,
        max_package_refs: 1024,
        max_xml_depth: 256,
    });
});
