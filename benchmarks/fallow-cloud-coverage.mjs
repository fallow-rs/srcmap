import { Bench } from "tinybench";
import { TraceMap, originalPositionFor } from "@jridgewell/trace-mapping";
import { createRequire } from "node:module";
import { encode } from "@jridgewell/sourcemap-codec";
import { SourceMap as NapiSourceMap } from "../packages/sourcemap/index.js";
import { GeneratedOffsetLookup } from "../packages/sourcemap-wasm/coverage.mjs";

const require = createRequire(import.meta.url);
// fallow-ignore-next-line unresolved-import
const {
  SourceMap: WasmSourceMap,
} = require("../packages/sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js");

const MAP_URL = "https://cdn.fallow-cloud.test/assets/app.js.map";
const MAP_LINE_COUNT = 6000;
const SEGS_PER_LINE = 18;
const SOURCE_COUNT = 24;
const NAME_COUNT = 32;
const BATCH_COUNT = 200;
const POSITIONS_PER_BATCH = 40;
const GENERATED_LINE_WIDTH = 220;

function buildLargeCoverageMap() {
  const sources = Array.from(
    { length: SOURCE_COUNT },
    (_, i) => `src/module-${String(i).padStart(2, "0")}.ts`,
  );
  const names = Array.from(
    { length: NAME_COUNT },
    (_, i) => `symbol_${String(i).padStart(2, "0")}`,
  );

  const mappings = [];
  let sourceIndex = 0;
  let sourceLine = 0;
  let sourceColumn = 0;
  let nameIndex = 0;

  for (let line = 0; line < MAP_LINE_COUNT; line++) {
    const generatedLine = [];
    let generatedColumn = 0;

    for (let segment = 0; segment < SEGS_PER_LINE; segment++) {
      generatedColumn += 1 + ((line + segment) % 11);
      sourceIndex = (sourceIndex + 1 + (segment % 3)) % SOURCE_COUNT;
      sourceLine = (sourceLine + 1 + (line % 2)) % MAP_LINE_COUNT;
      sourceColumn = (sourceColumn + 2 + ((line + segment) % 13)) % 180;

      if ((line + segment) % 5 === 0) {
        nameIndex = (nameIndex + 1) % NAME_COUNT;
        generatedLine.push([generatedColumn, sourceIndex, sourceLine, sourceColumn, nameIndex]);
      } else {
        generatedLine.push([generatedColumn, sourceIndex, sourceLine, sourceColumn]);
      }
    }

    mappings.push(generatedLine);
  }

  const json = JSON.stringify({
    version: 3,
    file: "app.js",
    sources,
    names,
    mappings: encode(mappings),
  });

  return { json, sources, names };
}

function buildGeneratedCode() {
  const lineStartOffsets = [];
  const lines = [];
  let offset = 0;

  for (let line = 0; line < MAP_LINE_COUNT; line++) {
    lineStartOffsets[line] = offset;
    const prefix = `cov${String(line).padStart(4, "0")}=`;
    const suffix = ";";
    const fillerWidth = GENERATED_LINE_WIDTH - prefix.length - suffix.length;
    const body = "x".repeat(Math.max(0, fillerWidth));
    const text = `${prefix}${body}${suffix}`;
    lines.push(text);
    offset += Buffer.byteLength(text, "utf8") + 1;
  }

  return {
    code: `${lines.join("\n")}\n`,
    lineStartOffsets,
  };
}

function buildBeaconBatches(lineStartOffsets) {
  return Array.from({ length: BATCH_COUNT }, (_, beaconIndex) => {
    const offsets = new Int32Array(POSITIONS_PER_BATCH);

    for (let i = 0; i < POSITIONS_PER_BATCH; i++) {
      const line = (beaconIndex * 41 + i * 17) % lineStartOffsets.length;
      const column = (beaconIndex * 23 + i * 13) % 180;
      offsets[i] = lineStartOffsets[line] + column;
    }

    return {
      beaconId: `coverage-${String(beaconIndex).padStart(4, "0")}`,
      mapUrl: MAP_URL,
      receivedAt: `2026-04-13T00:${String(beaconIndex % 60).padStart(2, "0")}:00.000Z`,
      offsets,
    };
  });
}

const fixture = buildLargeCoverageMap();
const generated = buildGeneratedCode();
const beacons = buildBeaconBatches(generated.lineStartOffsets);

const mapCache = new Map();

