#![no_main]

use indesign_idml::encoding::Base64Mode;
use indesign_idml::model::resources::{
    EmbeddedAssetParseOptions, extract_embedded_assets_with_options,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = core::str::from_utf8(data) else {
        return;
    };

    let _ = extract_embedded_assets_with_options(
        xml,
        EmbeddedAssetParseOptions {
            base64_mode: Base64Mode::Strict,
            max_encoded_bytes: 16 * 1024,
            max_decoded_bytes: 16 * 1024,
        },
    );
});
