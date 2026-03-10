# srcmap

[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/BartWaardenburg/srcmap/badges/coverage.json)](https://github.com/BartWaardenburg/srcmap/actions/workflows/coverage.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

High-performance source map tooling in Rust, with first-class Node.js bindings via NAPI and WebAssembly.

Built for the tools that power modern JavaScript: bundlers, compilers, minifiers, and dev servers.

## Why srcmap?

Source maps are on the critical path of every build. Existing Rust implementations are either tightly coupled to specific tools (oxc, parcel, swc) or lack key features. srcmap provides a **standalone**, **spec-compliant**, **fast** foundation that any tool can build on.

| Feature | srcmap | sourcemap | oxc_sourcemap | parcel_sourcemap |
|---------|--------|-----------|---------------|------------------|
| Standalone crate | **yes** | yes | no (Oxc-coupled) | no (Parcel-coupled) |
| Parse + consume | **yes** | yes | yes | yes |
| Generate | **yes** | yes | yes | yes |
| Composition/remapping | **yes** | no | no | no |
| Concatenation | **yes** | no | yes | yes |
| NAPI bindings | **yes** | no | no | no |
| WASM bindings | **yes** | no | no | yes |
| Indexed source maps | **yes** | yes | no | no |
| ECMA-426 compliant | **yes** | partial | partial | partial |

## Performance

### Rust core

Benchmarked against `@jridgewell/trace-mapping`, the fastest JavaScript implementation (used by Vite, Rollup, Webpack, and most modern bundlers).

| Operation | srcmap (Rust) | trace-mapping (JS) | Speedup |
|-----------|--------------|-------------------|---------|
| Single lookup | **3 ns** | 24 ns | **8x faster** |
| 1000x lookups | **5.8 μs** | 15 μs | **2.6x faster** |
| Parse 100K segments | 701 μs | 326 μs | 0.5x (V8 JSON.parse advantage) |

### Node.js WASM batch API

The WASM batch API amortizes FFI overhead across many lookups — the recommended path for Node.js consumers performing bulk operations.

| Operation | srcmap WASM batch | trace-mapping (JS) | Speedup |
|-----------|------------------|-------------------|---------|
| 1000x lookup (medium) | **12.9 μs** | 14.9 μs | **1.15x faster** |
| 1000x lookup (large) | **14.8 μs** | 22.0 μs | **1.49x faster** |
| Per lookup (amortized) | **13–15 ns** | 15–22 ns | **~1.3x faster** |

> Parse is slower than trace-mapping because V8's `JSON.parse` is a 15-year-old C++ machine. The Rust VLQ decoder itself is highly optimized with a single-char fast path covering ~85% of real-world values.

## Architecture

```
crates/
├── codec         # VLQ encode/decode primitives (srcmap-codec)
├── sourcemap     # Parser + consumer with O(log n) lookups (srcmap-sourcemap)
├── generator     # Incremental source map builder (srcmap-generator)
└── remapping     # Concatenation + composition/remapping (srcmap-remapping)

packages/
├── codec             # @srcmap/codec — NAPI bindings for codec
├── sourcemap         # @srcmap/sourcemap — NAPI bindings for parser
└── sourcemap-wasm    # @srcmap/sourcemap-wasm — WASM bindings for parser
```

## Usage

### Rust

Add to your `Cargo.toml`:

```toml
[dependencies]
srcmap-sourcemap = "0.1"
srcmap-generator = "0.1"
srcmap-remapping = "0.1"       # concatenation + composition
srcmap-codec = "0.1"           # only if you need raw VLQ encode/decode
```

#### Parse and look up positions

```rust
use srcmap_sourcemap::SourceMap;

let sm = SourceMap::from_json(json_string)?;

// Original position for generated line 42, column 10 (0-based)
if let Some(loc) = sm.original_position_for(42, 10) {
    println!("{}:{}:{}", sm.source(loc.source), loc.line + 1, loc.column);
    if let Some(name_idx) = loc.name {
        println!("name: {}", sm.name(name_idx));
    }
}

// Reverse lookup: generated position for an original position
if let Some(pos) = sm.generated_position_for("src/app.ts", 10, 0) {
    println!("generated: {}:{}", pos.line, pos.column);
}
```

#### Generate source maps

```rust
use srcmap_generator::SourceMapGenerator;

let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));

let src = builder.add_source("src/app.ts");
builder.set_source_content(src, source_code.to_string());

let name = builder.add_name("handleClick");
builder.add_named_mapping(
    0, 0,    // generated line, column
    src,     // source index
    10, 4,   // original line, column
    name,    // name index
);

let json = builder.to_json(); // Standard source map v3 JSON
```

#### VLQ codec

```rust
use srcmap_codec::{decode, encode, vlq_decode, vlq_encode};

// Decode a mappings string into structured data
let mappings = decode("AAAA;AACA,EAAE")?;

// Encode back to VLQ string
let encoded = encode(&mappings);

// Low-level VLQ primitives
let (value, bytes_read) = vlq_decode(b"AAAA", 0)?;
let mut buf = Vec::new();
vlq_encode(&mut buf, 42);
```

#### Concatenation + remapping

```rust
use srcmap_remapping::{ConcatBuilder, remap};
use srcmap_sourcemap::SourceMap;

// Concatenate source maps from multiple bundled files
let mut builder = ConcatBuilder::new(Some("bundle.js".to_string()));
builder.add_map(&chunk_a_map, 0);      // chunk A starts at line 0
builder.add_map(&chunk_b_map, 1000);   // chunk B starts at line 1000
let concat_map = builder.build();

// Compose source maps through multiple transforms
// (e.g. TS → JS → minified, producing TS → minified)
let result = remap(&minified_map, |source| {
    load_upstream_sourcemap(source)  // return Option<SourceMap>
});
```

### Node.js (NAPI)

```js
import { SourceMap } from '@srcmap/sourcemap';

const sm = new SourceMap(jsonString);

// Single lookup
const loc = sm.originalPositionFor(42, 10);
// → { source: 'src/app.ts', line: 10, column: 4, name: 'handleClick' }

// Batch lookup — amortizes NAPI overhead
const positions = [42, 10, 43, 0, 44, 5]; // [line, col, line, col, ...]
const results = sm.originalPositionsFor(positions);
// → [srcIdx, line, col, nameIdx, ...] — flat Int32Array, -1 = no mapping
```

### Node.js (WASM) — recommended for bulk lookups

```js
import { SourceMap } from '@srcmap/sourcemap-wasm';

const sm = new SourceMap(jsonString);

// Batch API — fastest path, beats trace-mapping by 1.3–1.5x
const positions = new Int32Array([42, 10, 43, 0, 44, 5]);
const results = sm.originalPositionsFor(positions);
// → Int32Array [srcIdx, line, col, nameIdx, ...]

// Resolve indices to strings
const source = sm.source(results[0]);
const name = results[3] >= 0 ? sm.name(results[3]) : null;
```

## Spec conformance

srcmap targets full compliance with [ECMA-426](https://tc39.es/ecma426/) (Source Map v3):

- All standard fields: `version`, `file`, `sourceRoot`, `sources`, `sourcesContent`, `names`, `mappings`
- `ignoreList` for filtering third-party sources (Chrome DevTools, Sentry)
- Indexed source maps with `sections` — flattened with source/name deduplication
- Proper `sourceRoot` resolution
- Robust error handling for malformed input (invalid base64, truncated VLQ, overflow)

## Internals

Key design decisions that make srcmap fast:

- **Flat Mapping struct** — 24 bytes (6 × u32), cache-friendly contiguous layout
- **Inlined VLQ decoder** — single-char fast path for values −15..15 (covers ~85% of real-world VLQ values), eliminates function call overhead
- **Lazy reverse index** — only built on first `generated_position_for` call, so parse-only workloads pay zero cost
- **Binary search lookups** — O(log n) for both forward and reverse position queries
- **Borrowed deserialization** — `mappings` string is borrowed from the JSON input, avoiding a large string copy
- **Pre-counted capacity** — segment and line counts estimated before allocation

## Roadmap

See [ROADMAP.md](ROADMAP.md) for the full development plan. Current status:

- [x] **Phase 0**: Hardened VLQ codec with error handling and safety guards
- [x] **Phase 1**: VLQ codec crate + NAPI bindings
- [x] **Phase 2**: Source map parser + consumer with NAPI and WASM bindings
- [x] **Phase 3**: Source map generator
- [x] **Phase 4**: Concatenation + composition/remapping (first standalone Rust implementation)
- [ ] **Phase 5**: CLI tool, streaming decode, scopes proposal

## Development

```bash
# Run all Rust tests (127 tests)
cargo test --workspace

# Run Criterion benchmarks
cargo bench -p srcmap-codec
cargo bench -p srcmap-sourcemap
cargo bench -p srcmap-generator

# Build NAPI packages
cd packages/sourcemap && npm run build
cd packages/codec && npm run build

# Build WASM package
cd packages/sourcemap-wasm && wasm-pack build --target nodejs

# Run JS benchmarks
cd benchmarks && npm install && npm run bench
```

## License

MIT
