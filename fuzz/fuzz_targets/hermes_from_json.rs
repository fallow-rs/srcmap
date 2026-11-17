#![no_main]

use libfuzzer_sys::fuzz_target;
use srcmap_hermes::SourceMapHermes;

const COLUMNS: [u32; 4] = [0, 1, 10, u32::MAX];

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    let Ok(sm) = SourceMapHermes::from_json(input) else { return };
    for line in 0..4 {
        for column in COLUMNS {
            let _ = sm.get_original_function_name(line, column);
            let _ = sm.get_scope_for_token(line, column);
        }
    }
    let _ = sm.to_json();
});
