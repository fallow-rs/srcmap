import { createBench, latencyMeanMs, latencyP99Ms, throughputHz } from "./codspeed.mjs";
import { createDeterministicLookups, setFailureExitCode } from "./workload.mjs";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { TraceMap, originalPositionFor } from "@jridgewell/trace-mapping";
import { SourceMapConsumer } from "source-map-js";
import { SourceMap } from "../packages/sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const sourcemapWasm = require("../packages/sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js");
const { LazySourceMap: FastSourceMap } = sourcemapWasm;
import { SourceMap as NapiSourceMap } from "../packages/sourcemap/index.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const fixturesDir = join(__dirname, "fixtures");
const LOOKUP_COUNT = 1_000;
const LOOKUP_MAX_COLUMN = 200;
const LOOKUP_SEED = 0x5eed1234;

const createOrderedLookups = (count, maxLine, descending) =>
  Array.from({ length: count }, (_, index) => {
    const line = Math.floor((index * maxLine) / count);
    return {
      line: descending ? maxLine - 1 - line : line,
      column: (index * 13) % LOOKUP_MAX_COLUMN,
    };
  });

const createLookupPatterns = (maxLine) => {
  const midLine = Math.floor(maxLine / 2);
  return [
    { name: "ascending", lookups: createOrderedLookups(LOOKUP_COUNT, maxLine, false) },
    { name: "descending", lookups: createOrderedLookups(LOOKUP_COUNT, maxLine, true) },
    {
      name: "repeated",
      lookups: Array.from({ length: LOOKUP_COUNT }, () => ({ line: midLine, column: 20 })),
    },
    {
      name: "randomized",
      lookups: createDeterministicLookups(LOOKUP_COUNT, maxLine, LOOKUP_MAX_COLUMN, LOOKUP_SEED),
    },
  ];
};

const assertLazyMatchesEager = (eager, lazy, lookups, context) => {
  for (const { line, column } of lookups) {
    assert.deepEqual(
      lazy.originalPositionFor(line, column),
      eager.originalPositionFor(line, column),
      `${context} at ${line}:${column}`,
    );
  }
};

const consumeLookups = (map, lookups) => {
  let checksum = 2_166_136_261;

  for (const { line, column } of lookups) {
    const result = map.originalPositionFor(line, column);
    const value =
      result === null
        ? 0
        : result.line ^ result.column ^ (result.source?.length ?? 0) ^ (result.name?.length ?? 0);
    checksum = Math.imul(checksum ^ value, 16_777_619) >>> 0;
  }

  return checksum;
};

let lookupChecksum = 0;

// ── Load fixtures ────────────────────────────────────────────────

const FIXTURES = [
  { name: "Preact", file: "preact.js.map" },
  { name: "Chart.js", file: "chartjs.js.map" },
  { name: "PDF.js", file: "pdfjs.js.map" },
];

const maps = [];

for (const fixture of FIXTURES) {
  const filePath = join(fixturesDir, fixture.file);

  if (!existsSync(filePath)) {
    console.error(`Missing: fixtures/${fixture.file}`);
    console.error("Run: npm run download-fixtures\n");
    process.exit(1);
  }

  const json = readFileSync(filePath, "utf-8");
  const parsed = JSON.parse(json);

  const lines = (parsed.mappings.match(/;/g) || []).length + 1;
  const segments = parsed.mappings
    .split(";")
    .reduce((total, line) => total + (line ? line.split(",").length : 0), 0);

  maps.push({
    name: fixture.name,
    json,
    size: json.length,
    lines,
    segments,
    sources: parsed.sources?.length || 0,
  });
}

// ── Header ───────────────────────────────────────────────────────

console.log("=== Real-World Source Map Benchmarks ===\n");
console.log("Libraries:");
console.log("  @jridgewell/trace-mapping  - de facto standard JS source map consumer");
console.log("  source-map-js              - Mozilla source-map fork (used by Vite, PostCSS)");
console.log("  srcmap WASM                - srcmap Rust core via WebAssembly");
console.log("  srcmap NAPI                - srcmap Rust core via N-API\n");

console.log("Source maps:");
for (const m of maps) {
  const sizeStr =
    m.size > 1024 * 1024
      ? `${(m.size / 1024 / 1024).toFixed(1)} MB`
      : `${(m.size / 1024).toFixed(0)} KB`;

  console.log(
    `  ${m.name.padEnd(12)} ${sizeStr.padStart(8)}  ${String(m.segments).padStart(7)} segments  ${String(m.lines).padStart(6)} lines  ${m.sources} sources`,
  );
}

// ── Correctness check ────────────────────────────────────────────

