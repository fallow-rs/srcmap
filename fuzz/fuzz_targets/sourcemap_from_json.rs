#![no_main]

use libfuzzer_sys::fuzz_target;
use srcmap_sourcemap::SourceMap;

const COLUMNS: [u32; 4] = [0, 1, 10, u32::MAX];

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    // from_json_lines has its own decoder for a range of lines.
    let _ = SourceMap::from_json_lines(input, 1, 3);
    let _ = SourceMap::from_json_lines(input, 0, u32::MAX);
    let Ok(sm) = SourceMap::from_json(input) else { return };
    for line in 0..4 {
        for column in COLUMNS {
            let _ = sm.original_position_for(line, column);
        }
    }
    let _ = sm.original_position_for(u32::MAX, 0);
    for source in &sm.sources {
        let _ = sm.generated_position_for(source, 0, 0);
        let _ = sm.all_generated_positions_for(source, 0, 0);
    }
    let _ = sm.to_json();
    let _ = srcmap_sourcemap::validate_deep(&sm);
});
