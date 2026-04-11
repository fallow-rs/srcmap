# srcmap-scopes

[![crates.io](https://img.shields.io/crates/v/srcmap-scopes.svg)](https://crates.io/crates/srcmap-scopes)
[![docs.rs](https://docs.rs/srcmap-scopes/badge.svg)](https://docs.rs/srcmap-scopes)

Scopes and variables decoder/encoder for source maps ([ECMA-426](https://tc39.es/ecma426/)).

Implements the "Scopes" proposal for source maps, enabling debuggers to reconstruct original scope trees, variable bindings, and inlined function call sites from generated code.

## Usage

```rust
use srcmap_scopes::{
    decode_scopes, encode_scopes, Binding, GeneratedRange,
    OriginalScope, Position, ScopeInfo,
};

// Build scope info
let info = ScopeInfo {
    scopes: vec![Some(OriginalScope {
        start: Position { line: 0, column: 0 },
        end: Position { line: 5, column: 0 },
        name: None,
        kind: Some("global".to_string()),
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
        bindings: vec![Binding::Expression("_x".to_string())],
        children: vec![],
    }],
};

// Encode to VLQ
let mut names = vec!["global".to_string(), "x".to_string(), "_x".to_string()];
let encoded = encode_scopes(&info, &mut names);

// Decode back
let decoded = decode_scopes(&encoded, &names, 1).unwrap();
assert_eq!(decoded.scopes.len(), 1);
```

## Key types

| Type | Description |
|------|-------------|
| `ScopeInfo` | Top-level container: original scopes + generated ranges |
| `OriginalScope` | A scope in authored source code (tree structure) |
| `GeneratedRange` | A range in generated output mapped to an original scope |
| `Binding` | Variable binding: expression, unavailable, or sub-range bindings |
| `SubRangeBinding` | A sub-range binding within a generated range |
| `CallSite` | Inlined function call site in original source |
| `Position` | 0-based line/column pair |
| `ScopesError` | Errors during scopes decoding |

## How it works

The scopes format uses tag-based VLQ encoding where each item is prefixed with a tag byte identifying the item type (scope start, scope end, range start, etc.) followed by flags and VLQ-encoded values. This enables efficient serialization of tree-structured scope and range data into a single flat string.

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Used by `srcmap-sourcemap` (decode) and `srcmap-generator` (encode). See the [main repo](https://github.com/fallow-rs/srcmap) for the full source map SDK.

## License

MIT
