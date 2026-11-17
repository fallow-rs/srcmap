/**
 * Cross-validation tests comparing @srcmap/trace-mapping against
 * @jridgewell/trace-mapping to verify drop-in compatibility.
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  TraceMap as SrcmapTraceMap,
  originalPositionFor as srcmapOriginal,
  generatedPositionFor as srcmapGenerated,
  eachMapping as srcmapEachMapping,
  encodedMappings as srcmapEncodedMappings,
  decodedMappings as srcmapDecodedMappings,
  sourceContentFor as srcmapSourceContentFor,
  isIgnored as srcmapIsIgnored,
  allGeneratedPositionsFor as srcmapAllGenerated,
} from "../src/trace-mapping.mjs";

import {
  TraceMap as JrTraceMap,
  originalPositionFor as jrOriginal,
  generatedPositionFor as jrGenerated,
  eachMapping as jrEachMapping,
  encodedMappings as jrEncodedMappings,
  decodedMappings as jrDecodedMappings,
  sourceContentFor as jrSourceContentFor,
  isIgnored as jrIsIgnored,
  allGeneratedPositionsFor as jrAllGenerated,
} from "@jridgewell/trace-mapping";

import { encode } from "@jridgewell/sourcemap-codec";

const SIMPLE_MAP = JSON.stringify({
  version: 3,
  file: "output.js",
  sources: ["input.js"],
  sourcesContent: ["const foo = 1;\nconst bar = 2;"],
  names: ["foo", "bar"],
  mappings: "AAAAA,SACIC",
});

const MULTI_SOURCE_MAP = JSON.stringify({
  version: 3,
  sources: ["a.js", "b.js"],
  sourcesContent: ["// a\nconst x = 1;", "// b\nconst y = 2;"],
  names: ["x", "y", "z"],
  mappings: "AAAAA;ACAAC,KACCC",
});

const IGNORE_LIST_MAP = JSON.stringify({
  version: 3,
  sources: ["app.js", "node_modules/lib.js"],
  sourcesContent: ["app code", "lib code"],
  names: [],
  mappings: "AAAA;ACAA",
  ignoreList: [1],
});

/**
 * Generate a realistic source map for stress-testing.
 */
const generateSourceMap = (lines, segsPerLine, numSources) => {
  const sources = Array.from({ length: numSources }, (_, i) => `src/file${i}.js`);
  const names = Array.from({ length: 20 }, (_, i) => `var${i}`);
  const sourcesContent = sources.map(
    (_, i) => `// source file ${i}\n${"const x = 1;\n".repeat(lines)}`,
  );

  const mappings = [];
  let src = 0;
  let srcLine = 0;
  let srcCol = 0;
  let name = 0;

  for (let line = 0; line < lines; line++) {
    const lineSegs = [];
    let genCol = 0;

    for (let s = 0; s < segsPerLine; s++) {
      genCol += 2 + ((s * 3) % 20);
      if (s % 7 === 0) src = (src + 1) % numSources;
      srcLine += 1;
      srcCol = (s * 5 + 1) % 30;

      if (s % 4 === 0) {
        name = (name + 1) % names.length;
        lineSegs.push([genCol, src, srcLine, srcCol, name]);
      } else {
        lineSegs.push([genCol, src, srcLine, srcCol]);
      }
    }
    mappings.push(lineSegs);
  }

  return JSON.stringify({
    version: 3,
    sources,
    sourcesContent,
    names,
    mappings: encode(mappings),
  });
};

const LARGE_MAP = generateSourceMap(200, 20, 5);

const TEST_MAPS = [
  { name: "simple", json: SIMPLE_MAP },
  { name: "multi-source", json: MULTI_SOURCE_MAP },
  { name: "ignore-list", json: IGNORE_LIST_MAP },
  { name: "large (200 lines)", json: LARGE_MAP },
];

