"use strict";

const { describe, it } = require("node:test");
const assert = require("node:assert/strict");
const remapping = require("../src/remapping.cjs");
const { SourceMap } = require("../src/remapping.cjs");

const ORIGINAL_MAP = {
  version: 3,
  sources: ["original.ts"],
  sourcesContent: ["const x = 1;"],
  names: [],
  mappings: "AAAA",
};

const INTERMEDIATE_MAP = {
  version: 3,
  sources: ["intermediate.js"],
  names: [],
  mappings: "AAAA;AACA",
};

const INNER_MAP = {
  version: 3,
  sources: ["original.js"],
  names: [],
  mappings: "AACA;AACA",
};

describe("CJS: remapping", () => {
  it("default export is the remapping function", () => {
    assert.equal(typeof remapping, "function");
  });

  it("named export: remapping", () => {
    assert.equal(typeof remapping.remapping, "function");
  });

  it("named export: SourceMap", () => {
    assert.equal(typeof SourceMap, "function");
  });

  it("remaps single map through loader", () => {
    const result = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === "intermediate.js") return INNER_MAP;
      return null;
    });
    assert.ok(result instanceof SourceMap);
    assert.deepEqual(result.sources, ["original.js"]);
  });

  it("composes array of maps", () => {
    const outer = {
      version: 3,
      sources: ["intermediate.js"],
      names: [],
      mappings: "AAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      names: [],
      mappings: "AAAA",
    };
    const result = remapping([outer, inner], () => null);
    assert.deepEqual(result.sources, ["original.ts"]);
  });

  it("passes through with no upstream", () => {
    const result = remapping(ORIGINAL_MAP, () => null);
    assert.deepEqual(result.sources, ["original.ts"]);
  });

  it("result has toString and toJSON", () => {
    const result = remapping(ORIGINAL_MAP, () => null);
    assert.equal(typeof result.toString, "function");
    assert.equal(typeof result.toJSON, "function");

    const json = result.toJSON();
    assert.equal(json.version, 3);

    const str = result.toString();
    const parsed = JSON.parse(str);
    assert.equal(parsed.version, 3);
  });
});

describe("CJS: remapping(decodedMap, loader)", () => {
  it("encodes source, line and name deltas across lines", () => {
    const decoded = {
      version: 3,
      sources: ["a.js", "b.js"],
      names: ["x", "y"],
      mappings: [[[0, 0, 5, 0, 0]], [[0, 1, 6, 2, 1]], [[3, 1, 6, 4]]],
    };
    const result = remapping(decoded, () => null);
    assert.equal(result.mappings, "AAKAA;ACCEC;GAAE");
  });
});

describe("CJS: decoded input maps", () => {
  it("accepts a map without version and with rangeMappings as arrays", () => {
    const jrRemapping = require("@jridgewell/remapping");
    const maps = [
      {
        sources: ["intermediate.js"],
        names: [],
        mappings: [[[0, 0, 0, 0]], [[0, 0, 1, 0]]],
        rangeMappings: [[], [0]],
      },
      INNER_MAP,
    ];
    const result = remapping(maps, () => null);
    const expected = jrRemapping(maps, () => null);
    assert.equal(result.mappings, expected.mappings);
    assert.deepEqual(result.sources, expected.sources);
  });
});

describe("CJS: loader tree", () => {
  it("follows the loader through more than one level, as jridgewell does", () => {
    const jrRemapping = require("@jridgewell/remapping");
    const map = (sources) => ({
      version: 3,
      sourceRoot: "root",
      sources,
      names: [],
      mappings: "AAAA",
    });
    const upstream = { "root/a.js": map(["b.js"]), "root/root/b.js": map(["c.js"]) };
    const run = (remapping) => remapping(map(["a.js"]), (source) => upstream[source] ?? null);
    assert.equal(JSON.stringify(run(remapping)), JSON.stringify(run(jrRemapping)));
    assert.deepEqual(run(remapping).sources, ["root/root/root/c.js"]);
  });
});
