use super::*;

const NO_SCOPES: [Option<&str>; 0] = [];
const NO_RANGES: [&str; 0] = [];

fn pos(line: u32, column: u32) -> Position {
    Position { line, column }
}

fn def(source_index: u32, scope_index: u32) -> ScopeDefinition {
    ScopeDefinition { source_index, scope_index }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_string()).collect()
}

fn scope(start: Position, end: Position) -> OriginalScope {
    OriginalScope {
        start,
        end,
        name: None,
        kind: None,
        is_stack_frame: false,
        variables: vec![],
        children: vec![],
    }
}

fn function(name: &str, start: Position, end: Position, variables: &[&str]) -> OriginalScope {
    OriginalScope {
        name: Some(name.to_string()),
        kind: Some("function".to_string()),
        is_stack_frame: true,
        variables: names(variables),
        ..scope(start, end)
    }
}

fn range(start: Position, end: Position) -> GeneratedRange {
    GeneratedRange {
        start,
        end,
        is_stack_frame: false,
        is_hidden: false,
        definition: None,
        call_site: None,
        bindings: vec![],
        children: vec![],
    }
}

/// Decode the scopes of one source and one `ranges` string.
fn decode_one(scopes: &str, ranges: &str, names: &[String]) -> Result<ScopeInfo, ScopesError> {
    decode_scopes(&[Some(scopes)], &[ranges], names, 1)
}

/// Encode `info`, decode the result, and return the decoded value.
fn roundtrip(info: &ScopeInfo) -> ScopeInfo {
    let mut names = vec![];
    let encoded = encode_scopes(info, &mut names);
    decode_scopes(&encoded.scopes, &encoded.ranges, &names, info.scopes.len()).unwrap()
}

/// Two sources with scopes, and one source without scopes between them.
fn comprehensive_info() -> ScopeInfo {
    let module = OriginalScope {
        kind: Some("module".to_string()),
        variables: names(&["x"]),
        children: vec![
            function("f", pos(1, 2), pos(4, 1), &["a", "b"]),
            OriginalScope { kind: Some("block".to_string()), ..scope(pos(5, 0), pos(5, 9)) },
        ],
        ..scope(pos(0, 0), pos(9, 0))
    };
    let other = OriginalScope {
        children: vec![function("g", pos(1, 0), pos(2, 1), &["c"])],
        ..scope(pos(0, 0), pos(3, 0))
    };
    let inlined = GeneratedRange {
        definition: Some(def(2, 1)),
        call_site: Some(CallSite { source_index: 0, line: 7, column: 3 }),
        bindings: vec![Binding::Expression("_c".to_string())],
        ..range(pos(2, 0), pos(3, 4))
    };
    let frame = GeneratedRange {
        is_stack_frame: true,
        definition: Some(def(0, 1)),
        bindings: vec![
            Binding::SubRanges(vec![
                SubRangeBinding { expression: Some("_a".to_string()), from: pos(1, 0) },
                SubRangeBinding { expression: None, from: pos(1, 10) },
                SubRangeBinding { expression: Some("_a2".to_string()), from: pos(2, 5) },
            ]),
            Binding::Unavailable,
        ],
        children: vec![inlined],
        ..range(pos(1, 0), pos(3, 40))
    };
    let hidden =
        GeneratedRange { is_stack_frame: true, is_hidden: true, ..range(pos(4, 0), pos(4, 9)) };
    let outer = GeneratedRange {
        definition: Some(def(0, 0)),
        bindings: vec![Binding::Expression("_x".to_string())],
        children: vec![frame, hidden],
        ..range(pos(0, 0), pos(5, 0))
    };
    ScopeInfo {
        scopes: vec![vec![module, scope(pos(10, 0), pos(12, 0))], vec![], vec![other]],
        ranges: vec![outer, range(pos(6, 0), pos(6, 3))],
    }
}

// ── Layout ───────────────────────────────────────────────────────

#[test]
fn empty_input_gives_no_scopes_and_no_ranges() {
    let info = decode_scopes(&NO_SCOPES, &NO_RANGES, &[], 0).unwrap();
    assert_eq!(info, ScopeInfo { scopes: vec![], ranges: vec![] });
}

#[test]
fn result_has_one_scopes_entry_for_each_source() {
    // A missing entry, a `null` entry and an empty string are all a source
    // without scopes. An entry past the number of sources is ignored.
    let short = decode_scopes(&[None, Some("")], &NO_RANGES, &[], 3).unwrap();
    assert_eq!(short.scopes, vec![vec![], vec![], vec![]]);

    let long = decode_scopes(&[Some("BAAA,CBA"), Some("BAAA,CCA")], &NO_RANGES, &[], 1).unwrap();
    assert_eq!(long.scopes, vec![vec![scope(pos(0, 0), pos(1, 0))]]);
}

