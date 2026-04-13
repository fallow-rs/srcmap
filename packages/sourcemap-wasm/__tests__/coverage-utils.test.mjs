import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import {
  GeneratedOffsetLookup,
  generatedPositionForOffset,
  originalPositionForOffset,
  originalPositionsForOffsets,
} from "../coverage.mjs";

const require = createRequire(import.meta.url);
// fallow-ignore-next-line unresolved-import
const { SourceMap: WasmSourceMap } = require("../pkg/srcmap_sourcemap_wasm.js");
// fallow-ignore-next-line unresolved-import
const { SourceMap: NapiSourceMap } = require("../../sourcemap/index.js");

const SOURCE = "const cafe = 'cafe';\nconst emoji = '😀';\n";
const MULTIBYTE_SOURCE = "const letter = 'é';\nconst emoji = '😀';\n";

const TWO_LINE_MAP = JSON.stringify({
  version: 3,
  sources: ["input.ts"],
  names: ["lineOne", "lineTwo"],
  mappings: "AAAAA;AACAC",
});

describe("GeneratedOffsetLookup", () => {
  it("maps ASCII byte offsets to 0-based line and column", () => {
    const lookup = new GeneratedOffsetLookup(SOURCE);
    assert.deepEqual(lookup.generatedPositionFor(0), { line: 0, column: 0 });
    assert.deepEqual(lookup.generatedPositionFor(5), { line: 0, column: 5 });
  });

  it("maps offsets after line breaks to the next generated line", () => {
    const newlineOffset = Buffer.byteLength("const cafe = 'cafe';\n", "utf8");
    const lookup = new GeneratedOffsetLookup(SOURCE);
    assert.deepEqual(lookup.generatedPositionFor(newlineOffset), { line: 1, column: 0 });
  });

  it("converts UTF-8 byte offsets to UTF-16 columns", () => {
    const lookup = new GeneratedOffsetLookup(MULTIBYTE_SOURCE);
    const beforeAccent = Buffer.byteLength("const letter = '", "utf8");
    const afterAccent = Buffer.byteLength("const letter = 'é", "utf8");
    const beforeEmoji = Buffer.byteLength("const letter = 'é';\nconst emoji = '", "utf8");
    const afterEmoji = Buffer.byteLength("const letter = 'é';\nconst emoji = '😀", "utf8");

    assert.deepEqual(lookup.generatedPositionFor(beforeAccent), { line: 0, column: 16 });
    assert.deepEqual(lookup.generatedPositionFor(afterAccent), { line: 0, column: 17 });
    assert.deepEqual(lookup.generatedPositionFor(beforeEmoji), { line: 1, column: 15 });
    assert.deepEqual(lookup.generatedPositionFor(afterEmoji), { line: 1, column: 17 });
  });

  it("batch-converts offsets to flat generated positions", () => {
    const lookup = new GeneratedOffsetLookup(SOURCE);
    const offsets = new Int32Array([0, Buffer.byteLength("const cafe = 'cafe';\n", "utf8")]);
    assert.deepEqual([...lookup.generatedPositionsFor(offsets)], [0, 0, 1, 0]);
  });
});

describe("offset lookup integration", () => {
  it("resolves a source map location from a generated offset", () => {
    const sm = new WasmSourceMap(TWO_LINE_MAP);
    const pos = originalPositionForOffset(sm, "alpha();\nbeta();\n", 0);
    assert.ok(pos);
    assert.equal(pos.source, "input.ts");
    assert.equal(pos.line, 0);
    assert.equal(pos.column, 0);
    assert.equal(pos.name, "lineOne");
    sm.free();
  });

  for (const [label, SourceMapCtor, expectTypedArray] of [
    ["WASM", WasmSourceMap, true],
    ["NAPI", NapiSourceMap, false],
  ]) {
    it(`supports batch source map lookups from coverage-style offsets with ${label} source maps`, () => {
      const sm = new SourceMapCtor(TWO_LINE_MAP);
      const lookup = new GeneratedOffsetLookup("alpha();\nbeta();\n");
      const offsets = [0, Buffer.byteLength("alpha();\n", "utf8")];
      const results = lookup.originalPositionsFor(sm, offsets);
      const fnResults = originalPositionsForOffsets(sm, "alpha();\nbeta();\n", offsets);

      assert.equal(Array.isArray(results), !expectTypedArray);
      assert.equal(Array.isArray(fnResults), !expectTypedArray);
      if (expectTypedArray) {
        assert.ok(results instanceof Int32Array);
        assert.ok(fnResults instanceof Int32Array);
      }
      assert.deepEqual([...results], [0, 0, 0, 0, 0, 1, 0, 1]);
      assert.deepEqual([...fnResults], [0, 0, 0, 0, 0, 1, 0, 1]);
      sm.free?.();
    });
  }

  it("provides a stateless convenience helper", () => {
    const sm = new WasmSourceMap(TWO_LINE_MAP);
    const pos = generatedPositionForOffset(
      "alpha();\nbeta();\n",
      Buffer.byteLength("alpha();\n", "utf8"),
    );
    assert.deepEqual(pos, { line: 1, column: 0 });
    sm.free();
  });
});
