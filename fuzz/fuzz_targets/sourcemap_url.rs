#![no_main]

use libfuzzer_sys::fuzz_target;
use srcmap_sourcemap::{SourceMap, SourceMappingUrl, parse_source_mapping_url};

// The input is JavaScript source text, and also a candidate data URL.
fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    if let Some(SourceMappingUrl::Inline(json)) = parse_source_mapping_url(input) {
        let _ = SourceMap::from_json(&json);
    }
    let _ = SourceMap::from_data_url(input);
});
