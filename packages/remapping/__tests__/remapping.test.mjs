import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import remapping, { SourceMap } from "../src/remapping.mjs";

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

describe("SourceMap", () => {
  it("has version, sources, names, mappings, sourcesContent", () => {
    const sm = new SourceMap(ORIGINAL_MAP);
    assert.equal(sm.version, 3);
    assert.deepEqual(sm.sources, ["original.ts"]);
    assert.deepEqual(sm.names, []);
    assert.equal(sm.mappings, "AAAA");
    assert.deepEqual(sm.sourcesContent, ["const x = 1;"]);
  });

  it("toString() returns JSON string", () => {
    const sm = new SourceMap(ORIGINAL_MAP);
    const str = sm.toString();
    const parsed = JSON.parse(str);
    assert.equal(parsed.version, 3);
    assert.deepEqual(parsed.sources, ["original.ts"]);
  });

  it("toJSON() returns plain object", () => {
    const sm = new SourceMap(ORIGINAL_MAP);
    const json = sm.toJSON();
    assert.equal(json.version, 3);
    assert.deepEqual(json.sources, ["original.ts"]);
    assert.equal(json.mappings, "AAAA");
  });

  it("file is undefined when not set", () => {
    const sm = new SourceMap(ORIGINAL_MAP);
    assert.equal(sm.file, undefined);
  });

  it("file is set when provided", () => {
    const sm = new SourceMap({ ...ORIGINAL_MAP, file: "output.js" });
    assert.equal(sm.file, "output.js");
  });
});

describe("remapping(singleMap, loader)", () => {
  it("remaps through a single upstream map", () => {
    const result = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === "intermediate.js") return INNER_MAP;
      return null;
    });

    assert.ok(result instanceof SourceMap);
    assert.equal(result.version, 3);
    assert.deepEqual(result.sources, ["original.js"]);
  });

  it("passes through sources with no upstream", () => {
    const result = remapping(ORIGINAL_MAP, () => null);
    assert.deepEqual(result.sources, ["original.ts"]);
  });

  it("accepts JSON string input", () => {
    const result = remapping(JSON.stringify(INTERMEDIATE_MAP), (source) => {
      if (source === "intermediate.js") return INNER_MAP;
      return null;
    });

    assert.deepEqual(result.sources, ["original.js"]);
  });

  it("accepts JSON string from loader", () => {
    const result = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === "intermediate.js") return JSON.stringify(INNER_MAP);
      return null;
    });

    assert.deepEqual(result.sources, ["original.js"]);
  });

  it("propagates sourcesContent from upstream", () => {
    const outer = {
      version: 3,
      sources: ["compiled.js"],
      names: [],
      mappings: "AAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["const x = 1;"],
      names: [],
      mappings: "AAAA",
    };

    const result = remapping(outer, (source) => (source === "compiled.js" ? inner : null));
    assert.deepEqual(result.sourcesContent, ["const x = 1;"]);
  });

  it("preserves names from outer map when upstream has none", () => {
    const outer = {
      version: 3,
      sources: ["compiled.js"],
      names: ["myFunc"],
      mappings: "AAAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      names: [],
      mappings: "AAAA",
    };

    const result = remapping(outer, (source) => (source === "compiled.js" ? inner : null));
    assert.ok(result.names.includes("myFunc"));
  });

  it("handles partial upstream maps", () => {
    const outer = {
      version: 3,
      sources: ["compiled.js", "passthrough.js"],
      names: [],
      mappings: "AAAA,KCCA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      names: [],
      mappings: "AAAA",
    };

    const result = remapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });

    assert.ok(result.sources.includes("original.ts"));
    assert.ok(result.sources.includes("passthrough.js"));
  });

  it("loader returning undefined is treated as no upstream", () => {
    const result = remapping(ORIGINAL_MAP, () => undefined);
    assert.deepEqual(result.sources, ["original.ts"]);
  });
});