#[test]
fn encoder_writes_the_draft_layout() {
    let info = ScopeInfo {
        scopes: vec![
            vec![OriginalScope {
                kind: Some("global".to_string()),
                ..scope(pos(0, 0), pos(10, 0))
            }],
            vec![],
        ],
        ranges: vec![GeneratedRange {
            definition: Some(def(0, 0)),
            ..range(pos(0, 0), pos(10, 0))
        }],
    };
    let mut names = vec![];
    let encoded = encode_scopes(&info, &mut names);
    assert_eq!(encoded.scopes, vec![Some("BCAAA,CKA".to_string()), None]);
    assert_eq!(encoded.ranges, vec!["ECAAA,FKA".to_string()]);
    assert_eq!(names, vec!["global"]);
}

#[test]
fn encoder_writes_no_ranges_entry_without_ranges() {
    let info = ScopeInfo { scopes: vec![vec![scope(pos(0, 0), pos(1, 0))]], ranges: vec![] };
    let encoded = encode_scopes(&info, &mut vec![]);
    assert_eq!(encoded.scopes, vec![Some("BAAA,CBA".to_string())]);
    assert_eq!(encoded.ranges, Vec::<String>::new());
}

#[test]
fn comprehensive_roundtrip() {
    let info = comprehensive_info();
    assert_eq!(roundtrip(&info), info);
}

#[test]
fn a_source_can_have_several_root_scopes() {
    // The position of the second root is relative to the end of the first.
    let info = decode_scopes(&[Some("BAAA,CCA,BABC,CAC")], &NO_RANGES, &[], 1).unwrap();
    assert_eq!(info.scopes, vec![vec![scope(pos(0, 0), pos(2, 0)), scope(pos(3, 2), pos(3, 4))]]);
}

#[test]
fn each_source_starts_with_new_relative_state() {
    let names = names(&["a", "b"]);
    // Each source has one scope with the name at index 1 and the variable at index 1.
    let entry = "BBCEC,DC,CAA";
    let info = decode_scopes(&[Some(entry), Some(entry)], &NO_RANGES, &names, 2).unwrap();
    let expected = OriginalScope {
        name: Some("b".to_string()),
        variables: vec!["b".to_string()],
        ..scope(pos(2, 4), pos(2, 4))
    };
    assert_eq!(info.scopes, vec![vec![expected.clone()], vec![expected]]);
}

#[test]
fn each_ranges_string_starts_with_new_relative_state() {
    let scopes = [Some("BAAA,BAAA,CAA,CAA")];
    // Each string has one range at column 2 with definition (0, 1).
    let info = decode_scopes(&scopes, &["ECCAC,FC", "ECCAC,FC"], &[], 1).unwrap();
    let expected = GeneratedRange { definition: Some(def(0, 1)), ..range(pos(0, 2), pos(0, 4)) };
    assert_eq!(info.ranges, vec![expected.clone(), expected]);
}

// ── Definitions ──────────────────────────────────────────────────

#[test]
fn definition_is_a_source_index_and_a_scope_index() {
    let two_scopes = vec![OriginalScope {
        children: vec![scope(pos(1, 0), pos(2, 0))],
        ..scope(pos(0, 0), pos(3, 0))
    }];
    let definitions = [def(0, 1), def(2, 0), def(2, 1), def(0, 0)];
    let info = ScopeInfo {
        scopes: vec![two_scopes.clone(), vec![], two_scopes],
        ranges: definitions
            .iter()
            .zip(0..)
            .map(|(definition, line)| GeneratedRange {
                definition: Some(*definition),
                ..range(pos(line, 0), pos(line, 5))
            })
            .collect(),
    };

    let encoded = encode_scopes(&info, &mut vec![]);
    // Same source: relative scope index. Other source: absolute scope index.
    assert_eq!(encoded.ranges, vec!["ECAAC,FF,EDBAEA,FF,EDBAAC,FF,EDBAFA,FF".to_string()]);
    assert_eq!(roundtrip(&info), info);
}

#[test]
fn original_scope_for_definition_counts_the_scopes_of_one_source() {
    let info = comprehensive_info();
    let name = |source, index| {
        info.original_scope_for_definition(def(source, index)).map(|scope| scope.start)
    };
    assert_eq!(name(0, 0), Some(pos(0, 0)));
    assert_eq!(name(0, 1), Some(pos(1, 2)));
    assert_eq!(name(0, 2), Some(pos(5, 0)));
    // The second root scope of the source follows the first tree.
    assert_eq!(name(0, 3), Some(pos(10, 0)));
    assert_eq!(name(0, 4), None);
    assert_eq!(name(1, 0), None);
    assert_eq!(name(2, 1), Some(pos(1, 0)));
    assert_eq!(name(3, 0), None);
}

