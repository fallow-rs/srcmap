#![no_main]

use libfuzzer_sys::fuzz_target;

// The input is the scopes string, a newline, then the comma-separated names.
fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    let (scopes, names_part) = input.split_once('\n').unwrap_or((input, ""));
    let names: Vec<String> = if names_part.is_empty() {
        Vec::new()
    } else {
        names_part.split(',').map(str::to_owned).collect()
    };
    let Ok(info) = srcmap_scopes::decode_scopes(scopes, &names, names.len()) else { return };
    let mut encode_names = names.clone();
    let encoded = srcmap_scopes::encode_scopes(&info, &mut encode_names);
    if let Err(err) = srcmap_scopes::decode_scopes(&encoded, &encode_names, names.len()) {
        panic!("encode output {encoded:?} does not decode: {err:?}");
    }
});
