// Replays source map chains that a real Vite run combined, and times each remapping
// implementation on them. See vite-workload/README.md for how to record the fixtures.
//
//   node vite-workload.mjs [fixture-name ...]

import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import jridgewellRemapping from "@jridgewell/remapping";
import { decode } from "@jridgewell/sourcemap-codec";
import { chainMaps, replayOneCall, replayPairwise } from "./vite-workload/replay.mjs";

const SELF = fileURLToPath(import.meta.url);
const FIXTURES_DIR = join(dirname(SELF), "fixtures", "vite-workload");
const WORKER_FLAG = "--worker";
const TIMING_SUFFIX = ".time.ndjson";
const ROUNDS = 7;

const loadSrcmapRemapping = async () => {
  try {
    return (await import("../packages/remapping/src/remapping.mjs")).default;
  } catch {
    return null;
  }
};

const median = (values) => values.toSorted((a, b) => a - b)[values.length >> 1];

const MICROSECONDS_PER_MS = 1000;

/** CPU time of this process in ms. Unlike wall time, it stays stable on a busy machine. */
const cpuMs = () => {
  const { user, system } = process.cpuUsage();
  return (user + system) / MICROSECONDS_PER_MS;
};

const timePass = (records, replay) => {
  const startedAt = cpuMs();
  for (const record of records) replay(record);
  return cpuMs() - startedAt;
};

/**
 * Times one implementation in this process. The cold pass is the first pass of a new
 * process, which is what a Vite run pays: it combines each module one time.
 */
const measure = (records, replay) => {
  const coldMs = timePass(records, replay);
  const rounds = [];
  for (let round = 0; round < ROUNDS; round++) rounds.push(timePass(records, replay));
  return { coldMs, warmMs: Math.min(...rounds) };
};

const segmentCount = (mappings) => {
  if (typeof mappings !== "string") return mappings.reduce((count, line) => count + line.length, 0);
  return decode(mappings).reduce((count, line) => count + line.length, 0);
};

const describeRecords = (records) => {
  const lengths = {};
  const segments = [];
  let combines = 0;
  for (const record of records) {
    const maps = record.kind === "pair" ? record.maps : chainMaps(record);
    if (!maps || maps.length < 2) continue;
    combines++;
    lengths[maps.length] = (lengths[maps.length] ?? 0) + 1;
    segments.push(segmentCount(maps[record.kind === "pair" ? 0 : maps.length - 1].mappings));
  }
  return {
    combines,
    lengths,
    medianSegments: segments.length ? median(segments) : 0,
    maxSegments: segments.length ? Math.max(...segments) : 0,
  };
};

const countMismatches = (records, reference, candidate) => {
  let mismatches = 0;
  for (const record of records) {
    const expected = reference(record);
    const actual = candidate(record);
    if ((expected?.mappings ?? null) !== (actual?.mappings ?? null)) mismatches++;
  }
  return mismatches;
};

const readNdjson = (file) =>
  readFileSync(file, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));

const formatMs = (ms) => `${ms.toFixed(1)} ms`;
const formatShare = (part, whole) => `${((100 * part) / whole).toFixed(1)}%`;

const printTimingSummary = (name) => {
  const file = join(FIXTURES_DIR, name + TIMING_SUFFIX);
  if (!existsSync(file)) return;
  const [summary] = readNdjson(file);
  const mapMs = summary.combineMs + summary.generateMapMs;
  console.log(
    `  recorded run: wall ${formatMs(summary.wallMs)}, ${summary.transformCalls} module transforms`,
  );
  console.log(
    `  combine ${formatMs(summary.combineMs)} (${formatShare(summary.combineMs, summary.wallMs)} of wall), ` +
      `map generation ${formatMs(summary.generateMapMs)}, both ${formatShare(mapMs, summary.wallMs)} of wall`,
  );
  console.log(
    `  maps per module at the end of the plugin chain: ${JSON.stringify(summary.chainLengths)}`,
  );
};

const loadRecords = (name) =>
  readNdjson(join(FIXTURES_DIR, `${name}.ndjson`)).filter((record) => record.kind !== "summary");

const loadImplementations = async () => {
  const implementations = [
    {
      label: "@jridgewell/remapping, pairwise (Vite today)",
      replay: replayPairwise(jridgewellRemapping),
    },
    {
      label: "@jridgewell/remapping, one call per chain",
      replay: replayOneCall(jridgewellRemapping),
    },
  ];
  const srcmapRemapping = await loadSrcmapRemapping();
  if (srcmapRemapping) {
    implementations.push({
      label: "@srcmap/remapping, one call per chain",
      replay: replayOneCall(srcmapRemapping),
    });
  }
  return implementations;
};

const runWorker = async (name, index) => {
  const records = loadRecords(name);
  const implementations = await loadImplementations();
  const { replay } = implementations[index];
  try {
    const timing = measure(records, replay);
    const mismatches =
      index === 0 ? 0 : countMismatches(records, implementations[0].replay, replay);
    console.log(JSON.stringify({ ...timing, mismatches }));
  } catch (error) {
    console.log(JSON.stringify({ error: String(error?.message ?? error).split("\n")[0] }));
  }
};

const runInNewProcess = (name, index) => {
  const child = spawnSync(process.execPath, [SELF, WORKER_FLAG, name, String(index)], {
    encoding: "utf8",
  });
  if (child.status !== 0) return { error: child.stderr.trim().split("\n").at(-1) };
  return JSON.parse(child.stdout);
};

const main = async () => {
  if (!existsSync(FIXTURES_DIR)) {
    console.error(`no fixtures in ${FIXTURES_DIR}; see vite-workload/README.md`);
    process.exit(1);
  }
  const requested = process.argv.slice(2);
  const names = readdirSync(FIXTURES_DIR)
    .filter((file) => file.endsWith(".ndjson") && !file.endsWith(TIMING_SUFFIX))
    .map((file) => file.slice(0, -".ndjson".length))
    .filter((name) => requested.length === 0 || requested.includes(name))
    .toSorted();

  const implementations = await loadImplementations();
  if (implementations.length < 3)
    console.log("@srcmap/remapping is not built; the benchmark skips it");

  for (const name of names) {
    const shape = describeRecords(loadRecords(name));
    console.log(`\n== ${name}`);
    printTimingSummary(name);
    console.log(
      `  replay: ${shape.combines} combines, chain lengths ${JSON.stringify(shape.lengths)}, ` +
        `segments per outermost map: median ${shape.medianSegments}, max ${shape.maxSegments}`,
    );
    console.log(
      `  ${"cold".padStart(10)} ${"warm".padStart(10)}  (CPU time; warm is the best of ${ROUNDS} passes)`,
    );
    implementations.forEach(({ label }, index) => {
      const result = runInNewProcess(name, index);
      if (result.error) {
        console.log(`  ${"failed".padStart(21)}  ${label}: ${result.error}`);
        return;
      }
      const note =
        result.mismatches > 0 ? `  (${result.mismatches} outputs differ from Vite today)` : "";
      console.log(
        `  ${formatMs(result.coldMs).padStart(10)} ${formatMs(result.warmMs).padStart(10)}  ${label}${note}`,
      );
    });
  }
};

if (process.argv[2] === WORKER_FLAG) await runWorker(process.argv[3], Number(process.argv[4]));
else await main();
