#![no_main]

use libfuzzer_sys::fuzz_target;
use srcmap_sourcemap::LazySourceMap;

const COLUMNS: [u32; 4] = [0, 1, 10, u32::MAX];

fn look_up_grid(sm: &LazySourceMap) {
    for line in 0..4 {
        for column in COLUMNS {
            let _ = sm.original_position_for(line, column);
        }
    }
    let _ = sm.original_position_for(u32::MAX, 0);
    // Decode up to the last line, then go back to line 0.
    let last_line = u32::try_from(sm.line_count().saturating_sub(1)).unwrap_or(u32::MAX);
    let _ = sm.original_position_for(last_line, 0);
    let _ = sm.original_position_for(0, 0);
}

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    if let Ok(fast) = LazySourceMap::from_json_fast(input) {
        look_up_grid(&fast);
    }
    if let Ok(lazy) = LazySourceMap::from_json(input) {
        look_up_grid(&lazy);
        let _ = lazy.into_sourcemap();
    }
});
