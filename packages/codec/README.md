# @srcmap/codec

[![npm](https://img.shields.io/npm/v/@srcmap/codec.svg)](https://www.npmjs.com/package/@srcmap/codec)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)

VLQ source map codec, written in Rust with a [NAPI](https://napi.rs) binding.

It has the `encode` and `decode` functions of [`@jridgewell/sourcemap-codec`](https://github.com/jridgewell/sourcemap-codec). Encodes and decodes source map `mappings` strings as specified in [ECMA-426](https://tc39.es/ecma426/).

## Install

```bash
npm install @srcmap/codec
```

Prebuilt binaries are available for:
- macOS (x64, arm64)
- Linux (x64, arm64, glibc + musl)
- Windows (x64)

## Usage

```js
import { decode, encode } from '@srcmap/codec';

const decoded = decode('AAAA;AACA,EAAE');
// [
//   [[0, 0, 0, 0]],
//   [[0, 0, 1, 0], [2, 0, 0, 2]]
// ]

const encoded = encode(decoded);
// 'AAAA;AACA,EAAE'
```

## API

### `decode(mappings: string): number[][][]`

Decode a VLQ-encoded mappings string into an array of lines, each containing an array of segments. Each segment is an array of 1, 4, or 5 numbers.

### `encode(mappings: number[][][]): string`

Encode decoded mappings back into a VLQ string.

### Segment format

| Fields | Meaning |
|--------|---------|
| `[genCol]` | Generated column only |
| `[genCol, srcIdx, origLine, origCol]` | With source mapping |
| `[genCol, srcIdx, origLine, origCol, nameIdx]` | With source mapping and name |

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

Source map tooling written in Rust. See also:
- [`@srcmap/sourcemap`](https://www.npmjs.com/package/@srcmap/sourcemap) - Source map parser (NAPI)
- [`@srcmap/sourcemap-wasm`](https://www.npmjs.com/package/@srcmap/sourcemap-wasm) - Source map parser (WASM)

## License

MIT
