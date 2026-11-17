/**
 * Cross-validation tests comparing @srcmap/remapping against
 * @jridgewell/remapping to verify drop-in compatibility.
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import srcmapRemapping from "../src/remapping.mjs";
import jrRemapping from "@jridgewell/remapping";
import { TraceMap } from "@jridgewell/trace-mapping";

const compareMaps = (srcmapResult, jrResult, label) => {
  assert.deepEqual(srcmapResult.sources, jrResult.sources, `${label}: sources mismatch`);
  assert.equal(srcmapResult.mappings, jrResult.mappings, `${label}: mappings mismatch`);
  assert.deepEqual(srcmapResult.names, jrResult.names, `${label}: names mismatch`);
};

describe("cross-validation with @jridgewell/remapping", () => {
  it("simple chain produces identical output", () => {
    const maps = [
      {
        version: 3,
        sources: ["intermediate.js"],
        names: [],
        mappings: "AAAA;AACA",
      },
      {
        version: 3,
        sources: ["original.js"],
        sourcesContent: ["line1\nline2\nline3"],
        names: [],
        mappings: "AACA;AACA",
      },
    ];
    const srcmap = srcmapRemapping(maps, () => null);
    const jr = jrRemapping(maps, () => null);
    compareMaps(srcmap, jr, "simple chain");
  });

  it("single map with loader produces identical output", () => {
    const outer = {
      version: 3,
      sources: ["compiled.js"],
      names: ["a", "b"],
      mappings: "AAAAA,EAACC,EAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["const a = 1;\nconst b = 2;\nconst c = 3;"],
      names: ["x", "y"],
      mappings: "AAAA,EAACC;AAAA",
    };
    const srcmap = srcmapRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    const jr = jrRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    compareMaps(srcmap, jr, "multi-segment");
  });

  it("partial upstream (passthrough + remapped) matches", () => {
    const outer = {
      version: 3,
      sources: ["a.js", "b.js"],
      names: [],
      mappings: "AAAA,KCCA",
    };
    const inner = {
      version: 3,
      sources: ["original.js"],
      sourcesContent: ["original code"],
      names: [],
      mappings: "AAAA",
    };
    const srcmap = srcmapRemapping(outer, (source) => {
      if (source === "a.js") return inner;
      return null;
    });
    const jr = jrRemapping(outer, (source) => {
      if (source === "a.js") return inner;
      return null;
    });
    compareMaps(srcmap, jr, "partial upstream");
  });

  it("no upstream (full passthrough) matches", () => {
    const map = {
      version: 3,
      sources: ["app.js"],
      sourcesContent: ["const x = 1;"],
      names: ["x"],
      mappings: "AAAAA",
    };
    const srcmap = srcmapRemapping(map, () => null);
    const jr = jrRemapping(map, () => null);
    compareMaps(srcmap, jr, "full passthrough");
  });

  it("empty-string source is filtered (matches jridgewell)", () => {
    const map = {
      version: 3,
      sources: [""],
      names: [],
      mappings: "AAAA",
    };
    const srcmap = srcmapRemapping(map, () => null);
    const jr = jrRemapping(map, () => null);

    assert.equal(
      srcmap.sources.filter((s) => s === "").length,
      jr.sources.filter((s) => s === "").length,
      "empty-string source count should match",
    );
    assert.equal(
      srcmap.mappings,
      jr.mappings,
      "mappings should match for empty-string source input",
    );
  });

  it("duplicate mappings are deduplicated (matches jridgewell)", () => {
    const map = {
      version: 3,
      sources: ["a.js"],
      sourcesContent: ["source"],
      names: [],
      mappings: "AAAA,EAAA,EAAA",
    };
    const srcmap = srcmapRemapping(map, () => null);
    const jr = jrRemapping(map, () => null);
    compareMaps(srcmap, jr, "deduplication");
  });

  it("upstream with no match drops segment (matches jridgewell)", () => {
    const outer = {
      version: 3,
      sources: ["compiled.js"],
      names: ["fn"],
      mappings: "AAAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["code"],
      names: [],
      // Only has mapping at line 5, not line 0
      mappings: ";;;;AAAA",
    };
    const srcmap = srcmapRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    const jr = jrRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    compareMaps(srcmap, jr, "no-match drop");
  });

  it("name propagation matches jridgewell (upstream name wins)", () => {
    const outer = {
      version: 3,
      sources: ["compiled.js"],
      names: ["outerName"],
      mappings: "AAAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["code"],
      names: ["innerName"],
      mappings: "AAAAA",
    };
    const srcmap = srcmapRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    const jr = jrRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    compareMaps(srcmap, jr, "name propagation (upstream wins)");
  });

  it("name propagation matches jridgewell (outer fallback)", () => {
    const outer = {
      version: 3,
      sources: ["compiled.js"],
      names: ["outerName"],
      mappings: "AAAAA",
    };
    const inner = {
      version: 3,
      sources: ["original.ts"],
      sourcesContent: ["code"],
      names: [],
      mappings: "AAAA",
    };
    const srcmap = srcmapRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    const jr = jrRemapping(outer, (source) => {
      if (source === "compiled.js") return inner;
      return null;
    });
    compareMaps(srcmap, jr, "name propagation (outer fallback)");
  });

  it("Vite-style chain matches jridgewell", () => {
    const maps = [
      {
        version: 3,
        sources: ["step1.js"],
        names: [],
        mappings: "AAAA;AACA",
      },
      {
        version: 3,
        sources: ["original.vue"],
        sourcesContent: ["<template>hello</template>\n<script>export default {}</script>"],
        names: [],
        mappings: "AAAA;AACA",
      },
    ];
    const srcmap = srcmapRemapping(maps, () => null);
    const jr = jrRemapping(maps, () => null);
    compareMaps(srcmap, jr, "Vite chain");
  });

  it("multi-line chain with different positions", () => {
    const maps = [
      {
        version: 3,
        sources: ["step2.js"],
        names: [],
        mappings: "AAAA;AACA;AACA",
      },
      {
        version: 3,
        sources: ["step1.js"],
        sourcesContent: ["a\nb\nc\nd"],
        names: [],
        mappings: "AAAA;AACA;AACA;AACA",
      },
    ];
    const srcmap = srcmapRemapping(maps, () => null);
    const jr = jrRemapping(maps, () => null);
    compareMaps(srcmap, jr, "multi-line chain");
  });
});

// Vite passes the result of `MagicString.generateDecodedMap()` to `remapping`. That
// map has no `version`, holds `mappings` as segment arrays, and holds `rangeMappings`
// as one array per line. @jridgewell/remapping reads neither `version` nor
// `rangeMappings`.
describe("Vite input compatibility with @jridgewell/remapping", () => {
  const innerMap = () => ({
    version: 3,
    sources: ["original.ts"],
    sourcesContent: ["const a = 1;\nconst b = 2;"],
    names: [],
    mappings: "AAAA,MAAM,EAAE;AACA,MAAM",
  });

  const decodedMap = () => ({
    sources: ["mid.ts"],
    sourcesContent: ["const a = 1;\nconst b = 2;"],
    names: [],
    mappings: [
      [
        [0, 0, 0, 0],
        [6, 0, 0, 6],
      ],
      [[0, 0, 1, 0]],
    ],
  });

  it("accepts a decoded map without version", () => {
    const maps = [decodedMap(), innerMap()];
    const srcmap = srcmapRemapping(maps, () => null);
    const jr = jrRemapping(maps, () => null);
    compareMaps(srcmap, jr, "decoded map without version");
  });

  it("ignores rangeMappings given as one array per line", () => {
    const maps = [{ version: 3, ...decodedMap(), rangeMappings: [[], [0]] }, innerMap()];
    const srcmap = srcmapRemapping(maps, () => null);
    const jr = jrRemapping(maps, () => null);
    compareMaps(srcmap, jr, "decoded map with rangeMappings array");
  });

  it("keeps a sourceless segment that an inner map gives", () => {
    const maps = [
      { version: 3, sources: ["mid.js"], names: [], mappings: "AAAA,IAAI,IAAI" },
      { version: 3, sources: ["original.js"], names: [], mappings: "AAAA,IAAI,I" },
    ];
    const srcmap = srcmapRemapping(maps, () => null);
    const jr = jrRemapping(maps, () => null);
    compareMaps(srcmap, jr, "inner sourceless segment");
  });

  it("takes the first inner segment when several share one column", () => {
    const maps = [
      { version: 3, sources: ["mid.js"], names: [], mappings: "CAAC,MAAM" },
      { version: 3, sources: ["original.js"], names: [], mappings: "CAAG,AAAC,OAAE" },
    ];
    const srcmap = srcmapRemapping(maps, () => null);
    const jr = jrRemapping(maps, () => null);
    compareMaps(srcmap, jr, "inner segments at one column");
  });
});

describe("input edge cases match @jridgewell/remapping", () => {
  const outerMap = (sources) => ({ version: 3, sources, names: [], mappings: "AAAA,MAAM" });
  const innerMap = () => ({
    version: 3,
    sources: ["original.js"],
    names: [],
    mappings: "AAAA,MAAC",
  });
  const compareBoth = (input, loader, label) => {
    compareMaps(srcmapRemapping(input, loader), jrRemapping(input, loader), label);
  };
  const throwsLikeJridgewell = (run) => {
    assert.throws(() => run(jrRemapping));
    assert.throws(() => run(srcmapRemapping));
  };

  it("accepts a JSON string map without version", () => {
    const { version: _version, ...inner } = innerMap();
    compareBoth([outerMap(["mid.js"]), JSON.stringify(inner)], () => null, "string map");
  });

  it("ignores a string rangeMappings in a JSON string map", () => {
    const inner = {
      version: 3,
      sources: ["original.js"],
      names: [],
      mappings: "AAAA",
      rangeMappings: "A",
    };
    compareBoth([outerMap(["mid.js"]), JSON.stringify(inner)], () => null, "string rangeMappings");
  });

  it("accepts a loader result as a JSON string without version", () => {
    const { version: _version, ...inner } = innerMap();
    const loader = (source) => (source === "mid.js" ? JSON.stringify(inner) : null);
    compareBoth(outerMap(["mid.js"]), loader, "loader string");
  });

  it("composes an outer map whose source is empty or null", () => {
    compareBoth([outerMap([""]), innerMap()], () => null, "empty source");
    compareBoth([outerMap([null]), innerMap()], () => null, "null source");
  });

  it("calls the loader for an empty source", () => {
    compareBoth(outerMap([""]), (source) => (source === "" ? innerMap() : null), "loader empty");
  });

  it("rejects an outer map with more than one source in a chain", () => {
    const outer = { ...outerMap(["a.js", "b.js"]), mappings: "AAAA,MCAM" };
    throwsLikeJridgewell((remapping) => remapping([outer, innerMap()], () => null));
  });

  it("reports an error that the loader throws", () => {
    throwsLikeJridgewell((remapping) =>
      remapping(outerMap(["mid.js"]), () => {
        throw new Error("loader failed");
      }),
    );
  });

  it("reports a map that is not valid JSON", () => {
    throwsLikeJridgewell((remapping) => remapping(outerMap(["mid.js"]), () => "{not json"));
    throwsLikeJridgewell((remapping) => remapping([outerMap(["mid.js"]), "{not json"], () => null));
  });
});

describe("source resolution and options match @jridgewell/remapping", () => {
  const map = (fields) => ({ version: 3, names: [], mappings: "AAAA", ...fields });
  const sameJson = (run, label) => {
    assert.equal(JSON.stringify(run(srcmapRemapping)), JSON.stringify(run(jrRemapping)), label);
  };

  it("normalizes source paths", () => {
    for (const source of ["a/../b.js", "/abs/./a.js", "a//b.js", "./a.js", "../x.js"]) {
      sameJson((remapping) => remapping(map({ sources: [source] }), () => null), source);
    }
  });

  it("joins sourceRoot and sources with a slash", () => {
    const cases = [
      ["root", "a.js"],
      ["root/", "a.js"],
      ["http://example.com/src", "a.js"],
      ["root", "../a.js"],
      ["a/b/", "../../c.js"],
      ["root", null],
    ];
    for (const [sourceRoot, source] of cases) {
      sameJson(
        (remapping) => remapping(map({ sourceRoot, sources: [source] }), () => null),
        `${sourceRoot} + ${source}`,
      );
    }
  });

  it("resolves the sources of a loaded map against the importer", () => {
    const outer = map({ sources: ["dist/sub/a.js"] });
    const inner = map({ sources: ["../src/a.ts"] });
    const loaded = [];
    const run = (remapping) =>
      remapping(outer, (source) => {
        loaded.push(source);
        return source === "dist/sub/a.js" ? inner : null;
      });
    sameJson(run, "importer-relative sources");
    assert.deepEqual(loaded.slice(0, loaded.length / 2), loaded.slice(loaded.length / 2));
  });

  it("gives the same output fields", () => {
    const withContent = map({ file: "out.js", sources: ["a.js"], sourcesContent: ["x"] });
    sameJson((remapping) => remapping(withContent, () => null), "with content");
    sameJson((remapping) => remapping(map({ sources: ["a.js"] }), () => null), "without content");
  });

  it("applies excludeContent and decodedMappings", () => {
    const input = map({ sources: ["a.js"], sourcesContent: ["x"], mappings: "AAAA;AACA" });
    for (const options of [true, false, { excludeContent: true }, { decodedMappings: true }]) {
      sameJson((remapping) => remapping(input, () => null, options), JSON.stringify(options));
    }
  });
});

describe("loader calls match @jridgewell/remapping", () => {
  const map = (sources, mappings = "AAAA") => ({ version: 3, sources, names: [], mappings });
  const compareRuns = (run, label) => {
    const srcmapCalls = [];
    const jrCalls = [];
    const srcmap = JSON.stringify(run(srcmapRemapping, srcmapCalls));
    const jr = JSON.stringify(run(jrRemapping, jrCalls));
    assert.equal(srcmap, jr, `${label}: output`);
    assert.deepEqual(srcmapCalls, jrCalls, `${label}: loader calls`);
  };

  it("follows the loader through more than one level", () => {
    const upstream = { "a.js": map(["b.js"]), "b.js": map(["c.js"]) };
    compareRuns(
      (remapping, calls) =>
        remapping(map(["a.js"]), (source) => {
          calls.push(source);
          return upstream[source] ?? null;
        }),
      "three levels",
    );
  });

  it("calls the loader by source index, depth first", () => {
    compareRuns((remapping, calls) => {
      const copies = [map(["x.js"]), map(["y.js"])];
      let next = 0;
      return remapping(map(["a.js", "b.js"], "ACAA,KDAA"), (source) => {
        calls.push(source);
        return copies[next++] ?? null;
      });
    }, "stateful loader");
  });

  it("passes a loader context that can change the source", () => {
    compareRuns(
      (remapping, calls) =>
        remapping(map(["dir/a.js", "dir/b.js"], "AAAA,KCAA"), (source, ctx) => {
          calls.push({ ...ctx });
          if (source === "dir/a.js") return map(["c.js"]);
          ctx.source = "renamed.js";
          ctx.content = "content";
          ctx.ignore = true;
          return null;
        }),
      "loader context",
    );
  });
});

describe("review cases match @jridgewell/remapping", () => {
  const map = (fields) => ({ version: 3, names: [], mappings: "AAAA", ...fields });
  const sameJson = (run, label) => {
    assert.equal(JSON.stringify(run(srcmapRemapping)), JSON.stringify(run(jrRemapping)), label);
  };

  it("makes a segment sourceless when a loaded map has a null source", () => {
    const loaded = map({ sources: [null] });
    sameJson(
      (remapping) =>
        remapping(map({ sources: ["a.js"] }), (source) => (source === "a.js" ? loaded : null)),
      "loader",
    );
    sameJson((remapping) => remapping([map({ sources: ["a.js"] }), loaded], () => null), "chain");
  });

  it("accepts a TraceMap as input and as loader result", () => {
    const top = map({ sources: ["a.js"], mappings: "AAAA,EAAC" });
    const upstream = map({ sources: ["b.ts"], sourcesContent: ["b"] });
    sameJson((remapping) => remapping(new TraceMap(top), () => null), "input");
    sameJson(
      (remapping) =>
        remapping(top, (source) => (source === "a.js" ? new TraceMap(upstream) : null)),
      "loader result",
    );
  });

  it("rejects a map without mappings", () => {
    const invalid = { version: 3, sources: ["a.js"], names: [] };
    assert.throws(() => jrRemapping(invalid, () => null), /invalid source map/);
    assert.throws(() => srcmapRemapping(invalid, () => null), /invalid source map/);
  });

  it("decodes an empty result to an empty array", () => {
    const empty = map({ sources: [], mappings: "" });
    sameJson((remapping) => remapping(empty, () => null, { decodedMappings: true }), "empty");
  });

  it("keeps the content of the last traced segment for sources with one name", () => {
    const duplicate = map({
      sources: ["top0.js", "top0.js"],
      sourcesContent: ["c-top0", "c-top1"],
      names: ["n0", "n1", "n2", "n3"],
      mappings: "CAAE,CCAAE;A,C,EDEA;AAAF",
    });
    sameJson((remapping) => remapping(duplicate, () => null), "duplicate names");
  });

  it("lists an ignored null source as jridgewell does", () => {
    const top = map({ sources: ["a.js", null], ignoreList: [1], mappings: "AAAA,ECAA" });
    sameJson((remapping) => remapping(top, () => null), "single map");
    sameJson((remapping) => remapping([map({ sources: ["x.js"] }), top], () => null), "chain");
    sameJson(
      (remapping) => remapping(map({ sources: ["x.js"] }), (s) => (s === "x.js" ? top : null)),
      "loader",
    );
  });
});

// jridgewell adds a source to `ignoreList` at the first traced segment from an
// ignored copy of that source. Two copies of one name can differ in that.
describe("ignoreList order matches @jridgewell/remapping", () => {
  const map = (fields) => ({ version: 3, names: [], ...fields });
  const sameJson = (run, label) => {
    assert.equal(JSON.stringify(run(srcmapRemapping)), JSON.stringify(run(jrRemapping)), label);
  };
  // The copies of `a.js` sit above a branch two levels deep, so the remap needs
  // more than one pass.
  const mixed = map({
    sources: ["a.js", "b.js", "a.js", "x.js"],
    ignoreList: [1, 2],
    mappings: "AAAA,ECAA,ECAA,ECAA",
  });
  const step = (source) => map({ sources: [source], mappings: "AAAA" });
  const deep = { "x.js": step("y.js"), "y.js": step("z.js") };

  it("orders entries in a loader tree", () => {
    sameJson((remapping) => remapping(mixed, (s) => deep[s] ?? null), "loader");
  });

  it("orders entries in a chain", () => {
    const outer = map({ sources: ["m.js"], mappings: "AAAA,EAAE,EAAE,EAAE" });
    sameJson((remapping) => remapping([outer, mixed], (s) => deep[s] ?? null), "chain");
  });

  it("orders entries with an ignored null source in one map", () => {
    const single = map({
      sources: ["top2.js", null, "top2.js"],
      names: ["n0", "n1", "n2"],
      sourcesContent: [null, null, "c-top2"],
      ignoreList: [0, 1],
      mappings: "EEAA;EDAE,EAAC,GDCH;EADA,EAAEA,EECC",
    });
    sameJson((remapping) => remapping(single, () => null), "single map");
    sameJson(
      (remapping) =>
        remapping(map({ sources: ["x.js"], mappings: "AAAA" }), (s) =>
          s === "x.js" ? single : null,
        ),
      "loaded",
    );
  });
});

// A loader can set `ctx.source` to a value that is not a string. jridgewell keeps
// that value: a falsy value gives sourceless segments, and an other value is a source.
describe("ctx.source values match @jridgewell/remapping", () => {
  const top = {
    version: 3,
    sources: ["a.js", "b.js"],
    sourcesContent: ["A", "B"],
    names: [],
    mappings: "AAAA,EAAC,ECAA",
  };
  const upstream = {
    version: 3,
    sources: ["o.js"],
    sourcesContent: ["O"],
    names: [],
    mappings: "AAAA",
  };
  const VALUES = [null, undefined, "", 0, NaN, false, 42, {}, ["x"], true, Symbol("s")];
  const labelOf = (value) => String(typeof value === "symbol" ? value.description : value);
  const fields = (result) => ({
    sources: result.sources,
    sourcesContent: result.sourcesContent,
    mappings: result.mappings,
    names: result.names,
    ignoreList: result.ignoreList,
  });
  const outcome = (remapping, values, overrides = {}) => {
    const { upstream: withUpstream, ...ctxFields } = overrides;
    try {
      const result = remapping(top, (source, ctx) => {
        if (ctx.depth !== 1 || !(source in values)) return null;
        ctx.source = values[source];
        Object.assign(ctx, ctxFields);
        return withUpstream ? upstream : null;
      });
      return fields(result);
    } catch (error) {
      return { error: error.constructor.name, message: error.message };
    }
  };
  const same = (values, overrides, label) => {
    assert.deepEqual(
      outcome(srcmapRemapping, values, overrides),
      outcome(jrRemapping, values, overrides),
      label,
    );
  };

  it("handles each value for a source without an upstream map", () => {
    for (const value of VALUES) {
      same({ "a.js": value }, {}, labelOf(value));
      same({ "a.js": value }, { ignore: true }, `${labelOf(value)} ignored`);
      same({ "a.js": value }, { content: "C", ignore: true }, `${labelOf(value)} content`);
    }
  });

  it("handles each value for a source with an upstream map", () => {
    for (const value of VALUES) same({ "a.js": value }, { upstream: true }, labelOf(value));
  });

  it("merges values that convert to the same string", () => {
    same({ "a.js": {}, "b.js": {} }, {}, "two objects");
    same({ "a.js": 42, "b.js": 42 }, {}, "two numbers");
    same({ "a.js": null, "b.js": null }, { ignore: true }, "two nulls");
    same({ "a.js": 42, "b.js": "42" }, {}, "number and string");
    same({ "a.js": "42", "b.js": 42 }, {}, "string and number");
  });

  it("keeps two symbols with the same description apart", () => {
    same({ "a.js": Symbol("s"), "b.js": Symbol("s") }, {}, "two symbols");
  });

  it("rejects an object without a string form only when a segment uses it", () => {
    same({ "a.js": Object.create(null) }, {}, "used");
    const unused = { ...top, mappings: "AAAA" };
    const run = (remapping) =>
      remapping(unused, (source, ctx) => {
        if (source === "b.js") ctx.source = Object.create(null);
        return null;
      }).sources;
    assert.deepEqual(run(srcmapRemapping), run(jrRemapping));
  });

  it("rejects an empty array of maps with a TypeError", () => {
    assert.throws(() => jrRemapping([], () => null), TypeError);
    assert.throws(() => srcmapRemapping([], () => null), TypeError);
  });
});
