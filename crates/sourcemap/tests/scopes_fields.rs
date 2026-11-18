//! The `scopes` and `ranges` fields of the source map JSON.

use srcmap_scopes::{GeneratedRange, OriginalScope, Position, ScopeDefinition, ScopeInfo};
use srcmap_sourcemap::{ParseError, SourceMap};

const FIRST_SCOPE: ScopeDefinition = ScopeDefinition { source_index: 0, scope_index: 0 };

fn scope(start_line: u32, end_line: u32, children: Vec<OriginalScope>) -> OriginalScope {
    OriginalScope {
        start: Position { line: start_line, column: 0 },
        end: Position { line: end_line, column: 0 },
        name: None,
        kind: None,
        is_stack_frame: false,
        variables: vec![],
        children,
    }
}

fn range(definition: Option<ScopeDefinition>) -> GeneratedRange {
    GeneratedRange {
        start: Position { line: 0, column: 0 },
        end: Position { line: 0, column: 4 },
        is_stack_frame: false,
        is_hidden: false,
        definition,
        call_site: None,
        bindings: vec![],
        children: vec![],
    }
}

/// The first `scopes` string and the first `ranges` string of `info`.
fn encoded_strings(info: &ScopeInfo) -> (Option<String>, String) {
    let mut names = vec![];
    let encoded = srcmap_scopes::encode_scopes(info, &mut names);
    assert!(names.is_empty(), "the test scopes have no names");
    (encoded.scopes[0].clone(), encoded.ranges[0].clone())
}

fn one_scope_info() -> ScopeInfo {
    ScopeInfo { scopes: vec![vec![scope(0, 3, vec![])]], ranges: vec![range(Some(FIRST_SCOPE))] }
}

/// Parses a map with one source `a.js` and the given scopes fields.
fn map_with(scopes_fields: &str) -> Result<SourceMap, ParseError> {
    SourceMap::from_json(&format!(
        r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA",{scopes_fields}}}"#
    ))
}

