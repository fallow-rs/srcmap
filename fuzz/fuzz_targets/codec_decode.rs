#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    let Ok(mappings) = srcmap_codec::decode(input) else { return };
    let encoded = srcmap_codec::encode(&mappings);
    // Zero lines and one empty line both encode to "", so the format cannot
    // keep that difference. Any other mappings must survive a round trip.
    if encoded.is_empty() {
        assert!(mappings.iter().all(Vec::is_empty), "non-empty mappings encode to \"\"");
        return;
    }
    match srcmap_codec::decode(&encoded) {
        Ok(round_trip) => {
            assert_eq!(round_trip, mappings, "encode output decodes to other mappings")
        }
        Err(err) => panic!("encode output {encoded:?} does not decode: {err:?}"),
    }
});
