/**
 * Cross-validation tests comparing @srcmap/remapping against
 * @jridgewell/remapping to verify drop-in compatibility.
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import srcmapRemapping from "../src/remapping.mjs";
import jrRemapping from "@jridgewell/remapping";

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
