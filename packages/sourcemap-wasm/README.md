# @srcmap/sourcemap-wasm

[![npm](https://img.shields.io/npm/v/@srcmap/sourcemap-wasm.svg)](https://www.npmjs.com/package/@srcmap/sourcemap-wasm)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)

Source map parser and consumer, written in Rust and compiled to WebAssembly.

Parses source map JSON and provides position lookups. Implements [ECMA-426](https://tc39.es/ecma426/) (Source Map v3). With 1000 lookups in one call, the batch API was faster than [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping) on a 988 KB map and on a 5 MB map. For a parse or a single lookup, `@jridgewell/trace-mapping` is faster.

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

// Batch lookup: one WASM call for all positions
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

Use one `GeneratedOffsetLookup` per generated asset and reuse it across batches instead of recomputing line starts for every lookup.

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
| `source(index)` | `string \| undefined` | Resolve source index to filename. Returns `undefined` for out-of-bounds indices |
| `name(index)` | `string \| undefined` | Resolve name index to string. Returns `undefined` for out-of-bounds indices |
| `sourceContentFor(index)` | `string \| null` | Get source content by index. Returns `null` if index is out of bounds or content is missing |
| `isIgnoredIndex(index)` | `boolean` | Check if a source index is in the `ignoreList` |
| `free()` | `void` | Release WASM memory (also via `Symbol.dispose`) |

### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `sources` | `string[]` | All source filenames |
| `names` | `string[]` | All names |
| `sourcesContent` | `(string \| null)[]` | Source file contents (parallel to `sources`) |
| `ignoreList` | `Uint32Array` | Source ignore list indices |
| `file` | `string \| undefined` | Output filename |
| `sourceRoot` | `string \| undefined` | Source root prefix |
| `debugId` | `string \| undefined` | `debugId` proposal field |
| `lineCount` | `number` | Number of generated lines |
| `mappingCount` | `number` | Total decoded mappings |
| `hasRangeMappings` | `boolean` | Whether any range mappings exist |
| `rangeMappingCount` | `number` | Number of range mappings |

---

### `new LazySourceMap(json: string)`

A fast-scan alternative to `SourceMap` that defers VLQ decoding until lookup time. On construction it only parses JSON metadata and byte-scans the mappings string for semicolons (to identify line boundaries). VLQ decoding happens per-line on demand with progressive state tracking. This makes parse time near-instant at the cost of slightly slower first lookups.

Import it directly from the supported package entrypoint:

```js
import { LazySourceMap } from "@srcmap/sourcemap-wasm";

const sm = new LazySourceMap(jsonString);
const loc = sm.originalPositionFor(42, 10);
sm.free();
```

`LazySourceMap` does **not** parse `sourcesContent`. Use `SourceMap` when source contents are required.

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
| `source(index)` | `string \| undefined` | Resolve source index to filename. Returns `undefined` for out-of-bounds indices |
| `name(index)` | `string \| undefined` | Resolve name index to string. Returns `undefined` for out-of-bounds indices |
| `isIgnoredIndex(index)` | `boolean` | Check if a source index is in the `ignoreList` |
| `free()` | `void` | Release WASM memory (also via `Symbol.dispose`) |

#### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `sources` | `string[]` | All source filenames |
| `names` | `string[]` | All names |
| `ignoreList` | `Uint32Array` | Source ignore list indices |
| `file` | `string \| undefined` | Output filename |
| `sourceRoot` | `string \| undefined` | Source root prefix |
| `debugId` | `string \| undefined` | `debugId` proposal field |
| `lineCount` | `number` | Number of generated lines |

## Performance

### Batch API vs trace-mapping

The batch API (`originalPositionsFor`) takes all positions in one call and returns a flat `Int32Array`. It is one call into WASM, and it allocates no object per lookup.

In the [repository benchmarks](https://github.com/fallow-rs/srcmap#performance), 1000 lookups in one call were faster than `@jridgewell/trace-mapping` on the Chart.js and PDF.js maps, and slower on the Preact map. A parse and a single lookup were slower than `@jridgewell/trace-mapping` on all three maps.

### When to use which

| Use case | Recommended package |
|----------|-------------------|
| Batch lookups (error stacks, coverage) | `@srcmap/sourcemap-wasm` (batch API) |
| A few lookups, without `sourcesContent` | `@srcmap/sourcemap-wasm` (`LazySourceMap`, which decodes mappings on access) |
| One lookup per call | `@jridgewell/trace-mapping` (a lookup costs less than one WASM call) |
| Native Node.js addons | `@srcmap/sourcemap` (NAPI) |
| Browser environments | `@srcmap/sourcemap-wasm` |

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Source map tooling written in Rust. See also:
- [`@srcmap/codec`](https://www.npmjs.com/package/@srcmap/codec) - VLQ codec (NAPI)
- [`@srcmap/sourcemap`](https://www.npmjs.com/package/@srcmap/sourcemap) - Source map parser (NAPI)

## License

MIT