#[test]
fn negative_definition_index_becomes_zero() {
    // Source index -1 and scope index -1 become 0, as AccumulateIndex specifies.
    let info = decode_one("BAAA,CAA", "ECADA,FA,ECAAD,FA", &[]).unwrap();
    assert_eq!(info.ranges[0].definition, Some(def(0, 0)));
    assert_eq!(info.ranges[1].definition, Some(def(0, 0)));
}

// ── Flags ────────────────────────────────────────────────────────

#[test]
fn hidden_flag_needs_the_stack_frame_flag() {
    let info = decode_one("", "EIA,FA,EMA,FA,EEA,FA", &[]).unwrap();
    let flags: Vec<(bool, bool)> =
        info.ranges.iter().map(|range| (range.is_stack_frame, range.is_hidden)).collect();
    assert_eq!(flags, vec![(false, false), (true, true), (true, false)]);

    // The encoder does not write the hidden flag without the stack frame flag.
    let hidden = GeneratedRange { is_hidden: true, ..range(pos(0, 0), pos(0, 1)) };
    let encoded = encode_scopes(&ScopeInfo { scopes: vec![], ranges: vec![hidden] }, &mut vec![]);
    assert_eq!(encoded.ranges, vec!["EAA,FB".to_string()]);
}

#[test]
fn scope_with_name_and_without_kind() {
    let info = ScopeInfo {
        scopes: vec![vec![OriginalScope {
            name: Some("f".to_string()),
            ..scope(pos(0, 0), pos(1, 0))
        }]],
        ranges: vec![],
    };
    assert_eq!(roundtrip(&info), info);
}

// ── Names ────────────────────────────────────────────────────────

#[test]
fn name_index_past_the_end_gives_no_name_and_no_kind() {
    // Name index 5 and kind index 6, with one name.
    let info = decode_one("BDAAKM,CAA", "", &names(&["a"])).unwrap();
    assert_eq!(info.scopes[0], vec![scope(pos(0, 0), pos(0, 0))]);
}

#[test]
fn negative_name_index_becomes_zero() {
    let info = decode_one("BBAAD,DD,CAA", "", &names(&["a", "b"])).unwrap();
    let root = &info.scopes[0][0];
    assert_eq!(root.name.as_deref(), Some("a"));
    assert_eq!(root.variables, vec!["a"]);
}

#[test]
fn variable_index_past_the_end_is_the_empty_name() {
    // Variables at index 0, 5 and 1. The second one is past the end.
    let info = decode_one("BAAA,DAKJ,CAA", "ECAAA,GBCA,FA", &names(&["a", "b"])).unwrap();
    assert_eq!(info.scopes[0][0].variables, vec!["a", "", "b"]);
    // Each binding stays with its variable.
    assert_eq!(
        info.ranges[0].bindings,
        vec![
            Binding::Expression("a".to_string()),
            Binding::Expression("b".to_string()),
            Binding::Unavailable
        ]
    );
}

#[test]
fn binding_index_past_the_end_is_not_available() {
    let info = decode_one("", "EAA,GBK,HBAKK,FU", &names(&["a"])).unwrap();
    assert_eq!(
        info.ranges[0].bindings,
        vec![
            Binding::Expression("a".to_string()),
            Binding::SubRanges(vec![
                SubRangeBinding { expression: None, from: pos(0, 0) },
                SubRangeBinding { expression: None, from: pos(0, 10) },
            ]),
        ]
    );
}

// ── Sub-range bindings ───────────────────────────────────────────

#[test]
fn sub_range_item_has_line_column_expression_groups() {
    // The example of the specification: "a" from 0:0, not available from
    // 0:10, "b" from 0:20.
    let info = decode_one("", "EAA,GB,HAAKAAKC,Fe", &names(&["a", "b"])).unwrap();
    let expected = Binding::SubRanges(vec![
        SubRangeBinding { expression: Some("a".to_string()), from: pos(0, 0) },
        SubRangeBinding { expression: None, from: pos(0, 10) },
        SubRangeBinding { expression: Some("b".to_string()), from: pos(0, 20) },
    ]);
    assert_eq!(info.ranges[0].bindings, vec![expected]);

    let mut encode_names = names(&["a", "b"]);
    let encoded = encode_scopes(&info, &mut encode_names);
    assert_eq!(encoded.ranges, vec!["EAA,GB,HAAKAAKC,Fe".to_string()]);
}

#[test]
fn sub_range_item_has_the_absolute_variable_index() {
    let variable = |first: &str, second: &str, column| {
        Binding::SubRanges(vec![
            SubRangeBinding { expression: Some(first.to_string()), from: pos(1, 4) },
            SubRangeBinding { expression: Some(second.to_string()), from: pos(1, column) },
        ])
    };
    let info = ScopeInfo {
        scopes: vec![],
        ranges: vec![GeneratedRange {
            bindings: vec![
                Binding::Unavailable,
                variable("a", "b", 9),
                Binding::Expression("c".to_string()),
                variable("d", "e", 6),
            ],
            ..range(pos(1, 4), pos(2, 0))
        }],
    };
    let encoded = encode_scopes(&info, &mut vec![]);
    // H items for variable 1 and variable 3. Each position is relative to the
    // start of the range.
    assert_eq!(encoded.ranges, vec!["EBBE,GABCD,HBAFE,HDACF,FBA".to_string()]);
    assert_eq!(roundtrip(&info), info);
}

