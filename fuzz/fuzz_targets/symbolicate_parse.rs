#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    let _ = srcmap_symbolicate::parse_stack_trace_full(input);
    let _ = srcmap_symbolicate::parse_stack_trace(input);
});
