#![no_main]

use indesign_idml::model::resources::{
    ResourceItemParseOptions, ResourceKind, extract_resource_items_with_options,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = core::str::from_utf8(data) else {
        return;
    };

    for kind in [
        ResourceKind::Styles,
        ResourceKind::Swatches,
        ResourceKind::Fonts,
        ResourceKind::Links,
        ResourceKind::Graphics,
        ResourceKind::Preferences,
        ResourceKind::Tags,
    ] {
        let _ = extract_resource_items_with_options(
            xml,
            kind,
            ResourceItemParseOptions {
                max_items: 1024,
                max_item_attributes: 64,
                max_item_attribute_bytes: 64 * 1024,
                max_xml_depth: 256,
            },
        );
    }
});