#[test]
fn sub_range_position_on_a_later_line_has_an_absolute_column() {
    let info = decode_one("", "EAE,GB,HABCA,FCA", &names(&["a"])).unwrap();
    assert_eq!(
        info.ranges[0].bindings,
        vec![Binding::SubRanges(vec![
            SubRangeBinding { expression: Some("a".to_string()), from: pos(0, 4) },
            SubRangeBinding { expression: None, from: pos(1, 2) },
        ])]
    );
}

#[test]
fn second_sub_range_item_for_a_variable_adds_to_the_list() {
    let info = decode_one("", "EAA,GB,HAAKA,HAAUC,Fe", &names(&["a", "b"])).unwrap();
    assert_eq!(
        info.ranges[0].bindings,
        vec![Binding::SubRanges(vec![
            SubRangeBinding { expression: Some("a".to_string()), from: pos(0, 0) },
            SubRangeBinding { expression: None, from: pos(0, 10) },
            SubRangeBinding { expression: Some("b".to_string()), from: pos(0, 20) },
        ])]
    );
}

#[test]
fn sub_range_item_for_a_variable_without_binding_is_ignored() {
    let info = decode_one("", "EAA,GB,HFAKA,Fe", &names(&["a"])).unwrap();
    assert_eq!(info.ranges[0].bindings, vec![Binding::Expression("a".to_string())]);
}

#[test]
fn sub_range_item_without_a_sub_range_is_ignored() {
    let info = decode_one("", "EAA,GB,HA,Fe", &names(&["a"])).unwrap();
    assert_eq!(info.ranges[0].bindings, vec![Binding::Expression("a".to_string())]);
}

#[test]
fn encoder_keeps_sub_ranges_that_are_not_in_order() {
    // Two sub-range items for one variable decode to a list that goes back.
    let info = decode_one("", "EAA,GB,HAAUA,HAAKC,Fe", &names(&["a", "b"])).unwrap();
    let mut encode_names = names(&["a", "b"]);
    let encoded = encode_scopes(&info, &mut encode_names);
    assert_eq!(encoded.ranges, vec!["EAA,GB,HAAUA,HAAKC,Fe".to_string()]);
    assert_eq!(roundtrip(&info), info);
}

#[test]
fn encoder_starts_a_new_ranges_string_for_a_range_that_goes_back() {
    let info = decode_scopes(&NO_SCOPES, &["EAK,FC", "EAC,FC,EAC,FC"], &[], 0).unwrap();
    let encoded = encode_scopes(&info, &mut vec![]);
    assert_eq!(encoded.ranges, vec!["EAK,FC".to_string(), "EAC,FC,EAC,FC".to_string()]);
    assert_eq!(roundtrip(&info), info);
}

#[test]
fn encoder_does_not_panic_on_positions_that_go_back() {
    // An end before its start has no encoding. The encoder writes the start.
    let info = ScopeInfo {
        scopes: vec![vec![scope(pos(3, 5), pos(1, 0))]],
        ranges: vec![range(pos(2, 5), pos(2, 1))],
    };
    let encoded = encode_scopes(&info, &mut vec![]);
    assert_eq!(encoded.scopes, vec![Some("BADF,CAA".to_string())]);
    assert_eq!(encoded.ranges, vec!["EBCF,FA".to_string()]);
}

// ── Call sites ───────────────────────────────────────────────────

/// Source 0 has a root scope at 2:1 and a function at 5:4. Source 1 has a
/// root scope at 7:3.
const CALL_SITE_SCOPES: [Option<&str>; 2] = [Some("BACB,BADE,CEB,CKA"), Some("BAHD,CBA")];

fn call_site_of(ranges: &str) -> Result<Option<CallSite>, ScopesError> {
    let info = decode_scopes(&CALL_SITE_SCOPES, &[ranges], &[], 2)?;
    let mut range = &info.ranges[0];
    while let Some(child) = range.children.first() {
        range = child;
    }
    Ok(range.call_site)
}

#[test]
fn absolute_call_site_roundtrip() {
    assert_eq!(
        call_site_of("ECAAA,EAC,IBHD,FC,FC").unwrap(),
        Some(CallSite { source_index: 1, line: 7, column: 3 })
    );
}

#[test]
fn relative_call_site_with_a_column_offset() {
    // The nearest parent with a definition is the range of the scope at 2:1.
    assert_eq!(
        call_site_of("ECAAA,EAC,JE,FC,FC").unwrap(),
        Some(CallSite { source_index: 0, line: 2, column: 5 })
    );
}

