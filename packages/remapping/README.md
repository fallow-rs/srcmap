# @srcmap/remapping

[![npm](https://img.shields.io/npm/v/@srcmap/remapping.svg)](https://www.npmjs.com/package/@srcmap/remapping)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

The API of [`@jridgewell/remapping`](https://github.com/jridgewell/remapping) and [`@ampproject/remapping`](https://github.com/ampproject/remapping) on the srcmap Rust remapping code, compiled to WebAssembly.

`remapping(maps, loader)` takes the arguments of `@jridgewell/remapping` and returns a `SourceMap` with the same fields.

Each call writes the input maps to JSON, calls into WASM, and parses the result. For a small map, those three steps cost more than the composition. This package is not the faster choice in a build tool. `benchmarks/vite-workload.mjs` in the repository compares it with `@jridgewell/remapping` on the maps of a recorded Vite run.

It fits a Node.js tool that must compose maps the same way as a Rust tool with `srcmap-remapping`.

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

## API compatibility

| Export | Description |
|--------|-------------|
| `remapping(input, loader, options?)` | Remap a single source map through a loader |
| `remapping(inputs[], loader, options?)` | Compose an array of source maps |
| `SourceMap` | Result class with `version`, `file`, `mappings`, `sources`, `sourcesContent`, `names`, `ignoreList` |

The `loader` function receives a source filename and a `LoaderContext`, and should return the upstream source map (as a JSON string, parsed object, or decoded source map), or `null` if no upstream map exists. As in `@jridgewell/remapping`, the loader is called again for the sources of each map that it returns, and it can set `ctx.source`, `ctx.content` and `ctx.ignore` for a source without an upstream map.

`options` takes `excludeContent` and `decodedMappings`. `true` means `{ excludeContent: true }`.

### SourceMap result

| Property / Method | Type | Description |
|--------|------|-------------|
| `version` | `number` | Source map version (always 3) |
| `file` | `string \| undefined` | Output filename |
| `mappings` | `string \| number[][][]` | VLQ-encoded mappings, or decoded segments with `decodedMappings` |
| `sources` | `(string \| null)[]` | Source filenames |
| `sourcesContent` | `(string \| null)[] \| undefined` | Inline source contents, left out with `excludeContent` |
| `names` | `string[]` | Name strings |
| `ignoreList` | `number[]` | Indices of ignored sources |
| `toString()` | `string` | Serialize as JSON string |
| `toJSON()` | `RawSourceMap` | Return as parsed object |

## Differences from @jridgewell/remapping

- **WASM-powered**: Composition runs in Rust via WebAssembly
- **Indexed source maps**: Handled natively (no JS-side flattening)
- **`ctx.source` that is not a string**: As in `@jridgewell/remapping`, a falsy value gives segments without a source, and an other value stays in `sources` as it is. One difference stays: when the loader ignores falsy sources of more than one kind in one call (for example `""` and `null`), the result has one entry for them, and jridgewell can have one entry for each kind.

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Source map tooling written in Rust. See also:
- [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) - The `@jridgewell/trace-mapping` API (consumer)
- [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) - The `@jridgewell/gen-mapping` API (generator)
- [`@srcmap/remapping-wasm`](https://www.npmjs.com/package/@srcmap/remapping-wasm) - Lower-level WASM remapping API

## License

MIT
