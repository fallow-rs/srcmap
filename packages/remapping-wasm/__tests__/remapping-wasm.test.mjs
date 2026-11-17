import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { ConcatBuilder, RemapTree, remap } = require("../pkg/srcmap_remapping_wasm.js");

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

  it("rethrows an error that the loader throws", () => {
    const outer = JSON.stringify({ version: 3, sources: ["a.js"], names: [], mappings: "AAAA" });
    assert.throws(
      () =>
        remap(outer, () => {
          throw new Error("loader failed");
        }),
      /loader failed/,
    );
  });

  it("throws when the loader gives an invalid map", () => {
    const outer = JSON.stringify({ version: 3, sources: ["a.js"], names: [], mappings: "AAAA" });
    assert.throws(() => remap(outer, () => "not json"));
    assert.throws(() => remap(outer, () => ({})), /not a JSON string/);
  });
});

describe("remap names outside names", () => {
  const json = (fields) =>
    JSON.stringify({ version: 3, sources: ["a.js"], names: [], mappings: "AAAA", ...fields });

  it("gives no name for an outer name index outside names", () => {
    const result = JSON.parse(remap(json({ names: ["x"], mappings: "AAAAA,CAAAC" }), () => null));
    assert.equal(result.mappings, "AAAAA,CAAA");
    assert.deepEqual(result.names, ["x"]);
  });

  it("gives no name and no outer name for an upstream name index outside names", () => {
    const outer = json({ names: ["outer"], mappings: "AAAAA" });
    const inner = json({ sources: ["orig.js"], mappings: "AAAAA" });
    const result = JSON.parse(remap(outer, () => inner));
    assert.equal(result.mappings, "AAAA");
    assert.deepEqual(result.names, []);
    assert.deepEqual(result.sources, ["orig.js"]);
  });

  it("removes a segment that repeats the previous one after the name is removed", () => {
    const outer = json({ names: ["outer"], mappings: "AAAAA,CAAC" });
    const inner = json({ sources: ["orig.js"], mappings: "AAAAA,CAAA" });
    const result = JSON.parse(remap(outer, () => inner));
    assert.equal(result.mappings, "AAAA");
  });

  it("gives no name for an empty name", () => {
    const outer = json({ names: ["", "y"], mappings: "AAAAA,CAAAC" });
    const result = JSON.parse(remap(outer, () => null));
    assert.equal(result.mappings, "AAAA,CAAAA");
    assert.deepEqual(result.names, ["y"]);
  });

  it("still throws for a source index outside sources", () => {
    assert.throws(
      () => remap(json({ mappings: "ACAA" }), () => null),
      /source index 1 out of range/,
    );
  });
});

describe("remap names entries that are not strings", () => {
  const json = (fields) =>
    JSON.stringify({ version: 3, sources: ["a.js"], names: [], mappings: "AAAA", ...fields });

  it("gives no name for a null, false or 0 entry", () => {
    const outer = json({ names: [null, false, 0, "x"], mappings: "AAAAA,CAACC,CAACC,CAACC" });
    const result = JSON.parse(remap(outer, () => null));
    assert.equal(result.mappings, "AAAA,CAAC,CAAC,CAACA");
    assert.deepEqual(result.names, ["x"]);
  });

  it("gives no name when names is null in the outer or the upstream map", () => {
    const outer = json({ names: null, mappings: "AAAAA" });
    assert.equal(JSON.parse(remap(outer, () => null)).mappings, "AAAA");
    const named = json({ names: ["outer"], mappings: "AAAAA" });
    const inner = json({ sources: ["orig.js"], names: null, mappings: "AAAAA" });
    const result = JSON.parse(remap(named, () => inner));
    assert.equal(result.mappings, "AAAA");
    assert.deepEqual(result.names, []);
  });

  it("still throws for a truthy entry that is not a string", () => {
    assert.throws(
      () => remap(json({ names: [1], mappings: "AAAAA" }), () => null),
      /JSON parse error/,
    );
  });

  it("still throws for invalid JSON", () => {
    assert.throws(() => remap("{", () => null), /JSON parse error/);
  });
});

