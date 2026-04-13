# @srcmap/sourcemap-wasm

[![npm](https://img.shields.io/npm/v/@srcmap/sourcemap-wasm.svg)](https://www.npmjs.com/package/@srcmap/sourcemap-wasm)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)

High-performance source map parser and consumer powered by Rust via WebAssembly.

Parses source map JSON and provides position lookups. Implements [ECMA-426](https://tc39.es/ecma426/) (Source Map v3). The batch API is **faster than [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping)** for bulk lookups.

## Install

```bash
npm install @srcmap/sourcemap-wasm
```

Works in Node.js, browsers, and any WebAssembly-capable runtime. No native compilation required.

## Usage

```js
import { SourceMap } from '@srcmap/sourcemap-wasm';

const sm = new SourceMap(jsonString);

// Single lookup (0-based lines and columns)
const loc = sm.originalPositionFor(42, 10);
// { source: 'src/app.ts', line: 10, column: 4, name: 'handleClick' }

// Batch lookup — recommended for bulk operations
const positions = new Int32Array([42, 10, 43, 0, 44, 5]);
const results = sm.originalPositionsFor(positions);
// Int32Array [srcIdx, line, col, nameIdx, srcIdx, line, col, nameIdx, ...]
// -1 = no mapping / no name

// Resolve indices to strings
const source = sm.source(results[0]);
const name = results[3] >= 0 ? sm.name(results[3]) : null;

// Reverse lookup
const pos = sm.generatedPositionFor('src/app.ts', 10, 4);
// { line: 42, column: 10 }

// Cleanup (or use `using sm = new SourceMap(json)` with Symbol.dispose)
sm.free();
```

## Coverage offsets

For V8 production coverage workloads you often start with generated code offsets, not `line,column` pairs.
`@srcmap/sourcemap-wasm/coverage` provides a cacheable helper that converts UTF-8 offsets into generated positions and then forwards them into the batch source map APIs.
It works with both `@srcmap/sourcemap-wasm` and `@srcmap/sourcemap`.

```js
import { SourceMap } from "@srcmap/sourcemap-wasm";
import { GeneratedOffsetLookup } from "@srcmap/sourcemap-wasm/coverage";

const sm = new SourceMap(mapJson);
const lookup = new GeneratedOffsetLookup(generatedCode);

const start = lookup.generatedPositionFor(startOffset);
// { line: 120, column: 18 }

const original = lookup.originalPositionFor(sm, startOffset);
// { source, line, column, name } | null

const positions = lookup.originalPositionsFor(sm, [startOffset, endOffset]);
// Int32Array in @srcmap/sourcemap-wasm, number[] in @srcmap/sourcemap
```

Use one `GeneratedOffsetLookup` per generated asset and reuse it across beacon batches. That matches the `fallow-cloud` shape better than recomputing line starts for every flush.
The helper always feeds a plain JavaScript array into the backend batch API, so both source map packages accept the same offset input shape.

## API

### `new SourceMap(json: string)`

Parse a source map from a JSON string.

### Static methods

| Method | Returns | Description |
|--------|---------|-------------|
| `SourceMap.fromJsonNoContent(json)` | `SourceMap` | Parse JSON, skipping `sourcesContent` allocation (used by JS wrappers that keep content on the JS side) |
| `SourceMap.fromVlq(mappings, sources, names, file?, sourceRoot?, ignoreList, debugId?)` | `SourceMap` | Build from pre-parsed components. JS calls `JSON.parse()` (V8-native speed), then only the VLQ mappings string crosses into WASM |

### Instance methods

| Method | Returns | Description |
|--------|---------|-------------|
| `originalPositionFor(line, column)` | `{ source, line, column, name } \| null` | Forward lookup (0-based) |
| `originalPositionForWithBias(line, column, bias)` | `{ source, line, column, name } \| null` | Forward lookup with bias (`0` = greatest lower bound (default), `-1` = least upper bound) |
| `originalPositionFlat(line, column)` | `Int32Array` | Forward lookup returning `[sourceIdx, line, column, nameIdx]` (`-1` = no mapping/name) |
| `originalPositionBuf(line, column)` | `boolean` | Zero-allocation lookup via static buffer. Read result with `resultPtr()` / `wasmMemory()` |
| `originalPositionsFor(positions: Int32Array)` | `Int32Array` | Batch forward lookup. Input: flat `[line, col, line, col, ...]`. Output: flat `[srcIdx, line, col, nameIdx, ...]` |
| `generatedPositionFor(source, line, column)` | `{ line, column } \| null` | Reverse lookup (0-based) |
| `generatedPositionForWithBias(source, line, column, bias)` | `{ line, column } \| null` | Reverse lookup with bias (`0` = greatest lower bound (default), `-1` = least upper bound) |
| `allGeneratedPositionsFor(source, line, column)` | `Array<{ line, column }>` | All generated positions for a given original position |
| `mapRange(startLine, startCol, endLine, endCol)` | `{ source, originalStartLine, originalStartColumn, originalEndLine, originalEndColumn } \| null` | Map a generated range to its original range |
| `allMappingsFlat()` | `Int32Array` | All mappings as a flat array. Each mapping: 7 values `[genLine, genCol, srcIdx, origLine, origCol, nameIdx, isRange]` (`-1` = unmapped) |
| `encodedMappings()` | `string` | Get the VLQ-encoded mappings string |
| `encodedRangeMappings()` | `string \| null` | Get the VLQ-encoded range mappings string, or `null` if none |
| `source(index)` | `string \| null` | Resolve source index to filename. Returns `null` for out-of-bounds indices |
| `name(index)` | `string \| null` | Resolve name index to string. Returns `null` for out-of-bounds indices |
| `sourceContentFor(index)` | `string \| null` | Get source content by index. Returns `null` if index is out of bounds or content is missing |
| `isIgnoredIndex(index)` | `boolean` | Check if a source index is in the `ignoreList` |
| `free()` | `void` | Release WASM memory (also via `Symbol.dispose`) |

### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `sources` | `string[]` | All source filenames |
| `names` | `string[]` | All names |
| `sourcesContent` | `(string \| null)[]` | Source file contents (parallel to `sources`) |
| `ignoreList` | `number[]` | Source ignore list indices |
| `file` | `string \| undefined` | Output filename |
| `sourceRoot` | `string \| undefined` | Source root prefix |
| `debugId` | `string \| undefined` | ECMA-426 debug ID |
| `lineCount` | `number` | Number of generated lines |
| `mappingCount` | `number` | Total decoded mappings |
| `hasRangeMappings` | `boolean` | Whether any range mappings exist |
| `rangeMappingCount` | `number` | Number of range mappings |

---

### `new LazySourceMap(json: string)`

A fast-scan alternative to `SourceMap` that defers VLQ decoding until lookup time. On construction it only parses JSON metadata and byte-scans the mappings string for semicolons (to identify line boundaries). VLQ decoding happens per-line on demand with progressive state tracking. This makes parse time near-instant at the cost of slightly slower first lookups.

`LazySourceMap` does **not** parse `sourcesContent` -- use the fast-scan wrapper (see below) if you need it.

#### Static methods

| Method | Returns | Description |
|--------|---------|-------------|
| `LazySourceMap.fromParts(mappings, metadataJson)` | `LazySourceMap` | Build from a VLQ mappings string and a metadata JSON string (containing `sources`, `names`, `file`, `sourceRoot`, `ignoreList`, `debugId` -- no `sourcesContent` or `mappings`) |

#### Instance methods

| Method | Returns | Description |
|--------|---------|-------------|
| `originalPositionFor(line, column)` | `{ source, line, column, name } \| null` | Forward lookup (0-based). Decodes the target line on first access |
| `originalPositionFlat(line, column)` | `Int32Array` | Forward lookup returning `[sourceIdx, line, column, nameIdx]` (`-1` = no mapping/name) |
| `originalPositionBuf(line, column)` | `boolean` | Zero-allocation lookup via static buffer |
| `originalPositionsFor(positions: Int32Array)` | `Int32Array` | Batch forward lookup |
| `source(index)` | `string \| null` | Resolve source index to filename. Returns `null` for out-of-bounds indices |
| `name(index)` | `string \| null` | Resolve name index to string. Returns `null` for out-of-bounds indices |
| `isIgnoredIndex(index)` | `boolean` | Check if a source index is in the `ignoreList` |
| `free()` | `void` | Release WASM memory (also via `Symbol.dispose`) |

#### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `sources` | `string[]` | All source filenames |
| `names` | `string[]` | All names |
| `ignoreList` | `number[]` | Source ignore list indices |
| `file` | `string \| undefined` | Output filename |
| `sourceRoot` | `string \| undefined` | Source root prefix |
| `debugId` | `string \| undefined` | ECMA-426 debug ID |
| `lineCount` | `number` | Number of generated lines |

---

### Fast-scan mode (`fast.js`)

The `fast.js` entry point wraps `LazySourceMap` with lazy `sourcesContent` extraction from the original JSON string. It exposes a `SourceMap` class with the same interface you would expect, but internally:

1. Parses using fast-scan mode (no VLQ decode at parse time).
2. Keeps the raw JSON string on the JS side and only calls `JSON.parse()` to extract `sourcesContent` on first access (then releases the JSON for GC).
3. Forwards all lookups to the underlying `LazySourceMap`.

This is ideal when you only need to look up a few positions and want the fastest possible parse time.

```js
const { SourceMap } = require('@srcmap/sourcemap-wasm/pkg/fast.js');

const sm = new SourceMap(jsonString);

// Lookups work the same as the regular SourceMap
const loc = sm.originalPositionFor(42, 10);

// sourcesContent is extracted lazily from JSON on first access
const content = sm.sourceContentFor(0);

sm.free();
```

#### Instance methods

| Method | Returns | Description |
|--------|---------|-------------|
| `originalPositionFor(line, column)` | `{ source, line, column, name } \| null` | Forward lookup (0-based) |
| `originalPositionFlat(line, column)` | `Int32Array` | Forward lookup returning flat indices |
| `originalPositionBuf(line, column)` | `boolean` | Zero-allocation lookup via static buffer |
| `originalPositionsFor(positions: Int32Array)` | `Int32Array` | Batch forward lookup |
| `source(index)` | `string \| null` | Resolve source index to filename |
| `name(index)` | `string \| null` | Resolve name index to string |
| `sourceContentFor(index)` | `string \| null` | Get source content by index (triggers lazy JSON parse on first call) |
| `isIgnoredIndex(index)` | `boolean` | Check if a source index is in the `ignoreList` |
| `free()` | `void` | Release WASM memory |

#### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `sources` | `string[]` | All source filenames |
| `names` | `string[]` | All names |
| `sourcesContent` | `(string \| null)[] \| null` | Source file contents (lazily extracted from JSON on first access) |
| `ignoreList` | `number[]` | Source ignore list indices |
| `file` | `string \| undefined` | Output filename |
| `sourceRoot` | `string \| undefined` | Source root prefix |
| `debugId` | `string \| undefined` | ECMA-426 debug ID |
| `lineCount` | `number` | Number of generated lines |

## Performance

### Batch API vs trace-mapping

The batch API (`originalPositionsFor`) returns a flat `Int32Array`, avoiding per-lookup object allocation. This makes it **faster than pure JS** for bulk operations.

| Operation | @srcmap/sourcemap-wasm | @jridgewell/trace-mapping | Speedup |
|-----------|----------------------|---------------------------|---------|
| 1000x lookup (medium map) | 12.9 us | 14.9 us | **1.15x faster** |
| 1000x lookup (large map) | 14.8 us | 22.0 us | **1.49x faster** |
| Per lookup (amortized) | 13-15 ns | 15-22 ns | **~1.3x faster** |

### When to use which

| Use case | Recommended package |
|----------|-------------------|
| Batch lookups (error stacks, coverage) | **@srcmap/sourcemap-wasm** (batch API) |
| Few lookups, fast parse | **@srcmap/sourcemap-wasm** (fast-scan mode via `fast.js`) |
| Few individual lookups | `@jridgewell/trace-mapping` (lower per-call overhead) |
| Native Node.js addons | `@srcmap/sourcemap` (NAPI) |
| Browser environments | **@srcmap/sourcemap-wasm** |

## Build targets

```bash
# Node.js (default)
npm run build

# Browser (ES module + .wasm)
npm run build:web

# Bundler (e.g. webpack, vite)
npm run build:bundler
```

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

High-performance source map tooling written in Rust. See also:
- [`@srcmap/codec`](https://www.npmjs.com/package/@srcmap/codec) - VLQ codec (NAPI)
- [`@srcmap/sourcemap`](https://www.npmjs.com/package/@srcmap/sourcemap) - Source map parser (NAPI)

## License

MIT
