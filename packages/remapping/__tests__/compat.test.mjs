/**
 * Cross-validation tests comparing @srcmap/remapping against
 * @jridgewell/remapping to verify drop-in compatibility.
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import srcmapRemapping from "../src/remapping.mjs";
import jrRemapping from "@jridgewell/remapping";

// ── Helper ───────────────────────────────────────────────────────

const compareMaps = (srcmapResult, jrResult, label) => {
  assert.deepEqual(srcmapResult.sources, jrResult.sources, `${label}: sources mismatch`);
  assert.equal(srcmapResult.mappings, jrResult.mappings, `${label}: mappings mismatch`);
  assert.deepEqual(srcmapResult.names, jrResult.names, `${label}: names mismatch`);
};

// ── Cross-validation ─────────────────────────────────────────────

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
