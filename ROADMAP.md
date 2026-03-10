# srcmap Roadmap

High-performance source map tooling in Rust, with Node.js bindings via NAPI and WASM.

## Competitive Landscape

| Crate | Maintainer | Strength | Limitation |
|-------|-----------|----------|------------|
| `sourcemap` | Sentry | General-purpose consumer/producer | No parallel encoding, no composition |
| `oxc_sourcemap` | Oxc | Parallel encode, concat builder | Tightly coupled to Oxc/Rolldown |
| `parcel_sourcemap` | Parcel | WASM target, fast concat | Parcel-specific, no standalone use |
| `swc_sourcemap` | SWC | Lazy deserialization | SWC-internal, not reusable |

**Gap**: No standalone Rust crate provides parallel encoding + map composition/remapping + concat + both NAPI and WASM targets.

## Performance Status

### Rust Core (crates/sourcemap)
| Operation | srcmap (Rust) | @jridgewell/trace-mapping (JS) |
|-----------|--------------|-------------------------------|
| Parse 100K segments | 718μs | 326μs (V8 JSON.parse advantage) |
| Single lookup | **3ns** | 24ns (**8x faster**) |
| 1000x lookups | **5.5μs** | 15μs (**2.7x faster**) |

### Node.js WASM Binding (batch API)
| Operation | srcmap WASM batch | trace-mapping JS | Speedup |
|-----------|------------------|-----------------|---------|
| Medium 1000x lookup | **12.9μs** | 14.9μs | **1.15x faster** |
| Large 1000x lookup | **14.8μs** | 22.0μs | **1.49x faster** |
| Per lookup (batch) | **13-15ns** | 15-22ns | **~1.3x faster** |

### Node.js NAPI Binding
NAPI adds ~300ns per function call and ~2ms for large string marshalling. Individual NAPI lookups are uncompetitive, but batch lookups through WASM beat trace-mapping.

| Operation | srcmap NAPI | srcmap WASM individual | trace-mapping JS |
|-----------|------------|----------------------|-----------------|
| Parse 100K | 3,138μs | 3,702μs | 326μs |
| Single lookup | 351ns | 778ns | 25ns |
| Batch 1000x | 160μs | **13-15μs** | 15-22μs |

**Strategy**: WASM batch API is the competitive path for Node.js/browser. NAPI for integration with other native modules. Rust crate for build tools.

---

## Phase 0: Harden Codec ✅

- [x] Error handling: return `Result` from `decode`/`vlq_decode` instead of panicking on malformed input
- [x] Guard `vlq_decode` against non-ASCII bytes (OOB on `BASE64_DECODE[128]`)
- [x] Cap VLQ shift to prevent overflow/infinite loop on crafted input
- [x] Guard `vlq_encode` against `i64::MIN` (negation overflow — uses u64 internally)
- [x] Fix `encode` emitting dangling commas for empty segments
- [x] Verify trailing `;` behavior matches `@jridgewell/sourcemap-codec`
- [x] Add adversarial/fuzz tests (invalid base64, truncated VLQ, non-ASCII, truncated segments)
- [x] Add realistic benchmark fixture (varied deltas, multi-byte VLQ sequences)
- [x] Remove unused `serde-json` feature and `serde_json` dependency from NAPI package

## Phase 1: Publish Codec ✅

- [x] VLQ encode/decode primitives
- [x] Source map mappings decode (`mappings` string → structured data)
- [x] Source map mappings encode (structured data → `mappings` string)
- [x] Node.js NAPI bindings (`@srcmap/codec`)
- [x] Criterion benchmarks (Rust)
- [x] Comparative benchmarks vs `@jridgewell/sourcemap-codec`
- [x] README with usage examples and benchmark results
- [x] LICENSE file (MIT)
- [x] GitHub Actions CI: test on Linux, macOS, Windows
- [x] GitHub Actions release workflow: build native binaries for all platforms
- [x] Add `exports` field to package.json for ESM consumers
- [x] Publish `srcmap-codec` to crates.io (0.1.2)
- [x] Publish `@srcmap/codec` to npm (0.1.2)

## Phase 2: Source Map Parser + Consumer ✅

Parser and consumer are tightly coupled — ship together as one crate.
Matches `@jridgewell/trace-mapping` in the JS ecosystem.

