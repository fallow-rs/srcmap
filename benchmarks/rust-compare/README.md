# Comparison with other Rust crates

This benchmark compares `srcmap-sourcemap` and `srcmap-remapping` with [`oxc_sourcemap`](https://crates.io/crates/oxc_sourcemap) 10.0.0 and the Sentry [`sourcemap`](https://crates.io/crates/sourcemap) crate 9.3.2. It uses the source maps of Preact, Chart.js and PDF.js.

The crate is a separate Cargo workspace. The other source map crates thus stay out of the root workspace, its lock file and its dependency checks.

## Run

Run these commands from the repository root:

```bash
corepack pnpm --dir benchmarks run download-fixtures
cargo run --release --manifest-path benchmarks/rust-compare/Cargo.toml
```

The program prints one Markdown table for each map. An optional argument gives another directory with `preact.js.map`, `chartjs.js.map` and `pdfjs.js.map`.

## Method

- The candidates of one operation run in turn in each round, in one process. A change in machine load then has the same effect on each candidate.
- The table shows the best time of each candidate. The best time has the least noise.
- A lookup measurement uses at most 100,000 positions of the map, in generated order.
- The composition measurement builds the map of a second transform over the generated file. That map has one segment for each segment of the fixture. Every second segment points one column after a segment start, so the trace also searches between segments.

## Operations

| Operation | srcmap | `oxc_sourcemap` | Sentry `sourcemap` |
|---|---|---|---|
| Parse | `SourceMap::from_json` | `SourceMap::from_json_string` | `SourceMap::from_slice` |
| Write JSON | `to_json` | `to_json_string` | `to_writer` |
| Lookup | `original_position_for` | `lookup_token` with a lookup table | `lookup_token` |
| Compose | `srcmap_remapping::remap_ref` | `compose` | not available |

## Results

These are the ratios of srcmap to `oxc_sourcemap` from three runs in November 2026 on an Apple Silicon laptop under load. A ratio below 1.00 means that srcmap is faster. The ratios were stable between the runs. The absolute times were not, so this file does not give them.

| Operation | Preact | Chart.js | PDF.js |
|---|---:|---:|---:|
| Parse | 0.98 to 1.00 | 1.00 to 1.01 | 0.90 to 0.92 |
| Write JSON | 1.04 to 1.20 | 1.06 to 1.08 | 1.07 to 1.14 |
| Lookup at a segment start | 1.20 to 1.21 | 1.10 to 1.13 | 1.06 to 1.11 |
| Lookup between two segments | 1.29 to 1.31 | 1.37 to 1.41 | 1.35 to 1.39 |
| Compose two parsed maps | 3.51 to 3.62 | 2.01 to 2.25 | 1.83 to 2.03 |
| Parse two maps, compose, write JSON | 1.34 to 1.37 | 1.20 | 1.02 to 1.12 |

The Sentry crate was slower than srcmap on each operation that it has: about 1.7 to 2.5 times on parse and on JSON output, and 1.5 to 8 times on a lookup.

## Why composition is slower

`srcmap_remapping::remap_ref` gives the same result as `@jridgewell/remapping`. It applies these rules to each segment:

- It skips a segment that repeats the previous segment of the line.
- It keeps a segment without a source when the trace ends on a segment without a source.
- It adds a source and a name to the result only when a segment uses them.

`compose` in `oxc_sourcemap` does not apply these rules, so the two results can differ. The program prints the number of composed segments of each crate.

## What the other crates do not have

Fallow and `oxc-coverage-instrument` use these parts of srcmap. `oxc_sourcemap` 10 and `sourcemap` 9 do not have them:

- A reverse lookup with a bias (`generated_position_for_with_bias`)
- All generated positions of an original position (`all_generated_positions_for`)
- `GeneratedOffsetLookup`, which converts a byte offset to a generated position
