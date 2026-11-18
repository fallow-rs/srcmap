# Roadmap

This file lists the planned work on srcmap, in order of priority. The proposal stages come from [tc39/ecma426](https://github.com/tc39/ecma426) and were checked in November 2026.

## Direction

srcmap is a source map toolkit for Rust, with Node.js bindings and a CLI. It follows ECMA-426 and its draft proposals. It parses, generates, composes, symbolicates and inspects maps, and it reads Hermes maps and scopes.

Speed in JavaScript is not a goal. Measurements in November 2026 gave these results:

- `@jridgewell/trace-mapping` parses faster and does a single lookup faster than the WASM and NAPI bindings.
- In four recorded Vite 8 workloads, `combineSourcemaps` took 0.1% to 2.5% of the wall time (see [benchmarks/vite-workload](benchmarks/vite-workload/README.md)).
- Rolldown composes maps in Rust with `oxc_sourcemap`.

## Now

### 1. Measure before more performance work

[benchmarks/rust-compare](benchmarks/rust-compare/README.md) compares srcmap with `oxc_sourcemap` and the Sentry `sourcemap` crate. Parse is equal to `oxc_sourcemap`. JSON output and a lookup are 1.1 to 1.4 times slower, and composition is 2 to 3.5 times slower. No measurement yet shows where a JavaScript tool spends much time on source maps. That measurement decides if the performance items under [Later](#later) are worth the work.

- [x] Benchmark parse, compose and lookup against `oxc_sourcemap` and the Sentry `sourcemap` crate
- [ ] Measure the share of source map work in a Vitest coverage run

### 2. Scopes (Stage 3)

`srcmap-scopes` decodes and encodes the [scopes proposal](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md). `srcmap-symbolicate` finds the original function name for a position, and it skips a hidden scope in that search.

- [x] Keep a variable whose index is past the end of `names`. [tc39/ecma426#244](https://github.com/tc39/ecma426/pull/244) gives it the empty string, and `variables` stays a `Vec<String>`. The decoder returned `InvalidNameIndex` before.
- [ ] Move to the wire layout of the [draft specification](https://tc39.es/ecma426/branch/proposal-scopes/). srcmap reads and writes the older layout of the proposal text, with one `scopes` string. The draft has these differences:
  - `scopes` is an array with one string for each source, and `ranges` is a second array of strings
  - a range definition is a source index and a scope index
  - tag `V` is the vendor extension item
  - tag `J` is a call site relative to a parent range
- [ ] `SymbolizeStackTrace`: translate a generated stack trace, expand inlined frames, and collapse outlined frames across bundles ([tc39/ecma426#219](https://github.com/tc39/ecma426/pull/219), open)
- [ ] `BuildScopeChain`: map the original variables at a position to concrete values, for a debugger
- [ ] Validate generated range boundaries against JavaScript syntax ([tc39/ecma426#249](https://github.com/tc39/ecma426/pull/249), open)
- [ ] Test against the Chrome DevTools codec ([@chrome-devtools/source-map-scopes-codec](https://jsr.io/@chrome-devtools/source-map-scopes-codec)). Versions 1.0.0 and later use the layout of the draft specification, so this test needs the move to that layout first.

### 3. Debug ID (Stage 2)

srcmap reads and writes `debugId` in the source map JSON. It also reads the `//# debugId=<UUID>` comment from the last 5 lines of a generated file, as the [debug ID proposal](https://github.com/tc39/ecma426/blob/main/proposals/debug-id.md) describes. The proposal specifies the comment only for JavaScript. The CSS form `/*# debugId=<UUID> */` is an srcmap extension.

- [x] `extract_debug_id(source: &str) -> Option<String>` in `srcmap-sourcemap` for the JavaScript comment, and `extract_css_debug_id` for the CSS comment
- [x] `srcmap validate --generated <PATH>` compares the debug ID of a generated file with the debug ID of its map
- [x] `srcmap-symbolicate` finds a source map by debug ID (`DebugIdIndex`, `symbolicate_with_debug_ids`)

### 4. Packages that copy an existing API

`@srcmap/remapping` gives the same output as `@jridgewell/remapping` on the four recorded Vite workloads. One difference is left.

- [ ] Accept a map as a JSON string without `version`, or with `rangeMappings` as an array. The wrapper corrects these fields only when the caller passes an object.
- [ ] Record a workload for `@srcmap/trace-mapping` and `@srcmap/gen-mapping`, and compare the output with the jridgewell packages

## Later

### Function name mappings

`sourcesFunctionMappings` is an array parallel to `sources`. Each entry maps generated positions to original function names. It comes from the Bloomberg extension `x_com_bloomberg_sourcesFunctionMappings` ([background](https://bloomberg.github.io/js-blog/post/standardizing-source-maps/)). It is not an ECMA-426 proposal.

The scopes proposal supersedes this field for function names. Most source maps have no scopes data, so a symbolicator still gains from it.

- [ ] Parse the field and decode it in `srcmap-sourcemap`
- [ ] `original_function_name_for(source, line, column)`
- [ ] Use it in `srcmap-symbolicate` when a map has no scopes
- [ ] Write it in `srcmap-generator`, and keep it through remapping and concatenation
- [ ] WASM bindings, and a line in `srcmap info`

### Rust performance

The benchmark against the other Rust crates gives the order of these items.

| Item | Affects |
|------|---------|
| Serialization cost | Rspack. Profile against rspack-sources before any change. |
| A composition mode without the `@jridgewell/remapping` rules | Rust users that compose maps and do not need that output |
| Lookup between two segments | All Rust users. `oxc_sourcemap` is 1.3 to 1.4 times faster. |
| SIMD VLQ decoding | All Rust users, most on maps of 1 MB and larger |

VLQ decoding takes 34% to 73% of the parse time on real maps (Preact 34%, Chart.js 73%, PDF.js 61%). The scalar decoder already has a single-character fast path and is fully inlined (see [perf_findings.md](https://github.com/fallow-rs/srcmap/commit/ebcac6a)). A SIMD decoder for base64 and VLQ can reduce that part.

A swap of the JSON parser does not help. Prototypes with `sonic-rs` and `simd-json` were slower on the input sizes that srcmap gets.

### Integration targets

| Target | Approach |
|--------|----------|
| Sentry CLI | Contribute improvements to `rust-sourcemap`, or offer srcmap for symbolicator |
| Webpack | Add a streaming API that matches the `StreamChunksOfCombinedSourceMap` pattern |
| Metro | Low priority. It needs several wrappers for custom source map extensions. |

## Watching

srcmap does not implement these yet. Each one starts when the proposal reaches Stage 2, or when a tool that srcmap serves emits the field.

| Proposal | Stage | What it adds |
|----------|-------|--------------|
| [Sources hash](https://github.com/tc39/ecma426/blob/main/proposals/sources-hash.md) | 1 | A `sourcesHash` array with a content hash per source |
| [Env](https://github.com/tc39/ecma426/blob/main/proposals/env.md) | 1 | Scopes and bindings of the original source |
| [Ignore try/catch](https://github.com/tc39/ecma426/blob/main/proposals/ignore-try-catch.md) | 1 | Marks a `try`/`catch` that a debugger must ignore for "pause on uncaught exceptions" |
| [Mappings v2](https://github.com/tc39/ecma426/issues/155) | Discussion | A smaller `mappings` encoding without `,` and `;` separators |

[Range mappings](https://github.com/tc39/ecma426/blob/main/proposals/range-mappings.md) is at Stage 2. srcmap parses the field, writes it, and keeps it through remapping and concatenation.

## Non-goals

- Source map v1 and v2 formats
- Tight coupling to one bundler or compiler
- A faster replacement for the `@jridgewell` packages in JavaScript
