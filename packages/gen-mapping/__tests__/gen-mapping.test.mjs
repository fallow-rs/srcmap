import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  GenMapping,
  addMapping,
  maybeAddMapping,
  setSourceContent,
  setIgnore,
  allMappings,
  toEncodedMap,
  toDecodedMap,
  fromMap,
} from "../src/gen-mapping.mjs";

// ── GenMapping constructor ──────────────────────────────────────

describe("GenMapping constructor", () => {
  it("creates with default options", () => {
    const map = new GenMapping();
    assert.equal(map.file, undefined);
    assert.equal(map.sourceRoot, undefined);
    map.free();
  });

  it("creates with file option", () => {
    const map = new GenMapping({ file: "output.js" });
    assert.equal(map.file, "output.js");
    map.free();
  });

  it("creates with file and sourceRoot", () => {
    const map = new GenMapping({ file: "output.js", sourceRoot: "src/" });
    assert.equal(map.file, "output.js");
    assert.equal(map.sourceRoot, "src/");
    map.free();
  });
});

// ── addMapping ──────────────────────────────────────────────────

describe("addMapping", () => {
  it("adds a generated-only mapping", () => {
    const map = new GenMapping({ file: "output.js" });
    addMapping(map, { generated: { line: 1, column: 0 } });
    const encoded = toEncodedMap(map);
    assert.equal(encoded.version, 3);
    assert.ok(encoded.mappings.length > 0);
    map.free();
  });

  it("adds a mapping with source", () => {
    const map = new GenMapping({ file: "output.js" });
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
    });
    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.sources, ["input.js"]);
    map.free();
  });

  it("adds a mapping with name", () => {
    const map = new GenMapping({ file: "output.js" });
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "myFunc",
    });
    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.names, ["myFunc"]);
    map.free();
  });

  it("adds mappings across multiple lines", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
    });
    addMapping(map, {
      generated: { line: 2, column: 4 },
      source: "input.js",
      original: { line: 2, column: 2 },
    });
    const encoded = toEncodedMap(map);
    assert.ok(encoded.mappings.includes(";"));
    map.free();
  });

  it("adds a mapping with inline content", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      content: "const x = 1;",
    });
    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.sourcesContent, ["const x = 1;"]);
    map.free();
  });
});

// ── maybeAddMapping ─────────────────────────────────────────────

describe("maybeAddMapping", () => {
  it("adds the first mapping", () => {
    const map = new GenMapping();
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 10, column: 0 },
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 1);
    map.free();
  });

  it("skips redundant mapping with same source position", () => {
    const map = new GenMapping();
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 10, column: 0 },
    });
    maybeAddMapping(map, {
      generated: { line: 1, column: 5 },
      source: "input.js",
      original: { line: 10, column: 0 },
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 1);
    map.free();
  });

  it("adds mapping with different source position", () => {
    const map = new GenMapping();
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 10, column: 0 },
    });
    maybeAddMapping(map, {
      generated: { line: 1, column: 10 },
      source: "input.js",
      original: { line: 11, column: 0 },
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 2);
    map.free();
  });

  it("adds mapping on a new line even with same source position", () => {
    const map = new GenMapping();
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 10, column: 0 },
    });
    maybeAddMapping(map, {
      generated: { line: 2, column: 0 },
      source: "input.js",
      original: { line: 10, column: 0 },
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 2);
    map.free();
  });

  it("skips redundant mapping with name", () => {
    const map = new GenMapping();
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "foo",
    });
    maybeAddMapping(map, {
      generated: { line: 1, column: 5 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "foo",
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 1);
    map.free();
  });

  it("adds mapping when name differs", () => {
    const map = new GenMapping();
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "foo",
    });
    maybeAddMapping(map, {
      generated: { line: 1, column: 5 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "bar",
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 2);
    map.free();
  });
});

// ── setSourceContent ────────────────────────────────────────────

describe("setSourceContent", () => {
  it("sets content by source name", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
    });
    setSourceContent(map, "input.js", "const x = 1;");
    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.sourcesContent, ["const x = 1;"]);
    map.free();
  });

  it("registers source if not already present", () => {
    const map = new GenMapping();
    setSourceContent(map, "other.js", "const y = 2;");
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "other.js",
      original: { line: 1, column: 0 },
    });
    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.sources, ["other.js"]);
    assert.deepEqual(encoded.sourcesContent, ["const y = 2;"]);
    map.free();
  });

  it("sets content for multiple sources", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "a.js",
      original: { line: 1, column: 0 },
    });
    addMapping(map, {
      generated: { line: 2, column: 0 },
      source: "b.js",
      original: { line: 1, column: 0 },
    });
    setSourceContent(map, "a.js", "// a");
    setSourceContent(map, "b.js", "// b");
    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.sourcesContent, ["// a", "// b"]);
    map.free();
  });
});