#[test]
fn relative_call_site_with_a_line_offset_and_a_column() {
    assert_eq!(
        call_site_of("ECACA,EAC,JDJ,FC,FC").unwrap(),
        Some(CallSite { source_index: 1, line: 10, column: 9 })
    );
}

#[test]
fn relative_call_site_with_a_parent_index() {
    // Parent 0 is the range of the scope at 7:3, parent 1 is the range of the
    // function at 5:4. A range without a definition does not count.
    let ranges = "ECAAC,ECACA,EAA,EAC,JBCG,FC,FA,FA,FA";
    assert_eq!(
        call_site_of(ranges).unwrap(),
        Some(CallSite { source_index: 0, line: 7, column: 6 })
    );
}

#[test]
fn relative_call_site_does_not_use_the_definition_of_its_own_range() {
    assert_eq!(
        call_site_of("ECAAA,ECCAC,JE,FC,FC").unwrap(),
        Some(CallSite { source_index: 0, line: 2, column: 5 })
    );
}

#[test]
fn relative_call_site_without_a_parent_definition_is_an_error() {
    // No parent range.
    assert_eq!(call_site_of("ECAAA,JE,FC").unwrap_err(), ScopesError::UnresolvedCallSite);
    // The parent has no definition.
    assert_eq!(call_site_of("EAA,EAC,JE,FC,FC").unwrap_err(), ScopesError::UnresolvedCallSite);
    // The parent index is past the last parent.
    assert_eq!(call_site_of("ECAAA,EAC,JBAA,FC,FC").unwrap_err(), ScopesError::UnresolvedCallSite);
    // The definition of the parent is not a decoded scope.
    assert_eq!(call_site_of("ECAAK,EAC,JE,FC,FC").unwrap_err(), ScopesError::UnresolvedCallSite);
    assert_eq!(call_site_of("ECAKA,EAC,JE,FC,FC").unwrap_err(), ScopesError::UnresolvedCallSite);
}

#[test]
fn encoder_writes_the_call_site_before_the_sub_range_items() {
    let info = ScopeInfo {
        scopes: vec![],
        ranges: vec![GeneratedRange {
            call_site: Some(CallSite { source_index: 0, line: 1, column: 2 }),
            bindings: vec![Binding::SubRanges(vec![
                SubRangeBinding { expression: Some("a".to_string()), from: pos(0, 0) },
                SubRangeBinding { expression: None, from: pos(0, 5) },
            ])],
            ..range(pos(0, 0), pos(0, 9))
        }],
    };
    let encoded = encode_scopes(&info, &mut vec![]);
    assert_eq!(encoded.ranges, vec!["EAA,GB,IABC,HAAFA,FJ".to_string()]);
    assert_eq!(roundtrip(&info), info);
}

#[test]
fn decoder_reads_a_call_site_after_the_sub_range_items() {
    // The Chrome DevTools codec writes the items in this order.
    let info = decode_one("", "EAA,GB,HAAFA,IABC,FJ", &names(&["a"])).unwrap();
    assert_eq!(info.ranges[0].call_site, Some(CallSite { source_index: 0, line: 1, column: 2 }));
    assert!(matches!(info.ranges[0].bindings[0], Binding::SubRanges(_)));
}

// ── Items that the decoder ignores ───────────────────────────────

#[test]
fn vendor_extension_items_are_ignored() {
    let names = names(&["vendor"]);
    // Tag V with the name index, with and without more values.
    let scopes = "VA,BAAA,VAKK,BABA,CAC,VA,CBA,VAC";
    let ranges = "VA,EAA,VAKK,EAC,FC,VA,FC,VAC";
    let info = decode_one(scopes, ranges, &names).unwrap();
    assert_eq!(info, decode_one("BAAA,BABA,CAC,CBA", "EAA,EAC,FC,FC", &names).unwrap());
    assert_eq!(info.scopes[0][0].children.len(), 1);
    assert_eq!(info.ranges[0].children.len(), 1);
}

#[test]
fn items_with_an_unknown_tag_are_ignored() {
    // Tag 0x20 is not in the specification.
    let info = decode_one("gBAKK,BAAA,gB,CBA", "gBA,EAA,gBKKK,FC", &[]).unwrap();
    assert_eq!(info.scopes[0], vec![scope(pos(0, 0), pos(1, 0))]);
    assert_eq!(info.ranges, vec![range(pos(0, 0), pos(0, 2))]);
}

#[test]
fn range_items_in_scopes_and_scope_items_in_ranges_are_ignored() {
    let info = decode_one("EAA,BAAA,FC,CBA", "BAAA,EAA,CBA,FC", &[]).unwrap();
    assert_eq!(info.scopes[0], vec![scope(pos(0, 0), pos(1, 0))]);
    assert_eq!(info.ranges, vec![range(pos(0, 0), pos(0, 2))]);
}

