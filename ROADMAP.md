# Roadmap

What's next for srcmap, and why.

## Range Mappings

**ECMA-426 proposal, Stage 2** — [tc39/ecma426#233](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)

A new `"rangeMappings"` field that marks certain mappings as covering an entire range, not just a single point. This directly solves precision loss during source map composition — the exact problem Rolldown hits ([rolldown#7555](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)).

### Encoding

The `rangeMappings` field uses the same `;`-separated line structure as `mappings`. Each line contains unsigned VLQs encoding **relative offsets** (1-based) to the index of each range mapping on that line. The offset is relative to the previous range mapping index on the same line.

```json
{
  "mappings": "AAAA,CAAC,GAAG,...",
  "rangeMappings": "ABCgB;;B"
}
```

Decoding `ABCgB` on line 1 gives offsets `[0, 1, 2, 32, 1]`, meaning mappings at indices 0, 1, 3, 35, and 36 are range mappings.

### Lookup behavior

A range mapping maps every position from its generated position up to (but not including) the next mapping. The original position is computed with a delta:

```
lineDelta = lookupLine - mapping.generatedLine
columnDelta = if lineDelta == 0 { lookupColumn - mapping.generatedColumn } else { 0 }
result = OriginalPosition {
    line: mapping.originalLine + lineDelta,
    column: mapping.originalColumn + columnDelta,
}
```

### Data model change

The `Mapping` struct gains an `is_range_mapping: bool` field (default `false`). On decode, `rangeMappings` is parsed and `is_range_mapping` is set on the referenced mappings. On encode, range mappings are collected and written to `rangeMappings`.

### What to implement

- [x] Decode `rangeMappings` field and set `is_range_mapping` on `Mapping` structs
- [x] Encode `rangeMappings` from `Mapping` structs with `is_range_mapping = true`
- [x] Update `original_position_for` to apply range delta when the matched mapping is a range mapping
- [x] Update `remap()` to preserve and compose range mappings through transform chains
- [x] Generator API: `add_range_mapping()` for marking a mapping as a range
- [x] WASM/NAPI bindings
- [x] CLI: show range mappings in `srcmap info` and `srcmap mappings`

---

## Streaming Source Map Composition

**Ecosystem demand** — [rolldown#8632](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)

Rolldown's `collapse_sourcemaps` materializes the entire token stream into `Vec<Token>` before constructing the final `SourceMap`. For large bundles with many transform plugins, this causes excessive allocations.

### What to implement

- [x] `MappingsIter`: lazy iterator over VLQ-encoded mappings (decodes one at a time, no `Vec<Mapping>`)
- [x] `StreamingGenerator`: on-the-fly VLQ encoder that emits mappings in sorted order without collecting
- [x] `remap_streaming()`: composition pipeline that streams through mappings without intermediate allocation
- [x] Criterion benchmarks (500 / 10K / 60K mappings) — 15-20% faster than materialized `remap()`
- [ ] Builder pattern for `SourceMap::new()` that consumes iterators for names/sources/source_contents
- [ ] Benchmark against Rolldown's current `collapse_sourcemaps`

---

## Scopes Spec Alignment

**ECMA-426 scopes proposal, actively evolving** — srcmap already has `srcmap-scopes`, but the spec is still changing.

### Open issues to track

**Boundary rules for generated ranges** ([tc39/ecma426#249](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)): Defines where generated ranges must start and end relative to JavaScript syntax. Key rules:

- Generated ranges must be strictly well-nested with syntactic scopes (no partial overlap)
- For callable scopes (functions, arrows, methods), the range starts inside the opening boundary (before default parameter evaluation)
- Boundary table maps ECMAScript productions to their opening/closing boundaries and callable status:
  - `FunctionDeclaration`: opening = `function` to `(`, closing = `}`, callable
  - `ArrowFunction`: opening = `ArrowParameters` to `=>`, closing = end of `ConciseBody`, callable
  - `BlockStatement`: opening = `{`, closing = `}`, not callable
  - `ClassDeclaration`: opening = `class` to `{`, closing = `}`, not callable

**Stack frame reconstruction algorithm** ([tc39/ecma426#219](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)): Three operations being standardized:

1. `FindOriginalFunctionName(position)` — find innermost generated range, walk scope chain outward to find `isStackFrame = true` scope
2. `SymbolizeStackTrace(rawFrames)` — translate generated stack traces to original, expand inlined frames and collapse outlined frames across different bundles
3. `BuildScopeChain(position)` — return `OriginalScopeWithValues[]` mapping original variables to concrete JS values

**Hidden scopes** ([tc39/ecma426#113](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)): A generated range without an `originalScope` reference signals compiler-generated code. Combined with `isFunctionScope`: if `range.isFunctionScope && !range.hasDefinition`, the function frame should be omitted from stack traces.

**Null variable names** ([tc39/ecma426#244](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)): When a `names` index is invalid, the variable entry becomes `null` (or empty string) instead of causing a parse error. Variables array type becomes `(string | null)[]`.

### What to implement

- [ ] Validate generated range boundaries against JavaScript syntax rules (#249)
- [ ] `FindOriginalFunctionName` in the symbolicate crate
- [ ] `SymbolizeStackTrace` with inlining expansion and outlining collapse (#219)
- [ ] `BuildScopeChain` for debugger integration
- [ ] Handle hidden scopes (`isFunctionScope && !hasDefinition`) in stack trace output
- [ ] Tolerate null variable names (#244)
- [ ] Track Chrome DevTools scopes codec ([@chrome-devtools/source-map-scopes-codec](https://jsr.io/@chrome-devtools/source-map-scopes-codec)) for compatibility

---

## Sources Hash

**ECMA-426 proposal, Stage 1** — [tc39/ecma426#208](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)

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

## Debug ID Extraction

**ECMA-426 proposal, Stage 2** — [tc39/ecma426#207](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)

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

## Source Map Diagnostics

Beyond basic validation, a deeper analysis mode for debugging broken source maps.

### What to implement

- [ ] **Mapping coverage** — what percentage of generated code has source mappings?
- [ ] **Redundant mapping detection** — consecutive mappings to the same original position ([sentry/rust-sourcemap#72](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md))
- [ ] **Composition chain validation** — given a chain of source maps, verify the composed result is correct
- [ ] **Size analysis** — breakdown of source map size by component (mappings, sourcesContent, names)
- [ ] **Mapping density** — mappings per line, identifying over/under-mapped regions
- [ ] CLI: `srcmap diagnose bundle.js.map` with `--json` output

---

## Parallel VLQ Encoding

Split the mappings string into chunks and encode VLQ segments in parallel using rayon. This is `oxc_sourcemap`'s main performance differentiator for generation workloads.

### What to implement

- [ ] Parallel VLQ encoding in `srcmap-generator` behind a `rayon` feature flag
- [ ] Benchmark against `oxc_sourcemap` parallel encoding
- [ ] Thread count configuration

---

## Mappings v2 Encoding

**ECMA-426 discussion** — [tc39/ecma426#155](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)

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

## Env Metadata

**ECMA-426 proposal, Stage 1** — [tc39/ecma426 proposals/env.md](https://github.com/nicolo-ribaudo/ecma-426/blob/main/proposals/range-mappings.md)

Environment metadata for source maps. The proposal is minimal at Stage 1 — track for details before implementing.

---

## Non-goals

- Full compatibility with the `mozilla/source-map` API surface
- Source map v1/v2 format support
- Tight coupling to any specific bundler or compiler
