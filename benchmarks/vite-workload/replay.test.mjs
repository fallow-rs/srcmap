import assert from "node:assert/strict";
import { test } from "node:test";
import remapping from "@jridgewell/remapping";
import { chainMaps, replayOneCall, replayPairwise } from "./replay.mjs";

// Each map moves three lines down by one: generated line n+1 maps to original line n.
// A combine drops a line when the inner map has no mapping for it.
const shiftMap = (source) => ({
  version: 3,
  sources: [source],
  names: [],
  mappings: ";AAAA;AACA;AACA",
});

const chainRecord = (maps, combined = null) => ({
  kind: "chain",
  file: "/src/a.ts",
  combined,
  maps,
});

test("a chain of three maps gives the same output pairwise and in one call", () => {
  const record = chainRecord([shiftMap("/src/a.ts"), shiftMap("/src/a.ts"), shiftMap("/src/a.ts")]);
  const pairwise = replayPairwise(remapping)(record);
  const oneCall = replayOneCall(remapping)(record);
  assert.equal(pairwise.mappings, ";;;AAAA");
  assert.equal(oneCall.mappings, pairwise.mappings);
  assert.deepEqual(oneCall.sources, ["/src/a.ts"]);
});

test("a chain continues from the map that an earlier combine produced", () => {
  const record = chainRecord([shiftMap("/src/a.ts")], shiftMap("/src/a.ts"));
  assert.equal(replayPairwise(remapping)(record).mappings, ";;AAAA;AACA");
});

test("a chain accepts a map as a JSON string", () => {
  const record = chainRecord([JSON.stringify(shiftMap("/src/a.ts")), shiftMap("/src/a.ts")]);
  assert.equal(replayOneCall(remapping)(record).mappings, ";;AAAA;AACA");
});

test("a map without a version ends the chain with no combine", () => {
  const record = chainRecord([shiftMap("/src/a.ts"), { mappings: "" }]);
  assert.equal(chainMaps(record), null);
  assert.equal(replayPairwise(remapping)(record), null);
  assert.equal(replayOneCall(remapping)(record), null);
});

test("a chain of one map needs no combine", () => {
  assert.equal(replayPairwise(remapping)(chainRecord([shiftMap("/src/a.ts")])), null);
});

test("the first map of a chain gets the module file when its source is empty", () => {
  const record = chainRecord([shiftMap(""), shiftMap("/src/a.ts")]);
  assert.deepEqual(chainMaps(record)[0].sources, ["/src/a.ts"]);
});

test("a pair record is one combine of its two maps, outermost first", () => {
  const record = {
    kind: "pair",
    file: "/src/a.ts",
    maps: [shiftMap("/src/a.ts"), shiftMap("/src/a.ts")],
  };
  assert.equal(replayPairwise(remapping)(record).mappings, ";;AAAA;AACA");
  assert.equal(replayOneCall(remapping)(record).mappings, ";;AAAA;AACA");
});

test("a chain with a multi-source link falls back to pairwise in the one-call replay", () => {
  const multi = {
    version: 3,
    sources: ["/src/a.ts", "/src/b.ts"],
    names: [],
    mappings: ";AAAA;ACAA",
  };
  const record = chainRecord([shiftMap("/src/a.ts"), multi, shiftMap("/src/a.ts")]);
  assert.equal(
    replayOneCall(remapping)(record).mappings,
    replayPairwise(remapping)(record).mappings,
  );
});
