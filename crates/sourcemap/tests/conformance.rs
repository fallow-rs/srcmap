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
    let json = r#"{"version":3,"sources":["a.js","b.js"],"sourcesContent":[null,"code"],"names":[],"mappings":"AAAA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.sources_content[0], None);
    assert_eq!(sm.sources_content[1], Some("code".to_string()));
}

#[test]
fn conformance_source_root() {
    let json =
        r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
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

// ── Validation tests (ECMA-426 spec conformance) ────────────────

#[test]
fn conformance_nested_index_map_rejected() {
    // Section maps must not themselves be indexed maps
    let json = r#"{
        "version": 3,
        "sections": [
            {
                "offset": {"line": 0, "column": 0},
                "map": {
                    "version": 3,
                    "sections": [
                        {
                            "offset": {"line": 0, "column": 0},
                            "map": {"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}
                        }
                    ]
                }
            }
        ]
    }"#;
    let result = SourceMap::from_json(json);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("nested") || err.contains("indexed"));
}

#[test]
fn conformance_sections_must_be_ordered() {
    // Sections must be in ascending (line, column) order
    let json = r#"{
        "version": 3,
        "sections": [
            {
                "offset": {"line": 1, "column": 0},
                "map": {"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}
            },
            {
                "offset": {"line": 0, "column": 0},
                "map": {"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}
            }
        ]
    }"#;
    let result = SourceMap::from_json(json);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("order"));
}

#[test]
fn conformance_sections_same_offset_rejected() {
    // Two sections at the same offset is not allowed
    let json = r#"{
        "version": 3,
        "sections": [
            {
                "offset": {"line": 0, "column": 0},
                "map": {"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}
            },
            {
                "offset": {"line": 0, "column": 0},
                "map": {"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}
            }
        ]
    }"#;
    assert!(SourceMap::from_json(json).is_err());
}

#[test]
fn conformance_ignore_list_explicit_empty_overrides_legacy() {
    // Explicit empty ignoreList: [] should NOT fall through to x_google_ignoreList
    let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA","ignoreList":[],"x_google_ignoreList":[1]}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert!(sm.ignore_list.is_empty());
}

#[test]
fn conformance_ignore_list_absent_falls_back_to_legacy() {
    // Missing ignoreList should fall back to x_google_ignoreList
    let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA","x_google_ignoreList":[1]}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.ignore_list, vec![1]);
}

#[test]
fn conformance_two_field_segment_rejected() {
    // A segment with 2 fields is invalid per ECMA-426
    let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AC"}"#;
    assert!(SourceMap::from_json(json).is_err());
}

#[test]
fn conformance_three_field_segment_rejected() {
    // A segment with 3 fields is invalid per ECMA-426
    let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACA"}"#;
    assert!(SourceMap::from_json(json).is_err());
}

#[test]
fn conformance_source_root_roundtrip() {
    // sourceRoot must not be double-applied on parse-serialize-parse
    let json =
        r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
    let sm = SourceMap::from_json(json).unwrap();
    assert_eq!(sm.source(0), "src/a.js");

    let output = sm.to_json();
    let sm2 = SourceMap::from_json(&output).unwrap();
    assert_eq!(sm2.source(0), "src/a.js"); // Not "src/src/a.js"
}

#[test]
fn conformance_scopes_roundtrip_via_to_json() {
    // Build a source map with scopes via the generator, then verify roundtrip through to_json
    use srcmap_scopes::{GeneratedRange, OriginalScope, Position, ScopeInfo};

    let scopes_info = ScopeInfo {
        scopes: vec![Some(OriginalScope {
            start: Position { line: 0, column: 0 },
            end: Position {
                line: 10,
                column: 0,
            },
            name: Some("global".to_string()),
            kind: None,
            is_stack_frame: false,
            variables: vec!["x".to_string()],
            children: vec![],
        })],
        ranges: vec![GeneratedRange {
            start: Position { line: 0, column: 0 },
            end: Position { line: 5, column: 0 },
            is_stack_frame: false,
            is_hidden: false,
            definition: Some(0),
            call_site: None,
            bindings: vec![],
            children: vec![],
        }],
    };

    // Encode scopes to a VLQ string
    let mut names = vec!["global".to_string(), "x".to_string()];
    let scopes_str = srcmap_scopes::encode_scopes(&scopes_info, &mut names);

    // Build JSON with the scopes string
    let names_json: Vec<String> = names.iter().map(|n| format!(r#""{n}""#)).collect();
    let json = format!(
        r#"{{"version":3,"sources":["a.js"],"names":[{}],"mappings":"AAAA","scopes":"{}"}}"#,
        names_json.join(","),
        scopes_str
    );

    let sm = SourceMap::from_json(&json).unwrap();
    assert!(sm.scopes.is_some());

    let output = sm.to_json();
    assert!(
        output.contains(r#""scopes":"#),
        "to_json must emit scopes field"
    );

    let sm2 = SourceMap::from_json(&output).unwrap();
    assert!(sm2.scopes.is_some());

    let scopes2 = sm2.scopes.unwrap();
    assert_eq!(scopes2.scopes.len(), 1);
    let scope = scopes2.scopes[0].as_ref().unwrap();
    assert_eq!(scope.name.as_deref(), Some("global"));
    assert_eq!(scopes2.ranges.len(), 1);
    assert_eq!(scopes2.ranges[0].definition, Some(0));
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
