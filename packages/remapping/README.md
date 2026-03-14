# @srcmap/remapping

[![npm](https://img.shields.io/npm/v/@srcmap/remapping.svg)](https://www.npmjs.com/package/@srcmap/remapping)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)

Drop-in replacement for [`@jridgewell/remapping`](https://github.com/jridgewell/remapping) and [`@ampproject/remapping`](https://github.com/nicolo-ribaudo/remapping) powered by Rust via WebAssembly.

Same API, same types, same behavior. Swap the import and get source map composition backed by Rust under the hood.

## Install

```bash
npm install @srcmap/remapping
```

## Usage

```js
// Before:
// import remapping from '@jridgewell/remapping'
// import remapping from '@ampproject/remapping'

// After:
import remapping from '@srcmap/remapping'

// Remap a single source map through a loader
const composed = remapping(minifiedMap, (sourcefile) => {
  // Return the upstream source map for this file, or null
  return upstreamMaps[sourcefile] ?? null
})

console.log(composed.sources)    // Original sources
console.log(composed.mappings)   // Composed VLQ mappings
console.log(composed.toString()) // JSON string
```

### Compose an array of source maps

```js
// When you have the full chain: [minified map, intermediate map, original map]
const composed = remapping([minifiedMap, intermediateMap], (sourcefile) => {
  return upstreamMaps[sourcefile] ?? null
})
```

### Options

```js
const composed = remapping(minifiedMap, loader, {
  excludeContent: true, // Omit sourcesContent from the output
})
```

## API compatibility

| Export | Description |
|--------|-------------|
| `remapping(input, loader, options?)` | Remap a single source map through a loader |
| `remapping(inputs[], loader, options?)` | Compose an array of source maps |
| `SourceMap` | Result class with `version`, `file`, `mappings`, `sources`, `sourcesContent`, `names`, `ignoreList` |

The `loader` function receives a source filename and should return the upstream source map (as a JSON string, parsed object, or decoded source map), or `null` if no upstream map exists.

### SourceMap result

| Property / Method | Type | Description |
|--------|------|-------------|
| `version` | `number` | Source map version (always 3) |
| `file` | `string \| undefined` | Output filename |
| `mappings` | `string` | VLQ-encoded mappings |
| `sources` | `(string \| null)[]` | Source filenames |
| `sourcesContent` | `(string \| null)[] \| undefined` | Inline source contents |
| `names` | `string[]` | Name strings |
| `ignoreList` | `number[] \| undefined` | Indices of ignored sources |
| `toString()` | `string` | Serialize as JSON string |
| `toJSON()` | `RawSourceMap` | Return as parsed object |

## Differences from @jridgewell/remapping

- **WASM-powered**: Composition runs in Rust via WebAssembly
- **Indexed source maps**: Handled natively (no JS-side flattening)

## Part of [srcmap](https://github.com/BartWaardenburg/srcmap)

High-performance source map tooling written in Rust. See also:
- [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) - Drop-in for `@jridgewell/trace-mapping` (consumer)
- [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) - Drop-in for `@jridgewell/gen-mapping` (generator)
- [`@srcmap/remapping-wasm`](https://www.npmjs.com/package/@srcmap/remapping-wasm) - Lower-level WASM remapping API

## License

MIT
