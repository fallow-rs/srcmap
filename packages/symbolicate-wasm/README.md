# @srcmap/symbolicate-wasm

[![npm](https://img.shields.io/npm/v/@srcmap/symbolicate-wasm.svg)](https://www.npmjs.com/package/@srcmap/symbolicate-wasm)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

Stack trace symbolication using source maps, powered by Rust via WebAssembly.

Parses stack traces from V8 (Chrome, Node.js), SpiderMonkey (Firefox), and JavaScriptCore (Safari), resolves each frame through source maps, and produces readable output. Built for error monitoring, crash reporting, and debugging services.

## Install

```bash
npm install @srcmap/symbolicate-wasm
```

Works in Node.js, browsers, and any WebAssembly-capable runtime. No native compilation required.

## Usage

### Symbolicate a stack trace

```js
import { symbolicate } from '@srcmap/symbolicate-wasm'

const stack = `Error: Something went wrong
    at handleClick (bundle.js:42:10)
    at processEvent (bundle.js:108:5)`

const result = symbolicate(stack, (file) => {
  // Called for each unique source file in the stack trace
  if (file === 'bundle.js') {
    return sourceMapJsonString // source map JSON for this file
  }
  return null // no source map available
})

// result is a JSON string:
// {
//   "message": "Error: Something went wrong",
//   "frames": [
//     { "functionName": "handleClick", "file": "src/app.ts", "line": 10, "column": 4, "symbolicated": true },
//     { "functionName": "processEvent", "file": "src/events.ts", "line": 25, "column": 1, "symbolicated": true }
//   ]
// }
```

### Parse a stack trace (without symbolication)

```js
import { parseStackTrace } from '@srcmap/symbolicate-wasm'

const frames = parseStackTrace(stack)
// [
//   { functionName: 'handleClick', file: 'bundle.js', line: 42, column: 10 },
//   { functionName: 'processEvent', file: 'bundle.js', line: 108, column: 5 }
// ]
```

## API

### `symbolicate(stack, loader)`

Symbolicate a stack trace using a source map loader.

| Parameter | Type | Description |
|-----------|------|-------------|
| `stack` | `string` | Stack trace string (V8, SpiderMonkey, or JSC format) |
| `loader` | `(file: string) => string \| null` | Returns source map JSON for a file, or `null` |

Returns a JSON string with `message` and `frames` (each frame has `functionName`, `file`, `line`, `column`, `symbolicated`).

### `parseStackTrace(stack)`

Parse a stack trace into individual frames without symbolication.

| Parameter | Type | Description |
|-----------|------|-------------|
| `stack` | `string` | Stack trace string |

Returns an array of `{ functionName: string | null, file: string, line: number, column: number }`.

## Supported formats

| Engine | Format |
|--------|--------|
| V8 (Chrome, Node.js) | `at func (file:line:col)` |
| SpiderMonkey (Firefox) | `func@file:line:col` |
| JavaScriptCore (Safari) | `func@file:line:col` |

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
- [`@srcmap/generator-wasm`](https://www.npmjs.com/package/@srcmap/generator-wasm) - Source map generator (WASM)
- [`@srcmap/remapping-wasm`](https://www.npmjs.com/package/@srcmap/remapping-wasm) - Concatenation + composition (WASM)

## License

MIT
