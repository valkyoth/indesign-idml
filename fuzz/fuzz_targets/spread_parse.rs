#![no_main]

use indesign_idml::model::spread::{Spread, SpreadParseOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = core::str::from_utf8(data) else {
        return;
    };

    let _ = Spread::from_xml_with_options(
        xml,
        SpreadParseOptions {
            max_text_frames: 1024,
            max_xml_depth: 256,
        },
    );
});
