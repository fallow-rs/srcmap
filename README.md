# srcmap

[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)
[![crates.io](https://img.shields.io/crates/v/srcmap-sourcemap.svg)](https://crates.io/crates/srcmap-sourcemap)
[![docs.rs](https://docs.rs/srcmap-sourcemap/badge.svg)](https://docs.rs/srcmap-sourcemap)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![ECMA-426](https://img.shields.io/badge/ECMA--426-core%20%2B%20proposals-44cc11.svg)](https://tc39.es/ecma426/)

> A source map toolkit for Rust: parse, generate, compose, symbolicate and inspect. It supports the [ECMA-426](https://tc39.es/ecma426/) core and selected draft proposals.

srcmap is a set of Rust crates for source maps. A bundler, compiler, minifier, linter or symbolication service can embed them. The same code is available in Node.js as WASM and NAPI packages, and in the shell as a CLI.

```
srcmap-sourcemap      Parser + consumer
srcmap-generator      Incremental source map builder
srcmap-remapping      Concatenation + composition through transform chains
srcmap-scopes         ECMA-426 scopes & variables (draft proposal)
srcmap-symbolicate    Stack trace symbolication
srcmap-hermes         Hermes/React Native source map extensions
srcmap-ram-bundle     React Native RAM bundle parser
srcmap-codec          VLQ encode/decode
srcmap-cli            CLI with structured JSON output
```

Most users start with `srcmap-sourcemap`. Add `srcmap-generator` if you produce maps, `srcmap-remapping` if you compose them.

```toml
[dependencies]
srcmap-sourcemap = "0.5"
srcmap-generator = "0.5"    # if you produce source maps
srcmap-remapping = "0.5"    # if you compose/concatenate source maps
```

> srcmap is pre-1.0. The parsing and lookup APIs are stable. The generator and remapping APIs can still change.

## Where srcmap fits

- **A Rust tool that needs more than parse and lookup.** srcmap composes maps through a transform chain, concatenates them, reads indexed maps, and symbolicates stack traces.
- **Debug metadata from the draft proposals.** `srcmap-scopes` reads and writes the ECMA-426 scopes proposal. The parser and generator handle `rangeMappings` and `debugId`.
- **React Native.** `srcmap-hermes` reads the Hermes extensions, and `srcmap-ram-bundle` reads RAM bundles.
- **Many lookups on one map.** A symbolication or coverage service can send positions in batches (see [Performance](#performance)).
- **Inspection from the shell.** The CLI validates, decodes, composes and symbolicates, with JSON output.

srcmap is not a faster `@jridgewell/trace-mapping`. In Node.js, that package parses faster and does a single lookup faster. If your JavaScript code does one lookup per call, keep it. `@srcmap/remapping` is the exception: it is faster than `@jridgewell/remapping` on recorded Vite maps (see [Performance](#performance)).

## How it compares

<!-- oxc_sourcemap column checked against v10.0.0 in November 2026. The other columns are as of March 2026. -->
| | srcmap | [sourcemap] (Sentry) | [oxc_sourcemap] | [parcel_sourcemap] |
|---|---|---|---|---|
| Parse + consume | yes | yes | yes | yes |
| Generate | yes | yes | yes | yes |
| Composition/remapping | yes | limited | yes | yes |
| Concatenation | yes | no | yes | yes |
| Indexed source maps | yes | yes | no | no |
| Scopes proposal | yes | no | no | no |
| Stack trace symbolication | yes | yes | no | no |
| Hermes/React Native | yes | yes | no | no |
| RAM bundle parsing | yes | no | no | no |

[sourcemap]: https://crates.io/crates/sourcemap
[oxc_sourcemap]: https://crates.io/crates/oxc_sourcemap
[parcel_sourcemap]: https://crates.io/crates/parcel-sourcemap

The table compares features. This repository has no speed benchmark against these crates.

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
// Your tool ran TS to JS to minified, and each step produced a map.
// remap() walks the minified map and resolves each position
// through the upstream maps. The result is one TS-to-minified map.
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
- Unrecognized properties do not fail the parse
- A typed `ParseError` for malformed input

Draft proposal support:

- `debugId` (ECMA-426 draft proposal, not part of the published core standard)
- Scopes & variables via [`srcmap-scopes`](https://crates.io/crates/srcmap-scopes) (the [ECMA-426 scopes proposal](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md))
- `rangeMappings` (ECMA-426 Stage 2 proposal)

## Performance

In Rust, a lookup is a binary search in a flat array, with no call into another runtime. `cargo bench -p srcmap-sourcemap` prints the parse and lookup times on your machine.

In Node.js, every lookup is a call into WASM or NAPI. The repository benchmarks compare the bindings with [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping) and [`source-map-js`](https://github.com/7rulnik/source-map-js) on the source maps of Preact, Chart.js and PDF.js:

| Operation | Result |
|---|---|
| Parse | `@jridgewell/trace-mapping` is faster than the WASM binding on all three maps. The NAPI binding ties on the Preact map and is slower on the other two. |
| Single lookup | `@jridgewell/trace-mapping` is faster on all three maps. `source-map-js` is also faster than the srcmap bindings. |
| 1000 lookups in one call | The WASM batch API is faster than `@jridgewell/trace-mapping` on the two larger maps, and slower on the Preact map. |

The batch API sends all positions in one `Int32Array`, so the 1000 lookups need one call into WASM.

To reproduce, run these commands from the repository root:

```bash
corepack pnpm --dir benchmarks run download-fixtures
corepack pnpm --dir benchmarks run bench:real-world
```

`benchmarks/vite-workload.mjs` replays the maps that a real Vite run combines. On three of four recorded Vite 8 runs, `@srcmap/remapping` is 1.3 to 1.7 times faster than `@jridgewell/remapping` on the first pass, with the same output. The fourth run has too few maps to compare. [benchmarks/vite-workload/README.md](benchmarks/vite-workload/README.md) describes how to record a run.

## Node.js bindings

srcmap has WASM and NAPI bindings for Node.js. They fit a symbolication service, an error monitor, or any code that does many lookups per map.

### Choosing a package

| Your code | Package |
|---|---|
| Does many lookups per map, or needs the full parser API | [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) |
| Symbolicates stack traces | [`@srcmap/symbolicate-wasm`](https://www.npmjs.com/package/@srcmap/symbolicate-wasm) |
| Uses the `@jridgewell/trace-mapping` API | [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) |
| Uses the `@jridgewell/gen-mapping` API | [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) |
| Uses the `@jridgewell/remapping` or `@ampproject/remapping` API | [`@srcmap/remapping`](https://www.npmjs.com/package/@srcmap/remapping) |
| Uses the Mozilla `source-map` v0.6 API | [`@srcmap/source-map`](https://www.npmjs.com/package/@srcmap/source-map) |

The last four packages copy the API of an existing package. Use them when a Node.js tool and a Rust tool must give the same results. Only `@srcmap/remapping` is also faster than the package it copies. Do not choose the other three for speed.

### Quick start (Node.js)

```js
// The API of @jridgewell/trace-mapping, with one extra call to release WASM memory
import { TraceMap, originalPositionFor } from '@srcmap/trace-mapping'

const map = new TraceMap(sourceMapJsonOrObject)
const pos = originalPositionFor(map, { line: 43, column: 10 })
// { source: 'src/app.ts', line: 11, column: 4, name: 'handleClick' }

map.free() // Release WASM memory (or use `using` with Symbol.dispose)
```

### Published npm packages

| Package | Description |
|---|---|
| [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) | The `@jridgewell/trace-mapping` API (WASM) |
| [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) | The `@jridgewell/gen-mapping` API (WASM) |
| [`@srcmap/source-map`](https://www.npmjs.com/package/@srcmap/source-map) | The Mozilla `source-map` v0.6 API (WASM) |
| [`@srcmap/remapping`](https://www.npmjs.com/package/@srcmap/remapping) | The `@jridgewell/remapping` and `@ampproject/remapping` API (WASM) |
| [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) | Parser + consumer (WASM) |
| [`@srcmap/generator-wasm`](https://www.npmjs.com/package/@srcmap/generator-wasm) | Source map builder (WASM) |
| [`@srcmap/remapping-wasm`](https://www.npmjs.com/package/@srcmap/remapping-wasm) | Concatenation + composition (WASM) |
| [`@srcmap/symbolicate-wasm`](https://www.npmjs.com/package/@srcmap/symbolicate-wasm) | Stack trace symbolication (WASM) |
| [`@srcmap/sourcemap`](https://www.npmjs.com/package/@srcmap/sourcemap) | Parser + consumer (NAPI) |
| [`@srcmap/codec`](https://www.npmjs.com/package/@srcmap/codec) | VLQ codec (NAPI) |

### Experimental bindings

The Rust workspace also has these experimental binding crates. The project does not publish them to npm:

| Workspace crate | Status |
|---|---|
| `srcmap-generator-napi` in `packages/generator` | Experimental NAPI binding, unpublished |
| `srcmap-remapping-napi` in `packages/remapping` | Experimental NAPI binding, unpublished. The `@srcmap/remapping` JavaScript wrapper in the same directory is published separately. |
| `srcmap-scopes-wasm` in `packages/scopes-wasm` | Experimental WASM binding, unpublished |

## CLI

```bash
cargo install srcmap-cli

srcmap info bundle.js.map --json            # Inspect metadata and statistics
srcmap validate bundle.js.map --json        # Validate a source map
srcmap lookup bundle.js.map 42 10 --context 5 --json  # Original position with source context
srcmap resolve bundle.js.map --source src/app.ts 10 0 --json  # Reverse lookup
srcmap mappings bundle.js.map --limit 100 --json              # List mappings
srcmap decode "AAAA;AACA" --compact         # Decode VLQ mappings string
srcmap encode mappings.json --json          # Encode back to VLQ
srcmap concat a.js.map b.js.map -o bundle.js.map              # Concatenate
srcmap remap minified.js.map --dir ./maps -o composed.js.map  # Compose
srcmap symbolicate stack.txt --dir ./maps --json               # Symbolicate
srcmap scopes bundle.js.map --json          # Inspect ECMA-426 scopes & bindings
srcmap fetch https://cdn.example.com/app.min.js -o ./debug     # Fetch bundle + source map
srcmap sources bundle.js.map --extract -o ./src                # Extract original sources
srcmap schema                               # All commands as JSON (for agents)
```

All commands except `decode` and `schema` support `--json` for structured output.

## Implementation notes

- A mapping is a flat 28-byte struct (6 × u32 and a bool) in one contiguous array.
- The VLQ decoder has a fast path for a value that fits in one character.
- The parser builds the reverse index on the first `generated_position_for` call.
- Forward and reverse lookups are binary searches.
- The parser borrows the `mappings` string from the JSON input.
- The parser estimates the segment and line counts before it allocates.

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
# Install the pinned workspace dependencies
corepack enable
corepack pnpm install --frozen-lockfile

# WASM packages
corepack pnpm --filter @srcmap/sourcemap-wasm build:all
corepack pnpm --filter @srcmap/generator-wasm build:all
corepack pnpm --filter @srcmap/remapping-wasm build:all
corepack pnpm --filter @srcmap/symbolicate-wasm build:all

# NAPI packages
corepack pnpm run build:test-artifacts:napi

# JS benchmarks
corepack pnpm --dir benchmarks run download-fixtures
corepack pnpm --dir benchmarks run bench:real-world
```

</details>

## License

MIT