// ── setIgnore ───────────────────────────────────────────────────

describe("setIgnore", () => {
  it("adds source to ignore list", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "app.js",
      original: { line: 1, column: 0 },
    });
    addMapping(map, {
      generated: { line: 2, column: 0 },
      source: "lib.js",
      original: { line: 1, column: 0 },
    });
    setIgnore(map, "lib.js");
    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.ignoreList, [1]);
    map.free();
  });
});

// ── allMappings ─────────────────────────────────────────────────

describe("allMappings", () => {
  it("returns empty array for no mappings", () => {
    const map = new GenMapping();
    const mappings = allMappings(map);
    assert.deepEqual(mappings, []);
    map.free();
  });

  it("returns mappings with 1-based lines", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 5, column: 10 },
    });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 1);
    assert.deepEqual(mappings[0].generated, { line: 1, column: 0 });
    assert.equal(mappings[0].source, "input.js");
    assert.deepEqual(mappings[0].original, { line: 5, column: 10 });
    map.free();
  });

  it("returns generated-only mappings without source/original", () => {
    const map = new GenMapping();
    addMapping(map, { generated: { line: 1, column: 0 } });
    const mappings = allMappings(map);
    assert.equal(mappings.length, 1);
    assert.deepEqual(mappings[0].generated, { line: 1, column: 0 });
    assert.equal(mappings[0].source, undefined);
    assert.equal(mappings[0].original, undefined);
    map.free();
  });

  it("returns mappings with names", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "foo",
    });
    const mappings = allMappings(map);
    assert.equal(mappings[0].name, "foo");
    map.free();
  });
});

// ── toEncodedMap ────────────────────────────────────────────────

describe("toEncodedMap", () => {
  it("returns a valid encoded source map object", () => {
    const map = new GenMapping({ file: "output.js" });
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
    });
    const encoded = toEncodedMap(map);
    assert.equal(encoded.version, 3);
    assert.equal(encoded.file, "output.js");
    assert.equal(typeof encoded.mappings, "string");
    assert.ok(encoded.mappings.length > 0);
    assert.deepEqual(encoded.sources, ["input.js"]);
    assert.ok(Array.isArray(encoded.names));
    map.free();
  });

  it("includes sourceRoot when set", () => {
    const map = new GenMapping({ sourceRoot: "src/" });
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
    });
    const encoded = toEncodedMap(map);
    assert.equal(encoded.sourceRoot, "src/");
    map.free();
  });
});

// ── toDecodedMap ────────────────────────────────────────────────

describe("toDecodedMap", () => {
  it("returns a valid decoded source map object", () => {
    const map = new GenMapping({ file: "output.js" });
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
    });
    const decoded = toDecodedMap(map);
    assert.equal(decoded.version, 3);
    assert.equal(decoded.file, "output.js");
    assert.ok(Array.isArray(decoded.mappings));
    assert.deepEqual(decoded.sources, ["input.js"]);
    map.free();
  });

  it("decoded mappings contain correct segments", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 3, column: 5 },
    });
    const decoded = toDecodedMap(map);
    assert.equal(decoded.mappings.length, 1);
    assert.equal(decoded.mappings[0].length, 1);
    // segment: [genCol, sourceIdx, origLine(0-based), origCol]
    assert.deepEqual(decoded.mappings[0][0], [0, 0, 2, 5]);
    map.free();
  });

  it("decoded mappings with name contain 5-element segments", () => {
    const map = new GenMapping();
    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "x",
    });
    const decoded = toDecodedMap(map);
    assert.equal(decoded.mappings[0][0].length, 5);
    assert.equal(decoded.mappings[0][0][4], 0); // nameIdx
    map.free();
  });

  it("generated-only mappings produce 1-element segments", () => {
    const map = new GenMapping();
    addMapping(map, { generated: { line: 1, column: 5 } });
    const decoded = toDecodedMap(map);
    assert.deepEqual(decoded.mappings[0][0], [5]);
    map.free();
  });
});

// ── fromMap ─────────────────────────────────────────────────────