#[test]
fn more_values_at_the_end_of_an_item_are_ignored() {
    let names = names(&["a"]);
    let info = decode_one("BBABAKK,CBAKK", "ECCAAKK,FCKK", &names).unwrap();
    let expected = OriginalScope { name: Some("a".to_string()), ..scope(pos(0, 1), pos(1, 0)) };
    assert_eq!(info.scopes[0], vec![expected]);
    assert_eq!(info.ranges[0].start, pos(0, 2));
    assert_eq!(info.ranges[0].end, pos(2, 10));
}

#[test]
fn items_outside_a_scope_or_a_range_are_ignored() {
    let info = decode_one("DAC,BAAA,CBA", "GB,HAAKA,IAAA,EAA,FC", &names(&["a"])).unwrap();
    assert_eq!(info.scopes[0], vec![scope(pos(0, 0), pos(1, 0))]);
    assert_eq!(info.ranges, vec![range(pos(0, 0), pos(0, 2))]);
}

// ── Positions ────────────────────────────────────────────────────

#[test]
fn range_start_and_end_have_an_optional_line() {
    let info = ScopeInfo {
        scopes: vec![],
        ranges: vec![
            GeneratedRange {
                children: vec![range(pos(0, 8), pos(2, 3))],
                ..range(pos(0, 5), pos(2, 9))
            },
            range(pos(4, 1), pos(4, 2)),
        ],
    };
    let encoded = encode_scopes(&info, &mut vec![]);
    assert_eq!(encoded.ranges, vec!["EAF,EAD,FCD,FG,EBCB,FB".to_string()]);
    assert_eq!(roundtrip(&info), info);
}

// ── Errors ───────────────────────────────────────────────────────

#[test]
fn decode_errors() {
    let error = |scopes, ranges| decode_one(scopes, ranges, &[]).unwrap_err();
    assert_eq!(error("CAA", ""), ScopesError::UnmatchedScopeEnd);
    assert_eq!(error("BAAA", ""), ScopesError::UnclosedScope);
    assert_eq!(error("", "FA"), ScopesError::UnmatchedRangeEnd);
    assert_eq!(error("", "EAA"), ScopesError::UnclosedRange);
    assert!(matches!(error("!!", ""), ScopesError::Vlq(_)));
    assert!(matches!(error("", "!!"), ScopesError::Vlq(_)));
    // An item needs a tag.
    assert!(matches!(error("BAAA,,CAA", ""), ScopesError::Vlq(_)));
    // A range start with the definition flag needs two definition values.
    assert!(matches!(error("", "ECAA,FA"), ScopesError::Vlq(_)));
}

#[test]
fn a_scope_must_end_in_the_string_of_its_source() {
    let err = decode_scopes(&[Some("BAAA"), Some("CAA")], &NO_RANGES, &[], 2).unwrap_err();
    assert_eq!(err, ScopesError::UnclosedScope);
}

#[test]
fn a_range_must_end_in_its_ranges_string() {
    let err = decode_scopes(&NO_SCOPES, &["EAA", "FA"], &[], 0).unwrap_err();
    assert_eq!(err, ScopesError::UnclosedRange);
}

#[test]
fn scopes_error_display() {
    let messages: Vec<String> = [
        ScopesError::UnmatchedScopeEnd,
        ScopesError::UnclosedScope,
        ScopesError::UnmatchedRangeEnd,
        ScopesError::UnclosedRange,
        ScopesError::UnresolvedCallSite,
        ScopesError::ValueOverflow,
        ScopesError::NestingTooDeep,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        messages,
        vec![
            "scope end without matching start".to_string(),
            "scope opened but never closed".to_string(),
            "range end without matching start".to_string(),
            "range opened but never closed".to_string(),
            "relative call site has no parent range with a known scope".to_string(),
            "scopes position or index is out of range".to_string(),
            format!("scopes nest deeper than {MAX_NESTING_DEPTH} levels"),
        ]
    );

    let err = decode_one("!!", "", &[]).unwrap_err();
    assert!(err.to_string().contains("VLQ decode error"));
}

fn vlq_unsigned(value: u64) -> String {
    let mut out = Vec::new();
    srcmap_codec::vlq_encode_unsigned(&mut out, value);
    String::from_utf8(out).unwrap()
}

fn vlq_signed(value: i64) -> String {
    let mut out = Vec::new();
    srcmap_codec::vlq_encode(&mut out, value);
    String::from_utf8(out).unwrap()
}

fn overflow_error(scopes: &str, ranges: &str) {
    let names = names(&["a", "b"]);
    assert_eq!(decode_one(scopes, ranges, &names).unwrap_err(), ScopesError::ValueOverflow);
}