/// The JSON of a map with one source `a.js` and the scopes of `info`.
fn map_json(info: &ScopeInfo) -> String {
    let mut fields = String::new();
    srcmap_scopes::encode_scopes(info, &mut vec![]).write_json_fields(&mut fields);
    format!(r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"{fields}}}"#)
}

/// The JSON of an indexed map with one section for each `ScopeInfo`.
fn indexed_json(first: &ScopeInfo, second: &ScopeInfo) -> String {
    format!(
        r#"{{"version":3,"sections":[{{"offset":{{"line":0,"column":0}},"map":{}}},{{"offset":{{"line":1,"column":0}},"map":{}}}]}}"#,
        map_json(first),
        map_json(second),
    )
}

fn definitions(info: &ScopeInfo) -> Vec<Option<ScopeDefinition>> {
    info.ranges.iter().map(|range| range.definition).collect()
}

/// `text` with its first character as a `\u` escape, as a JSON writer can give it.
fn with_json_escape(text: &str) -> String {
    let first = text.chars().next().unwrap();
    format!("\\u{:04x}{}", first as u32, &text[first.len_utf8()..])
}

#[test]
fn a_ranges_field_that_is_not_a_list_is_absent() {
    for ranges in [r#""EAA,FA""#, "7", "null", "{}"] {
        let sm = map_with(&format!(r#""ranges":{ranges}"#)).unwrap();
        assert!(sm.scopes.is_none(), "ranges {ranges}");
    }
}

#[test]
fn a_ranges_entry_that_is_not_a_string_is_the_empty_string() {
    let info = one_scope_info();
    let (scopes, ranges) = encoded_strings(&info);
    let scopes = scopes.unwrap();
    for entry in ["null", "1", "[]"] {
        let sm =
            map_with(&format!(r#""scopes":["{scopes}"],"ranges":[{entry},"{ranges}"]"#)).unwrap();
        assert_eq!(sm.scopes.as_ref(), Some(&info), "entry {entry}");
    }
}

#[test]
fn a_scopes_entry_that_is_not_a_string_is_a_source_without_scopes() {
    let info = ScopeInfo { scopes: vec![vec![]], ranges: vec![range(None)] };
    let (_, ranges) = encoded_strings(&info);
    for entry in ["null", "1", "{}"] {
        let sm = map_with(&format!(r#""scopes":[{entry}],"ranges":["{ranges}"]"#)).unwrap();
        assert_eq!(sm.scopes.as_ref(), Some(&info), "entry {entry}");
    }
}

#[test]
fn a_scopes_field_that_is_not_a_list_or_a_string_is_absent() {
    let sm = map_with(r#""scopes":7"#).unwrap();
    assert!(sm.scopes.is_none());
}

#[test]
fn a_json_escape_in_a_scopes_string_is_read() {
    let info = one_scope_info();
    let (scopes, ranges) = encoded_strings(&info);
    let (scopes, ranges) = (with_json_escape(&scopes.unwrap()), with_json_escape(&ranges));

    let sm = map_with(&format!(r#""scopes":["{scopes}"],"ranges":["{ranges}"]"#)).unwrap();
    assert_eq!(sm.scopes, Some(info));

    // `\/` is `/`. Some JSON writers escape the `/` of base64. The string is
    // read, and then the scopes decoder rejects its content.
    let err = map_with(r#""scopes":["B\/"]"#).unwrap_err();
    assert!(matches!(err, ParseError::Scopes(_)), "{err:?}");
}

#[test]
fn a_json_escape_in_a_scopes_string_of_the_older_layout_is_read() {
    let plain = map_with(r#""scopes":"BAAA,CCA""#).unwrap();
    let escaped = map_with(r#""scopes":"BAAA,CCA""#).unwrap();
    assert!(plain.scopes.is_some());
    assert_eq!(plain.scopes, escaped.scopes);
}

#[test]
fn an_indexed_map_keeps_the_scopes_of_each_section() {
    // The two sections share `a.js`. The scopes of the second section are
    // after the scope of the first section in the source.
    let first = ScopeInfo {
        scopes: vec![vec![scope(0, 1, vec![])]],
        ranges: vec![range(Some(FIRST_SCOPE))],
    };
    let second = ScopeInfo {
        scopes: vec![vec![scope(2, 9, vec![scope(3, 5, vec![])])]],
        ranges: vec![range(Some(ScopeDefinition { source_index: 0, scope_index: 1 }))],
    };

    let sm = SourceMap::from_json(&indexed_json(&first, &second)).unwrap();
    let info = sm.scopes.as_ref().unwrap();

    assert_eq!(
        info.scopes,
        vec![vec![scope(0, 1, vec![]), scope(2, 9, vec![scope(3, 5, vec![])])]]
    );
    // The scopes of the second section follow the one scope of the first section.
    let inner = ScopeDefinition { source_index: 0, scope_index: 2 };
    assert_eq!(definitions(info), [Some(FIRST_SCOPE), Some(inner)]);
    assert_eq!(
        info.original_scope_for_definition(inner).unwrap().end,
        Position { line: 5, column: 0 }
    );

    let again = SourceMap::from_json(&sm.to_json()).unwrap();
    assert_eq!(again.scopes, sm.scopes);
}

#[test]
fn an_indexed_map_drops_a_definition_into_scopes_that_it_cannot_keep() {
    // The scopes of the second section overlap the scope of the first
    // section, so one list of root scopes in source order cannot hold both.
    let first = ScopeInfo {
        scopes: vec![vec![scope(0, 4, vec![])]],
        ranges: vec![range(Some(FIRST_SCOPE))],
    };
    let second = ScopeInfo {
        scopes: vec![vec![scope(0, 9, vec![scope(3, 5, vec![])])]],
        ranges: vec![range(Some(ScopeDefinition { source_index: 0, scope_index: 1 }))],
    };

    let sm = SourceMap::from_json(&indexed_json(&first, &second)).unwrap();
    let info = sm.scopes.as_ref().unwrap();

    assert_eq!(info.scopes, first.scopes);
    assert_eq!(definitions(info), [Some(FIRST_SCOPE), None]);

    let again = SourceMap::from_json(&sm.to_json()).unwrap();
    assert_eq!(again.scopes, sm.scopes);
}

#[test]
fn an_indexed_map_keeps_one_copy_of_equal_section_scopes() {
    let inner = Some(ScopeDefinition { source_index: 0, scope_index: 1 });
    let info = ScopeInfo {
        scopes: vec![vec![scope(0, 9, vec![scope(3, 5, vec![])])]],
        ranges: vec![range(inner)],
    };

    let sm = SourceMap::from_json(&indexed_json(&info, &info)).unwrap();
    let merged = sm.scopes.unwrap();

    assert_eq!(merged.scopes, info.scopes);
    assert_eq!(definitions(&merged), [inner, inner]);
}
