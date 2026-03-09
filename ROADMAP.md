# srcmap Roadmap

High-performance source map tooling in Rust, with Node.js bindings via NAPI.

## Competitive Landscape

| Crate | Maintainer | Strength | Limitation |
|-------|-----------|----------|------------|
| `sourcemap` | Sentry | General-purpose consumer/producer | No parallel encoding, no composition |
| `oxc_sourcemap` | Oxc | Parallel encode, concat builder | Tightly coupled to Oxc/Rolldown |
| `parcel_sourcemap` | Parcel | WASM target, fast concat | Parcel-specific, no standalone use |
| `swc_sourcemap` | SWC | Lazy deserialization | SWC-internal, not reusable |

**Gap**: No standalone Rust crate provides parallel encoding + map composition/remapping + concat + both NAPI and WASM targets.

---

## Phase 0: Harden Codec (priority — before publishing)

- [x] Error handling: return `Result` from `decode`/`vlq_decode` instead of panicking on malformed input
- [x] Guard `vlq_decode` against non-ASCII bytes (OOB on `BASE64_DECODE[128]`)
- [x] Cap VLQ shift to prevent overflow/infinite loop on crafted input
- [x] Guard `vlq_encode` against `i64::MIN` (negation overflow — uses u64 internally)
- [x] Fix `encode` emitting dangling commas for empty segments
- [x] Verify trailing `;` behavior matches `@jridgewell/sourcemap-codec`
- [x] Add adversarial/fuzz tests (invalid base64, truncated VLQ, non-ASCII, truncated segments)
- [x] Add realistic benchmark fixture (varied deltas, multi-byte VLQ sequences)
- [x] Remove unused `serde-json` feature and `serde_json` dependency from NAPI package

## Phase 1: Publish Codec

- [x] VLQ encode/decode primitives
- [x] Source map mappings decode (`mappings` string → structured data)
- [x] Source map mappings encode (structured data → `mappings` string)
- [x] Node.js NAPI bindings (`@srcmap/codec`)
- [x] Criterion benchmarks (Rust)
- [x] Comparative benchmarks vs `@jridgewell/sourcemap-codec`
- [ ] README with usage examples and benchmark results
- [ ] LICENSE file (MIT)
- [ ] NAPI integration tests (JS-side roundtrip tests)
- [ ] GitHub Actions CI: test on Linux, macOS, Windows
- [ ] GitHub Actions release workflow: build native binaries for all platforms
- [ ] Generate `index.js` / `index.d.ts` via `napi build`
- [ ] Add `exports` field to package.json for ESM consumers
- [ ] Publish `srcmap-codec` to crates.io
- [ ] Publish `@srcmap/codec` to npm

## Phase 2: Source Map Parser + Consumer

Parser and consumer are tightly coupled — ship together as one crate.
Matches `@jridgewell/trace-mapping` in the JS ecosystem.

- [ ] `crates/sourcemap` — full source map v3 parser (ECMA-426)
- [ ] Parse all fields: `version`, `sources`, `sourcesContent`, `names`, `file`, `sourceRoot`, `mappings`
- [ ] Support `ignoreList` field (third-party source filtering)
- [ ] Support indexed source maps (sections)
- [ ] Validation and structured error reporting
- [ ] Original position lookup: `original_position_for(line, col)` — binary search, O(log n)
- [ ] Generated position lookup: `generated_position_for(source, line, col)`
- [ ] Iterate all mappings / mappings for a given source file
- [ ] Debug ID support (ECMA-426 Stage 2 proposal — already adopted by Webpack, Rollup, Vite, Bun)
- [ ] Node.js NAPI bindings (`@srcmap/sourcemap`)
- [ ] API compatible with `@jridgewell/trace-mapping` conventions

## Phase 3: Source Map Generator

Matches `@jridgewell/gen-mapping` in the JS ecosystem.

- [ ] Build source maps from scratch
- [ ] `add_mapping(generated, original, source, name)` — incremental segment addition
- [ ] `maybe_add_segment` — skip redundant mappings (important for map size)
- [ ] `sourcesContent` embedding
- [ ] Parallel VLQ encoding (encode segments concurrently, join results)
- [ ] Parallel `sourcesContent` JSON quoting (expensive for large sources)
- [ ] Output to JSON (`to_encoded_map` / `to_decoded_map`)
- [ ] Node.js NAPI bindings (`@srcmap/gen`)
- [ ] API compatible with `@jridgewell/gen-mapping` conventions

## Phase 4: Concatenation + Composition

The biggest gap in the Rust ecosystem. Matches `@ampproject/remapping` (39M weekly npm downloads).

- [ ] **Source map concatenation** — merge maps from bundled files, rebase line/column offsets
  - Used by every bundler (esbuild, Rollup, Webpack, Rolldown)
  - `ConcatSourceMapBuilder` API (similar to oxc_sourcemap)
- [ ] **Source map composition/remapping** — chain maps through multiple transforms
  - TS → JS → minified: compose 2+ maps into one pointing to original source
  - Loader-based API: `remap(output_map, |source| load_upstream_map(source))`
  - No standalone Rust implementation exists today
- [ ] Node.js NAPI bindings (`@srcmap/remap`)

## Phase 5: Advanced Features

- [ ] WASM build target (browser devtools, online playgrounds, edge runtimes)
- [ ] Streaming/lazy decode for very large source maps
- [ ] CLI tool: inspect, validate, compose, and manipulate source maps
- [ ] Scopes & variables support (ECMA-426 proposal — no library supports this yet)

## Non-goals

- Full compatibility with the `mozilla/source-map` API surface — focus on performance and correctness over legacy API shape
- Support for source map v1/v2 formats
- Tight coupling to any specific bundler or compiler
