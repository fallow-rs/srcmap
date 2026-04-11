# @srcmap/gen-mapping

[![npm](https://img.shields.io/npm/v/@srcmap/gen-mapping.svg)](https://www.npmjs.com/package/@srcmap/gen-mapping)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

Drop-in replacement for [`@jridgewell/gen-mapping`](https://github.com/jridgewell/gen-mapping) powered by Rust via WebAssembly.

Same API, same types, same behavior. Swap the import and get source map generation backed by Rust under the hood.

## Install

```bash
npm install @srcmap/gen-mapping
```

## Usage

```js
// Before:
// import { GenMapping, addMapping, toEncodedMap } from '@jridgewell/gen-mapping'

// After:
import { GenMapping, addMapping, toEncodedMap } from '@srcmap/gen-mapping'

const map = new GenMapping({ file: 'bundle.js' })

addMapping(map, {
  generated: { line: 1, column: 0 },
  source: 'src/app.ts',
  original: { line: 10, column: 4 },
  name: 'handleClick',
  content: 'const handleClick = () => { ... }',
})

const encoded = toEncodedMap(map)

// Cleanup WASM memory (optional, also via `using` with Symbol.dispose)
map.free()
```

## API compatibility

All exports from `@jridgewell/gen-mapping` are supported:

| Export | Description |
|--------|-------------|
| `GenMapping` | Main class — creates a new source map builder |
| `addMapping(map, mapping)` | Add a mapping (1-based lines, 0-based columns) |
| `maybeAddMapping(map, mapping)` | Add only if it differs from the previous mapping on the same line |
| `setSourceContent(map, source, content)` | Set source content for a source file |
| `setIgnore(map, source, ignore?)` | Mark a source as ignored |
| `toEncodedMap(map)` | Return as encoded source map (VLQ string mappings) |
| `toDecodedMap(map)` | Return as decoded source map (array mappings) |
| `allMappings(map)` | Return all mappings as `Mapping[]` |
| `fromMap(input)` | Construct from an existing source map |

### Line/column convention

Follows `@jridgewell/gen-mapping` conventions:
- `addMapping` / `maybeAddMapping`: **1-based lines**, 0-based columns
- Decoded mappings output: **0-based lines**, 0-based columns

## Differences from @jridgewell/gen-mapping

- **WASM memory**: Call `map.free()` when done, or use `using map = new GenMapping(...)` with `Symbol.dispose`
- **VLQ encoding runs in WASM**: No JS-side encoding overhead

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

High-performance source map tooling written in Rust. See also:
- [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) - Drop-in for `@jridgewell/trace-mapping` (consumer)
- [`@srcmap/remapping`](https://www.npmjs.com/package/@srcmap/remapping) - Drop-in for `@ampproject/remapping` (composition)
- [`@srcmap/generator-wasm`](https://www.npmjs.com/package/@srcmap/generator-wasm) - Lower-level WASM generator API

## License

MIT
