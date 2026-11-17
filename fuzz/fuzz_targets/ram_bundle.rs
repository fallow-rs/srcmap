#![no_main]

use libfuzzer_sys::fuzz_target;
use srcmap_ram_bundle::IndexedRamBundle;

fuzz_target!(|data: &[u8]| {
    let Ok(bundle) = IndexedRamBundle::from_bytes(data) else { return };
    let _ = bundle.module_count();
    for module in bundle.modules() {
        let _ = bundle.get_module(module.id);
    }
    let _ = bundle.get_module(0);
    let _ = bundle.get_module(u32::MAX);
});
