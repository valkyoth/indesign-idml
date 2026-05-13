#![no_main]

use indesign_idml::model::designmap::DesignMap;
use indesign_idml::model::resources::ResourceInventory;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = core::str::from_utf8(data) else {
        return;
    };

    if let Ok(design_map) = DesignMap::from_xml(xml) {
        let _ = design_map.validate();
        let _ = ResourceInventory::from_designmap(&design_map);
    }
});
