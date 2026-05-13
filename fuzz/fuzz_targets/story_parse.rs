#![no_main]

use indesign_idml::model::story::{Story, StoryParseOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = core::str::from_utf8(data) else {
        return;
    };

    let _ = Story::from_xml_with_options(
        xml,
        StoryParseOptions {
            max_text_bytes: 16 * 1024,
        },
    );
});
