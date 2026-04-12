import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
// fallow-ignore-next-line unresolved-import
const { ConcatBuilder, remap } = require("../pkg/srcmap_remapping_wasm.js");

const MAP_A = JSON.stringify({
  version: 3,
  sources: ["a.js"],
  names: ["foo"],
  mappings: "AAAAA",
});

const MAP_B = JSON.stringify({
  version: 3,
  sources: ["b.js"],
  names: ["bar"],
  mappings: "AAAAA",
});

// ── ConcatBuilder ────────────────────────────────────────────────

describe("ConcatBuilder", () => {
  it("concatenates two source maps", () => {
    const builder = new ConcatBuilder("bundle.js");
    builder.addMap(MAP_A, 0);
    builder.addMap(MAP_B, 1);

    const map = JSON.parse(builder.toJSON());
    assert.equal(map.version, 3);
    assert.deepEqual(map.sources, ["a.js", "b.js"]);
    assert.deepEqual(map.names, ["foo", "bar"]);
    assert.equal(map.file, "bundle.js");
    builder.free();
  });

  it("deduplicates shared sources", () => {
    const builder = new ConcatBuilder();
    builder.addMap(MAP_A, 0);
    builder.addMap(MAP_A, 10);

    const map = JSON.parse(builder.toJSON());
    assert.equal(map.sources.length, 1);
    assert.deepEqual(map.sources, ["a.js"]);
    builder.free();
  });

  it("preserves sourcesContent", () => {
    const mapWithContent = JSON.stringify({
      version: 3,
      sources: ["a.js"],
      sourcesContent: ["var a;"],
      names: [],
      mappings: "AAAA",
    });

    const builder = new ConcatBuilder();
    builder.addMap(mapWithContent, 0);

    const map = JSON.parse(builder.toJSON());
    assert.deepEqual(map.sourcesContent, ["var a;"]);
    builder.free();
  });

  it("handles empty builder", () => {
    const builder = new ConcatBuilder("empty.js");
    const map = JSON.parse(builder.toJSON());
    assert.equal(map.version, 3);
    assert.deepEqual(map.sources, []);
    builder.free();
  });

  it("throws on invalid JSON input", () => {
    const builder = new ConcatBuilder();
    assert.throws(() => builder.addMap("not json", 0));
    builder.free();
  });
});

// ── remap ────────────────────────────────────────────────────────

describe("remap", () => {
  it("remaps through a single upstream map", () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ["intermediate.js"],
      names: [],
      mappings: "AAAA;AACA",
    });

    const inner = JSON.stringify({
      version: 3,
      sources: ["original.js"],
      names: [],
      mappings: "AACA;AACA",
    });

    const result = JSON.parse(
      remap(outer, (source) => {
        if (source === "intermediate.js") return inner;
        return null;
      }),
    );

    assert.deepEqual(result.sources, ["original.js"]);
  });

  it("passes through sources with no upstream", () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ["already-original.js"],
      names: [],
      mappings: "AAAA",
    });

    const result = JSON.parse(remap(outer, () => null));
    assert.deepEqual(result.sources, ["already-original.js"]);
  });

  it("handles partial upstream maps", () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ["compiled.js", "passthrough.js"],
      names: [],
      mappings: "AAAA,KCCA",
    });

    const inner = JSON.stringify({
      version: 3,
      sources: ["original.ts"],
      names: [],
      mappings: "AAAA",
    });

    const result = JSON.parse(
      remap(outer, (source) => {
        if (source === "compiled.js") return inner;
        return null;
      }),
    );

    assert.ok(result.sources.includes("original.ts"));
    assert.ok(result.sources.includes("passthrough.js"));
  });

  it("preserves names from outer map when upstream has none", () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ["compiled.js"],
      names: ["myFunc"],
      mappings: "AAAAA",
    });

    const inner = JSON.stringify({
      version: 3,
      sources: ["original.ts"],
      names: [],
      mappings: "AAAA",
    });

    const result = JSON.parse(remap(outer, () => inner));
    assert.ok(result.names.includes("myFunc"));
  });

  it("prefers upstream names over outer names", () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ["compiled.js"],
      names: ["outerName"],
      mappings: "AAAAA",
    });

    const inner = JSON.stringify({
      version: 3,
      sources: ["original.ts"],
      names: ["innerName"],
      mappings: "AAAAA",
    });

    const result = JSON.parse(remap(outer, () => inner));
    assert.ok(result.names.includes("innerName"));
  });

  it("propagates sourcesContent from upstream", () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ["compiled.js"],
      names: [],
      mappings: "AAAA",
    });

    const inner = JSON.stringify({
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["const x = 1;"],
      names: [],
      mappings: "AAAA",
    });

    const result = JSON.parse(remap(outer, () => inner));
    assert.deepEqual(result.sourcesContent, ["const x = 1;"]);
  });

  it("throws on invalid outer map", () => {
    assert.throws(() => remap("not json", () => null));
  });
});
