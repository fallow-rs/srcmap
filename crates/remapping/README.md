# srcmap-remapping

[![crates.io](https://img.shields.io/crates/v/srcmap-remapping.svg)](https://crates.io/crates/srcmap-remapping)
[![docs.rs](https://docs.rs/srcmap-remapping/badge.svg)](https://docs.rs/srcmap-remapping)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/fallow-rs/srcmap/badges/rust-coverage.json)](https://github.com/fallow-rs/srcmap/actions/workflows/coverage.yml)

Source map concatenation and composition/remapping for Rust.

Merges or chains source maps from multiple build steps into a single map. Drop-in Rust equivalent of [`@ampproject/remapping`](https://github.com/nicolo-ribaudo/amp-remapping).

## Install

```toml
[dependencies]
srcmap-remapping = "0.3"
```

## Usage

### Concatenation

Merge source maps from multiple bundled files into one, adjusting line offsets. Used by bundlers (esbuild, Rollup, Webpack).

```rust
use srcmap_remapping::ConcatBuilder;
use srcmap_sourcemap::SourceMap;

let map_a = SourceMap::from_json(r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#).unwrap();
let map_b = SourceMap::from_json(r#"{"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}"#).unwrap();

let mut builder = ConcatBuilder::new(Some("bundle.js".to_string()));
builder.add_map(&map_a, 0);    // a.js starts at line 0
builder.add_map(&map_b, 100);  // b.js starts at line 100

let result = builder.build();
```

### Composition / Remapping

Chain source maps through multiple transforms (e.g. TypeScript -> JavaScript -> minified) into a single map pointing to the original sources.

```rust
use srcmap_remapping::remap;
use srcmap_sourcemap::SourceMap;

let minified_map = SourceMap::from_json(outer_json).unwrap();

let result = remap(&minified_map, |source| {
    // Return the upstream source map for this source file, if any
    if source == "intermediate.js" {
        Some(SourceMap::from_json(ts_to_js_map_json).unwrap())
    } else {
        None // no upstream map — keep as-is
    }
});

// result now maps minified output directly to TypeScript sources
```

## API

### Concatenation

| Method | Description |
|--------|-------------|
| `ConcatBuilder::new(file) -> Self` | Create a new concatenation builder |
| `builder.add_map(sourcemap, line_offset)` | Add a source map at the given line offset |
| `builder.build() -> SourceMap` | Serialize the current state as a decoded `SourceMap` |
| `builder.to_json() -> String` | Serialize the current state as a JSON string |

### Composition

| Function | Description |
|----------|-------------|
| `remap(outer, loader) -> SourceMap` | Compose through upstream maps resolved by `loader` |
| `remap_streaming(iter, sources, names, sources_content, ignore_list, file, loader) -> SourceMap` | Streaming variant — avoids materializing the outer map |

The `loader` function receives each source filename and returns `Option<SourceMap>`. Return `Some` to trace through an upstream map, or `None` to keep the source as-is.

`remap_streaming` accepts a `MappingsIter` (lazy VLQ iterator) and uses `StreamingGenerator` for on-the-fly encoding — 15-20% faster than `remap` for large maps.

## Features

- **Source and name deduplication** across concatenated maps
- **`sourcesContent` merging** from all inputs
- **`ignoreList` propagation** through concatenation
- **Name resolution** prefers upstream names over outer names
- **Lazy loading** via the `loader` callback — only loads maps that are actually referenced
- **Range mapping preservation** through both concatenation and composition
- **Streaming composition** via `remap_streaming` for reduced-allocation pipelines

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

See also:
- [`srcmap-sourcemap`](https://crates.io/crates/srcmap-sourcemap) - Parser and consumer
- [`srcmap-generator`](https://crates.io/crates/srcmap-generator) - Source map builder
- [`srcmap-codec`](https://crates.io/crates/srcmap-codec) - VLQ encode/decode

## License

MIT