describe("fromMap", () => {
  it("round-trips an encoded source map", () => {
    const original = new GenMapping({ file: "output.js" });
    addMapping(original, {
      generated: { line: 1, column: 0 },
      source: "input.js",
      original: { line: 1, column: 0 },
      name: "foo",
    });
    addMapping(original, {
      generated: { line: 2, column: 4 },
      source: "input.js",
      original: { line: 2, column: 2 },
    });
    setSourceContent(original, "input.js", "const foo = 1;\nconst bar = 2;");

    const encoded = toEncodedMap(original);
    const restored = fromMap(encoded);
    const restoredEncoded = toEncodedMap(restored);

    assert.equal(restoredEncoded.file, "output.js");
    assert.deepEqual(restoredEncoded.sources, ["input.js"]);
    assert.deepEqual(restoredEncoded.names, ["foo"]);
    assert.deepEqual(restoredEncoded.sourcesContent, ["const foo = 1;\nconst bar = 2;"]);
    assert.equal(restoredEncoded.mappings, encoded.mappings);

    original.free();
    restored.free();
  });

  it("round-trips a JSON string", () => {
    const json = JSON.stringify({
      version: 3,
      file: "out.js",
      sources: ["a.js"],
      names: [],
      mappings: "AAAA",
      sourcesContent: ["const a = 1;"],
    });

    const gen = fromMap(json);
    const encoded = toEncodedMap(gen);

    assert.equal(encoded.file, "out.js");
    assert.deepEqual(encoded.sources, ["a.js"]);
    assert.equal(encoded.mappings, "AAAA");
    assert.deepEqual(encoded.sourcesContent, ["const a = 1;"]);

    gen.free();
  });
});

// ── Compatibility with Babel usage patterns ─────────────────────

describe("Babel integration patterns", () => {
  it("handles typical Babel gen-mapping workflow", () => {
    // Simulate babel-generator's SourceMap class usage pattern
    const map = new GenMapping({ sourceRoot: "" });

    // Babel registers sources and content early
    setSourceContent(map, "src/index.ts", "const x: number = 1;\nexport default x;");

    // Babel calls maybeAddMapping for each AST node
    maybeAddMapping(map, {
      generated: { line: 1, column: 0 },
      source: "src/index.ts",
      original: { line: 1, column: 0 },
      name: "x",
    });
    maybeAddMapping(map, {
      generated: { line: 1, column: 6 },
      source: "src/index.ts",
      original: { line: 1, column: 0 },
      name: "x",
    });
    // Same source position → should be deduped
    maybeAddMapping(map, {
      generated: { line: 1, column: 10 },
      source: "src/index.ts",
      original: { line: 1, column: 0 },
      name: "x",
    });
    // Different line → should not be deduped
    maybeAddMapping(map, {
      generated: { line: 2, column: 0 },
      source: "src/index.ts",
      original: { line: 2, column: 0 },
    });

    // Babel reads the result via toEncodedMap
    const encoded = toEncodedMap(map);
    assert.equal(encoded.version, 3);
    assert.deepEqual(encoded.sources, ["src/index.ts"]);
    assert.deepEqual(encoded.sourcesContent, ["const x: number = 1;\nexport default x;"]);
    assert.equal(typeof encoded.mappings, "string");

    // Also check decoded output
    const decoded = toDecodedMap(map);
    assert.ok(Array.isArray(decoded.mappings));

    // Also check allMappings
    const all = allMappings(map);
    // First mapping on line 1 added, second and third on line 1 deduped (same source pos + name),
    // fourth on line 2 added = 2 total
    assert.equal(all.length, 2);

    map.free();
  });

  it("handles multiple source files", () => {
    const map = new GenMapping({ file: "bundle.js" });

    setSourceContent(map, "a.js", "// a");
    setSourceContent(map, "b.js", "// b");

    addMapping(map, {
      generated: { line: 1, column: 0 },
      source: "a.js",
      original: { line: 1, column: 0 },
    });
    addMapping(map, {
      generated: { line: 2, column: 0 },
      source: "b.js",
      original: { line: 1, column: 0 },
    });

    const encoded = toEncodedMap(map);
    assert.deepEqual(encoded.sources, ["a.js", "b.js"]);
    assert.deepEqual(encoded.sourcesContent, ["// a", "// b"]);
    assert.equal(encoded.file, "bundle.js");

    map.free();
  });
});

// ── Large roundtrip ─────────────────────────────────────────────

describe("large roundtrip", () => {
  it("handles 1000 mappings", () => {
    const map = new GenMapping({ file: "bundle.js" });

    for (let i = 0; i < 5; i++) {
      setSourceContent(map, `src/file${i}.js`, `// file ${i}`);
    }

    for (let line = 1; line <= 100; line++) {
      for (let col = 0; col < 10; col++) {
        const src = `src/file${(line * 10 + col) % 5}.js`;
        if (col % 3 === 0) {
          addMapping(map, {
            generated: { line, column: col * 10 },
            source: src,
            original: { line, column: col * 5 },
            name: `var${col % 10}`,
          });
        } else {
          addMapping(map, {
            generated: { line, column: col * 10 },
            source: src,
            original: { line, column: col * 5 },
          });
        }
      }
    }

    const encoded = toEncodedMap(map);
    assert.equal(encoded.version, 3);
    assert.equal(encoded.sources.length, 5);
    assert.equal(typeof encoded.mappings, "string");

    const all = allMappings(map);
    assert.equal(all.length, 1000);

    map.free();
  });
});
