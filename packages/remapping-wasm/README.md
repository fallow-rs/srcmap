# @srcmap/remapping-wasm

[![npm](https://img.shields.io/npm/v/@srcmap/remapping-wasm.svg)](https://www.npmjs.com/package/@srcmap/remapping-wasm)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)

High-performance source map concatenation and composition powered by Rust via WebAssembly.

**Concatenation** merges source maps from multiple bundled files into one, adjusting line offsets. **Composition** chains source maps through multiple transforms (e.g. TS -> JS -> minified) into a single map pointing to original sources. Alternative to [`@ampproject/remapping`](https://github.com/nicolo-ribaudo/source-map-js).

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
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Source map parser (WASM)
- [`@srcmap/generator-wasm`](https://www.npmjs.com/package/@srcmap/generator-wasm) - Source map generator (WASM)
- [`@srcmap/symbolicate-wasm`](https://www.npmjs.com/package/@srcmap/symbolicate-wasm) - Stack trace symbolication (WASM)

## License

MIT
