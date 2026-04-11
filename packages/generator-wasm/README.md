# @srcmap/generator-wasm

[![npm](https://img.shields.io/npm/v/@srcmap/generator-wasm.svg)](https://www.npmjs.com/package/@srcmap/generator-wasm)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

High-performance source map generator powered by Rust via WebAssembly.

Builds source maps incrementally by registering sources, names, and mappings. Outputs standard source map v3 JSON (ECMA-426). Alternative to [`source-map`](https://github.com/nicolo-ribaudo/source-map-js)'s `SourceMapGenerator`.

## Install

```bash
npm install @srcmap/generator-wasm
```

Works in Node.js, browsers, and any WebAssembly-capable runtime. No native compilation required.

## Usage

```js
import { SourceMapGenerator } from '@srcmap/generator-wasm'

const gen = new SourceMapGenerator('bundle.js')

// Register sources and names (returns indices for use in mappings)
const src = gen.addSource('src/app.ts')
gen.setSourceContent(src, 'const x = 1;')

const name = gen.addName('x')

// Add mappings (all positions are 0-based)
gen.addNamedMapping(0, 0, src, 0, 6, name) // generated(0:0) -> original(0:6) name:"x"
gen.addMapping(1, 0, src, 1, 0)            // generated(1:0) -> original(1:0)

// Deduplicated mapping: only adds if different from previous on same line
gen.maybeAddMapping(1, 4, src, 1, 4)

const json = gen.toJSON()
```

## API

### `new SourceMapGenerator(file?: string)`

Create a new source map generator. `file` is the optional output filename.

### Instance methods

| Method | Returns | Description |
|--------|---------|-------------|
| `addSource(source)` | `number` | Register a source file, returns its index |
| `addName(name)` | `number` | Register a name, returns its index |
| `setSourceRoot(root)` | `void` | Set the `sourceRoot` prefix |
| `setDebugId(id)` | `void` | Set the debug ID (ECMA-426) |
| `setSourceContent(sourceIdx, content)` | `void` | Attach source content to a source |
| `addToIgnoreList(sourceIdx)` | `void` | Add a source to the `ignoreList` |
| `addGeneratedMapping(genLine, genCol)` | `void` | Add a mapping with no source info |
| `addMapping(genLine, genCol, src, origLine, origCol)` | `void` | Add a mapping |
| `addNamedMapping(genLine, genCol, src, origLine, origCol, name)` | `void` | Add a mapping with a name |
| `maybeAddMapping(genLine, genCol, src, origLine, origCol)` | `boolean` | Add only if different from previous |
| `toJSON()` | `string` | Generate the source map JSON string |

### Instance properties

| Property | Type | Description |
|----------|------|-------------|
| `mappingCount` | `number` | Total number of mappings added |

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
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Source map parser (WASM)
- [`@srcmap/remapping-wasm`](https://www.npmjs.com/package/@srcmap/remapping-wasm) - Concatenation + composition (WASM)
- [`@srcmap/symbolicate-wasm`](https://www.npmjs.com/package/@srcmap/symbolicate-wasm) - Stack trace symbolication (WASM)

## License

MIT
