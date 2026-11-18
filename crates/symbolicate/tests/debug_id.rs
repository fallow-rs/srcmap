//! Tests for source map lookup by debug ID.

use srcmap_sourcemap::SourceMap;
use srcmap_symbolicate::{DebugIdIndex, symbolicate_with_debug_ids};

const ID: &str = "85314830-023f-4cf1-a267-535f4e37bb17";
const OTHER_ID: &str = "00000000-0000-4000-8000-000000000000";
const STACK: &str = "Error: test\n    at foo (https://cdn.example.com/app.min.js:1:1)";

fn map(source: &str, debug_id: Option<&str>) -> SourceMap {
    let debug_id = debug_id.map(|id| format!(r#","debugId":"{id}""#)).unwrap_or_default();
    let json =
        format!(r#"{{"version":3,"sources":["{source}"],"names":[],"mappings":"AAAA"{debug_id}}}"#);
    SourceMap::from_json(&json).unwrap()
}

fn index_of(map: SourceMap) -> DebugIdIndex {
    let mut index = DebugIdIndex::new();
    index.insert(map);
    index
}

#[test]
fn index_finds_a_map_by_debug_id() {
    let mut index = DebugIdIndex::new();
    assert!(index.is_empty());
    assert!(index.insert(map("src/app.ts", Some(ID))));
    assert_eq!(index.len(), 1);

    assert_eq!(index.get(ID).unwrap().source(0), "src/app.ts");
    assert!(index.get(OTHER_ID).is_none());
}

#[test]
fn index_ignores_the_case_of_hex_digits() {
    let mut index = DebugIdIndex::new();
    index.insert(map("src/app.ts", Some(&ID.to_ascii_uppercase())));
    assert!(index.get(ID).is_some());
    assert!(index.get(&ID.to_ascii_uppercase()).is_some());
}

#[test]
fn index_rejects_a_map_without_debug_id() {
    let mut index = DebugIdIndex::new();
    assert!(!index.insert(map("src/app.ts", None)));
    assert!(index.is_empty());
}

#[test]
fn index_accepts_an_explicit_debug_id() {
    let mut index = DebugIdIndex::new();
    index.insert_with_id(ID, map("src/app.ts", None));
    assert_eq!(index.get(ID).unwrap().source(0), "src/app.ts");
}

#[test]
fn index_collects_from_an_iterator() {
    let index: DebugIdIndex =
        [map("a.ts", Some(ID)), map("b.ts", Some(OTHER_ID)), map("c.ts", None)]
            .into_iter()
            .collect();
    assert_eq!(index.len(), 2);
    assert_eq!(index.get(OTHER_ID).unwrap().source(0), "b.ts");
}

#[test]
fn symbolicate_uses_the_debug_id_of_the_file() {
    let index = index_of(map("src/app.ts", Some(ID)));

    let result = symbolicate_with_debug_ids(
        STACK,
        |file| (file == "https://cdn.example.com/app.min.js").then(|| ID.to_string()),
        |debug_id| index.get(debug_id).cloned(),
        |_| None,
    );

    assert!(result.frames[0].symbolicated);
    assert_eq!(result.frames[0].file, "src/app.ts");
}

#[test]
fn symbolicate_prefers_the_debug_id_map_to_the_file_map() {
    let index = index_of(map("by-id.ts", Some(ID)));

    let result = symbolicate_with_debug_ids(
        STACK,
        |_| Some(ID.to_string()),
        |debug_id| index.get(debug_id).cloned(),
        |_| Some(map("by-file.ts", None)),
    );

    assert_eq!(result.frames[0].file, "by-id.ts");
}

#[test]
fn symbolicate_uses_the_file_loader_without_a_debug_id() {
    let result = symbolicate_with_debug_ids(
        STACK,
        |_| None,
        |_| panic!("no debug ID to resolve"),
        |_| Some(map("by-file.ts", None)),
    );

    assert_eq!(result.frames[0].file, "by-file.ts");
}

#[test]
fn symbolicate_uses_the_file_loader_when_no_map_has_the_debug_id() {
    let index = DebugIdIndex::new();

    let result = symbolicate_with_debug_ids(
        STACK,
        |_| Some(ID.to_string()),
        |debug_id| index.get(debug_id).cloned(),
        |_| Some(map("by-file.ts", None)),
    );

    assert_eq!(result.frames[0].file, "by-file.ts");
}

#[test]
fn symbolicate_keeps_the_frame_when_no_map_is_found() {
    let result = symbolicate_with_debug_ids(STACK, |_| Some(ID.to_string()), |_| None, |_| None);

    assert!(!result.frames[0].symbolicated);
    assert_eq!(result.frames[0].file, "https://cdn.example.com/app.min.js");
    assert_eq!(result.message.as_deref(), Some("Error: test"));
}
