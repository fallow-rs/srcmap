"use strict";

const { describe, it } = require("node:test");
const assert = require("node:assert/strict");
const {
  GenMapping,
  addMapping,
  maybeAddMapping,
  setSourceContent,
  setIgnore,
  allMappings,
  toEncodedMap,
  toDecodedMap,
  fromMap,
} = require("../src/gen-mapping.cjs");

describe("CJS: GenMapping", () => {
  it("exports all expected functions", () => {
    assert.equal(typeof GenMapping, "function");
    assert.equal(typeof addMapping, "function");
    assert.equal(typeof maybeAddMapping, "function");
    assert.equal(typeof setSourceContent, "function");
    assert.equal(typeof setIgnore, "function");
    assert.equal(typeof allMappings, "function");
    assert.equal(typeof toEncodedMap, "function");
    assert.equal(typeof toDecodedMap, "function");
    assert.equal(typeof fromMap, "function");
  });

  it("basic addMapping and toEncodedMap workflow", () => {
    const map = new GenMapping({ file: "output.js" });
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "x",
    });
    setSourceContent(map, "input.js", "const x = 1;");

    const encoded = toEncodedMap(map);
    assert.equal(encoded.version, 3);
    assert.equal(encoded.file, "output.js");
    assert.deepEqual(encoded.sources, ["input.js"]);
    assert.deepEqual(encoded.names, ["x"]);
    assert.deepEqual(encoded.sourcesContent, ["const x = 1;"]);
    assert.equal(typeof encoded.mappings, "string");

    map.free();
  });

  it("maybeAddMapping deduplicates", () => {
    const map = new GenMapping();
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "a.js",
      original: { line: 1, column: 0 },
    });
    maybeAddMapping(map, {
      generated: { line: 1, column: 5 },
      source: "a.js",
      original: { line: 1, column: 0 },
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 1);
    map.free();
  });

  it("toDecodedMap returns array mappings", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
    });
    const decoded = toDecodedMap(map);
    assert.ok(Array.isArray(decoded.mappings));
    assert.ok(Array.isArray(decoded.mappings[0]));
    map.free();
  });
});
