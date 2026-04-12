import { Bench } from "tinybench";
import {
  decode as jridgewellDecode,
  encode as jridgewellEncode,
} from "@jridgewell/sourcemap-codec";
import {
  decode as srcmapDecode,
  encode as srcmapEncode,
  decodeJson as srcmapDecodeJson,
  encodeJson as srcmapEncodeJson,
  decodeBuf as srcmapDecodeBuf,
  encodeBuf as srcmapEncodeBuf,
} from "../packages/codec/index.js";

// Test mappings of increasing complexity
const SMALL_MAP = "AAAA;AACA,GAAG;AACA,IAAI,EAAE";
const MEDIUM_MAP = Array.from({ length: 100 }, () => "AAAA,GAAG,EAAE,IAAI,KAAK").join(";");
const LARGE_MAP = Array.from({ length: 1000 }, () =>
  Array.from({ length: 50 }, () => "AAAA").join(","),
).join(";");

const maps = [
  { name: "small (3 lines)", mappings: SMALL_MAP },
  { name: "medium (100 lines, 500 segments)", mappings: MEDIUM_MAP },
  { name: "large (1000 lines, 50K segments)", mappings: LARGE_MAP },
];

// Verify correctness for all approaches
console.log("Verifying correctness...\n");
for (const { name, mappings } of maps) {
  const reference = jridgewellDecode(mappings);

  const napiResult = srcmapDecode(mappings);
  const jsonResult = srcmapDecodeJson(mappings);
  const bufResult = srcmapDecodeBuf(mappings);

  const refJson = JSON.stringify(reference);

  const napiMatch = JSON.stringify(napiResult) === refJson;
  const jsonMatch = JSON.stringify(jsonResult) === refJson;
  const bufMatch = JSON.stringify(bufResult) === refJson;

  console.log(`${name}:`);
  console.log(`  NAPI decode:   ${napiMatch ? "PASS" : "FAIL"}`);
  console.log(`  JSON decode:   ${jsonMatch ? "PASS" : "FAIL"}`);
  console.log(`  Buffer decode: ${bufMatch ? "PASS" : "FAIL"}`);

  // Verify encode
  const refEncoded = jridgewellEncode(reference);
  const napiEncoded = srcmapEncode(napiResult);
  const jsonEncoded = srcmapEncodeJson(jsonResult);
  const bufEncoded = srcmapEncodeBuf(bufResult);

  console.log(`  NAPI encode:   ${napiEncoded === refEncoded ? "PASS" : "FAIL"}`);
  console.log(`  JSON encode:   ${jsonEncoded === refEncoded ? "PASS" : "FAIL"}`);
  console.log(`  Buffer encode: ${bufEncoded === refEncoded ? "PASS" : "FAIL"}`);

  if (!napiMatch || !jsonMatch || !bufMatch) {
    console.log("  reference:", refJson.slice(0, 200));
    if (!napiMatch) console.log("  napi:    ", JSON.stringify(napiResult).slice(0, 200));
    if (!jsonMatch) console.log("  json:    ", JSON.stringify(jsonResult).slice(0, 200));
    if (!bufMatch) console.log("  buf:     ", JSON.stringify(bufResult).slice(0, 200));
  }
}

console.log("\n--- Decode Benchmarks ---\n");

for (const { name, mappings } of maps) {
  console.log(`\n### ${name}\n`);

  const bench = new Bench({ warmupIterations: 100, iterations: 1000 });

  bench
    .add("jridgewell decode", () => jridgewellDecode(mappings))
    .add("srcmap NAPI decode", () => srcmapDecode(mappings))
    .add("srcmap JSON decode", () => srcmapDecodeJson(mappings))
    .add("srcmap Buffer decode", () => srcmapDecodeBuf(mappings));

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      "ops/sec": Math.round(task.result.hz).toLocaleString(),
      "avg (ns)": Math.round(task.result.mean * 1_000_000).toLocaleString(),
      "p99 (ns)": Math.round(task.result.p99 * 1_000_000).toLocaleString(),
    })),
  );
}

console.log("\n--- Encode Benchmarks ---\n");

for (const { name, mappings } of maps) {
  console.log(`\n### ${name}\n`);

  const decoded = jridgewellDecode(mappings);

  const bench = new Bench({ warmupIterations: 100, iterations: 1000 });

  bench
    .add("jridgewell encode", () => jridgewellEncode(decoded))
    .add("srcmap NAPI encode", () => srcmapEncode(decoded))
    .add("srcmap JSON encode", () => srcmapEncodeJson(decoded))
    .add("srcmap Buffer encode", () => srcmapEncodeBuf(decoded));

  await bench.run();

  console.table(
    bench.tasks.map((task) => ({
      Name: task.name,
      "ops/sec": Math.round(task.result.hz).toLocaleString(),
      "avg (ns)": Math.round(task.result.mean * 1_000_000).toLocaleString(),
      "p99 (ns)": Math.round(task.result.p99 * 1_000_000).toLocaleString(),
    })),
  );
}
