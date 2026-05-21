#![no_main]

use indesign_idml::model::designmap::{DesignMap, DesignMapParseOptions};
use indesign_idml::model::resources::ResourceInventory;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = core::str::from_utf8(data) else {
        return;
    };

    if let Ok(design_map) = DesignMap::from_xml_with_options(
        xml,
        DesignMapParseOptions {
            max_layers: 1024,
            max_package_refs: 1024,
            max_xml_depth: 256,
        },
    ) {
        let _ = design_map.validate();
        let _ = ResourceInventory::from_designmap(&design_map);
    }
});