describe("RemapTree", () => {
  const NONE = -1;
  // Content tokens index into this list.
  const CONTENTS = ["inner source", "outer source"];

  // A map for both APIs: the JSON for `remap`, and the parts for `RemapTree`.
  const map = ({ sources, names = [], mappings, contents = [], ignoreList = [], file }) => ({
    json: JSON.stringify({
      version: 3,
      file,
      sources,
      names,
      mappings: typeof mappings === "string" ? mappings : "",
      sourcesContent: contents.map((token) => (token === NONE ? null : CONTENTS[token])),
      ignoreList,
    }),
    add: (tree, key) => {
      const args = [key, file, sources, Int32Array.from(contents), names, ignoreList];
      if (typeof mappings === "string")
        tree.addEncoded(...args, new TextEncoder().encode(mappings));
      else tree.addDecoded(...args, Int32Array.from(mappings.flat()));
    },
  });

  // Runs `RemapTree` and reads the result as the parsed JSON of `remap`.
  const runTree = (root, upstreams, passes) => {
    const tree = new RemapTree();
    try {
      for (const [key, upstream] of Object.entries(upstreams)) upstream.add(tree, key);
      root.add(tree, undefined);
      tree.remap(passes);
      const result = {
        version: 3,
        file: tree.resultFile(),
        sources: tree.resultSources(),
        names: tree.resultNames(),
        mappings: tree.resultMappings(),
      };
      const tokens = tree.resultContents();
      if (tokens.some((token) => token !== NONE)) {
        result.sourcesContent = Array.from(tokens, (token) =>
          token === NONE ? null : CONTENTS[token],
        );
      }
      const ignoreList = Array.from(tree.resultIgnoreList());
      if (ignoreList.length > 0) result.ignoreList = ignoreList;
      return JSON.parse(JSON.stringify(result));
    } finally {
      tree.free();
    }
  };

  const runJson = (root, upstreams, passes) => {
    const lookup = (source) => upstreams[source]?.json ?? null;
    let result = remap(root.json, lookup);
    for (let pass = 1; pass < passes; pass++) result = remap(result, lookup);
    return JSON.parse(result);
  };

  const inner = map({
    sources: ["orig.js", "lib.js"],
    names: ["inner"],
    mappings: "AAAAA,EAAE,ECAA;AACA",
    contents: [0, NONE],
    ignoreList: [1],
  });

  it("gives the result of remap for encoded maps", () => {
    const root = map({
      file: "out.js",
      sources: ["k"],
      names: ["o"],
      mappings: "AAAAA,CAAC,CAAE;AACA",
    });
    assert.deepEqual(runTree(root, { k: inner }, 1), runJson(root, { k: inner }, 1));
  });

  it("gives the result of remap for decoded segments", () => {
    const decoded = [
      [0, 0, 0, 0, 0, 0],
      [0, 1, 0, 0, 1, NONE],
      [0, 2, NONE, 0, 0, NONE],
      [0, 3, 0, 0, 2, NONE],
      [1, 0, 0, 1, 0, NONE],
    ];
    const root = map({ sources: ["k"], names: ["o"], mappings: decoded });
    const encoded = map({ sources: ["k"], names: ["o"], mappings: "AAAAA,CAAC,C,CAAC;AACA" });
    assert.deepEqual(runTree(root, { k: inner }, 1), runJson(encoded, { k: inner }, 1));
  });

  it("sorts an unsorted line of decoded segments, as the VLQ decode does", () => {
    const decoded = [
      [0, 4, 0, 0, 4, NONE],
      [0, 0, 0, 0, 0, NONE],
    ];
    const root = map({ sources: ["k"], mappings: decoded });
    const encoded = map({ sources: ["k"], mappings: "IAAI,JAAJ" });
    assert.deepEqual(runTree(root, { k: inner }, 1), runJson(encoded, { k: inner }, 1));
  });

  it("gives no outer name for a decoded upstream name index outside names", () => {
    const upstream = map({ sources: ["orig.js"], mappings: [[0, 0, 0, 0, 0, 5]] });
    const root = map({ sources: ["k"], names: ["outer"], mappings: "AAAAA" });
    const result = runTree(root, { k: upstream }, 1);
    assert.equal(result.mappings, "AAAA");
    assert.deepEqual(result.names, []);
  });

  it("gives the content tokens of the traced sources", () => {
    const root = map({ sources: ["k", "own.js"], mappings: "AAAA,CCAA", contents: [NONE, 1] });
    const result = runTree(root, { k: inner }, 1);
    assert.deepEqual(result.sourcesContent, ["inner source", "outer source"]);
    assert.deepEqual(result, runJson(root, { k: inner }, 1));
  });

  it("runs one pass per level", () => {
    const middle = map({ sources: ["k2"], names: ["m"], mappings: "AAAAA,EAAE;AACA" });
    const root = map({ sources: ["k1"], mappings: "AAAA,CAAE;AACA" });
    const upstreams = { k1: middle, k2: inner };
    assert.deepEqual(runTree(root, upstreams, 2), runJson(root, upstreams, 2));
    assert.deepEqual(runTree(root, upstreams, 2).sources, ["orig.js", "lib.js"]);
    assert.deepEqual(runTree(root, upstreams, 1).sources, ["k2"]);
  });

  it("throws the parse error of the root map", () => {
    const root = map({ sources: ["a.js"], mappings: "ACAA" });
    assert.throws(() => runTree(root, {}, 1), /source index 1 out of range/);
  });

  it("parses an upstream map only when a segment needs it", () => {
    const broken = map({ sources: ["orig.js"], mappings: "!" });
    const unused = map({ sources: ["k"], mappings: "A" });
    assert.equal(runTree(unused, { k: broken }, 1).mappings, "");
    const used = map({ sources: ["k"], mappings: "AAAA" });
    assert.throws(() => runTree(used, { k: broken }, 1), /invalid base64/);
  });

  it("rejects encoded mappings that are not UTF-8", () => {
    const tree = new RemapTree();
    try {
      const args = [undefined, undefined, ["a.js"], Int32Array.of(NONE), [], new Uint32Array()];
      assert.throws(() => tree.addEncoded(...args, Uint8Array.of(0xff)), /not valid UTF-8/);
    } finally {
      tree.free();
    }
  });
});
