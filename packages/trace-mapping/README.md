# @srcmap/trace-mapping

[![npm](https://img.shields.io/npm/v/@srcmap/trace-mapping.svg)](https://www.npmjs.com/package/@srcmap/trace-mapping)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

The API of [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping) on the srcmap Rust parser, compiled to WebAssembly.

The package has the same functions and types as `@jridgewell/trace-mapping`. Code written for that package runs on this one after you change the import.

Use it when a Node.js tool and a Rust tool must give the same lookup results. Do not use it for speed. Each lookup is a call into WASM, and `@jridgewell/trace-mapping` does a single lookup in less time (see the [benchmarks](https://github.com/fallow-rs/srcmap#performance)). For many lookups on one map, use the batch API of [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm).

## Install

```bash
npm install @srcmap/trace-mapping
```

## Usage

```js
// Before:
// import { TraceMap, originalPositionFor } from '@jridgewell/trace-mapping'

// After:
import { TraceMap, originalPositionFor } from '@srcmap/trace-mapping'

const map = new TraceMap(sourceMapJsonOrObject)

// Forward lookup (1-based lines, 0-based columns)
const pos = originalPositionFor(map, { line: 43, column: 10 })
// { source: 'src/app.ts', line: 11, column: 4, name: 'handleClick' }

// Reverse lookup
const gen = generatedPositionFor(map, { source: 'src/app.ts', line: 11, column: 4 })
// { line: 43, column: 10 }

// Cleanup WASM memory (optional, also via `using` with Symbol.dispose)
map.free()
```

## API compatibility

All exports from `@jridgewell/trace-mapping` are supported:

| Export | Description |
|--------|-------------|
| `TraceMap` | Main class: parses source maps (regular, indexed, decoded) |
| `originalPositionFor(map, needle)` | Forward lookup (1-based lines, 0-based columns) |
| `generatedPositionFor(map, needle)` | Reverse lookup |
| `allGeneratedPositionsFor(map, needle)` | All generated positions for an original position |
| `eachMapping(map, callback)` | Iterate all mappings in generated order |
| `traceSegment(map, line, column)` | Low-level segment lookup (0-based) |
| `encodedMappings(map)` | Get the VLQ-encoded mappings string |
| `decodedMappings(map)` | Get decoded mappings as `SourceMapSegment[][]` |
| `sourceContentFor(map, source)` | Get source content for a file |
| `isIgnored(map, source)` | Check if a source is in the `ignoreList` |
| `presortedDecodedMap(map)` | Create from pre-sorted decoded mappings |
| `decodedMap(map)` | Export as decoded source map object |
| `encodedMap(map)` | Export as encoded source map object |
| `AnyMap` / `FlattenMap` | Aliases for `TraceMap` (indexed maps handled natively) |
| `LEAST_UPPER_BOUND` / `GREATEST_LOWER_BOUND` | Search bias constants |

### Line/column convention

Follows `@jridgewell/trace-mapping` conventions:
- `originalPositionFor` / `generatedPositionFor`: **1-based lines**, 0-based columns
- `traceSegment` / decoded mappings: **0-based lines**, 0-based columns

### TraceMap properties

| Property | Type | Description |
|----------|------|-------------|
| `version` | `3` | Source map version |
| `file` | `string \| null` | Output filename |
| `names` | `string[]` | Name strings |
| `sources` | `(string \| null)[]` | Raw source filenames |
| `sourcesContent` | `(string \| null)[]` | Source file contents |
| `sourceRoot` | `string` | Source root prefix |
| `resolvedSources` | `string[]` | Sources resolved against `sourceRoot` and map URL |
| `ignoreList` | `number[]` | Indices of sources to ignore |

## Differences from @jridgewell/trace-mapping

- **WASM memory**: Call `map.free()` when done, or use `using map = new TraceMap(...)` with `Symbol.dispose`
- **Indexed source maps**: Handled natively by the WASM engine (no JS-side flattening)
- **No `@jridgewell/sourcemap-codec` dependency**: VLQ encoding/decoding runs in WASM

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Source map tooling written in Rust. See also:
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Lower-level WASM API (0-based lines)
- [`@srcmap/generator-wasm`](https://www.npmjs.com/package/@srcmap/generator-wasm) - Source map generator (WASM)
- [`@srcmap/remapping-wasm`](https://www.npmjs.com/package/@srcmap/remapping-wasm) - Concatenation + composition (WASM)

## License

MIT
