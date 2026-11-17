# @srcmap/source-map

[![npm](https://img.shields.io/npm/v/@srcmap/source-map.svg)](https://www.npmjs.com/package/@srcmap/source-map)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

The v0.6 API of [Mozilla's `source-map`](https://github.com/mozilla/source-map) on the srcmap Rust parser and generator, compiled to WebAssembly.

The package has the `SourceMapConsumer` and `SourceMapGenerator` classes of `source-map` v0.6. Existing code needs a changed import, and it must free WASM memory (see [Differences](#differences-from-mozilla-source-map)).

A lookup is a call into WASM, and [`source-map-js`](https://github.com/7rulnik/source-map-js) does a single lookup in less time (see the [benchmarks](https://github.com/fallow-rs/srcmap#performance)). Choose this package when the results must match a Rust tool that uses srcmap.

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
| `applySourceMap(consumer, source?, path?)` | Accepted for API parity; validates its arguments and does not rewrite mappings |
| `toJSON()` | Return as parsed source map object |
| `toString()` | Return as JSON string |
| `destroy()` | Free WASM resources |

## Differences from Mozilla source-map

- **Synchronous API.** The constructor returns a consumer directly. The package has no `SourceMapConsumer.with()` and no promise-based initialization.
- **WASM memory.** Call `consumer.destroy()` or `generator.destroy()` when you are done.
- **Indexed source maps.** The parser reads them directly.

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Source map tooling written in Rust. See also:
- [`@srcmap/trace-mapping`](https://www.npmjs.com/package/@srcmap/trace-mapping) - The `@jridgewell/trace-mapping` API
- [`@srcmap/gen-mapping`](https://www.npmjs.com/package/@srcmap/gen-mapping) - The `@jridgewell/gen-mapping` API
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Lower-level WASM API (0-based lines)

## License

MIT
