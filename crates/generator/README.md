# srcmap-generator

[![crates.io](https://img.shields.io/crates/v/srcmap-generator.svg)](https://crates.io/crates/srcmap-generator)
[![docs.rs](https://docs.rs/srcmap-generator/badge.svg)](https://docs.rs/srcmap-generator)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/BartWaardenburg/srcmap/badges/rust-coverage.json)](https://github.com/BartWaardenburg/srcmap/actions/workflows/coverage.yml)

High-performance source map generator for Rust.

Builds source maps incrementally by adding mappings one at a time. Outputs standard [ECMA-426](https://tc39.es/ecma426/) source map v3 JSON. Drop-in Rust equivalent of [`@jridgewell/gen-mapping`](https://github.com/jridgewell/gen-mapping).

## Install

```toml
[dependencies]
srcmap-generator = "0.1"
```

## Usage

```rust
use srcmap_generator::SourceMapGenerator;

let mut gen = SourceMapGenerator::new(Some("bundle.js".to_string()));

// Register sources and names
let src = gen.add_source("src/app.ts");
gen.set_source_content(src, "const x = 1;".to_string());
let name = gen.add_name("x");

// Add mappings (generated_line, generated_col, source, original_line, original_col)
gen.add_mapping(0, 0, src, 0, 6);
gen.add_named_mapping(1, 0, src, 1, 0, name);

// Skip redundant mappings automatically
gen.maybe_add_mapping(1, 5, src, 1, 0); // skipped — same source position

let json = gen.to_json();
// {"version":3,"file":"bundle.js","sources":["src/app.ts"],...}
```

## API

### `SourceMapGenerator`

| Method | Description |
|--------|-------------|
| `new(file) -> Self` | Create a new generator with optional output filename |
| `add_source(path) -> u32` | Register a source file, returns its index (deduped) |
| `set_source_content(idx, content)` | Set inline source content |
| `add_name(name) -> u32` | Register a name, returns its index (deduped) |
| `set_source_root(root)` | Set the `sourceRoot` prefix |
| `add_mapping(gen_line, gen_col, src, orig_line, orig_col)` | Add a mapping |
| `add_named_mapping(gen_line, gen_col, src, orig_line, orig_col, name)` | Add a mapping with a name |
| `add_generated_mapping(gen_line, gen_col)` | Add a generated-only mapping (no source) |
| `maybe_add_mapping(gen_line, gen_col, src, orig_line, orig_col) -> bool` | Add only if different from previous |
| `add_range_mapping(gen_line, gen_col, src, orig_line, orig_col)` | Add a range mapping (ECMA-426) |
| `add_named_range_mapping(gen_line, gen_col, src, orig_line, orig_col, name)` | Add a named range mapping |
| `add_to_ignore_list(source_idx)` | Mark a source as ignored (third-party) |
| `to_json() -> String` | Serialize to source map v3 JSON |
| `to_decoded_map() -> SourceMap` | Build a `SourceMap` directly (no JSON roundtrip) |
| `mapping_count() -> usize` | Number of mappings added |

### Parallel encoding

Enable the `parallel` feature for multi-threaded VLQ encoding via [rayon](https://crates.io/crates/rayon). Automatically used for maps with 4K+ mappings.

```toml
[dependencies]
srcmap-generator = { version = "0.1", features = ["parallel"] }
```

### `StreamingGenerator`

On-the-fly VLQ encoder that emits mappings as they are added, without collecting into a `Vec`. Ideal for composition pipelines where mappings arrive in sorted order.

| Method | Description |
|--------|-------------|
| `new(file) -> Self` | Create a new streaming generator |
| `add_source(path) -> u32` | Register a source file (deduped) |
| `add_name(name) -> u32` | Register a name (deduped) |
| `set_source_root(root)` | Set the `sourceRoot` prefix |
| `set_debug_id(id)` | Set the `debugId` field |
| `add_to_ignore_list(source_idx)` | Mark a source as ignored (third-party) |
| `add_mapping(...)` | Add a mapping (encoded immediately) |
| `add_named_mapping(...)` | Add a mapping with a name |
| `add_range_mapping(...)` | Add a range mapping (ECMA-426) |
| `add_named_range_mapping(...)` | Add a named range mapping |
| `to_json() -> String` | Serialize to source map v3 JSON |
| `to_decoded_map() -> SourceMap` | Build a `SourceMap` directly |

## Features

- **Automatic deduplication** of sources and names
- **`maybe_add_mapping`** skips redundant mappings to reduce output size
- **Range mappings** (`rangeMappings` field, ECMA-426 Stage 2)
- **Streaming generation** via `StreamingGenerator` — zero-allocation VLQ encoding
- **`ignoreList`** support for filtering third-party sources in DevTools
- **Parallel VLQ encoding** for large maps (opt-in via `parallel` feature)
- **Hand-rolled JSON serialization** — no serde overhead in output path

## Part of [srcmap](https://github.com/BartWaardenburg/srcmap)

See also:
- [`srcmap-sourcemap`](https://crates.io/crates/srcmap-sourcemap) - Parser and consumer
- [`srcmap-codec`](https://crates.io/crates/srcmap-codec) - VLQ encode/decode
- [`srcmap-remapping`](https://crates.io/crates/srcmap-remapping) - Concatenation and composition

## License

MIT
