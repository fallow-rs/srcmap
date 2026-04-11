# srcmap-sourcemap

[![crates.io](https://img.shields.io/crates/v/srcmap-sourcemap.svg)](https://crates.io/crates/srcmap-sourcemap)
[![docs.rs](https://docs.rs/srcmap-sourcemap/badge.svg)](https://docs.rs/srcmap-sourcemap)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/rust-coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)

High-performance source map parser and consumer for Rust.

Parses source map JSON and provides O(log n) position lookups. Implements the [ECMA-426](https://tc39.es/ecma426/) Source Map v3 specification. Drop-in Rust equivalent of [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping).

## Install

```toml
[dependencies]
srcmap-sourcemap = "0.3"
```

## Usage

```rust
use srcmap_sourcemap::SourceMap;

let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA;AACA"}"#;
let sm = SourceMap::from_json(json).unwrap();

// Forward lookup: generated position -> original position (0-based)
let loc = sm.original_position_for(0, 0).unwrap();
assert_eq!(sm.source(loc.source), "input.js");
assert_eq!(loc.line, 0);
assert_eq!(loc.column, 0);

// Reverse lookup: original position -> generated position
let pos = sm.generated_position_for("input.js", 0, 0).unwrap();
assert_eq!(pos.line, 0);
assert_eq!(pos.column, 0);
```

## API

### `SourceMap`

| Method | Description |
|--------|-------------|
| `from_json(json) -> Result<SourceMap>` | Parse a source map from a JSON string (regular or indexed) |
| `from_json_no_content(json) -> Result<SourceMap>` | Parse JSON without allocating `sourcesContent` strings |
| `from_parts(file, source_root, sources, ...) -> SourceMap` | Build from pre-decoded components and mappings |
| `from_vlq(mappings_str, sources, names, ...) -> Result<SourceMap>` | Build from pre-parsed components and a VLQ mappings string |
| `from_vlq_with_range_mappings(mappings_str, ..., range_mappings_str) -> Result<SourceMap>` | Build from VLQ mappings and optional range mappings strings |
| `from_json_lines(json, start_line, end_line) -> Result<SourceMap>` | Parse and decode only the given generated-line range |
| `builder() -> SourceMapBuilder` | Create a builder for constructing a `SourceMap` step by step |
| `original_position_for(line, col) -> Option<OriginalLocation>` | Look up original position for a generated position |
| `original_position_for_with_bias(line, col, bias) -> Option<OriginalLocation>` | Look up original position with explicit search bias |
| `generated_position_for(source, line, col) -> Option<GeneratedLocation>` | Reverse lookup: original position to generated position |
| `generated_position_for_with_bias(source, line, col, bias) -> Option<GeneratedLocation>` | Reverse lookup with explicit search bias |
| `all_generated_positions_for(source, line, col) -> Vec<GeneratedLocation>` | Find all generated positions for an original position |
| `map_range(start_line, start_col, end_line, end_col) -> Option<MappedRange>` | Map a generated range to its original range |
| `all_mappings() -> &[Mapping]` | All decoded mappings |
| `mappings_for_line(line) -> &[Mapping]` | Slice of mappings for a single generated line |
| `source(index) -> &str` | Resolve a source index to its filename |
| `get_source(index) -> Option<&str>` | Resolve a source index to its filename, or `None` if out of bounds |
| `name(index) -> &str` | Resolve a name index to its string |
| `get_name(index) -> Option<&str>` | Resolve a name index to its string, or `None` if out of bounds |
| `source_index(name) -> Option<u32>` | Look up a source filename to its index |
| `line_count() -> usize` | Number of generated lines |
| `mapping_count() -> usize` | Total number of decoded mappings |
| `has_range_mappings() -> bool` | Whether any mappings are range mappings |
| `range_mapping_count() -> usize` | Number of range mappings |
| `encode_mappings() -> String` | Encode all mappings to a VLQ mappings string |
| `encode_range_mappings() -> Option<String>` | Encode range mappings to VLQ string |
| `to_json() -> String` | Serialize the source map to JSON |
| `to_json_with_options(exclude_content) -> String` | Serialize to JSON, optionally excluding `sourcesContent` |

### `LazySourceMap`

| Method | Description |
|--------|-------------|
| `from_json(json) -> Result<LazySourceMap>` | Parse JSON eagerly but defer VLQ decoding |
| `from_json_no_content(json) -> Result<LazySourceMap>` | Parse JSON without allocating `sourcesContent` strings |
| `from_json_fast(json) -> Result<LazySourceMap>` | Fast-scan mode: skips `sourcesContent`, only byte-scans for semicolons |
| `from_vlq(mappings, sources, names, ...) -> Result<LazySourceMap>` | Build from pre-parsed components and a VLQ mappings string |
| `decode_line(line) -> Result<Vec<Mapping>>` | Decode mappings for a single generated line on demand |
| `original_position_for(line, col) -> Option<OriginalLocation>` | Look up original position (decodes the line lazily) |
| `line_count() -> usize` | Number of generated lines |
| `source(index) -> &str` | Resolve a source index to its filename |
| `get_source(index) -> Option<&str>` | Resolve a source index to its filename, or `None` if out of bounds |
| `name(index) -> &str` | Resolve a name index to its string |
| `get_name(index) -> Option<&str>` | Resolve a name index to its string, or `None` if out of bounds |
| `source_index(name) -> Option<u32>` | Look up a source filename to its index |
| `mappings_for_line(line) -> Vec<Mapping>` | Decoded mappings for a single generated line |
| `into_sourcemap() -> Result<SourceMap>` | Fully decode and convert to a `SourceMap` |

### Types

| Type | Description |
|------|-------------|
| `Mapping` | A single decoded mapping entry (28 bytes) |
| `OriginalLocation` | Result of a forward lookup (source, line, column, name) |
| `GeneratedLocation` | Result of a reverse lookup (line, column) |
| `MappedRange` | Original start/end positions for a generated range |
| `Bias` | Search bias: `GreatestLowerBound` or `LeastUpperBound` |
| `MappingsIter<'a>` | Zero-allocation streaming iterator over VLQ-encoded mappings |
| `SourceMappingUrl` | Parsed sourceMappingURL: `Inline(String)` or `External(String)` |
| `ParseError` | Errors that can occur during source map parsing |

### Free Functions

| Function | Description |
|----------|-------------|
| `parse_source_mapping_url(source) -> Option<SourceMappingUrl>` | Extract and parse a `sourceMappingURL` comment from source code |
| `validate_deep(sm) -> Vec<String>` | Deep structural validation with bounds and ordering checks |

### Fields

| Field | Type |
|-------|------|
| `sources` | `Vec<String>` |
| `sources_content` | `Vec<Option<String>>` |
| `names` | `Vec<String>` |
| `file` | `Option<String>` |
| `source_root` | `Option<String>` |
| `ignore_list` | `Vec<u32>` |
| `debug_id` | `Option<String>` |
| `extensions` | `HashMap<String, serde_json::Value>` |

## Features

- **O(log n) binary search** for both forward and reverse lookups
- **Flat 28-byte mapping structs** for cache-friendly iteration
- **Lazy reverse index** built on first `generated_position_for` call
- **Indexed source maps** (`sections`) with automatic flattening
- **Zero-copy JSON parsing** via borrowed mappings string
- **Range mappings** (`rangeMappings` field, ECMA-426 Stage 2) with cross-line delta lookup
- **Lazy iterator** (`MappingsIter`) for zero-allocation streaming over encoded mappings
- **Robust error handling** for malformed input

## Performance

Benchmarked on a 100K segment source map (Criterion):

| Operation | Time |
|-----------|------|
| Parse | 701 us |
| Single lookup | 3 ns |
| 1000x lookups | 5.8 us (5.8 ns/lookup) |

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

This crate is the core of the srcmap source map toolkit. See also:
- [`srcmap-codec`](https://crates.io/crates/srcmap-codec) - VLQ encode/decode
- [`srcmap-generator`](https://crates.io/crates/srcmap-generator) - Source map builder
- [`srcmap-remapping`](https://crates.io/crates/srcmap-remapping) - Concatenation and composition

## License

MIT
