# srcmap Roadmap

High-performance source map tooling in Rust, with Node.js bindings via NAPI and WASM.

## Completed

| Phase | Description |
|-------|-------------|
| **Phase 0** | Hardened VLQ codec with error handling and safety guards |
| **Phase 1** | VLQ codec crate + NAPI bindings, published to crates.io and npm |
| **Phase 2** | Source map parser + consumer with NAPI and WASM bindings (batch API 1.3-1.5x faster than trace-mapping) |
| **Phase 3** | Source map generator with parallel VLQ encoding |
| **Phase 4** | Concatenation + composition/remapping (first standalone Rust implementation) |
| **Phase 5** | CLI tool with 9 subcommands, structured JSON output, agent introspection, input hardening |
| **Phase 6** | WASM bindings for generator and remapping |
| **Phase 7** | Scopes & variables — first Rust implementation of the ECMA-426 scopes proposal |
| **Phase 8** | Drop-in `@jridgewell/trace-mapping` compatibility wrapper (`@srcmap/trace-mapping`) |
| **Publishing** | All 6 crates on crates.io, all 6 npm packages published |

## Competitive Landscape

| Crate | Maintainer | Strength | Limitation |
|-------|-----------|----------|------------|
| `sourcemap` | Sentry | General-purpose consumer/producer | No parallel encoding, no composition |
| `oxc_sourcemap` | Oxc | Parallel encode, concat builder | Tightly coupled to Oxc/Rolldown |
| `parcel_sourcemap` | Parcel | WASM target, fast concat | Parcel-specific, no standalone use |
| `swc_sourcemap` | SWC | Lazy deserialization | SWC-internal, not reusable |

**Gap**: No standalone Rust crate provides parallel encoding + map composition/remapping + concat + scopes + both NAPI and WASM targets.

---

## Phase 8: Drop-in trace-mapping Compatibility

A thin wrapper around `@srcmap/sourcemap-wasm` that implements the `@jridgewell/trace-mapping` API surface. Enables zero-effort migration for the 80M+ weekly downloads ecosystem.

- [x] `TraceMap` class matching `@jridgewell/trace-mapping` API
- [x] `originalPositionFor` / `generatedPositionFor` with matching return types
- [x] `allGeneratedPositionsFor(source, line, column)` — all generated positions for a given original location (needed for breakpoint setting)
- [x] `eachMapping(callback)` iteration API
- [x] `sourceContentFor(source)` convenience accessor
- [x] `isIgnored(source)` for `ignoreList` checking
- [x] `presortedDecodedMap` constructor for pre-decoded input
- [x] Drop-in benchmark comparison vs trace-mapping
- [ ] Publish as `@srcmap/trace-mapping` to npm

## Phase 9: Lookup Bias & Range Mapping

Critical for coverage mapping and stack trace resolution where exact positions often don't match.

- [x] **Bias parameter** — `LEAST_UPPER_BOUND` / `GREATEST_LOWER_BOUND` on `originalPositionFor` and `generatedPositionFor`
- [x] **Range-to-range mapping** — map a range `(startLine:startCol → endLine:endCol)` through a source map, not just individual positions
- [x] **`allGeneratedPositionsFor`** — return all generated positions for a given original location
- [x] Expose bias in WASM and NAPI bindings
- [x] Expose bias in CLI `lookup` and `resolve` commands

## Phase 10: Extension Fields & Spec Conformance

- [x] **Extension field passthrough** — preserve unknown `x_*` fields when reading and re-emitting source maps (tools like Metro use `x_facebook_sources`, Chrome uses `x_google_linecount`)
- [x] **`x_google_ignoreList` fallback** — read deprecated field when `ignoreList` is absent
- [x] **`sourceMappingURL` parsing** — extract source map references from generated files (inline data URIs and external URLs)
- [x] **tc39/source-map-tests integration** — run the official cross-implementation conformance test suite in CI
- [x] **Deep validation** — bounds checking, segment ordering, source resolution, unreferenced sources detection (beyond JSON schema validation)
- [x] **`excludeContent` option** — strip `sourcesContent` from output to reduce map size

## Phase 11: Stack Trace Symbolication

High-value feature — no good standalone library exists for this.

- [x] **Stack trace parser** — parse V8, SpiderMonkey, and JavaScriptCore stack trace formats into structured frames
- [x] **`symbolicate(stackTrace, sourceMapLoader)`** — resolve each frame through source maps, return readable stack trace
- [x] **Batch symbolication** — resolve multiple stack traces against pre-loaded source maps efficiently (error monitoring use case)
- [x] **Debug ID resolution** — given a `debugId`, look up the corresponding source map
- [x] CLI `symbolicate` command with `--json` output
- [x] WASM bindings for browser-side symbolication

## Phase 12: Browser WASM Target

WASM builds targeting browsers for DevTools extensions, online playgrounds, and edge runtimes.

- [x] `--target web` and `--target bundler` builds for all WASM packages
- [x] Minimal JS wrapper with async initialization
- [x] Bundle size optimization (tree-shaking, wasm-opt)
- [x] Published to npm with `browser` and `module` exports
- [x] Example: source map visualization web component

## Performance & Scalability

- [ ] **Streaming/lazy decode** — parse source map JSON lazily, only decoding mappings on demand (for 100MB+ maps)
- [ ] **Incremental parsing** — decode only a subset of mappings (e.g., lines 100-200) without processing the entire map
- [ ] **Generator `toDecodedMap`** — output decoded segments directly, avoiding encode-then-decode round-trips in composition pipelines

## Binding Gaps

| Feature | Rust | NAPI | WASM |
|---------|------|------|------|
| Codec | yes | yes | no |
| Sourcemap | yes | yes | yes |
| Generator | yes | no | yes |
| Remapping | yes | no | yes |
| Scopes | yes | no | no |

- [ ] WASM bindings for scopes decode/encode
- [ ] NAPI bindings for generator
- [ ] NAPI bindings for remapping

## Non-goals

- Full compatibility with the `mozilla/source-map` API surface — focus on performance and correctness over legacy API shape
- Support for source map v1/v2 formats
- Tight coupling to any specific bundler or compiler
