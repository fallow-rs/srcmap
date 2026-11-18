# srcmap-scopes

[![crates.io](https://img.shields.io/crates/v/srcmap-scopes.svg)](https://crates.io/crates/srcmap-scopes)
[![docs.rs](https://docs.rs/srcmap-scopes/badge.svg)](https://docs.rs/srcmap-scopes)

Scopes and variables decoder/encoder for source maps ([ECMA-426](https://tc39.es/ecma426/)).

Implements the "Scopes" proposal for source maps, enabling debuggers to reconstruct original scope trees, variable bindings, and inlined function call sites from generated code.

## Usage

```rust
use srcmap_scopes::{
    decode_scopes, encode_scopes, Binding, GeneratedRange, OriginalScope,
    Position, ScopeDefinition, ScopeInfo,
};

// Build scope info
let info = ScopeInfo {
    // One list of root scopes for each source.
    scopes: vec![vec![OriginalScope {
        start: Position { line: 0, column: 0 },
        end: Position { line: 5, column: 0 },
        name: None,
        kind: Some("global".to_string()),
        is_stack_frame: false,
        variables: vec!["x".to_string()],
        children: vec![],
    }]],
    ranges: vec![GeneratedRange {
        start: Position { line: 0, column: 0 },
        end: Position { line: 5, column: 0 },
        is_stack_frame: false,
        is_hidden: false,
        definition: Some(ScopeDefinition { source_index: 0, scope_index: 0 }),
        call_site: None,
        bindings: vec![Binding::Expression("_x".to_string())],
        children: vec![],
    }],
};

// Encode to the `scopes` and `ranges` fields of a source map
let mut names = vec!["global".to_string(), "x".to_string(), "_x".to_string()];
let encoded = encode_scopes(&info, &mut names);
assert_eq!(encoded.scopes, vec![Some("BCAAA,DC,CFA".to_string())]);
assert_eq!(encoded.ranges, vec!["ECAAA,GD,FFA".to_string()]);

// Decode back
let decoded = decode_scopes(&encoded.scopes, &encoded.ranges, &names, 1).unwrap();
assert_eq!(decoded, info);
```

## Wire layout

This crate reads and writes the layout of the [draft specification](https://tc39.es/ecma426/branch/proposal-scopes/):

```json
{
  "scopes": ["BCAAA,DC,CFA", null],
  "ranges": ["ECAAA,GD,FFA"]
}
```

- `scopes` is an array with one entry for each source. An entry is a string, or `null` for a source without scopes.
- `ranges` is an array of strings. Each string has the generated ranges of one region of the generated code.
- The definition of a generated range is a source index and a scope index. The scope index counts the scopes of that source in pre-order.

`encode_scopes` returns an `EncodedScopes` value with these two fields. `EncodedScopes::write_json_fields` appends them to a JSON object.

The encoder writes each call site as the absolute item (tag `I`). The decoder also reads a call site relative to a parent range (tag `J`). The decoder ignores a vendor extension item (tag `V`) and each item with a tag that it does not know.

The Chrome DevTools codec ([@chrome-devtools/source-map-scopes-codec](https://jsr.io/@chrome-devtools/source-map-scopes-codec), version 2) reads the output of this crate, and this crate reads the output of the codec. A test in `packages/scopes-wasm` compares the two.

### The older layout

The older layout of the [proposal text](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md) has one `scopes` string for the full map. srcmap 0.6.0 and version 0.9.0 of the codec wrote it. `decode_legacy_scopes` reads this layout into the same types. No function writes it.

A source map has the older layout when its `scopes` field is a JSON string, and the draft layout when the field is an array. `srcmap-sourcemap` uses this rule.

## Names that are not in `names`

The decoder follows the specification for an index past the end of `names`:

| Item | Result |
|------|--------|
| Scope name, scope kind | `None` |
| Variable | The empty string. The variable keeps its position, so each binding of a generated range stays with its variable. |
| Binding expression | `Binding::Unavailable` |

A relative index that goes below zero becomes zero.

## Errors

`decode_scopes` returns a `ScopesError` for these inputs:

- an invalid VLQ
- a scope or a range without its start or its end
- a value that does not fit in a `u32`
- nesting deeper than `MAX_NESTING_DEPTH`
- a relative call site without a parent range that has a known scope
- scopes that refer to more than `MAX_NAME_BYTES` bytes of names

The decoder does not check the order of the items in a scope or a range. For example, it accepts a second variables item in one scope, and it adds those variables to the scope.

## Key types

| Type | Description |
|------|-------------|
| `ScopeInfo` | Top-level container: original scopes + generated ranges |
| `OriginalScope` | A scope in authored source code (tree structure) |
| `GeneratedRange` | A range in generated output mapped to an original scope |
| `ScopeDefinition` | The original scope of a generated range: a source index and a scope index |
| `EncodedScopes` | The encoded `scopes` and `ranges` fields |
| `Binding` | Variable binding: expression, unavailable, or sub-range bindings |
| `SubRangeBinding` | A sub-range binding within a generated range |
| `CallSite` | Inlined function call site in original source |
| `Position` | 0-based line/column pair |
| `ScopesError` | Errors during scopes decoding |

## How it works

The scopes format uses tag-based VLQ encoding where each item is prefixed with a tag byte identifying the item type (scope start, scope end, range start, etc.) followed by flags and VLQ-encoded values. Each tree of scopes or ranges becomes a flat list of items. A relative value continues from the item before it, and each string starts with new relative state.

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Used by `srcmap-sourcemap` (decode) and `srcmap-generator` (encode). See the [main repo](https://github.com/fallow-rs/srcmap) for the full source map SDK.

## License

MIT