#[test]
fn decode_rejects_position_overflow() {
    let max = vlq_unsigned(u64::from(u32::MAX));
    let big = vlq_unsigned(1 << 32);
    // Outer scope at column u32::MAX, nested scope one column later.
    overflow_error(&format!("BAA{max},BAAC,CAA,CAA"), "");
    overflow_error(&format!("BA{max}A,BACA,CAA,CAA"), "");
    overflow_error(&format!("BAA{big},CAA"), "");
    overflow_error("", &format!("EA{max},EAC,FA,FA"));
    // A sub-range position and a relative call site also overflow.
    overflow_error("", &format!("EAC,GBB,HBA{max}A,FA"));
    overflow_error(&format!("BA{max}A,CAA"), "ECAAA,EAA,JCA,FA,FA");
    overflow_error(&format!("BAA{max},CAA"), "ECAAA,EAA,JC,FA,FA");
}

#[test]
fn decode_rejects_index_overflow() {
    let max = vlq_signed(i64::MAX);
    let above_u32 = vlq_signed(1 << 32);
    // The first range moves the scope index to 1, the second adds i64::MAX.
    overflow_error("", &format!("ECAAC,FA,ECAA{max},FA"));
    overflow_error("", &format!("ECACA,FA,ECA{max}A,FA"));
    overflow_error("", &format!("ECAA{above_u32},FA"));
    overflow_error("", &format!("ECA{above_u32}A,FA"));
    // An absolute scope index of a new source.
    overflow_error("", &format!("ECAC{},FA", vlq_unsigned(1 << 32)));
    overflow_error("", &format!("ECAC{},FA", vlq_unsigned(u64::MAX)));
    // The first scope moves the name index to 1, the second adds i64::MAX.
    overflow_error(&format!("BBAAC,CAA,BBAA{max},CAA"), "");
}

#[test]
fn decode_checks_the_nesting_limit() {
    let nested = |start: &str, end: &str, depth: usize| {
        [vec![start; depth], vec![end; depth]].concat().join(",")
    };
    let at_limit = decode_one(
        &nested("BAAA", "CAA", MAX_NESTING_DEPTH),
        &nested("EAA", "FA", MAX_NESTING_DEPTH),
        &[],
    )
    .unwrap();
    assert_eq!(at_limit.scopes[0].len(), 1);
    assert_eq!(at_limit.ranges.len(), 1);

    let depth = MAX_NESTING_DEPTH + 1;
    assert_eq!(
        decode_one(&nested("BAAA", "CAA", depth), "", &[]).unwrap_err(),
        ScopesError::NestingTooDeep
    );
    assert_eq!(
        decode_one("", &nested("EAA", "FA", depth), &[]).unwrap_err(),
        ScopesError::NestingTooDeep
    );
}

#[test]
fn scope_trees_at_the_nesting_limit_fit_on_a_default_thread_stack() {
    const DEFAULT_THREAD_STACK: usize = 2 * 1024 * 1024;
    let scopes =
        [vec!["BAAA"; MAX_NESTING_DEPTH], vec!["CAA"; MAX_NESTING_DEPTH]].concat().join(",");
    let ranges = [vec!["EAA"; MAX_NESTING_DEPTH], vec!["FA"; MAX_NESTING_DEPTH]].concat().join(",");
    std::thread::Builder::new()
        .stack_size(DEFAULT_THREAD_STACK)
        .spawn(move || {
            let info = decode_one(&scopes, &ranges, &[]).unwrap();
            let copy = info.clone();
            assert!(format!("{copy:?}").starts_with("ScopeInfo"));
            assert_eq!(roundtrip(&info), copy);
        })
        .unwrap()
        .join()
        .unwrap();
}

// ── Older layout ─────────────────────────────────────────────────

/// The output of the encoder of srcmap 0.6.0 for three sources. The second
/// source has no scopes.
const LEGACY_SCOPES: &str = concat!(
    "BCAAA,DC,BHBCEG,DGC,CDB,CFA,,BCAAH,BHBAIG,DE,CBB,CBA,ECAA,GJ,EHBAC,GKA,HAAAKLAK,",
    "F",
    "oB,EDBAE,GM,IAHD,FBE,ENBA,FJ,FBA",
);
const LEGACY_NAMES: [&str; 12] =
    ["module", "x", "f", "function", "a", "b", "g", "c", "_x", "_a", "_a2", "_c"];

