# srcmap-codec

[![crates.io](https://img.shields.io/crates/v/srcmap-codec.svg)](https://crates.io/crates/srcmap-codec)
[![docs.rs](https://docs.rs/srcmap-codec/badge.svg)](https://docs.rs/srcmap-codec)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/rust-coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)

High-performance VLQ source map codec for Rust.

Encodes and decodes source map `mappings` strings using the Base64 VLQ format specified in [ECMA-426](https://tc39.es/ecma426/) (Source Map v3). Drop-in Rust equivalent of [`@jridgewell/sourcemap-codec`](https://github.com/jridgewell/sourcemap-codec).

## Install

```toml
[dependencies]
srcmap-codec = "0.3"
```

## Usage

### Decode and encode mappings

```rust
use srcmap_codec::{decode, encode};

let mappings = decode("AAAA;AACA,EAAE").unwrap();
assert_eq!(mappings.len(), 2); // 2 lines
assert_eq!(mappings[0][0], vec![0, 0, 0, 0]); // first segment

let encoded = encode(&mappings);
assert_eq!(encoded, "AAAA;AACA,EAAE");
```

### Low-level VLQ primitives

```rust
use srcmap_codec::{vlq_decode, vlq_encode};

let mut buf = Vec::new();
vlq_encode(&mut buf, 42);

let (value, bytes_read) = vlq_decode(&buf, 0).unwrap();
assert_eq!(value, 42);
```

### Parallel encoding

Enable the `parallel` feature for multi-threaded encoding via [rayon](https://crates.io/crates/rayon). ~1.5x faster for large maps (5K+ lines).

```toml
[dependencies]
srcmap-codec = { version = "0.3", features = ["parallel"] }
```

```rust
use srcmap_codec::encode_parallel;

let encoded = encode_parallel(&mappings);
```

## API

| Function | Description |
|----------|-------------|
| `decode(mappings) -> Result<SourceMapMappings>` | Decode a VLQ mappings string into lines of segments |
| `encode(mappings) -> String` | Encode decoded mappings back to a VLQ string |
| `encode_parallel(mappings) -> String` | Parallel encoding via rayon (requires `parallel` feature) |
| `vlq_decode(bytes, offset) -> Result<(i64, usize)>` | Decode a single signed VLQ value at the given byte offset |
| `vlq_encode(buf, value)` | Encode a single signed VLQ value and append to buffer |
| `vlq_decode_unsigned(bytes, offset) -> Result<(u64, usize)>` | Decode a single unsigned VLQ value at the given byte offset |
| `vlq_encode_unsigned(buf, value)` | Encode a single unsigned VLQ value and append to buffer |

### Types

```rust
type Segment = Vec<i64>;          // 1, 4, or 5 fields
type Line = Vec<Segment>;         // segments on one generated line
type SourceMapMappings = Vec<Line>; // all lines
```

Segments have 1, 4, or 5 fields:
- **1 field:** `[generated_column]`
- **4 fields:** `[generated_column, source_index, original_line, original_column]`
- **5 fields:** `[generated_column, source_index, original_line, original_column, name_index]`

## Performance

Standard VLQ loop with a pre-computed base64 lookup table and continuation-bit processing. The encoder includes a single-char fast path for small values.

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

This crate is the foundation of the srcmap source map toolkit. See the [main repository](https://github.com/fallow-rs/srcmap) for the full suite including parser, generator, and remapping.

## License

MIT
