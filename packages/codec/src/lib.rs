use napi_derive::napi;

/// Decode a VLQ-encoded source map mappings string.
///
/// Returns an array of lines, each containing an array of segments.
/// Each segment is an array of 1, 4, or 5 numbers.
///
/// Compatible with `@jridgewell/sourcemap-codec` decode().
#[napi]
pub fn decode(mappings: String) -> napi::Result<Vec<Vec<Vec<i64>>>> {
    srcmap_codec::decode(&mappings).map_err(|e| napi::Error::from_reason(e.to_string()))
}

/// Encode decoded source map mappings back into a VLQ string.
///
/// Takes an array of lines, each containing an array of segments.
/// Each segment should be an array of 1, 4, or 5 numbers.
///
/// Compatible with `@jridgewell/sourcemap-codec` encode().
#[napi]
pub fn encode(mappings: Vec<Vec<Vec<i64>>>) -> String {
    srcmap_codec::encode(&mappings)
}
