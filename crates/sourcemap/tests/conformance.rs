//! Conformance tests based on tc39/source-map-tests.
//!
//! These tests validate behavior against the official ECMA-426 test expectations.
//! They cover the key behaviors that the tc39 test suite checks:
//! - Basic parsing and field preservation
//! - VLQ decoding edge cases
//! - Indexed source map handling
//! - Lookup behavior for various mapping patterns

use srcmap_sourcemap::SourceMap;

// ── Basic parsing tests ──────────────────────────────────────────

#[test]
fn conformance_version_3_required() {
    let json = r#"{"version":2,"sources":[],"names":[],"mappings":""}"#;
    assert!(SourceMap::from_json(json).is_err());
}

#[test]
fn conformance_minimal_valid_map() {
    let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.mapping_count(), 0);
    assert_eq!(sm.sources.len(), 0);
}

#[test]
fn conformance_single_segment() {
    // AAAA = generated col 0, source 0, line 0, col 0
    let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    let loc = sm.original_position_for(0, 0).unwrap();
    assert_eq!(sm.source(loc.source), "a.js");
    assert_eq!(loc.line, 0);
    assert_eq!(loc.column, 0);
}

#[test]
fn conformance_multiple_lines() {
    // Two lines, each with one mapping
    let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;AACA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.line_count(), 2);

    let loc0 = sm.original_position_for(0, 0).unwrap();
    assert_eq!(loc0.line, 0);

    let loc1 = sm.original_position_for(1, 0).unwrap();
    assert_eq!(loc1.line, 1);
}

#[test]
fn conformance_empty_lines() {
    // Three semicolons = 4 lines, mappings only on line 0 and line 3
    let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;;;AACA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.line_count(), 4);
    assert!(sm.original_position_for(1, 0).is_none());
    assert!(sm.original_position_for(2, 0).is_none());
    assert!(sm.original_position_for(3, 0).is_some());
}

#[test]
fn conformance_names() {
    // AAAAA = col 0, source 0, line 0, col 0, name 0
    let json = r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AAAAA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    let loc = sm.original_position_for(0, 0).unwrap();
    assert_eq!(loc.name, Some(0));
    assert_eq!(sm.name(0), "foo");
}

#[test]
fn conformance_source_content() {
    let json = r#"{"version":3,"sources":["a.js"],"sourcesContent":["var a = 1;"],"names":[],"mappings":"AAAA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.sources_content[0], Some("var a = 1;".to_string()));
}

#[test]
fn conformance_null_source_content() {
    let json =
        r#"{"version":3,"sources":["a.js","b.js"],"sourcesContent":[null,"code"],"names":[],"mappings":"AAAA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.sources_content[0], None);
    assert_eq!(sm.sources_content[1], Some("code".to_string()));
}

#[test]
fn conformance_source_root() {
    let json = r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.source(0), "src/a.js");
}

#[test]
fn conformance_ignore_list() {
    let json = r#"{"version":3,"sources":["app.js","vendor.js"],"names":[],"mappings":"AAAA","ignoreList":[1]}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.ignore_list, vec![1]);
}

#[test]
fn conformance_debug_id() {
    let json = r#"{"version":3,"sources":[],"names":[],"mappings":"","debugId":"12345678-1234-1234-1234-123456789abc"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(
        sm.debug_id.as_deref(),
        Some("12345678-1234-1234-1234-123456789abc")
    );
}

// ── Indexed source maps ──────────────────────────────────────────

#[test]
fn conformance_indexed_basic() {
    let json = r#"{
        "version": 3,
        "sections": [
            {
                "offset": {"line": 0, "column": 0},
                "map": {"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}
            },
            {
                "offset": {"line": 1, "column": 0},
                "map": {"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}
            }
        ]
    }"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.sources.len(), 2);

    let loc0 = sm.original_position_for(0, 0).unwrap();
    assert_eq!(sm.source(loc0.source), "a.js");

    let loc1 = sm.original_position_for(1, 0).unwrap();
    assert_eq!(sm.source(loc1.source), "b.js");
}

#[test]
fn conformance_indexed_shared_sources() {
    let json = r#"{
        "version": 3,
        "sections": [
            {
                "offset": {"line": 0, "column": 0},
                "map": {"version":3,"sources":["shared.js"],"names":[],"mappings":"AAAA"}
            },
            {
                "offset": {"line": 1, "column": 0},
                "map": {"version":3,"sources":["shared.js"],"names":[],"mappings":"AAAA"}
            }
        ]
    }"#;
    let sm = SourceMap::from_json(json).unwrap();
    // shared.js should be deduplicated
    assert_eq!(sm.sources.len(), 1);
    assert_eq!(sm.sources[0], "shared.js");
}

// ── Lookup behavior ──────────────────────────────────────────────

#[test]
fn conformance_column_snapping() {
    // Mappings at columns 0, 10, 20
    let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,UAAU,UAAU"}"#;
    let sm = SourceMap::from_json(json).unwrap();

    // Column 5 should snap to mapping at column 0
    let loc = sm.original_position_for(0, 5).unwrap();
    assert_eq!(loc.column, 0);

    // Column 15 should snap to mapping at column 10
    let loc = sm.original_position_for(0, 15).unwrap();
    assert_eq!(loc.column, 10);
}

#[test]
fn conformance_unmapped_segment() {
    // Single-field segment (generated column only, no source)
    let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    // Unmapped segment should return None
    assert!(sm.original_position_for(0, 0).is_none());
}

// ── Roundtrip tests ──────────────────────────────────────────────

#[test]
fn conformance_roundtrip() {
    let json = r#"{"version":3,"file":"out.js","sourceRoot":"src/","sources":["a.js","b.js"],"sourcesContent":["var a;",null],"names":["foo","bar"],"mappings":"AAAAA,IAAAC;ACAA","ignoreList":[1]}"#;
    let sm = SourceMap::from_json(json).unwrap();
    let output = sm.to_json();
    let sm2 = SourceMap::from_json(&output).unwrap();

    assert_eq!(sm2.sources.len(), sm.sources.len());
    assert_eq!(sm2.names.len(), sm.names.len());
    assert_eq!(sm2.mapping_count(), sm.mapping_count());
    assert_eq!(sm2.ignore_list, sm.ignore_list);

    // All lookups should match
    for m in sm.all_mappings() {
        let a = sm.original_position_for(m.generated_line, m.generated_column);
        let b = sm2.original_position_for(m.generated_line, m.generated_column);
        match (a, b) {
            (Some(a), Some(b)) => {
                assert_eq!(a.source, b.source);
                assert_eq!(a.line, b.line);
                assert_eq!(a.column, b.column);
                assert_eq!(a.name, b.name);
            }
            (None, None) => {}
            _ => panic!(
                "roundtrip mismatch at {}:{}",
                m.generated_line, m.generated_column
            ),
        }
    }
}
