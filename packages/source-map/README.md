# @srcmap/source-map

[![npm](https://img.shields.io/npm/v/@srcmap/source-map.svg)](https://www.npmjs.com/package/@srcmap/source-map)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)

Drop-in replacement for [Mozilla's `source-map`](https://github.com/nicolo-ribaudo/source-map-js) v0.6 API powered by Rust via WebAssembly.

Same `SourceMapConsumer` and `SourceMapGenerator` classes, same behavior. Swap the import and get source map operations backed by Rust under the hood.

## Install

```bash
npm install @srcmap/source-map
```

## Usage

### Consumer

```js
// Before:
// import { SourceMapConsumer } from 'source-map'

// After:
import { SourceMapConsumer } from '@srcmap/source-map'

const consumer = new SourceMapConsumer(sourceMapJsonOrObject)

const pos = consumer.originalPositionFor({ line: 43, column: 10 })
// { source: 'src/app.ts', line: 11, column: 4, name: 'handleClick' }

const content = consumer.sourceContentFor('src/app.ts')

consumer.eachMapping((mapping) => {
  console.log(mapping.generatedLine, mapping.generatedColumn)
})

// Cleanup WASM memory
consumer.destroy()
```

### Generator

```js
import { SourceMapGenerator } from '@srcmap/source-map'

const generator = new SourceMapGenerator({ file: 'bundle.js' })

generator.addMapping({
  generated: { line: 1, column: 0 },
  source: 'src/app.ts',
  original: { line: 10, column: 4 },
  name: 'handleClick',
})

generator.setSourceContent('src/app.ts', sourceCode)

const json = generator.toJSON()
const str = generator.toString()

generator.destroy()
```

## API compatibility

### SourceMapConsumer

| Method / Property | Description |
|--------|-------------|
| `originalPositionFor(needle)` | Forward lookup (1-based lines, 0-based columns) |
| `generatedPositionFor(needle)` | Reverse lookup |
| `eachMapping(callback)` | Iterate all mappings in generated order |
| `sourceContentFor(source)` | Get source content for a file |
| `sources` | Resolved source file URLs |
| `sourcesContent` | Inline source contents |
| `file` | Output filename |
| `sourceRoot` | Source root prefix |
| `destroy()` | Free WASM resources |
| `GREATEST_LOWER_BOUND` / `LEAST_UPPER_BOUND` | Search bias constants |

### SourceMapGenerator

| Method | Description |
|--------|-------------|
| `addMapping(mapping)` | Add a mapping (object-based API) |
| `setSourceContent(source, content)` | Set source content for a file |
| `applySourceMap(consumer, source?, path?)` | Apply a consumer's mappings to this generator |
| `toJSON()` | Return as parsed source map object |
| `toString()` | Return as JSON string |
| `destroy()` | Free WASM resources |

## Differences from Mozilla source-map

- **Synchronous API**: No `SourceMapConsumer.with()` or promise-based initialization — construction is synchronous
- **WASM memory**: Call `consumer.destroy()` / `generator.destroy()` when done to free WASM memory
- **Indexed source maps**: Handled natively by the WASM engine

## Part of [srcmap](https://github.com/BartWaardenburg/srcmap)

High-performance source map tooling written in Rust. See also:
- [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) - Drop-in for `@jridgewell/trace-mapping`
- [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) - Drop-in for `@jridgewell/gen-mapping`
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Lower-level WASM API (0-based lines)

## License

MIT
