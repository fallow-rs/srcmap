# Roadmap

What's next for srcmap, and why.

## Function Name Mappings

**Interop work, not part of the published ECMA-426 core.** See [bloomberg.github.io/js-blog/post/standardizing-source-maps](https://bloomberg.github.io/js-blog/post/standardizing-source-maps/)

A `"sourcesFunctionMappings"` array parallel to `"sources"`, where each entry is a VLQ-encoded string mapping generated positions to original function names. Previously the `x_com_bloomberg_sourcesFunctionMappings` extension, and worth tracking for ecosystem interoperability.

Note: this repository tracks it as forward-looking ecosystem interoperability work. It is not currently in the published ECMA-426 core standard or the official `tc39/ecma426` proposal set.

This is a simpler alternative to the full scopes proposal for resolving minified function names in stack traces. A tool that does not need full scope and binding information can use this field alone. For tools that support scopes, `FindOriginalFunctionName` from the scopes proposal supersedes this. srcmap must support both for interop, because most source maps have no scopes data.

### What to implement

- [ ] Parse `sourcesFunctionMappings` field in `srcmap-sourcemap`
- [ ] Decode per-source function name mappings (VLQ → function name index at position)
- [ ] `original_function_name_for(source, line, col)` lookup API
- [ ] Generate `sourcesFunctionMappings` in `srcmap-generator`
- [ ] Preserve through remapping/concatenation
- [ ] Use in symbolicate crate as fallback when scopes data is absent
- [ ] WASM bindings
- [ ] CLI: show function mappings in `srcmap info`

---

## Scopes Spec Alignment

**ECMA-426 scopes proposal, still changing.** srcmap already has `srcmap-scopes`. The scopes proposal is the broader approach to debugging metadata. It supersedes `sourcesFunctionMappings` for function name resolution and adds variable bindings, scope chains, and inlined frame reconstruction.

### Open issues to track

**Boundary rules for generated ranges** ([tc39/ecma426#249](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md)): Defines where generated ranges must start and end relative to JavaScript syntax. Key rules:

- Generated ranges must be strictly well-nested with syntactic scopes (no partial overlap)
- For callable scopes (functions, arrows, methods), the range starts inside the opening boundary (before default parameter evaluation)
- Boundary table maps ECMAScript productions to their opening/closing boundaries and callable status:
  - `FunctionDeclaration`: opening = `function` to `(`, closing = `}`, callable
  - `ArrowFunction`: opening = `ArrowParameters` to `=>`, closing = end of `ConciseBody`, callable
  - `BlockStatement`: opening = `{`, closing = `}`, not callable
  - `ClassDeclaration`: opening = `class` to `{`, closing = `}`, not callable

**Stack frame reconstruction algorithm** ([tc39/ecma426#219](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md)): Three operations being standardized:

1. `FindOriginalFunctionName(position)`: find the innermost generated range, then walk the scope chain outward to the first scope with `isStackFrame = true`
2. `SymbolizeStackTrace(rawFrames)`: translate a generated stack trace to the original one, expand inlined frames, and collapse outlined frames across bundles
3. `BuildScopeChain(position)`: return `OriginalScopeWithValues[]`, which maps the original variables to concrete JS values

**Hidden scopes** ([tc39/ecma426#113](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md)): A generated range without an `originalScope` reference signals compiler-generated code. Combined with `isFunctionScope`: if `range.isFunctionScope && !range.hasDefinition`, the function frame should be omitted from stack traces.

**Null variable names** ([tc39/ecma426#244](https://github.com/tc39/ecma426/blob/main/proposals/scopes.md)): When a `names` index is invalid, the variable entry becomes `null` (or empty string) instead of causing a parse error. Variables array type becomes `(string | null)[]`.

### What to implement

- [ ] Validate generated range boundaries against JavaScript syntax rules (#249)
- [x] `FindOriginalFunctionName` in the symbolicate crate
- [ ] `SymbolizeStackTrace` with inlining expansion and outlining collapse (#219)
- [ ] `BuildScopeChain` for debugger integration
- [x] Handle hidden scopes (`isFunctionScope && !hasDefinition`) in stack trace output
- [ ] Tolerate null variable names (#244)
- [ ] Track Chrome DevTools scopes codec ([@chrome-devtools/source-map-scopes-codec](https://jsr.io/@chrome-devtools/source-map-scopes-codec)) for compatibility

---

## Sources Hash `[Stage 1, not implementing yet]`

**ECMA-426 proposal, Stage 1**: [sources-hash proposal](https://github.com/tc39/ecma426/blob/main/proposals/sources-hash.md)

A new `"sourcesHash"` array parallel to `"sources"`, containing content hashes for source file integrity verification.

```json
{
  "sources": ["src/foo.ts", "src/bar.ts"],
  "sourcesHash": ["sha256-abc123...", "sha256-def456..."]
}
```

Hash algorithm is implementation-defined (SHA-256 recommended). Format is a prefixed string like `"sha256-<hex>"`.

### Use cases

- Deduplication of sources across code-split source maps
- Cache invalidation during HMR without comparing full source content
- Skipping redundant network fetches when `sourcesContent` is omitted

### What to implement

- [ ] Parse `sourcesHash` field
- [ ] Generate `sourcesHash` from `sourcesContent` (SHA-256 default)
- [ ] Verify source content against hash
- [ ] Preserve through remapping/concatenation
- [ ] CLI: show hashes in `srcmap info`, verify with `srcmap validate`

---

## Debug ID Extraction `[Stage 2, not implementing yet]`

**ECMA-426 proposal, Stage 2**: [debug-id proposal](https://github.com/tc39/ecma426/blob/main/proposals/debug-id.md)

srcmap already parses `debugId` from source map JSON. What's missing is extracting it from generated JavaScript/CSS files via the `//# debugId=<UUID>` comment.

### Spec extraction rules

- Scan the **last 5 lines** of the generated file
- Match pattern: `//# debugId=<UUID>` (JS) or `/*# debugId=<UUID> */` (CSS)
- Only `//# ` prefix (no legacy `//@` support, unlike `sourceMappingURL`)
- Return the **first** match found scanning from end
- UUID format: canonical 128-bit hex with dashes (`85314830-023f-4cf1-a267-535f4e37bb17`)
- For reproducible builds, UUIDv3/v5 based on content hash is recommended

### What to implement

- [ ] `extract_debug_id(source: &str) -> Option<String>` in `srcmap-sourcemap`
- [ ] CSS comment format support (`/*# debugId=... */`)
- [ ] Match debug IDs between generated file and source map for validation
- [ ] CLI: `srcmap validate` checks debug ID consistency
- [ ] Symbolicate crate: resolve source maps by debug ID

---

## Mappings v2 Encoding `[Discussion, not implementing yet]`

**ECMA-426 discussion**: [range-mappings proposal](https://github.com/tc39/ecma426/blob/main/proposals/range-mappings.md)

A proposed encoding that eliminates `,` and `;` separators and packs metadata into VLQ bits. Claims ~35-50% raw size reduction.

### Encoding changes

1. **Prefix with line count**, then mapping count per line, then emit all VLQs without separators
2. **Bit flags in genColumn** (lowest 4 bits of unsigned VLQ):
   - Bits 0,2: mapping length (`0b01` = 4-field, `0b11` = 5-field, else 1-field)
   - Bit 1: `sourcesIndexPresent` (if 0, delta = 0, reuse last)
   - Bit 3: `sourceLinePresent` (if 0, delta = 0, reuse last)
   - Actual `genColumn = data >>> 4`
3. **8-bit VLQ option**: 7 data bits + 1 continuation bit per byte (binary) instead of 5+1 per base64 char

### Size reduction benchmarks (Google Search, 2.79MB source map)

| Variant | Raw | gzip-6 | brotli-6 |
|---|---|---|---|
| Flags (6-bit VLQ) | -34.9% | -6.0% | -5.6% |
| Flags (8-bit VLQ) | -48.1% | -8.2% | -8.0% |
| Flags + remove 1-length (8-bit VLQ) | -48.8% | -9.6% | -9.5% |

This is early-stage and may never land. Worth implementing as an experimental encoder/decoder behind a feature flag if the proposal advances.

### What to implement

- [ ] Experimental v2 encoder behind `mappings-v2` feature flag
- [ ] v2 decoder
- [ ] Benchmark against v3 encoding on real-world source maps
- [ ] Track proposal status

---

## Env Metadata `[Stage 1, not implementing yet]`

**ECMA-426 proposal, Stage 1**: [env proposal](https://github.com/tc39/ecma426/blob/main/proposals/env.md)

Environment metadata for source maps. The proposal is minimal at Stage 1. Track it for details before any implementation.

---

## Positioning

srcmap is a source map toolkit for Rust, with Node.js bindings and a CLI. It follows the specification and its draft proposals. It composes, symbolicates and inspects maps, and it reads Hermes maps and scopes.

Speed in JavaScript is not a goal. Measurements in November 2026 gave these results:

- `@jridgewell/trace-mapping` parses faster and does a single lookup faster than the WASM and NAPI bindings.
- In four recorded Vite 8 workloads, `combineSourcemaps` took 0.1% to 2.5% of the wall time (see `benchmarks/vite-workload/`).
- Rolldown composes maps in Rust with `oxc_sourcemap`.

### Next

- [ ] Make the packages that copy an existing API accept real inputs: a decoded map without `version`, and `rangeMappings` as an array
- [ ] Benchmark parse, compose and lookup against `oxc_sourcemap` and the Sentry `sourcemap` crate
- [ ] Measure the share of source map work in a Vitest coverage run

### Rust performance gaps

These items wait for the benchmark against the other Rust crates.

| Gap | Severity | Affects |
|-----|----------|---------|
| Serialization cost | Medium | Rspack |
| SIMD VLQ decoding | Medium | All Rust users, most on maps of 1 MB and larger |

Profile serialization against rspack-sources before any change.

VLQ decoding takes 34% to 73% of the parse time on real maps (Preact 34%, Chart.js 73%, PDF.js 61%). The scalar decoder already has a single-character fast path and is fully inlined (see [perf_findings.md](https://github.com/fallow-rs/srcmap/commit/ebcac6a)). A SIMD decoder for base64 and VLQ can reduce that part. It needs `base64-simd` with custom VLQ logic, or a new SIMD pass.

A swap of the JSON parser does not help. Prototypes with `sonic-rs` and `simd-json` were slower on the input sizes that srcmap gets, because JSON parsing is not the bottleneck.

### Integration targets

| Target | Approach |
|--------|----------|
| Sentry CLI | Contribute improvements to `rust-sourcemap`, or offer srcmap for symbolicator |
| Webpack | Add a streaming API that matches the `StreamChunksOfCombinedSourceMap` pattern |
| Metro | Low priority. It needs several wrappers for custom source map extensions |

---

## Non-goals

- Source map v1/v2 format support
- Tight coupling to any specific bundler or compiler
- A faster replacement for the `@jridgewell` packages in JavaScript