// trace-mapping normalizes source paths (resolves ./ segments),
// srcmap returns raw paths. Normalize both for fair comparison.
const normalizePath = (s) => s?.replace(/\/\.\//g, "/") ?? null;

console.log("\n--- Correctness Check ---\n");

const correctnessResults = [];

for (const { name, json } of maps) {
  const trace = new TraceMap(json);
  const wasm = new SourceMap(json);
  const napi = new NapiSourceMap(json);
  const maxLine = wasm.lineCount;

  let wasmPass = true;
  let napiPass = true;
  let checked = 0;

  for (
    let line = 0;
    line < maxLine && checked < 200;
    line += Math.max(1, Math.floor(maxLine / 100))
  ) {
    for (let col = 0; col < 300; col += 30) {
      const expected = originalPositionFor(trace, { line: line + 1, column: col });
      const expectedNull = !expected.source;

      // WASM
      const wr = wasm.originalPositionFor(line, col);
      if (expectedNull !== (wr === null)) wasmPass = false;
      else if (!expectedNull && wr !== null) {
        if (
          normalizePath(expected.source) !== normalizePath(wr.source) ||
          expected.line !== wr.line + 1 ||
          expected.column !== wr.column
        )
          wasmPass = false;
      }

      // NAPI
      const nr = napi.originalPositionFor(line, col);
      if (expectedNull !== (nr === null)) napiPass = false;
      else if (!expectedNull && nr !== null) {
        if (
          normalizePath(expected.source) !== normalizePath(nr.source) ||
          expected.line !== nr.line + 1 ||
          expected.column !== nr.column
        )
          napiPass = false;
      }

      checked++;
    }
  }

  console.log(
    `  ${name}: WASM ${wasmPass ? "PASS" : "FAIL"}, NAPI ${napiPass ? "PASS" : "FAIL"} (${checked} lookups)`,
  );
  correctnessResults.push({ wasmPass, napiPass });
}

setFailureExitCode(correctnessResults);

// ── Parse benchmarks ─────────────────────────────────────────────

console.log("\n--- Parse ---\n");

for (const { name, json, size } of maps) {
  console.log(`### ${name}\n`);

  // Fewer iterations for large maps
  const iterations = size > 1024 * 1024 ? 50 : 200;
  const bench = createBench({ warmupIterations: 10, iterations });
  const prefix = `real_world_parse[${name}]`;

  bench
    .add(`${prefix} trace-mapping`, () => new TraceMap(json))
    .add(`${prefix} source-map-js`, () => new SourceMapConsumer(json))
    .add(`${prefix} srcmap WASM`, () => new SourceMap(json))
    .add(`${prefix} srcmap WASM fast`, () => new FastSourceMap(json))
    .add(`${prefix} srcmap NAPI`, () => new NapiSourceMap(json));

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      "ops/sec": Math.round(throughputHz(task)).toLocaleString(),
      "avg (ms)": latencyMeanMs(task).toFixed(2),
      "p99 (ms)": latencyP99Ms(task).toFixed(2),
    })),
  );
}

// ── Fast-lazy cold map lookup order ─────────────────────────────

console.log("\n--- Fast-Lazy Cold Map: Construct and First Lookup Pass ---\n");

for (const { name, json, size, lines } of maps) {
  console.log(`### ${name}\n`);

  const eager = new SourceMap(json);
  const patterns = createLookupPatterns(lines);

  for (const pattern of patterns) {
    const lazy = new FastSourceMap(json);
    assertLazyMatchesEager(eager, lazy, pattern.lookups, `${name} ${pattern.name}`);
    lazy.free();
  }
  eager.free();

  const isLargeMap = size > 1024 * 1024;
  const bench = createBench({
    warmupIterations: isLargeMap ? 2 : 5,
    iterations: isLargeMap ? 10 : 50,
  });
  const prefix = `real_world_lazy_lookup_cold_1000x[${name}]`;

  for (const pattern of patterns) {
    bench.add(`${prefix} ${pattern.name}`, () => {
      const lazy = new FastSourceMap(json);
      lookupChecksum = (lookupChecksum + consumeLookups(lazy, pattern.lookups)) >>> 0;
      lazy.free();
    });
  }

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      "ops/sec": Math.round(throughputHz(task)).toLocaleString(),
      "avg (μs)": (latencyMeanMs(task) * 1000).toFixed(1),
      "per lookup (ns)": Math.round(
        (latencyMeanMs(task) * 1_000_000) / LOOKUP_COUNT,
      ).toLocaleString(),
    })),
  );
}

// ── Fast-lazy warm cache lookup order ────────────────────────────

console.log("\n--- Fast-Lazy Warm Cache: Reuse Decoded Lines ---\n");

