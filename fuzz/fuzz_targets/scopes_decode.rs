#![no_main]

use libfuzzer_sys::fuzz_target;
use srcmap_scopes::{ScopeInfo, decode_legacy_scopes, decode_scopes, encode_scopes};

// The input has three lines:
// 1. the entries of `scopes`, with `;` between them
// 2. the entries of `ranges`, with `;` between them
// 3. the names, with `,` between them
//
// An input with two lines has no `ranges` line. The first line is also the
// one `scopes` string of the older layout.
fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    let mut lines = input.splitn(3, '\n');
    let scopes_line = lines.next().unwrap_or("");
    let second = lines.next().unwrap_or("");
    let (ranges_line, names_line) = match lines.next() {
        Some(third) => (second, third),
        None => ("", second),
    };

    let names: Vec<String> = if names_line.is_empty() {
        Vec::new()
    } else {
        names_line.split(',').map(str::to_owned).collect()
    };
    let scopes: Vec<Option<&str>> = scopes_line.split(';').map(Some).collect();
    let ranges: Vec<&str> =
        if ranges_line.is_empty() { Vec::new() } else { ranges_line.split(';').collect() };

    if let Ok(info) = decode_scopes(&scopes, &ranges, &names, scopes.len()) {
        check_roundtrip(&info, &names);
    }
    if let Ok(info) = decode_legacy_scopes(scopes_line, &names, names.len()) {
        check_roundtrip(&info, &names);
    }
});

/// The encoder output of decoded data must decode to the same data.
fn check_roundtrip(info: &ScopeInfo, names: &[String]) {
    let mut encode_names = names.to_vec();
    let encoded = encode_scopes(info, &mut encode_names);
    match decode_scopes(&encoded.scopes, &encoded.ranges, &encode_names, info.scopes.len()) {
        Ok(decoded) => assert_eq!(&decoded, info, "encode output {encoded:?} decodes to other data"),
        Err(err) => panic!("encode output {encoded:?} does not decode: {err:?}"),
    }
}
