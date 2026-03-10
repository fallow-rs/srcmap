# @srcmap/sourcemap-wasm

[![npm](https://img.shields.io/npm/v/@srcmap/sourcemap-wasm.svg)](https://www.npmjs.com/package/@srcmap/sourcemap-wasm)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/BartWaardenburg/srcmap/badges/coverage.json)](https://github.com/BartWaardenburg/srcmap/actions/workflows/coverage.yml)

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

## API

### `new SourceMap(json: string)`

Parse a source map from a JSON string.

### Instance methods

| Method | Returns | Description |
|--------|---------|-------------|
| `originalPositionFor(line, column)` | `{ source, line, column, name } \| null` | Forward lookup (0-based) |
| `generatedPositionFor(source, line, column)` | `{ line, column } \| null` | Reverse lookup (0-based) |
| `originalPositionsFor(positions: Int32Array)` | `Int32Array` | Batch forward lookup |
| `source(index)` | `string` | Resolve source index to filename |
| `name(index)` | `string` | Resolve name index to string |
| `free()` | `void` | Release WASM memory (also via `Symbol.dispose`) |

### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `lineCount` | `number` | Number of generated lines |
| `mappingCount` | `number` | Total decoded mappings |
| `sources` | `string[]` | All source filenames |
| `names` | `string[]` | All names |

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

## Part of [srcmap](https://github.com/BartWaardenburg/srcmap)

High-performance source map tooling written in Rust. See also:
- [`@srcmap/codec`](https://www.npmjs.com/package/@srcmap/codec) - VLQ codec (NAPI)
- [`@srcmap/sourcemap`](https://www.npmjs.com/package/@srcmap/sourcemap) - Source map parser (NAPI)

## License

MIT
