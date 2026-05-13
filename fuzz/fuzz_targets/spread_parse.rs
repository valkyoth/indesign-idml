#![no_main]

use indesign_idml::model::spread::Spread;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = core::str::from_utf8(data) else {
        return;
    };

    let _ = Spread::from_xml(xml);
});