describe("remapping(decodedMap, loader)", () => {
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

describe("remapping([maps], loader)", () => {
  it("composes two source maps", () => {
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
    assert.ok(result instanceof SourceMap);
    assert.deepEqual(result.sources, ["original.ts"]);
  });

  it("composes three source maps", () => {
    const step3 = {
      version: 3,
      sources: ["step2.js"],
      names: [],
      mappings: "AAAA",
    };
    const step2 = {
      version: 3,
      sources: ["step1.js"],
      names: [],
      mappings: "AAAA",
    };
    const step1 = {
      version: 3,
      sources: ["original.ts"],
      names: [],
      mappings: "AAAA",
    };

    const result = remapping([step3, step2, step1], () => null);
    assert.deepEqual(result.sources, ["original.ts"]);
  });

  it("handles single-element array", () => {
    const result = remapping([ORIGINAL_MAP], () => null);
    assert.deepEqual(result.sources, ["original.ts"]);
  });

  it("propagates sourcesContent through chain", () => {
    const outer = {
      version: 3,
      sources: ["intermediate.js"],
      names: [],
      mappings: "AAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["const x = 1;"],
      names: [],
      mappings: "AAAA",
    };

    const result = remapping([outer, inner], () => null);
    assert.deepEqual(result.sourcesContent, ["const x = 1;"]);
  });

  it("consults loader after chain exhaustion", () => {
    const outer = {
      version: 3,
      sources: ["intermediate.js"],
      names: [],
      mappings: "AAAA",
    };
    const inner = {
      version: 3,
      sources: ["another-intermediate.js"],
      names: [],
      mappings: "AAAA",
    };
    const deepest = {
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["const x = 1;"],
      names: [],
      mappings: "AAAA",
    };

    const result = remapping([outer, inner], (source) => {
      if (source === "another-intermediate.js") return deepest;
      return null;
    });

    assert.deepEqual(result.sources, ["original.ts"]);
  });

  it("works with JSON string inputs in array", () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ["intermediate.js"],
      names: [],
      mappings: "AAAA",
    });
    const inner = JSON.stringify({
      version: 3,
      sources: ["original.ts"],
      names: [],
      mappings: "AAAA",
    });

    const result = remapping([outer, inner], () => null);
    assert.deepEqual(result.sources, ["original.ts"]);
  });
});

describe("Vite combineSourcemaps pattern", () => {
  it("remapping(sourcemapList, () => null) — simple chain", () => {
    // Vite's fast path: array of maps, no loader
    const maps = [
      {
        version: 3,
        sources: ["step1.js"],
        names: [],
        mappings: "AAAA",
      },
      {
        version: 3,
        sources: ["original.vue"],
        sourcesContent: ["<template>hello</template>"],
        names: [],
        mappings: "AAAA",
      },
    ];

    const result = remapping(maps, () => null);
    assert.ok(result instanceof SourceMap);
    assert.deepEqual(result.sources, ["original.vue"]);
  });

  it("remapping(map, loader) — loader-based chain", () => {
    // Vite's loader path for multi-source chains
    const sourcemapList = [
      {
        version: 3,
        sources: ["test.vue"],
        names: [],
        mappings: "AAAA",
      },
      {
        version: 3,
        sources: ["test.vue"],
        sourcesContent: ["<script>export default {}</script>"],
        names: [],
        mappings: "AAAA",
      },
    ];

    const filename = "test.vue";
    let mapIndex = 1;

    const result = remapping(sourcemapList[0], (sourcefile) => {
      if (sourcefile === filename && sourcemapList[mapIndex]) {
        return sourcemapList[mapIndex++];
      }
      return null;
    });

    assert.ok(result instanceof SourceMap);
  });
});

describe("return value interface", () => {
  it("result can be passed as input to another remapping call", () => {
    const step1 = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === "intermediate.js") return INNER_MAP;
      return null;
    });

    // Use the result as input to another remapping
    const step2 = remapping(step1, () => null);
    assert.ok(step2 instanceof SourceMap);
    assert.deepEqual(step2.sources, ["original.js"]);
  });
});

describe("error handling", () => {
  it("throws on invalid JSON string input", () => {
    assert.throws(() => remapping("not json", () => null));
  });

  it("throws a TypeError for an empty array, as jridgewell does", () => {
    let calls = 0;
    assert.throws(
      () =>
        remapping([], () => {
          calls++;
          return null;
        }),
      { name: "TypeError", message: /at least one source map/ },
    );
    assert.equal(calls, 0);
  });
});

