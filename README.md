# srcmap

[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)
[![crates.io](https://img.shields.io/crates/v/srcmap-sourcemap.svg)](https://crates.io/crates/srcmap-sourcemap)
[![docs.rs](https://docs.rs/srcmap-sourcemap/badge.svg)](https://docs.rs/srcmap-sourcemap)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![ECMA-426](https://img.shields.io/badge/ECMA--426-core%20%2B%20proposals-44cc11.svg)](https://tc39.es/ecma426/)

> The source map SDK for Rust tooling. Parse, generate, remap, and compose — with stable [ECMA-426](https://tc39.es/ecma426/) core support plus selected draft proposals.

A standalone source map library that any Rust tool can embed. If you're building a bundler, compiler, minifier, linter, or symbolication service — srcmap gives you the complete source map stack so you don't have to build it yourself.

```
srcmap-sourcemap      Parser + consumer with O(log n) lookups
srcmap-generator      Incremental source map builder
srcmap-remapping      Concatenation + composition through transform chains
srcmap-scopes         ECMA-426 scopes & variables (first Rust implementation of the draft proposal)
srcmap-symbolicate    Stack trace symbolication
srcmap-hermes         Hermes/React Native source map extensions
srcmap-ram-bundle     React Native RAM bundle parser
srcmap-codec          VLQ encode/decode primitives
srcmap-cli            CLI with structured JSON output
```

Most users start with `srcmap-sourcemap`. Add `srcmap-generator` if you produce maps, `srcmap-remapping` if you compose them.

```toml
[dependencies]
srcmap-sourcemap = "0.3"
srcmap-generator = "0.3"    # if you produce source maps
srcmap-remapping = "0.3"    # if you compose/concatenate source maps
```

> srcmap is pre-1.0. The parsing and lookup APIs are stable; generator and remapping APIs may evolve.

## How it compares

<!-- Comparison as of March 2026 -->
| | srcmap | [sourcemap] (Sentry) | [oxc_sourcemap] | [parcel_sourcemap] |
|---|---|---|---|---|
| Parse + consume | **yes** | yes | yes | yes |
| Generate | **yes** | yes | yes | yes |
| Composition/remapping | **yes** | limited | no | yes |
| Concatenation | **yes** | no | yes | yes |
| Indexed source maps | **yes** | yes | no | no |
| Scopes proposal | **yes** | no | no | no |
| Stack trace symbolication | **yes** | yes | no | no |
| Hermes/React Native | **yes** | yes | no | no |
| RAM bundle parsing | **yes** | no | no | no |

[sourcemap]: https://crates.io/crates/sourcemap
[oxc_sourcemap]: https://crates.io/crates/oxc_sourcemap
[parcel_sourcemap]: https://crates.io/crates/parcel-sourcemap

All four crates can be used standalone. The difference is scope: srcmap is the only one that covers parse, generate, compose, concatenate, scopes, and symbolication in a single coherent API.

> **Composition** is the hard part. When your tool chains transforms (TypeScript → Babel → minifier), each step produces a source map. srcmap composes the full chain into a single map that traces back to the original source — with a clean `remap()` API that takes a closure to resolve upstream maps.

## Quick start

### Parse and look up positions

```rust
use srcmap_sourcemap::SourceMap;

let json_string = std::fs::read_to_string("bundle.js.map")?;
let sm = SourceMap::from_json(&json_string)?;

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

### Generate source maps

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

let json = builder.to_json();
```

### Compose through a transform chain

```rust
use srcmap_remapping::{ConcatBuilder, remap};
use srcmap_sourcemap::SourceMap;

// Concatenate source maps from multiple bundled files
let mut builder = ConcatBuilder::new(Some("bundle.js".to_string()));
builder.add_map(&chunk_a_map, 0);      // chunk A starts at line 0
builder.add_map(&chunk_b_map, 1000);   // chunk B starts at line 1000
let concat_map = builder.build();

// Compose source maps through a transform chain:
// Your tool ran TS → JS → minified, each step produced a map.
// remap() walks the minified map and resolves each position
// through the upstream maps, producing a single TS → minified map.
let composed = remap(&minified_map, |source| {
    load_upstream_sourcemap(source) // returns Option<SourceMap>
});
```

### VLQ codec

```rust
use srcmap_codec::{decode, encode, vlq_decode, vlq_encode};

let mappings = decode("AAAA;AACA,EAAE")?;
let encoded = encode(&mappings);

let (value, bytes_read) = vlq_decode(b"AAAA", 0)?;
let mut buf = Vec::new();
vlq_encode(&mut buf, 42);
```

## Spec support

Stable [ECMA-426](https://tc39.es/ecma426/) core support:

- Standard Source Map v3 fields: `version`, `file`, `sourceRoot`, `sources`, `sourcesContent`, `names`, `mappings`
- `ignoreList`
- Indexed source maps with `sections`
- `sourceRoot` resolution
- Required extension tolerance for unrecognized properties
- Robust error handling for malformed input

Draft proposal support:

- `debugId` (ECMA-426 draft proposal, not part of the published core standard)
- Scopes & variables via [`srcmap-scopes`](https://crates.io/crates/srcmap-scopes) (first Rust implementation of the [ECMA-426 scopes proposal](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md))
- `rangeMappings` (ECMA-426 Stage 2 proposal)

## Performance

For Rust consumers there is no FFI overhead. Benchmarked with Criterion:

| Operation | srcmap | trace-mapping (JS) | Speedup |
|---|---|---|---|
| Single lookup | **3 ns** | 24 ns | **8x** |
| 1000 lookups | **5.5 μs** | 15 μs | **2.7x** |
| Parse 100K segments | 718 μs | 326 μs | 0.5x |

Parse is dominated by JSON deserialization — V8's `JSON.parse` is highly optimized C++. The VLQ decoder itself is fast (single-char fast path covers ~85% of real-world values).

<details>
<summary>Node.js benchmarks (WASM/NAPI bindings)</summary>

Benchmarked against [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping) and [`source-map-js`](https://github.com/nicolo-ribaudo/source-map-js) using real-world source maps:

| Source map | Size | Segments |
|---|---|---|
| [Preact](https://preactjs.com/) | 82 KB | 2,775 |
| [Chart.js](https://www.chartjs.org/) | 988 KB | 83,942 |
| [PDF.js](https://mozilla.github.io/pdf.js/) | 5.0 MB | 410,455 |

**Parsing** — trace-mapping wins. V8's `JSON.parse` is hard to beat across an FFI boundary.

| Source map | trace-mapping | source-map-js | srcmap WASM | srcmap NAPI |
|---|---|---|---|---|
| Preact | **0.06 ms** | 0.06 ms | 0.41 ms | 0.06 ms |
| Chart.js | **0.69 ms** | 0.79 ms | 2.57 ms | 1.54 ms |
| PDF.js | **3.56 ms** | 4.27 ms | 23.08 ms | 7.84 ms |

**Single lookup** — trace-mapping wins. Pure JS with zero FFI overhead.

| Source map | trace-mapping | source-map-js | srcmap WASM | srcmap NAPI |
|---|---|---|---|---|
| Preact | **26 ns** | 177 ns | 898 ns | 531 ns |
| Chart.js | **26 ns** | 318 ns | 1,010 ns | 536 ns |
| PDF.js | **25 ns** | 257 ns | 809 ns | 385 ns |

**Batch lookup (1000 per call)** — srcmap wins. The WASM batch API sends all positions in a single `Int32Array`, amortizing the FFI boundary.

| Source map | trace-mapping | source-map-js | srcmap WASM batch | srcmap NAPI batch |
|---|---|---|---|---|
| Preact | 18.5 μs | 206.6 μs | **20.7 μs** | 186.0 μs |
| Chart.js | 17.2 μs | 328.1 μs | **11.6 μs** | 162.2 μs |
| PDF.js | 16.6 μs | 368.6 μs | **12.1 μs** | 172.7 μs |

Per-lookup amortized cost on a large map: **12 ns** (WASM batch) vs 17 ns (trace-mapping) — **1.4x faster**.

Run `cd benchmarks && npm run download-fixtures && npm run bench:real-world` to reproduce.

</details>

## Node.js bindings

srcmap ships WASM and NAPI bindings for use in Node.js — useful for symbolication services, error monitoring, and bulk source map operations.

### Choosing a package

- **Consuming source maps?** → [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) (drop-in for `@jridgewell/trace-mapping`)
- **Generating source maps?** → [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) (drop-in for `@jridgewell/gen-mapping`)
- **Using Mozilla's API?** → [`@srcmap/source-map`](https://www.npmjs.com/package/@srcmap/source-map) (drop-in for `source-map` v0.6)
- **Composing/remapping?** → [`@srcmap/remapping`](https://www.npmjs.com/package/@srcmap/remapping) (drop-in for `@ampproject/remapping`)
- **Batch lookups or low-level control?** → [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) (raw WASM API, fastest for bulk ops)

### Quick start (Node.js)

```js
// Swap the import — the rest of your code stays the same
import { TraceMap, originalPositionFor } from '@srcmap/trace-mapping'

const map = new TraceMap(sourceMapJsonOrObject)
const pos = originalPositionFor(map, { line: 43, column: 10 })
// { source: 'src/app.ts', line: 11, column: 4, name: 'handleClick' }

map.free() // Release WASM memory (or use `using` with Symbol.dispose)
```

### All packages

| Package | Description |
|---|---|
| [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) | Drop-in for `@jridgewell/trace-mapping` (WASM) |
| [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) | Drop-in for `@jridgewell/gen-mapping` (WASM) |
| [`@srcmap/source-map`](https://www.npmjs.com/package/@srcmap/source-map) | Drop-in for Mozilla `source-map` v0.6 (WASM) |
| [`@srcmap/remapping`](https://www.npmjs.com/package/@srcmap/remapping) | Drop-in for `@ampproject/remapping` (WASM) |
| [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) | Parser + consumer (WASM) |
| [`@srcmap/generator-wasm`](https://www.npmjs.com/package/@srcmap/generator-wasm) | Source map builder (WASM) |
| [`@srcmap/remapping-wasm`](https://www.npmjs.com/package/@srcmap/remapping-wasm) | Concatenation + composition (WASM) |
| [`@srcmap/symbolicate-wasm`](https://www.npmjs.com/package/@srcmap/symbolicate-wasm) | Stack trace symbolication (WASM) |
| [`@srcmap/sourcemap`](https://www.npmjs.com/package/@srcmap/sourcemap) | Parser + consumer (NAPI) |
| [`@srcmap/codec`](https://www.npmjs.com/package/@srcmap/codec) | VLQ codec (NAPI) |

## CLI

```bash
cargo install srcmap-cli

srcmap info bundle.js.map --json            # Inspect metadata and statistics
srcmap validate bundle.js.map --json        # Validate a source map
srcmap lookup bundle.js.map 42 10 --context 5 --json  # Original position with source context
srcmap resolve bundle.js.map --source src/app.ts 10 0 --json  # Reverse lookup
srcmap mappings bundle.js.map --limit 100 --json              # List mappings
srcmap decode "AAAA;AACA" --json            # Decode VLQ mappings string
srcmap encode mappings.json --json          # Encode back to VLQ
srcmap concat a.js.map b.js.map -o bundle.js.map              # Concatenate
srcmap remap minified.js.map --dir ./maps -o composed.js.map  # Compose
srcmap symbolicate stack.txt --dir ./maps --json               # Symbolicate
srcmap scopes bundle.js.map --json          # Inspect ECMA-426 scopes & bindings
srcmap fetch https://cdn.example.com/app.min.js -o ./debug     # Fetch bundle + source map
srcmap sources bundle.js.map --extract -o ./src                # Extract original sources
srcmap schema                               # All commands as JSON (for agents)
```

All commands support `--json` for structured output.

## Why srcmap is fast

- **Cache-friendly layout** — 28-byte flat Mapping struct in contiguous memory (6 × u32 + bool)
- **Single-char VLQ fast path** — ~85% of real-world values decode in one operation
- **Lazy reverse index** — only built on first `generated_position_for` call
- **Binary search** — O(log n) for both forward and reverse lookups
- **Zero-copy parsing** — `mappings` string borrowed directly from JSON input
- **Pre-counted capacity** — segment and line counts estimated before allocation

## Development

```bash
cargo test --workspace                # Run all tests
cargo bench -p srcmap-sourcemap       # Criterion benchmarks
cargo bench -p srcmap-codec
cargo bench -p srcmap-generator
cargo bench -p srcmap-remapping       # remap vs remap_streaming
```

<details>
<summary>Building WASM/NAPI packages and running JS benchmarks</summary>

```bash
# WASM packages
cd packages/sourcemap-wasm && npm run build:all
cd packages/generator-wasm && npm run build:all
cd packages/remapping-wasm && npm run build:all
cd packages/symbolicate-wasm && npm run build:all

# NAPI packages
cd packages/sourcemap && npm run build
cd packages/codec && npm run build

# JS benchmarks
cd benchmarks && npm install && npm run bench:real-world
```

</details>

## License

MIT