function getCachedMaps(mapUrl) {
  let entry = mapCache.get(mapUrl);
  if (entry) return entry;

  entry = {
    trace: new TraceMap(fixture.json),
    wasm: new WasmSourceMap(fixture.json),
    napi: new NapiSourceMap(fixture.json),
    offsetLookup: new GeneratedOffsetLookup(generated.code),
  };

  mapCache.set(mapUrl, entry);
  return entry;
}

function verifyBatchResults(entry, beacon, resolvePosition) {
  const generatedPositions = entry.offsetLookup.generatedPositionsFor(beacon.offsets);

  for (let i = 0; i < generatedPositions.length; i += 2) {
    const line = generatedPositions[i];
    const column = generatedPositions[i + 1];
    const expected = originalPositionFor(entry.trace, { line: line + 1, column });
    const actual = resolvePosition(line, column);
    const expectedSource = expected.source ?? null;
    const expectedLine = expected.line ?? null;
    const expectedColumn = expected.column ?? null;
    const expectedName = expected.name ?? null;

    if (expectedSource === null) {
      if (actual !== null) return false;
      continue;
    }

    const actualSource = actual?.source ?? null;
    const actualLine = actual?.line ?? null;
    const actualColumn = actual?.column ?? null;
    const actualName = actual?.name ?? null;

    if (
      actualSource !== expectedSource ||
      actualLine !== expectedLine - 1 ||
      actualColumn !== expectedColumn ||
      actualName !== expectedName
    ) {
      return false;
    }
  }

  return true;
}

console.log("=== Fallow Cloud Coverage Workload ===\n");
console.log(`Map cache: 1 large map reused across ${BATCH_COUNT} beacon batches`);
console.log(
  `Batch size: ${POSITIONS_PER_BATCH} offsets per beacon (${BATCH_COUNT * POSITIONS_PER_BATCH} lookups per run)`,
);
console.log(
  `Fixture map: ${fixture.json.length.toLocaleString()} bytes, ${MAP_LINE_COUNT} lines, ${SEGS_PER_LINE * MAP_LINE_COUNT} segments\n`,
);

const cachedMaps = getCachedMaps(MAP_URL);

console.log("--- Correctness Check ---\n");

let wasmPass = true;
let napiPass = true;

for (const beacon of beacons.slice(0, 8)) {
  if (
    !verifyBatchResults(cachedMaps, beacon, (line, column) =>
      cachedMaps.wasm.originalPositionFor(line, column),
    )
  ) {
    wasmPass = false;
  }
  if (
    !verifyBatchResults(cachedMaps, beacon, (line, column) =>
      cachedMaps.napi.originalPositionFor(line, column),
    )
  ) {
    napiPass = false;
  }
}

console.log(`  WASM single lookup: ${wasmPass ? "PASS" : "FAIL"}`);
console.log(`  NAPI single lookup: ${napiPass ? "PASS" : "FAIL"}`);

if (!wasmPass || !napiPass) {
  process.exitCode = 1;
}

console.log("\n--- Cached Coverage Lookup ---\n");

const bench = new Bench({ warmupIterations: 20, iterations: 200 });

bench
  .add("trace-mapping individual lookup", () => {
    for (const beacon of beacons) {
      const positions = cachedMaps.offsetLookup.generatedPositionsFor(beacon.offsets);
      for (let i = 0; i < positions.length; i += 2) {
        originalPositionFor(cachedMaps.trace, {
          line: positions[i] + 1,
          column: positions[i + 1],
        });
      }
    }
  })
  .add("srcmap WASM individual lookup", () => {
    for (const beacon of beacons) {
      const positions = cachedMaps.offsetLookup.generatedPositionsFor(beacon.offsets);
      for (let i = 0; i < positions.length; i += 2) {
        cachedMaps.wasm.originalPositionFor(positions[i], positions[i + 1]);
      }
    }
  })
  .add("srcmap NAPI individual lookup", () => {
    for (const beacon of beacons) {
      const positions = cachedMaps.offsetLookup.generatedPositionsFor(beacon.offsets);
      for (let i = 0; i < positions.length; i += 2) {
        cachedMaps.napi.originalPositionFor(positions[i], positions[i + 1]);
      }
    }
  });

await bench.run();

console.table(
  bench.tasks.map((task) => ({
    Name: task.name,
    "ops/sec": Math.round(task.result.hz).toLocaleString(),
    "avg (μs)": (task.result.mean * 1000).toFixed(1),
    "per lookup (ns)": Math.round(
      (task.result.mean * 1_000_000) / (BATCH_COUNT * POSITIONS_PER_BATCH),
    ).toLocaleString(),
  })),
);

cachedMaps.wasm.free();