// trace-mapping normalizes paths (resolves ./ segments), srcmap returns raw paths
const normalizePath = (s) => s?.replace(/\/\.\//g, "/") ?? null;

describe("cross-validation with @jridgewell/trace-mapping", () => {
  for (const { name, json } of TEST_MAPS) {
    describe(name, () => {
      it("originalPositionFor matches for random lookups", () => {
        const srcmap = new SrcmapTraceMap(json);
        const jr = new JrTraceMap(json);

        const maxLine = srcmap._wasm.lineCount;

        let checked = 0;
        for (
          let line = 0;
          line < maxLine && checked < 500;
          line += Math.max(1, Math.floor(maxLine / 50))
        ) {
          for (let col = 0; col < 200; col += 10) {
            const expected = jrOriginal(jr, { line: line + 1, column: col });
            const actual = srcmapOriginal(srcmap, { line: line + 1, column: col });

            const expectedNull = expected.source === null;
            const actualNull = actual.source === null;

            assert.equal(
              actualNull,
              expectedNull,
              `null mismatch at ${line}:${col}: jr=${JSON.stringify(expected)}, srcmap=${JSON.stringify(actual)}`,
            );

            if (!expectedNull && !actualNull) {
              assert.equal(
                normalizePath(actual.source),
                normalizePath(expected.source),
                `source mismatch at ${line}:${col}`,
              );
              assert.equal(actual.line, expected.line, `line mismatch at ${line}:${col}`);
              assert.equal(actual.column, expected.column, `column mismatch at ${line}:${col}`);
              assert.equal(actual.name, expected.name, `name mismatch at ${line}:${col}`);
            }

            checked++;
          }
        }

        assert.ok(checked > 0, "should have checked at least one lookup");
        srcmap.free();
      });

      it("generatedPositionFor matches", () => {
        const srcmap = new SrcmapTraceMap(json);
        const jr = new JrTraceMap(json);

        // Get some source positions to test
        const positions = [];
        jrEachMapping(jr, (m) => {
          if (m.source && positions.length < 50) {
            positions.push({
              source: m.source,
              line: m.originalLine,
              column: m.originalColumn,
            });
          }
        });

        for (const pos of positions) {
          const expected = jrGenerated(jr, pos);
          const actual = srcmapGenerated(srcmap, pos);

          // Both should be found or both null
          if (expected.line !== null) {
            assert.equal(
              actual.line,
              expected.line,
              `generated line mismatch for ${pos.source}:${pos.line}:${pos.column}`,
            );
            assert.equal(
              actual.column,
              expected.column,
              `generated column mismatch for ${pos.source}:${pos.line}:${pos.column}`,
            );
          }
        }

        srcmap.free();
      });

      it("eachMapping produces same data", () => {
        const srcmap = new SrcmapTraceMap(json);
        const jr = new JrTraceMap(json);

        const srcmapMappings = [];
        const jrMappings = [];
        srcmapEachMapping(srcmap, (m) => srcmapMappings.push(m));
        jrEachMapping(jr, (m) => jrMappings.push(m));

        assert.equal(srcmapMappings.length, jrMappings.length);

        for (let i = 0; i < jrMappings.length; i++) {
          const sm = srcmapMappings[i];
          const jm = jrMappings[i];

          assert.equal(sm.generatedLine, jm.generatedLine, `generatedLine[${i}]`);
          assert.equal(sm.generatedColumn, jm.generatedColumn, `generatedColumn[${i}]`);
          assert.equal(normalizePath(sm.source), normalizePath(jm.source), `source[${i}]`);
          assert.equal(sm.originalLine, jm.originalLine, `originalLine[${i}]`);
          assert.equal(sm.originalColumn, jm.originalColumn, `originalColumn[${i}]`);
          assert.equal(sm.name, jm.name, `name[${i}]`);
        }

        srcmap.free();
      });

      it("sourceContentFor matches", () => {
        const parsed = JSON.parse(json);
        if (!parsed.sourcesContent) return;

        const srcmap = new SrcmapTraceMap(json);
        const jr = new JrTraceMap(json);

        for (const source of parsed.sources || []) {
          if (!source) continue;
          const expected = jrSourceContentFor(jr, source);
          const actual = srcmapSourceContentFor(srcmap, source);
          assert.equal(actual, expected, `sourceContentFor(${source})`);
        }

        srcmap.free();
      });

      it("isIgnored matches", () => {
        const parsed = JSON.parse(json);
        const srcmap = new SrcmapTraceMap(json);
        const jr = new JrTraceMap(json);

        for (const source of parsed.sources || []) {
          if (!source) continue;
          const expected = jrIsIgnored(jr, source);
          const actual = srcmapIsIgnored(srcmap, source);
          assert.equal(actual, expected, `isIgnored(${source})`);
        }

        srcmap.free();
      });
    });
  }
});

describe("regression: duplicate-column segments", () => {
  // Two segments at the same generated column on the same line, where the
  // second is a single-value segment (no source). `@jridgewell/trace-mapping`
  // GLB = earliest-equal (returns the source-bearing segment); LUB = latest-equal
  // (returns the no-source segment, reported as OMapping(null, …)).
  //
  // Rust's `binary_search_by_key` returns an unspecified index among duplicates,
  // which previously made srcmap pick the NO_SOURCE segment for GLB/default
  // lookups and break drop-in parity. See fallow-cloud test fixture VALID_SOURCE_MAP.
  const DUP_COL_MAP = JSON.stringify({
    version: 3,
    file: "bundle.js",
    sources: ["src/original.ts"],
    names: ["originalFn", "helperFn"],
    mappings: "AASA,A;AAIA,C;AAIA",
    sourcesContent: ["const a = 1;\nconst b = 2;"],
  });

  it("originalPositionFor default (GLB) at (1, 0) returns source-bearing segment", () => {
    const srcmap = new SrcmapTraceMap(DUP_COL_MAP);
    const jr = new JrTraceMap(DUP_COL_MAP);

    const expected = jrOriginal(jr, { line: 1, column: 0 });
    const actual = srcmapOriginal(srcmap, { line: 1, column: 0 });

    assert.equal(normalizePath(actual.source), normalizePath(expected.source));
    assert.equal(actual.line, expected.line);
    assert.equal(actual.column, expected.column);
    assert.equal(actual.name, expected.name);
    assert.equal(actual.source, "src/original.ts");
    assert.equal(actual.line, 10);

    srcmap.free();
  });
});

describe("decoded/encoded mappings cross-validation", () => {
  it("decodedMappings structure matches", () => {
    const srcmap = new SrcmapTraceMap(SIMPLE_MAP);
    const jr = new JrTraceMap(SIMPLE_MAP);

    const srcmapDecoded = srcmapDecodedMappings(srcmap);
    const jrDecoded = jrDecodedMappings(jr);

    assert.equal(srcmapDecoded.length, jrDecoded.length, "line count");

    for (let i = 0; i < jrDecoded.length; i++) {
      assert.equal(srcmapDecoded[i].length, jrDecoded[i].length, `segment count on line ${i}`);

      for (let j = 0; j < jrDecoded[i].length; j++) {
        const srcmapSeg = srcmapDecoded[i][j];
        const jrSeg = jrDecoded[i][j];

        assert.equal(srcmapSeg.length, jrSeg.length, `segment length at [${i}][${j}]`);
        for (let k = 0; k < jrSeg.length; k++) {
          assert.equal(srcmapSeg[k], jrSeg[k], `segment[${i}][${j}][${k}]`);
        }
      }
    }

    srcmap.free();
  });

  it("encodedMappings matches", () => {
    const srcmap = new SrcmapTraceMap(SIMPLE_MAP);
    const jr = new JrTraceMap(SIMPLE_MAP);

    const srcmapEncoded = srcmapEncodedMappings(srcmap);
    const jrEncoded = jrEncodedMappings(jr);

    assert.equal(srcmapEncoded, jrEncoded);
    srcmap.free();
  });
});

// Source resolution must follow @jridgewell/resolve-uri: it collapses "//", keeps a
// leading "./", and does not go above the first directory of a relative path.
const RESOLVE_SOURCE_ROOTS = [
  undefined,
  "",
  ".",
  "./",
  "..",
  "a",
  "a/b/",
  "a//b",
  "/abs",
  "http://x.com/r/",
  "//cdn/r",
  "webpack://",
  "webpack:///src",
  "C:\\w",
  "file:///C:/w/",
];
const RESOLVE_SOURCES = [
  "x.js",
  "./a.js",
  "../../c.js",
  "a//b.js",
  "a/./b/../c.js",
  "/r.js",
  "http://h/p/../q.js",
  "//s/t.js",
  "data:text/plain,hi",
  "webpack:///./s.js",
  "C:\\a\\b.js",
  ".",
  "..",
  "",
  null,
];
const RESOLVE_MAP_URLS = [
  undefined,
  null,
  "",
  "out.js.map",
  "dist/out.js.map",
  "../m.map",
  "/dist/x.map",
  "http://e.com/d/x.map",
  "//cdn/d/x.map",
  "file:///C:/d/x.map",
  "data:application/json,{}",
];

const resolveCaseMap = (sourceRoot) => {
  const map = {
    version: 3,
    names: [],
    sources: RESOLVE_SOURCES,
    sourcesContent: RESOLVE_SOURCES.map((_, i) => `content ${i}`),
    ignoreList: [1, 3],
    mappings: "AAAA" + ",CCAA".repeat(RESOLVE_SOURCES.length - 1),
  };
  if (sourceRoot !== undefined) map.sourceRoot = sourceRoot;
  return map;
};

describe("source resolution cross-validation", () => {
  for (const sourceRoot of RESOLVE_SOURCE_ROOTS) {
    for (const mapUrl of RESOLVE_MAP_URLS) {
      it(`sourceRoot ${JSON.stringify(sourceRoot)}, mapUrl ${JSON.stringify(mapUrl)}`, () => {
        const map = resolveCaseMap(sourceRoot);
        const srcmap = new SrcmapTraceMap(map, mapUrl);
        const jr = new JrTraceMap(map, mapUrl);

        assert.deepEqual(srcmap.resolvedSources, jr.resolvedSources, "resolvedSources");

        for (let column = 0; column < RESOLVE_SOURCES.length; column++) {
          for (const bias of [undefined, -1]) {
            const needle = { line: 1, column, bias };
            assert.deepEqual(
              srcmapOriginal(srcmap, needle),
              jrOriginal(jr, needle),
              `column ${column}`,
            );
          }
        }

        const srcmapEach = [];
        const jrEach = [];
        srcmapEachMapping(srcmap, (m) => srcmapEach.push(m));
        jrEachMapping(jr, (m) => jrEach.push(m));
        assert.deepEqual(srcmapEach, jrEach, "eachMapping");

        const names = new Set([
          ...RESOLVE_SOURCES.filter((source) => source !== null),
          ...jr.resolvedSources,
          ...srcmap.resolvedSources,
        ]);
        for (const source of names) {
          const needle = { source, line: 1, column: 0 };
          const label = JSON.stringify(source);
          assert.deepEqual(
            srcmapGenerated(srcmap, needle),
            jrGenerated(jr, needle),
            `generated ${label}`,
          );
          assert.deepEqual(
            srcmapAllGenerated(srcmap, needle),
            jrAllGenerated(jr, needle),
            `all ${label}`,
          );
          assert.equal(
            srcmapSourceContentFor(srcmap, source),
            jrSourceContentFor(jr, source),
            `content ${label}`,
          );
          assert.equal(
            srcmapIsIgnored(srcmap, source),
            jrIsIgnored(jr, source),
            `ignored ${label}`,
          );
        }

        srcmap.free();
      });
    }
  }
});

describe("lookup by source name cross-validation", () => {
  // Without a `sourceRoot`, a null source and a "" source both resolve to "".
  // jridgewell finds "" at the index of the "" source, not of the null source.
  for (const sourceRoot of [undefined, "", "root"]) {
    it(`null source before "" source, sourceRoot ${JSON.stringify(sourceRoot)}`, () => {
      const map = {
        version: 3,
        names: [],
        sources: [null, "", "a.js", null],
        sourcesContent: ["null", "empty", "a", "null again"],
        ignoreList: [1],
        mappings: "AAAA,CCAA,CCAA,CCAA",
      };
      if (sourceRoot !== undefined) map.sourceRoot = sourceRoot;
      for (const input of [map, JSON.stringify(map)]) {
        const srcmap = new SrcmapTraceMap(input);
        const jr = new JrTraceMap(input);
        assert.deepEqual(srcmap.resolvedSources, jr.resolvedSources);
        for (const source of ["", "a.js", "root/", "root/a.js"]) {
          for (const line of [1, 3]) {
            const needle = { source, line, column: 0 };
            assert.deepEqual(
              srcmapGenerated(srcmap, needle),
              jrGenerated(jr, needle),
              `${source}:${line}`,
            );
            assert.deepEqual(
              srcmapAllGenerated(srcmap, needle),
              jrAllGenerated(jr, needle),
              `${source}:${line}`,
            );
          }
          assert.equal(srcmapSourceContentFor(srcmap, source), jrSourceContentFor(jr, source));
          assert.equal(srcmapIsIgnored(srcmap, source), jrIsIgnored(jr, source));
        }
        for (let column = 0; column < 4; column++) {
          const needle = { line: 1, column };
          assert.deepEqual(
            srcmapOriginal(srcmap, needle),
            jrOriginal(jr, needle),
            `column ${column}`,
          );
        }
        srcmap.free();
      }
    });
  }
});
