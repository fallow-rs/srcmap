# @srcmap/remapping-wasm

[![npm](https://img.shields.io/npm/v/@srcmap/remapping-wasm.svg)](https://www.npmjs.com/package/@srcmap/remapping-wasm)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

Source map concatenation and composition, written in Rust and compiled to WebAssembly.

**Concatenation** merges source maps from multiple bundled files into one, adjusting line offsets. **Composition** chains source maps through multiple transforms (e.g. TS -> JS -> minified) into a single map pointing to original sources. Alternative to [`@ampproject/remapping`](https://github.com/7rulnik/source-map-js).

## Install

```bash
npm install @srcmap/remapping-wasm
```

Works in Node.js, browsers, and any WebAssembly-capable runtime. No native compilation required.

## Usage

### Concatenation

Merge source maps from multiple files into a single combined map:

```js
import { ConcatBuilder } from '@srcmap/remapping-wasm'

const builder = new ConcatBuilder('bundle.js')

// Add source maps with their line offsets in the output
builder.addMap(chunkASourceMapJson, 0)      // chunk A starts at line 0
builder.addMap(chunkBSourceMapJson, 1000)   // chunk B starts at line 1000

const combinedJson = builder.toJSON()
```

### Composition / Remapping

Chain source maps through a transform pipeline into a single map:

```js
import { remap } from '@srcmap/remapping-wasm'

// Your build: original.ts -> intermediate.js -> minified.js
// You have: minified.js.map (outer) and intermediate.js.map (inner)

const composedJson = remap(minifiedSourceMapJson, (source) => {
  // Called for each source in the outer map
  if (source === 'intermediate.js') {
    return intermediateSourceMapJson // upstream source map JSON
  }
  return null // no upstream map, keep as-is
})
```

## API

### `new ConcatBuilder(file?: string)`

Create a builder for concatenating source maps.

| Method | Returns | Description |
|--------|---------|-------------|
| `addMap(json, lineOffset)` | `void` | Add a source map JSON at the given line offset |
| `toJSON()` | `string` | Generate the concatenated source map JSON |

### `remap(outerJson, loader)`

Compose source maps through a transform chain.

| Parameter | Type | Description |
|-----------|------|-------------|
| `outerJson` | `string` | The final-stage source map JSON |
| `loader` | `(source: string) => string \| null` | Returns upstream source map JSON, or `null` |

Returns the composed source map as a JSON string.

### `new RemapTree()`

Compose source maps that you have as parts, not as JSON. `@srcmap/remapping` uses it for maps that it gets as objects. The result is the same as for `remap` with the same maps as JSON.

| Method | Returns | Description |
|--------|---------|-------------|
| `addEncoded(key, file, sources, contents, names, ignoreList, mappings)` | `void` | Add a map with VLQ `mappings` as UTF-8 bytes (`Uint8Array`) |
| `addDecoded(key, file, sources, contents, names, ignoreList, segments)` | `void` | Add a map with decoded `segments` (`Int32Array`) |
| `remap(passes)` | `void` | Remap the root map, with one remap pass for each level of upstream maps |
| `resultMappings()`, `resultSources()`, `resultNames()`, `resultContents()`, `resultIgnoreList()`, `resultFile()` | | Read the result |
| `free()` | `void` | Release the WASM memory of the tree |

- `key` is the source name of the map in another map. Without `key`, the map is the root map.
- `contents` holds a content token for each source: an index into an array that you keep in JavaScript, or `-1` for no content. `resultContents()` gives the tokens of the result.
- `segments` holds 6 values for each segment: the generated line, the generated column, the source index, the original line, the original column and the name index. A source index or a name index of `-1` gives a segment without a source or without a name.

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Source map tooling written in Rust. See also:
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Source map parser (WASM)
- [`@srcmap/generator-wasm`](https://www.npmjs.com/package/@srcmap/generator-wasm) - Source map generator (WASM)
- [`@srcmap/symbolicate-wasm`](https://www.npmjs.com/package/@srcmap/symbolicate-wasm) - Stack trace symbolication (WASM)

## License

MIT
