# srcmap-sourcemap

[![crates.io](https://img.shields.io/crates/v/srcmap-sourcemap.svg)](https://crates.io/crates/srcmap-sourcemap)
[![docs.rs](https://docs.rs/srcmap-sourcemap/badge.svg)](https://docs.rs/srcmap-sourcemap)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/BartWaardenburg/srcmap/badges/rust-coverage.json)](https://github.com/BartWaardenburg/srcmap/actions/workflows/coverage.yml)

High-performance source map parser and consumer for Rust.

Parses source map JSON and provides O(log n) position lookups. Implements the [ECMA-426](https://tc39.es/ecma426/) Source Map v3 specification. Drop-in Rust equivalent of [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping).

## Install

```toml
[dependencies]
srcmap-sourcemap = "0.1"
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
| `from_json(json) -> Result<SourceMap>` | Parse a source map from a JSON string |
| `original_position_for(line, col) -> Option<OriginalLocation>` | Look up original position for a generated position |
| `generated_position_for(source, line, col) -> Option<GeneratedPosition>` | Reverse lookup |
| `all_mappings() -> &[Mapping]` | Iterate all decoded mappings |
| `source(index) -> &str` | Resolve a source index to its filename |
| `name(index) -> &str` | Resolve a name index to its string |
| `line_count() -> usize` | Number of generated lines |
| `mapping_count() -> usize` | Total number of decoded mappings |

### Fields

| Field | Type |
|-------|------|
| `sources` | `Vec<String>` |
| `sources_content` | `Vec<Option<String>>` |
| `names` | `Vec<String>` |
| `file` | `Option<String>` |
| `source_root` | `Option<String>` |
| `ignore_list` | `Vec<u32>` |

## Features

- **O(log n) binary search** for both forward and reverse lookups
- **Flat 24-byte mapping structs** for cache-friendly iteration
- **Lazy reverse index** built on first `generated_position_for` call
- **Indexed source maps** (`sections`) with automatic flattening
- **Zero-copy JSON parsing** via borrowed mappings string
- **Robust error handling** for malformed input

## Performance

Benchmarked on a 100K segment source map (Criterion):

| Operation | Time |
|-----------|------|
| Parse | 701 us |
| Single lookup | 3 ns |
| 1000x lookups | 5.8 us (5.8 ns/lookup) |

## Part of [srcmap](https://github.com/BartWaardenburg/srcmap)

This crate is the core of the srcmap source map toolkit. See also:
- [`srcmap-codec`](https://crates.io/crates/srcmap-codec) - VLQ encode/decode
- [`srcmap-generator`](https://crates.io/crates/srcmap-generator) - Source map builder
- [`srcmap-remapping`](https://crates.io/crates/srcmap-remapping) - Concatenation and composition

## License

MIT
