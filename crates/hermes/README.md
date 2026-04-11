# srcmap-hermes

[![crates.io](https://img.shields.io/crates/v/srcmap-hermes.svg)](https://crates.io/crates/srcmap-hermes)
[![docs.rs](https://docs.rs/srcmap-hermes/badge.svg)](https://docs.rs/srcmap-hermes)

Hermes/React Native source map support.

Wraps a regular `SourceMap` and decodes Facebook-specific extension fields produced by Metro (the React Native bundler): `x_facebook_sources` (VLQ-encoded function scope mappings), `x_facebook_offsets` (byte offsets for RAM bundle modules), and `x_metro_module_paths` (module paths). Enables function-name resolution for Hermes stack frames, similar to `SourceMapHermes` in getsentry/rust-sourcemap.

## Usage

```rust
use srcmap_hermes::SourceMapHermes;

let json = r#"{
  "version": 3,
  "sources": ["app.js"],
  "names": [],
  "mappings": "AAAA,CAAC",
  "x_facebook_sources": [
    [{"names": ["<global>", "handlePress"], "mappings": "AAA,CCA"}]
  ]
}"#;

let sm = SourceMapHermes::from_json(json).unwrap();

// Look up the enclosing function name for a generated position
let name = sm.get_original_function_name(0, 1);
assert_eq!(name, Some("handlePress"));

// Access the underlying SourceMap via Deref
assert_eq!(sm.sources.len(), 1);

// Check for RAM bundle support
assert!(!sm.is_for_ram_bundle());
```

## API

| Type / Function | Description |
|-----------------|-------------|
| `SourceMapHermes` | Hermes-enhanced source map wrapping a `SourceMap` (implements `Deref<Target = SourceMap>`) |
| `SourceMapHermes::from_json(json)` | Parse a Hermes source map from JSON, extracting extension fields |
| `SourceMapHermes::get_function_map(source_idx)` | Get the function map for a source by index |
| `SourceMapHermes::get_scope_for_token(line, col)` | Find the enclosing function scope for a generated position |
| `SourceMapHermes::get_original_function_name(line, col)` | Get the original function name for a generated position |
| `SourceMapHermes::is_for_ram_bundle()` | Check if `x_facebook_offsets` is present |
| `SourceMapHermes::x_facebook_offsets()` | Get RAM bundle byte offsets |
| `SourceMapHermes::x_metro_module_paths()` | Get Metro module paths |
| `SourceMapHermes::inner()` | Borrow the inner `SourceMap` |
| `SourceMapHermes::into_inner()` | Consume and return the inner `SourceMap` |
| `SourceMapHermes::to_json()` | Serialize back to JSON, preserving extensions |
| `HermesFunctionMap` | Function map for a single source: `names` + `mappings` |
| `HermesScopeOffset` | A scope boundary: 0-based `line`, `column`, and `name_index` |
| `HermesError` | Error type: `Parse`, `Vlq`, or `InvalidFunctionMap` |

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

See the [main repo](https://github.com/fallow-rs/srcmap) for the full source map SDK.

## License

MIT
