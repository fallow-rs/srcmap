# Agentic Use Cases for `srcmap-cli`

Critical and pragmatic assessment of where `srcmap-cli` fits into agentic workflows.

## What makes `srcmap-cli` agent-friendly today

The CLI has solid foundations for agent consumption:
- `--json` on every command for structured output
- Typed error codes (`IO_ERROR`, `PARSE_ERROR`, `NOT_FOUND`, `VALIDATION_ERROR`, `PATH_TRAVERSAL`, `INVALID_INPUT`)
- `srcmap schema` for programmatic introspection of all commands
- stdin support for pipeline composition
- `--dry-run` on mutating commands

---

## Tier 1: High-value, realistic today

### 1. Automated error triage in production monitoring pipelines

An agent receives a minified stack trace from a crash reporter (Sentry, Datadog, PagerDuty alert), calls `srcmap symbolicate` to resolve it to original source, then reasons about the root cause.

```
alert → agent → srcmap symbolicate stack.txt --map bundle.js.map --json → agent reads original source → drafts issue/PR
```

**Why it's compelling:** Engineers waste time manually symbolicating traces. The structured JSON output lets the agent parse results reliably. The tool handles the hard part (VLQ decoding, binary search); the agent handles the reasoning part (root cause analysis, fix proposal).

**Caveat:** The agent needs access to the source maps, which are often stored in artifact storage (S3, GCS), not locally. A real pipeline needs a fetching layer around this.

### 2. Build pipeline validation and debugging

When a multi-stage build (TypeScript → Babel → Terser) produces broken source maps, an agent can:
- Run `srcmap validate` on each intermediate map
- Run `srcmap info` to compare source counts and mapping counts across stages
- Run `srcmap remap --dry-run` to verify composition would succeed
- Diagnose where in the chain mappings broke

A "source map health check" agent that runs after builds and flags regressions in mapping quality (e.g., "Terser dropped 40% of name mappings compared to last build") is a practical CI addition.

### 3. Debugging mapping accuracy during bundler/compiler development

For teams building or maintaining tools like esbuild, SWC, Rollup, or oxc, an agent can use `srcmap lookup` and `srcmap resolve` to spot-check that generated positions round-trip correctly. Feed it known positions, verify each one, report discrepancies. Automated source map fuzz-testing with the agent as oracle.

---

## Tier 2: Useful but more niche

### 4. MCP server wrapping `srcmap-cli` for IDE agents

Wrapping the CLI as an MCP tool server so IDE-embedded agents (Copilot, Claude Code, Cursor) can symbolicate on the fly. When an agent sees a minified error in a terminal, it resolves it without the developer switching context.

**Pragmatic concern:** Value depends on how often the developer hits minified traces in their IDE. For React Native developers, this could be daily; for backend developers, never.

### 5. Source map migration/upgrade automation

An agent that audits a monorepo's source maps for spec compliance:
- Detecting and flattening indexed source maps
- Validating ECMA-426 compliance across hundreds of output bundles
- Checking for path traversal vulnerabilities in `sources` arrays

A "run once per quarter" task, but `srcmap validate --json` makes it trivially automatable.

### 6. React Native / Hermes crash analysis

Hermes bytecode crashes produce positions that need Hermes-specific source map extensions to resolve. An agent handling the full pipeline (RAM bundle parsing → Hermes offset resolution → symbolication) is genuinely useful for React Native teams. The Hermes and RAM bundle crates give `srcmap` a differentiator here that competitors lack.

---

## Tier 3: Theoretically interesting, practically questionable

### 7. "Explain this minified code" agent

An agent uses `srcmap mappings` to walk through segments and reconstructs a human-readable explanation by cross-referencing original sources.

**Skepticism:** LLMs can already read minified code reasonably well. If you have the source map, you usually also have the original source.

### 8. Automated source map generation for legacy code

An agent generates source maps for hand-written transformations lacking them, using `srcmap encode`.

**Skepticism:** The agent would need to understand transformation semantics deeply enough to produce correct mappings. This is a hard problem that source maps alone don't solve.

---

## Gaps for stronger agentic adoption

1. **No batch mode** — looking up 50 positions means 50 process spawns. A batch lookup command (JSON array in, JSON array out) would cut overhead significantly.

2. **No URL/artifact fetching** — the tool only works with local files. A `--fetch` flag or `srcmap fetch` command would eliminate a step for cloud pipelines.

3. **No diff/comparison command** — "how did the source map change between builds?" requires calling `info` twice and diffing manually.

4. **Schema could include examples** — for tool-using LLMs, a concrete example per command dramatically improves first-try accuracy.

---

## Bottom line

The strongest agentic use case is **production crash triage** — symbolicating stack traces and feeding results into an agent for root cause analysis. Clear, recurring pain point with real ROI.

The second strongest is **build pipeline validation** — catching source map regressions automatically in CI.

Everything else is either niche (Hermes/RN-specific) or more theoretical than practical. The main gaps for deeper adoption are batch operations and remote fetching.
