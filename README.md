# srcmap

[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/BartWaardenburg/srcmap/badges/coverage.json)](https://github.com/BartWaardenburg/srcmap/actions/workflows/coverage.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-f74c00.svg?logo=rust)](https://www.rust-lang.org/)
[![ECMA-426](https://img.shields.io/badge/ECMA--426-compliant-44cc11.svg)](https://tc39.es/ecma426/)

> High-performance source map tooling in Rust, with first-class Node.js bindings via NAPI and WebAssembly.

Built for the tools that power modern JavaScript: bundlers, compilers, minifiers, and dev servers.

## Why srcmap?

Source maps are on the critical path of every build. Existing Rust implementations are either tightly coupled to specific tools (oxc, parcel, swc) or lack key features. srcmap provides a **standalone**, **spec-compliant**, **fast** foundation that any tool can build on.

| Feature | srcmap | [sourcemap] | [oxc_sourcemap] | [parcel_sourcemap] |
|---------|--------|-------------|-----------------|---------------------|
| Standalone crate | **yes** | yes | no (Oxc-coupled) | no (Parcel-coupled) |
| Parse + consume | **yes** | yes | yes | yes |
| Generate | **yes** | yes | yes | yes |
| Composition/remapping | **yes** | no | no | no |
| Concatenation | **yes** | no | yes | yes |
| NAPI bindings | **yes** | no | no | no |
| WASM bindings | **yes** | no | no | yes |
| Indexed source maps | **yes** | yes | no | no |
| ECMA-426 compliant | **yes** | partial | partial | partial |

[sourcemap]: https://crates.io/crates/sourcemap
[oxc_sourcemap]: https://crates.io/crates/oxc_sourcemap
[parcel_sourcemap]: https://crates.io/crates/parcel-sourcemap

## Performance

Benchmarked against [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping) (used by Vite, Rollup, Webpack) and [`source-map-js`](https://github.com/nicolo-ribaudo/source-map-js) (used by PostCSS, Vite CSS), using real-world source maps from popular open source projects:

| Source map | Size | Segments | Lines | Sources |
|-----------|------|----------|-------|---------|
| [Preact](https://preactjs.com/) | 82 KB | 2,775 | 1 | 12 |
| [Chart.js](https://www.chartjs.org/) | 988 KB | 83,942 | 11,467 | 53 |
| [PDF.js](https://mozilla.github.io/pdf.js/) | 5.0 MB | 410,455 | 56,284 | 110 |

Run `cd benchmarks && npm run download-fixtures && npm run bench:real-world` to reproduce. Numbers below are from an Apple M-series machine — results will vary by hardware.

### Parsing

trace-mapping is fastest at parsing. V8's native `JSON.parse` is highly optimized C++ and hard to beat from WASM/NAPI where JSON must cross a serialization boundary.

| Source map | trace-mapping | source-map-js | srcmap WASM | srcmap NAPI |
|-----------|--------------|--------------|-------------|-------------|
| Preact (82 KB) | **0.06 ms** | 0.06 ms | 0.41 ms | 0.06 ms |
| Chart.js (988 KB) | **0.69 ms** | 0.79 ms | 2.57 ms | 1.54 ms |
| PDF.js (5.0 MB) | **3.56 ms** | 4.27 ms | 23.08 ms | 7.84 ms |

### Single lookup

For individual `originalPositionFor` calls, trace-mapping is fastest — pure JS with zero FFI overhead and pre-sorted arrays. srcmap's per-call cost is dominated by the WASM/NAPI boundary crossing (~500–1000 ns overhead), not the actual lookup.

| Source map | trace-mapping | source-map-js | srcmap WASM | srcmap NAPI |
|-----------|--------------|--------------|-------------|-------------|
| Preact | **26 ns** | 177 ns | 898 ns | 531 ns |
| Chart.js | **26 ns** | 318 ns | 1,010 ns | 536 ns |
| PDF.js | **25 ns** | 257 ns | 809 ns | 385 ns |

**If your workload is parsing a source map and doing a handful of lookups, use trace-mapping** — it's excellent and has no FFI overhead.

### Batch lookup (1000 lookups per call)

This is where srcmap shines. The WASM batch API sends all positions in a single `Int32Array`, performing 1000 lookups in one boundary crossing. This eliminates per-call FFI overhead and is ideal for bulk operations like **stack trace symbolication**, **code coverage mapping**, and **error monitoring pipelines**.

| Source map | trace-mapping | source-map-js | srcmap WASM batch | srcmap NAPI batch |
|-----------|--------------|--------------|-------------------|-------------------|
| Preact (82 KB) | 18.5 μs | 206.6 μs | **20.7 μs** | 186.0 μs |
| Chart.js (988 KB) | 17.2 μs | 328.1 μs | **11.6 μs** | 162.2 μs |
| PDF.js (5.0 MB) | 16.6 μs | 368.6 μs | **12.1 μs** | 172.7 μs |

Per-lookup amortized cost on a large map: **12 ns** (WASM batch) vs 17 ns (trace-mapping) — **1.4x faster**.

> On small maps where the overhead is a larger fraction of total work, trace-mapping and srcmap WASM batch are roughly equivalent. The advantage grows with map size.

### Rust core (Criterion)

For Rust consumers — build tools, compilers, and bundlers written in Rust — there is no FFI overhead:

| Operation | srcmap (Rust) | trace-mapping (JS) | Speedup |
|-----------|--------------|-------------------|---------|
| Single lookup | **3 ns** | 24 ns | **8x faster** |
| 1000x lookups | **5.5 μs** | 15 μs | **2.7x faster** |
| Parse 100K segments | 718 μs | 326 μs | 0.5x |

> Parse is slower because V8's `JSON.parse` is highly optimized C++. The Rust VLQ decoder itself is fast — a single-char fast path covers ~85% of real-world values. Stripping `sourcesContent` (parse mappings + metadata only) brings parse down to 531 μs.

### When to use what

| Use case | Recommendation |
|----------|---------------|
| Rust build tool / bundler / compiler | **srcmap crate** — 3 ns lookups, full feature set |
| Few lookups from Node.js (dev server, single error) | **trace-mapping** — fastest for individual calls |
| Bulk lookups from Node.js (stack traces, coverage, monitoring) | **srcmap WASM batch** — 1.4x faster at scale |
| Drop-in trace-mapping replacement | **@srcmap/trace-mapping** — same API, WASM-powered |
| Need generation, remapping, or scopes | **srcmap** — only standalone Rust lib with all features |

## Architecture

```
crates/
├── codec         # VLQ encode/decode primitives (srcmap-codec)
├── sourcemap     # Parser + consumer with O(log n) lookups (srcmap-sourcemap)
├── generator     # Incremental source map builder (srcmap-generator)
├── remapping     # Concatenation + composition/remapping (srcmap-remapping)
├── scopes        # ECMA-426 scopes & variables encode/decode (srcmap-scopes)
└── cli           # CLI tool with structured JSON output (srcmap-cli)

packages/
├── codec             # @srcmap/codec — NAPI bindings for codec
├── sourcemap         # @srcmap/sourcemap — NAPI bindings for parser
├── sourcemap-wasm    # @srcmap/sourcemap-wasm — WASM bindings for parser
├── generator-wasm    # @srcmap/generator-wasm — WASM bindings for generator
├── remapping-wasm    # @srcmap/remapping-wasm — WASM bindings for remapping
└── trace-mapping     # @srcmap/trace-mapping — drop-in trace-mapping replacement
```

## Usage

### Rust

Add to your `Cargo.toml`:

```toml
[dependencies]
srcmap-sourcemap = "0.1"
srcmap-generator = "0.1"
srcmap-remapping = "0.1"       # concatenation + composition
srcmap-scopes = "0.1"          # ECMA-426 scopes & variables
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

// Batch API — amortizes WASM overhead across many lookups
const positions = new Int32Array([42, 10, 43, 0, 44, 5]);
const results = sm.originalPositionsFor(positions);
// → Int32Array [srcIdx, line, col, nameIdx, ...]

// Resolve indices to strings
const source = sm.source(results[0]);
const name = results[3] >= 0 ? sm.name(results[3]) : null;
```

### Node.js (trace-mapping drop-in)

Drop-in replacement for `@jridgewell/trace-mapping` — same API, powered by Rust via WASM:

```js
// Replace this:
// import { TraceMap, originalPositionFor } from '@jridgewell/trace-mapping';
// With this:
import { TraceMap, originalPositionFor } from '@srcmap/trace-mapping';

const map = new TraceMap(jsonString);

// Same API — 1-based lines, 0-based columns
const pos = originalPositionFor(map, { line: 42, column: 10 });
// → { source: 'src/app.ts', line: 10, column: 4, name: 'handleClick' }

// All functions work the same
import {
  generatedPositionFor,
  allGeneratedPositionsFor,
  eachMapping,
  sourceContentFor,
  isIgnored,
  encodedMappings,
  decodedMappings,
} from '@srcmap/trace-mapping';
```

### CLI

```bash
# Install
cargo install srcmap-cli

# Inspect a source map
srcmap info bundle.js.map
srcmap info bundle.js.map --json

# Validate
srcmap validate bundle.js.map --json

# Look up original position (0-based line:column)
srcmap lookup bundle.js.map 42 10 --json

# Reverse lookup
srcmap resolve bundle.js.map --source src/app.ts 10 0 --json

# List mappings with pagination
srcmap mappings bundle.js.map --limit 100 --offset 0 --json

# Decode/encode VLQ
srcmap decode "AAAA;AACA"
echo '[[[0,0,0,0]]]' | srcmap encode --json

# Concatenate source maps
srcmap concat a.js.map b.js.map -o bundle.js.map
srcmap concat a.js.map b.js.map --dry-run --json

# Compose/remap through a transform chain
srcmap remap minified.js.map --dir ./maps -o composed.js.map
srcmap remap minified.js.map --upstream src/app.ts=app.ts.map --dry-run --json

# Agent introspection — dump all commands/args/flags as JSON
srcmap schema
```

All commands support `--json` for structured machine-readable output. Errors are returned as `{"error": "...", "code": "..."}` when `--json` is active. The `schema` command enables runtime introspection for AI agents and tooling.

## Spec conformance

srcmap targets full compliance with [ECMA-426](https://tc39.es/ecma426/) (Source Map v3):

- All standard fields: `version`, `file`, `sourceRoot`, `sources`, `sourcesContent`, `names`, `mappings`
- `ignoreList` for filtering third-party sources (Chrome DevTools, Sentry)
- Indexed source maps with `sections` — flattened with source/name deduplication
- Proper `sourceRoot` resolution
- Robust error handling for malformed input (invalid base64, truncated VLQ, overflow)
- `debugId` for associating generated files with source maps
- Scopes & variables (first Rust implementation of the ECMA-426 scopes proposal)

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

- [x] VLQ codec with error handling, safety guards, and NAPI bindings
- [x] Source map parser + consumer with NAPI and WASM bindings
- [x] Source map generator with WASM bindings
- [x] Concatenation + composition/remapping with WASM bindings
- [x] CLI tool with structured JSON output and agent introspection
- [x] Scopes & variables (first Rust implementation of the ECMA-426 scopes proposal)
- [x] Drop-in trace-mapping compatibility wrapper (`@srcmap/trace-mapping`)
- [ ] Lookup bias (LEAST_UPPER_BOUND / GREATEST_LOWER_BOUND)
- [ ] Stack trace symbolication API
- [ ] Browser WASM target

## Development

```bash
# Run all Rust tests
cargo test --workspace

# Run Criterion benchmarks
cargo bench -p srcmap-codec
cargo bench -p srcmap-sourcemap
cargo bench -p srcmap-generator

# Build NAPI packages
cd packages/sourcemap && npm run build
cd packages/codec && npm run build

# Build WASM packages
cd packages/sourcemap-wasm && wasm-pack build --target nodejs
cd packages/generator-wasm && wasm-pack build --target nodejs
cd packages/remapping-wasm && wasm-pack build --target nodejs

# Run JS benchmarks (synthetic)
cd benchmarks && npm install && npm run bench

# Run real-world benchmarks (Preact, Chart.js, PDF.js)
cd benchmarks && npm run download-fixtures && npm run bench:real-world
```

## License

MIT