#[test]
fn legacy_layout_decodes_to_the_same_structure() {
    let module = |end: Position, variables: &[&str], child: OriginalScope| OriginalScope {
        kind: Some("module".to_string()),
        variables: names(variables),
        children: vec![child],
        ..scope(pos(0, 0), end)
    };
    let expected = ScopeInfo {
        scopes: vec![
            vec![module(pos(9, 0), &["x"], function("f", pos(1, 2), pos(4, 1), &["a", "b"]))],
            vec![],
            vec![module(pos(3, 0), &[], function("g", pos(1, 0), pos(2, 1), &["c"]))],
        ],
        ranges: vec![GeneratedRange {
            definition: Some(def(0, 0)),
            bindings: vec![Binding::Expression("_x".to_string())],
            children: vec![
                GeneratedRange {
                    is_stack_frame: true,
                    definition: Some(def(0, 1)),
                    bindings: vec![
                        Binding::SubRanges(vec![
                            SubRangeBinding { expression: Some("_a".to_string()), from: pos(1, 0) },
                            SubRangeBinding { expression: None, from: pos(1, 10) },
                            SubRangeBinding {
                                expression: Some("_a2".to_string()),
                                from: pos(1, 20),
                            },
                        ]),
                        Binding::Unavailable,
                    ],
                    ..range(pos(1, 0), pos(1, 40))
                },
                GeneratedRange {
                    // Index 3 of all scopes is scope 1 of source 2.
                    definition: Some(def(2, 1)),
                    call_site: Some(CallSite { source_index: 0, line: 7, column: 3 }),
                    bindings: vec![Binding::Expression("_c".to_string())],
                    ..range(pos(2, 0), pos(3, 4))
                },
                GeneratedRange {
                    is_stack_frame: true,
                    is_hidden: true,
                    ..range(pos(4, 0), pos(4, 9))
                },
            ],
            ..range(pos(0, 0), pos(5, 0))
        }],
    };

    let info = decode_legacy_scopes(LEGACY_SCOPES, &names(&LEGACY_NAMES), 3).unwrap();
    assert_eq!(info, expected);
    // The draft layout keeps the structure.
    assert_eq!(roundtrip(&info), expected);
}

#[test]
fn legacy_layout_is_not_the_draft_layout() {
    // The same string as one entry of the draft layout gives other data.
    let names = names(&LEGACY_NAMES);
    let draft = decode_scopes(&[Some(LEGACY_SCOPES)], &NO_RANGES, &names, 3);
    assert_ne!(draft.ok(), decode_legacy_scopes(LEGACY_SCOPES, &names, 3).ok());
}

#[test]
fn legacy_layout_sources_without_scopes() {
    assert_eq!(
        decode_legacy_scopes("", &[], 0).unwrap(),
        ScopeInfo { scopes: vec![], ranges: vec![] }
    );
    // An empty item is a source without scopes.
    let info = decode_legacy_scopes(",", &[], 2).unwrap();
    assert_eq!(info.scopes, vec![vec![], vec![]]);
    // The first range starts the ranges part, and the missing sources follow.
    let info = decode_legacy_scopes(",BAAA,CBA,ECAA,FC", &[], 4).unwrap();
    assert_eq!(info.scopes, vec![vec![], vec![scope(pos(0, 0), pos(1, 0))], vec![], vec![]]);
    assert_eq!(info.ranges[0].definition, Some(def(1, 0)));
}

#[test]
fn legacy_definition_past_the_last_scope_stays_out_of_range() {
    let info = decode_legacy_scopes("BAAA,CBA,ECAK,FC", &[], 1).unwrap();
    let definition = info.ranges[0].definition.unwrap();
    assert_eq!(definition, def(0, 5));
    assert_eq!(info.original_scope_for_definition(definition), None);
}

#[test]
fn legacy_layout_errors() {
    let error = |input| decode_legacy_scopes(input, &[], 1).unwrap_err();
    assert_eq!(error("CAA"), ScopesError::UnmatchedScopeEnd);
    assert_eq!(error("BAAA"), ScopesError::UnclosedScope);
    assert_eq!(error("FA"), ScopesError::UnmatchedRangeEnd);
    assert_eq!(error("EAA"), ScopesError::UnclosedRange);
    assert!(matches!(error("!!"), ScopesError::Vlq(_)));
    // Each sub-range item adds a large delta to the variable index.
    let item = format!("H{}", vlq_unsigned(1 << 58));
    let input = format!("EAA,{},FA", vec![item.as_str(); 80].join(","));
    assert_eq!(error(&input), ScopesError::ValueOverflow);
}

#[test]
fn readme_example_strings() {
    let info = ScopeInfo {
        scopes: vec![vec![OriginalScope {
            kind: Some("global".to_string()),
            variables: names(&["x"]),
            ..scope(pos(0, 0), pos(5, 0))
        }]],
        ranges: vec![GeneratedRange {
            definition: Some(def(0, 0)),
            bindings: vec![Binding::Expression("_x".to_string())],
            ..range(pos(0, 0), pos(5, 0))
        }],
    };
    let mut names = names(&["global", "x", "_x"]);
    let encoded = encode_scopes(&info, &mut names);
    assert_eq!(encoded.scopes, vec![Some("BCAAA,DC,CFA".to_string())]);
    assert_eq!(encoded.ranges, vec!["ECAAA,GD,FFA".to_string()]);

    let mut json = String::new();
    encoded.write_json_fields(&mut json);
    assert_eq!(json, r#","scopes":["BCAAA,DC,CFA"],"ranges":["ECAAA,GD,FFA"]"#);
}