- [x] `crates/sourcemap` — full source map v3 parser (ECMA-426)
- [x] Parse all fields: `version`, `sources`, `sourcesContent`, `names`, `file`, `sourceRoot`, `mappings`
- [x] Support `ignoreList` field (third-party source filtering)
- [x] Validation and structured error reporting
- [x] Original position lookup: `original_position_for(line, col)` — binary search, O(log n)
- [x] Generated position lookup: `generated_position_for(source, line, col)` — reverse index
- [x] Iterate all mappings / mappings for a given source file
- [x] Compact Mapping struct (24 bytes, 6×u32) — cache-friendly flat layout
- [x] Lazy reverse index — only built on first `generated_position_for` call
- [x] Inlined VLQ decoder with single-char fast path
- [x] Node.js NAPI bindings (`@srcmap/sourcemap`) with batch API
- [x] Criterion benchmarks (Rust) + comparative benchmarks vs trace-mapping
- [x] Correctness verification against trace-mapping
- [x] Support indexed source maps (sections)
- [x] WASM bindings (`@srcmap/sourcemap-wasm`) — batch API **1.3-1.5x faster** than trace-mapping
- [x] README with usage examples and benchmark results
- [x] Comprehensive test suite (90+ tests: edge cases, malformed input, spec conformance)

## Phase 3: Source Map Generator ✅

Matches `@jridgewell/gen-mapping` in the JS ecosystem.

- [x] Build source maps from scratch
- [x] `add_mapping(generated, original, source, name)` — incremental segment addition
- [x] `maybe_add_mapping` — skip redundant mappings (important for map size)
- [x] `sourcesContent` embedding
- [x] Parallel VLQ encoding (encode segments concurrently, join results) — `parallel` feature, 1.2-1.5x faster at scale
- [x] Parallel `sourcesContent` JSON quoting (expensive for large sources) — `parallel` feature
- [x] Output to JSON (`to_json`) — generates valid source map v3 JSON

## Phase 4: Concatenation + Composition ✅

The biggest gap in the Rust ecosystem. Matches `@ampproject/remapping` (39M weekly npm downloads).

- [x] **Source map concatenation** — merge maps from bundled files, rebase line/column offsets
  - Used by every bundler (esbuild, Rollup, Webpack, Rolldown)
  - `ConcatBuilder` API with source/name deduplication
- [x] **Source map composition/remapping** — chain maps through multiple transforms
  - TS → JS → minified: compose 2+ maps into one pointing to original source
  - Loader-based API: `remap(output_map, |source| load_upstream_map(source))`
  - First standalone Rust implementation

## Phase 5: CLI Tool ✅

Agent-friendly command-line interface for inspecting, validating, composing, and manipulating source maps.

- [x] 9 subcommands: `info`, `validate`, `lookup`, `resolve`, `decode`, `encode`, `mappings`, `concat`, `remap`
- [x] `--json` flag on all commands for structured machine-readable output
- [x] `srcmap schema` command — runtime introspection of all commands, args, types, and flags as JSON
- [x] `--dry-run` for mutating commands (`concat`, `remap`) — validate without writing
- [x] Structured JSON error output with error codes (`IO_ERROR`, `PARSE_ERROR`, `NOT_FOUND`, `VALIDATION_ERROR`, `PATH_TRAVERSAL`, `INVALID_INPUT`)
- [x] Input hardening: reject control characters, path traversals, percent-encoding, `?`/`#` in source names
- [x] Output path sandboxing — all file writes validated against CWD
- [x] Remap directory search sandboxed — canonicalized paths verified within search directory
- [x] stdin support (`-`) for all file-reading commands
- [x] Pagination metadata (`total`, `offset`, `hasMore`) in `mappings --json` output

## Publishing

- [x] Publish `srcmap-codec` to crates.io (0.1.2)
- [x] Publish `srcmap-sourcemap` to crates.io (0.1.2)
- [x] Publish `srcmap-generator` to crates.io (0.1.2)
- [x] Publish `srcmap-remapping` to crates.io (0.1.2)
- [ ] Publish `srcmap-cli` to crates.io
- [x] Publish `@srcmap/codec` to npm (0.1.2)
- [x] Publish `@srcmap/sourcemap` to npm (0.1.2)
- [x] Publish `@srcmap/sourcemap-wasm` to npm (0.1.2)

## Future

- [ ] Debug ID support (`debugId` field, part of ECMA-426)
- [ ] Node.js bindings for generator and remapping (NAPI + WASM)
- [ ] WASM build target for browser (devtools, playgrounds, edge runtimes)
- [ ] Scopes & variables support (ECMA-426 proposal — no library supports this yet)
- [ ] Streaming/lazy decode for very large source maps

## Non-goals

- Full compatibility with the `mozilla/source-map` API surface — focus on performance and correctness over legacy API shape
- Support for source map v1/v2 formats
- Tight coupling to any specific bundler or compiler