for (const { name, json, size, lines } of maps) {
  console.log(`### ${name}\n`);

  const patterns = createLookupPatterns(lines).map((pattern) => ({
    ...pattern,
    map: new FastSourceMap(json),
  }));

  for (const pattern of patterns) {
    lookupChecksum = (lookupChecksum + consumeLookups(pattern.map, pattern.lookups)) >>> 0;
  }

  const isLargeMap = size > 1024 * 1024;
  const bench = createBench({
    warmupIterations: isLargeMap ? 5 : 20,
    iterations: isLargeMap ? 50 : 200,
  });
  const prefix = `real_world_lazy_lookup_warm_1000x[${name}]`;

  for (const pattern of patterns) {
    bench.add(`${prefix} ${pattern.name}`, () => {
      lookupChecksum = (lookupChecksum + consumeLookups(pattern.map, pattern.lookups)) >>> 0;
    });
  }

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      "ops/sec": Math.round(throughputHz(task)).toLocaleString(),
      "avg (μs)": (latencyMeanMs(task) * 1000).toFixed(1),
      "per lookup (ns)": Math.round(
        (latencyMeanMs(task) * 1_000_000) / LOOKUP_COUNT,
      ).toLocaleString(),
    })),
  );

  for (const pattern of patterns) {
    pattern.map.free();
  }
}

console.log(`Lookup checksum: ${lookupChecksum}`);

// ── Single lookup ────────────────────────────────────────────────

console.log("\n--- Single Lookup ---\n");

for (const { name, json, size } of maps) {
  console.log(`### ${name}\n`);

  const trace = new TraceMap(json);
  const smjs = new SourceMapConsumer(json);
  const wasm = new SourceMap(json);
  const napi = new NapiSourceMap(json);

  // Pick a lookup position roughly in the middle of the map
  const midLine = Math.floor(wasm.lineCount / 2);

  const isLargeMap = size > 1024 * 1024;
  const bench = createBench({
    warmupIterations: isLargeMap ? 50 : 500,
    iterations: isLargeMap ? 500 : 5000,
  });
  const prefix = `real_world_lookup_single[${name}]`;

  bench
    .add(`${prefix} trace-mapping`, () =>
      originalPositionFor(trace, { line: midLine + 1, column: 20 }),
    )
    .add(`${prefix} source-map-js`, () =>
      smjs.originalPositionFor({ line: midLine + 1, column: 20 }),
    )
    .add(`${prefix} srcmap WASM`, () => wasm.originalPositionFor(midLine, 20))
    .add(`${prefix} srcmap WASM flat`, () => wasm.originalPositionFlat(midLine, 20))
    .add(`${prefix} srcmap WASM buf`, () => {
      wasm.originalPositionBuf(midLine, 20);
    })
    .add(`${prefix} srcmap NAPI`, () => napi.originalPositionFor(midLine, 20));

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      "ops/sec": Math.round(throughputHz(task)).toLocaleString(),
      "avg (ns)": Math.round(latencyMeanMs(task) * 1_000_000).toLocaleString(),
      "p99 (ns)": Math.round(latencyP99Ms(task) * 1_000_000).toLocaleString(),
    })),
  );
}

// ── Batch lookup (1000x) ─────────────────────────────────────────

console.log("\n--- 1000x Lookup ---\n");

for (const { name, json, size } of maps) {
  console.log(`### ${name}\n`);

  const trace = new TraceMap(json);
  const smjs = new SourceMapConsumer(json);
  const wasm = new SourceMap(json);
  const napi = new NapiSourceMap(json);
  const maxLine = wasm.lineCount;

  const lookups = createDeterministicLookups(LOOKUP_COUNT, maxLine, LOOKUP_MAX_COLUMN, LOOKUP_SEED);
  const flatPositions = lookups.flatMap(({ line, column }) => [line, column]);
  const posArray = new Int32Array(flatPositions);

  const isLargeMap = size > 1024 * 1024;
  const bench = createBench({
    warmupIterations: isLargeMap ? 5 : 20,
    iterations: isLargeMap ? 50 : 200,
  });
  const prefix = `real_world_lookup_1000x[${name}]`;

  bench
    .add(`${prefix} trace-mapping`, () => {
      for (const { line, column } of lookups)
        originalPositionFor(trace, { line: line + 1, column });
    })
    .add(`${prefix} source-map-js`, () => {
      for (const { line, column } of lookups) smjs.originalPositionFor({ line: line + 1, column });
    })
    .add(`${prefix} srcmap WASM individual`, () => {
      for (const { line, column } of lookups) wasm.originalPositionFor(line, column);
    })
    .add(`${prefix} srcmap WASM batch`, () => {
      wasm.originalPositionsFor(posArray);
    })
    .add(`${prefix} srcmap NAPI batch`, () => {
      napi.originalPositionsFor(flatPositions);
    });

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      "ops/sec": Math.round(throughputHz(task)).toLocaleString(),
      "avg (μs)": (latencyMeanMs(task) * 1000).toFixed(1),
      "per lookup (ns)": Math.round(latencyMeanMs(task) * 1_000_000).toLocaleString(),
    })),
  );
}
