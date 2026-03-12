# @srcmap/sourcemap

[![npm](https://img.shields.io/npm/v/@srcmap/sourcemap.svg)](https://www.npmjs.com/package/@srcmap/sourcemap)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/BartWaardenburg/srcmap/badges/coverage.json)](https://github.com/BartWaardenburg/srcmap/actions/workflows/coverage.yml)

High-performance source map parser and consumer powered by Rust via [NAPI](https://napi.rs).

Parses source map JSON and provides position lookups. Implements [ECMA-426](https://tc39.es/ecma426/) (Source Map v3). Alternative to [`@jridgewell/trace-mapping`](https://github.com/jridgewell/trace-mapping).

> For batch lookups in Node.js, consider [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) which avoids NAPI per-call overhead and is faster for bulk operations.

## Install

```bash
npm install @srcmap/sourcemap
```

Prebuilt binaries are available for:
- macOS (x64, arm64)
- Linux (x64, arm64, glibc + musl)
- Windows (x64)

## Usage

```js
import { SourceMap } from '@srcmap/sourcemap';

const sm = new SourceMap(jsonString);

// Forward lookup: generated -> original (0-based lines and columns)
const loc = sm.originalPositionFor(42, 10);
// { source: 'src/app.ts', line: 10, column: 4, name: 'handleClick' }
// Returns null if no mapping exists

// Reverse lookup: original -> generated
const pos = sm.generatedPositionFor('src/app.ts', 10, 4);
// { line: 42, column: 10 }

// Batch lookup — amortizes NAPI overhead
const positions = new Int32Array([42, 10, 43, 0, 44, 5]);
const results = sm.originalPositionsFor(positions);
// Int32Array [srcIdx, line, col, nameIdx, ...]
// -1 means no mapping / no name

// Resolve indices
const source = sm.source(results[0]);
const name = results[3] >= 0 ? sm.name(results[3]) : null;
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

### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `lineCount` | `number` | Number of generated lines |
| `mappingCount` | `number` | Total decoded mappings |
| `hasRangeMappings` | `boolean` | Whether any range mappings exist |
| `rangeMappingCount` | `number` | Number of range mappings |
| `sources` | `string[]` | All source filenames |
| `names` | `string[]` | All names |

## Performance

| Operation | @srcmap/sourcemap | @jridgewell/trace-mapping |
|-----------|-------------------|---------------------------|
| 1000x batch lookup (large) | 160 us | 15 us |
| Single lookup | 345 ns | 24 ns |

NAPI has ~300ns overhead per call. For bulk operations, use the batch API (`originalPositionsFor`) or consider the [WASM package](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) which has lower per-call overhead.

## Part of [srcmap](https://github.com/BartWaardenburg/srcmap)

High-performance source map tooling written in Rust. See also:
- [`@srcmap/codec`](https://www.npmjs.com/package/@srcmap/codec) - VLQ codec (NAPI)
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Source map parser (WASM, recommended for batch ops)

## License

MIT
