# srcmap-ram-bundle

[![crates.io](https://img.shields.io/crates/v/srcmap-ram-bundle.svg)](https://crates.io/crates/srcmap-ram-bundle)
[![docs.rs](https://docs.rs/srcmap-ram-bundle/badge.svg)](https://docs.rs/srcmap-ram-bundle)

React Native RAM bundle parser for source map tooling.

Supports two RAM bundle formats used by React Native / Metro:

- **Indexed RAM bundles** (iOS): single binary file with magic number `0xFB0BD1E5`
- **Unbundles** (Android): directory-based with `js-modules/` structure

## Usage

```rust
use srcmap_ram_bundle::{IndexedRamBundle, is_ram_bundle};

let data = std::fs::read("app.bundle").unwrap();

if is_ram_bundle(&data) {
    let bundle = IndexedRamBundle::from_bytes(&data).unwrap();

    println!("Startup code: {} bytes", bundle.startup_code().len());
    println!("Modules: {}", bundle.module_count());

    for module in bundle.modules() {
        println!("Module {}: {} bytes", module.id, module.source_code.len());
    }
}
```

## API

| Item | Kind | Description |
|------|------|-------------|
| `IndexedRamBundle` | struct | Parsed indexed RAM bundle (iOS format) |
| `IndexedRamBundle::from_bytes(data)` | fn | Parse from raw bytes |
| `IndexedRamBundle::module_count()` | fn | Number of module slots |
| `IndexedRamBundle::get_module(id)` | fn | Get a module by ID, or `None` if the slot is empty |
| `IndexedRamBundle::modules()` | fn | Iterate over all non-empty modules |
| `IndexedRamBundle::startup_code()` | fn | Returns the startup (prelude) code |
| `RamBundleModule` | struct | A parsed module with `id` and `source_code` fields |
| `RamBundleType` | enum | `Indexed` (iOS) or `Unbundle` (Android) |
| `RamBundleError` | enum | Error type: `InvalidMagic`, `TooShort`, `InvalidEntry`, `Io`, `SourceMap` |
| `is_ram_bundle(data)` | fn | Check if data starts with the RAM bundle magic number |
| `is_unbundle_dir(path)` | fn | Check if a path looks like an unbundle directory |

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

See the [main repo](https://github.com/fallow-rs/srcmap) for the full source map SDK.

## License

MIT