// Object maps go to the WASM remap as parts, other maps as JSON. A key that the
// parts path does not know sends the call to the JSON path, so each case runs on
// both paths and must give the same result or the same error.
describe("parts path and JSON path", () => {
  const require = createRequire(import.meta.url);
  const { RemapTree } = require("@srcmap/remapping-wasm");

  const outcome = (run) => {
    try {
      return { result: run().toJSON() };
    } catch (error) {
      // The extra key of the JSON path moves the error position in the JSON.
      const message = error.message.replace(/ at line \d+ column \d+$/, "");
      return { error: `${error.constructor.name}: ${message}` };
    }
  };

  // Runs `make()` on both paths. Returns the number of parts-path remaps.
  const compare = (make, loader = () => null) => {
    const remap = RemapTree.prototype.remap;
    let partsRemaps = 0;
    RemapTree.prototype.remap = function (passes) {
      partsRemaps++;
      return remap.call(this, passes);
    };
    let parts;
    try {
      parts = outcome(() => remapping(make(), loader));
    } finally {
      RemapTree.prototype.remap = remap;
    }
    const forceJson = (input) =>
      Array.isArray(input)
        ? input.map((map, i) => (i === 0 ? { ...map, x_json_path: true } : map))
        : { ...input, x_json_path: true };
    const json = outcome(() => remapping(forceJson(make()), loader));
    assert.deepEqual(parts, json);
    return partsRemaps;
  };

  const inner = () => ({
    version: 3,
    file: "inner.js",
    sources: ["a.ts", "b.ts"],
    sourcesContent: ["a content", null],
    names: ["n", "m"],
    mappings: "AAAAA,EAAE,ECAAC;AACA",
    ignoreList: [1],
  });

  const decodedOuter = (mappings, names = ["outer"]) => ({
    file: "out.js",
    sources: ["inner.js"],
    sourcesContent: ["inner content"],
    names,
    mappings,
  });

  it("gives the same result for a decoded outer map and an encoded inner map", () => {
    const mappings = [[[0, 0, 0, 0], [2, 0, 0, 2, 0], [3], [4, 0, 0, 4]], [], [[0, 0, 1, 0]]];
    assert.equal(
      compare(() => [decodedOuter(mappings), inner()]),
      1,
    );
  });

  it("gives the same result for names outside names and a name index of -1", () => {
    const mappings = [
      [
        [0, 0, 0, 0, 7],
        [2, 0, 0, 2, -1],
        [4, 0, 0, 4, 0],
      ],
    ];
    compare(() => [decodedOuter(mappings), inner()]);
    compare(() => [decodedOuter([[[0, 0, 0, 0]]]), { ...inner(), mappings: "AAAAK" }]);
    // In an upstream map, a name outside `names` must not take the outer name.
    const named = { version: 3, sources: ["mid.js"], names: ["outer"], mappings: "AAAAA,EAAEA" };
    const unnamed = () => ({ ...inner(), names: [], mappings: "AAAA,EAAE" });
    for (const name of [-1, 9]) {
      const middle = { sources: ["inner.js"], names: ["m"], mappings: [[[0, 0, 0, 0, name]]] };
      assert.equal(
        compare(() => [named, middle, unnamed()]),
        name === -1 ? 0 : 1,
      );
      assert.deepEqual(remapping([named, middle, unnamed()], () => null).names, []);
    }
  });

  it("gives the same result for an unsorted decoded line", () => {
    const mappings = [
      [
        [4, 0, 0, 4],
        [0, 0, 0, 0],
        [4, 0, 0, 2],
      ],
    ];
    assert.equal(
      compare(() => [decodedOuter(mappings), inner()]),
      1,
    );
  });

  it("gives the same result for a chain of three maps and for a loader", () => {
    const middle = () => ({
      version: 3,
      sources: ["inner.js"],
      names: ["mid"],
      mappings: [
        [
          [0, 0, 0, 0, 0],
          [3, 0, 0, 2],
        ],
        [[0, 0, 1, 0]],
      ],
    });
    const outer = () =>
      decodedOuter([
        [
          [0, 0, 0, 0],
          [1, 0, 0, 3],
        ],
        [[0, 0, 1, 0]],
      ]);
    assert.equal(
      compare(() => [outer(), middle(), inner()]),
      1,
    );
    const loader = (file) => (file === "inner.js" ? inner() : null);
    compare(() => decodedOuter([[[0, 0, 0, 0]]]), loader);
  });

  it("gives the same errors for maps that the parts path does not take", () => {
    const decoded = [[[0, 0, 0, 0, 0]]];
    compare(() => [decodedOuter(decoded, ["\ud800"]), inner()]);
    compare(() => [decodedOuter(decoded), { ...inner(), sourcesContent: ["\udc00", null] }]);
    compare(() => [{ ...decodedOuter(decoded), file: 5 }, inner()]);
    compare(() => [decodedOuter([[[0, 1, 0, 0]]]), inner()]);
    compare(() => [decodedOuter([[[0, 0]]]), inner()]);
    compare(() => [decodedOuter([[["0", 0, 0, 0]]]), inner()]);
    compare(() => [decodedOuter([[[0.5, 0, 0, 0]]]), inner()]);
    compare(() => [decodedOuter([[[2 ** 30, 0, 0, 0]]]), inner()]);
    compare(() => [decodedOuter(decoded, [null]), inner()]);
    compare(() => [decodedOuter(decoded), { ...inner(), mappings: "!" }]);
  });

  // The VLQ mappings go to WASM as UTF-8 bytes in one shared buffer.
  it("gives the same result for VLQ mappings of each length and with non-ASCII text", () => {
    const outer = () => decodedOuter([[[0, 0, 0, 0]], [[0, 0, 1, 0]]]);
    // Longer than the first buffer, so the buffer grows. The short map after
    // it must not read the old bytes after its own.
    const long = Array.from({ length: 40_000 }, () => "AAAA").join(";") + ";AACA";
    assert.equal(
      compare(() => [outer(), { ...inner(), mappings: long }]),
      1,
    );
    compare(() => [outer(), inner()]);
    // A character outside ASCII takes more bytes than its string length.
    compare(() => [outer(), { ...inner(), mappings: "AAAA,é" }]);
    compare(() => [outer(), { ...inner(), mappings: "AAAA;" + "€".repeat(200_000) }]);
    compare(() => [outer(), inner()]);
  });
});

// remapping.mjs and remapping.cjs have the same body. Only the module header and
// the export lines differ.
describe("ESM and CJS builds", () => {
  const BODY_START = "/**\n * Result class";
  const bodyOf = (file, end) => {
    const text = readFileSync(fileURLToPath(new URL(`../src/${file}`, import.meta.url)), "utf8");
    const start = text.indexOf(BODY_START);
    const stop = text.indexOf(end);
    assert.ok(start >= 0 && stop > start, `${file}: body markers not found`);
    return text.slice(start, stop);
  };

  it("have the same body", () => {
    const esm = bodyOf("remapping.mjs", "\nexport default remapping;");
    const cjs = bodyOf("remapping.cjs", "\nmodule.exports = remapping;");
    assert.equal(cjs, esm, "copy the body of remapping.mjs to remapping.cjs");
  });
});

describe("remapping(indexedMap, loader)", () => {
  it("calls the loader for each section source, depth first", () => {
    const indexed = {
      version: 3,
      sections: [
        {
          offset: { line: 0, column: 0 },
          map: { version: 3, sources: ["a.js"], names: [], mappings: "AAAA" },
        },
        {
          offset: { line: 1, column: 0 },
          map: { version: 3, sources: ["b.js"], names: [], mappings: "AAAA" },
        },
      ],
    };
    const upstream = { version: 3, sources: ["a.ts"], names: [], mappings: "AAAA" };
    const calls = [];
    const result = remapping(indexed, (source) => {
      calls.push(source);
      return source === "a.js" ? upstream : null;
    });
    assert.deepEqual(calls, ["a.js", "a.ts", "b.js"]);
    assert.deepEqual(result.sources, ["a.ts", "b.js"]);
    assert.equal(result.mappings, "AAAA;ACAA");
  });
});
