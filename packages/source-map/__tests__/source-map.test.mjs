import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  SourceMapConsumer,
  SourceMapGenerator,
  GREATEST_LOWER_BOUND,
  LEAST_UPPER_BOUND,
} from "../src/source-map.mjs";

// ── Test fixtures ────────────────────────────────────────────────

const SIMPLE_MAP = {
  version: 3,
  file: "output.js",
  sourceRoot: "",
  sources: ["input.js"],
  sourcesContent: ["const foo = 1;\nconst bar = 2;"],
  names: ["foo", "bar"],
  mappings: "AAAAA,SACIC",
};

const MULTI_SOURCE_MAP = {
  version: 3,
  sources: ["a.js", "b.js"],
  sourcesContent: ["// a.js\nconst x = 1;", "// b.js\nconst y = 2;"],
  names: ["x", "y", "z"],
  mappings: "AAAAA;ACAAC,KACCC",
};

// ── Constants ────────────────────────────────────────────────────

describe("constants", () => {
  it("exports GREATEST_LOWER_BOUND = 1 (Mozilla v0.6 convention)", () => {
    assert.equal(GREATEST_LOWER_BOUND, 1);
  });

  it("exports LEAST_UPPER_BOUND = 2 (Mozilla v0.6 convention)", () => {
    assert.equal(LEAST_UPPER_BOUND, 2);
  });
});

// ── SourceMapConsumer constructor ─────────────────────────────────

describe("SourceMapConsumer constructor", () => {
  it("accepts a plain object", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    assert.ok(consumer);
    assert.equal(consumer.file, "output.js");
    consumer.destroy();
  });

  it("accepts a JSON string", () => {
    const consumer = new SourceMapConsumer(JSON.stringify(SIMPLE_MAP));
    assert.ok(consumer);
    assert.equal(consumer.file, "output.js");
    consumer.destroy();
  });

  it("exposes sources array", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    assert.ok(Array.isArray(consumer.sources));
    assert.equal(consumer.sources.length, 1);
    assert.equal(consumer.sources[0], "input.js");
    consumer.destroy();
  });

  it("exposes sourcesContent", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    assert.ok(Array.isArray(consumer.sourcesContent));
    assert.equal(consumer.sourcesContent[0], "const foo = 1;\nconst bar = 2;");
    consumer.destroy();
  });

  it("exposes file property", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    assert.equal(consumer.file, "output.js");
    consumer.destroy();
  });

  it("exposes sourceRoot property", () => {
    const consumer = new SourceMapConsumer({
      version: 3,
      sourceRoot: "src/",
      sources: ["a.js"],
      names: [],
      mappings: "AAAA",
    });
    assert.equal(consumer.sourceRoot, "src/");
    consumer.destroy();
  });
});

// ── originalPositionFor ──────────────────────────────────────────

describe("SourceMapConsumer.originalPositionFor", () => {
  it("maps generated position to original (1-based lines)", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.originalPositionFor({ line: 1, column: 0 });
    assert.equal(pos.source, "input.js");
    assert.equal(pos.line, 1);
    assert.equal(pos.column, 0);
    assert.equal(pos.name, "foo");
    consumer.destroy();
  });

  it("returns nulls for unmapped position", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.originalPositionFor({ line: 999, column: 0 });
    assert.equal(pos.source, null);
    assert.equal(pos.line, null);
    assert.equal(pos.column, null);
    assert.equal(pos.name, null);
    consumer.destroy();
  });

  it("throws for line < 1", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    assert.throws(() => consumer.originalPositionFor({ line: 0, column: 0 }));
    consumer.destroy();
  });

  it("throws for column < 0", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    assert.throws(() => consumer.originalPositionFor({ line: 1, column: -1 }));
    consumer.destroy();
  });

  it("uses GREATEST_LOWER_BOUND by default", () => {
    const gapMap = {
      version: 3,
      sources: ["x.js"],
      names: [],
      mappings: "AAAA,UAAS",
    };
    const consumer = new SourceMapConsumer(gapMap);
    const pos = consumer.originalPositionFor({ line: 1, column: 5 });
    assert.equal(pos.source, "x.js");
    assert.equal(pos.column, 0); // snapped to segment at col 0
    consumer.destroy();
  });

  it("supports LEAST_UPPER_BOUND bias", () => {
    const gapMap = {
      version: 3,
      sources: ["x.js"],
      names: [],
      mappings: "AAAA,UAAS",
    };
    const consumer = new SourceMapConsumer(gapMap);
    const pos = consumer.originalPositionFor({ line: 1, column: 5, bias: LEAST_UPPER_BOUND });
    assert.equal(pos.source, "x.js");
    assert.ok(pos.line != null);
    consumer.destroy();
  });
});

// ── generatedPositionFor ─────────────────────────────────────────

describe("SourceMapConsumer.generatedPositionFor", () => {
  it("reverse-looks up a position (1-based lines)", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.generatedPositionFor({ source: "input.js", line: 1, column: 0 });
    assert.equal(pos.line, 1);
    assert.equal(pos.column, 0);
    consumer.destroy();
  });

  it("returns nulls for unknown source", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.generatedPositionFor({ source: "nonexistent.js", line: 1, column: 0 });
    assert.equal(pos.line, null);
    assert.equal(pos.column, null);
    consumer.destroy();
  });

  it("includes lastColumn (null) in result", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.generatedPositionFor({ source: "input.js", line: 1, column: 0 });
    assert.ok("lastColumn" in pos);
    consumer.destroy();
  });
});

// ── eachMapping ──────────────────────────────────────────────────

describe("SourceMapConsumer.eachMapping", () => {
  it("iterates all mappings", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const mappings = [];
    consumer.eachMapping((m) => mappings.push(m));
    assert.ok(mappings.length >= 2);
    consumer.destroy();
  });

  it("provides 1-based lines", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const mappings = [];
    consumer.eachMapping((m) => mappings.push(m));
    assert.ok(mappings[0].generatedLine >= 1);
    if (mappings[0].originalLine != null) {
      assert.ok(mappings[0].originalLine >= 1);
    }
    consumer.destroy();
  });

  it("includes source, name, lastGeneratedColumn fields", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const mappings = [];
    consumer.eachMapping((m) => mappings.push(m));

    const first = mappings[0];
    assert.ok("source" in first);
    assert.ok("name" in first);
    assert.ok("lastGeneratedColumn" in first);
    assert.equal(first.source, "input.js");
    assert.equal(first.name, "foo");
    consumer.destroy();
  });

  it("supports context binding", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const ctx = { mappings: [] };
    consumer.eachMapping(function (m) {
      this.mappings.push(m);
    }, ctx);
    assert.ok(ctx.mappings.length >= 2);
    consumer.destroy();
  });
});

// ── sourceContentFor ─────────────────────────────────────────────

describe("SourceMapConsumer.sourceContentFor", () => {
  it("returns source content by source name", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const content = consumer.sourceContentFor("input.js");
    assert.equal(content, "const foo = 1;\nconst bar = 2;");
    consumer.destroy();
  });

  it("returns null for unknown source", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const content = consumer.sourceContentFor("nonexistent.js");
    assert.equal(content, null);
    consumer.destroy();
  });

  it("returns null when sourcesContent is not present", () => {
    const consumer = new SourceMapConsumer({
      version: 3,
      sources: ["a.js"],
      names: [],
      mappings: "AAAA",
    });
    const content = consumer.sourceContentFor("a.js");
    assert.equal(content, null);
    consumer.destroy();
  });
});

// ── destroy ──────────────────────────────────────────────────────

describe("SourceMapConsumer.destroy", () => {
  it("can be called without error", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    consumer.destroy();
    // double-destroy should be safe
    consumer.destroy();
  });
});

// ── SourceMapGenerator ──────────────────────────────────────────

describe("SourceMapGenerator constructor", () => {
  it("creates a generator with file and sourceRoot", () => {
    const gen = new SourceMapGenerator({ file: "output.js", sourceRoot: "src/" });
    assert.ok(gen);
    gen.destroy();
  });

  it("creates a generator without options", () => {
    const gen = new SourceMapGenerator();
    assert.ok(gen);
    gen.destroy();
  });
});

describe("SourceMapGenerator.addMapping", () => {
  it("adds a mapping with source, original, and name", () => {
    const gen = new SourceMapGenerator({ file: "output.js" });
    gen.addMapping({
      generated: { line: 1, column: 0 },
      original: { line: 1, column: 0 },
      source: "input.js",
      name: "foo",
    });
    const map = gen.toJSON();
    assert.equal(map.version, 3);
    assert.equal(map.file, "output.js");
    assert.deepEqual(map.sources, ["input.js"]);
    assert.deepEqual(map.names, ["foo"]);
    assert.ok(map.mappings.length > 0);
    gen.destroy();
  });

  it("adds a mapping without name", () => {
    const gen = new SourceMapGenerator({ file: "output.js" });
    gen.addMapping({
      generated: { line: 1, column: 0 },
      original: { line: 1, column: 0 },
      source: "input.js",
    });
    const map = gen.toJSON();
    assert.deepEqual(map.sources, ["input.js"]);
    assert.deepEqual(map.names, []);
    gen.destroy();
  });

  it("adds a generated-only mapping (no source/original)", () => {
    const gen = new SourceMapGenerator();
    gen.addMapping({
      generated: { line: 1, column: 0 },
    });
    const map = gen.toJSON();
    assert.ok(map.mappings.length > 0);
    gen.destroy();
  });
});

describe("SourceMapGenerator.setSourceContent", () => {
  it("embeds source content", () => {
    const gen = new SourceMapGenerator();
    gen.addMapping({
      generated: { line: 1, column: 0 },
      original: { line: 1, column: 0 },
      source: "input.js",
    });
    gen.setSourceContent("input.js", "const x = 1;");
    const map = gen.toJSON();
    assert.deepEqual(map.sourcesContent, ["const x = 1;"]);
    gen.destroy();
  });
});

describe("SourceMapGenerator.toJSON / toString", () => {
  it("toJSON returns an object", () => {
    const gen = new SourceMapGenerator({ file: "out.js" });
    gen.addMapping({
      generated: { line: 1, column: 0 },
      original: { line: 1, column: 0 },
      source: "in.js",
    });
    const map = gen.toJSON();
    assert.equal(typeof map, "object");
    assert.equal(map.version, 3);
    gen.destroy();
  });

  it("toString returns a JSON string", () => {
    const gen = new SourceMapGenerator({ file: "out.js" });
    gen.addMapping({
      generated: { line: 1, column: 0 },
      original: { line: 1, column: 0 },
      source: "in.js",
    });
    const str = gen.toString();
    assert.equal(typeof str, "string");
    const parsed = JSON.parse(str);
    assert.equal(parsed.version, 3);
    gen.destroy();
  });
});

// ── Round-trip: Generator → Consumer ─────────────────────────────

describe("round-trip: SourceMapGenerator → SourceMapConsumer", () => {
  it("generates a map that the consumer can read", () => {
    const gen = new SourceMapGenerator({ file: "output.js" });
    gen.addMapping({
      generated: { line: 1, column: 0 },
      original: { line: 1, column: 0 },
      source: "input.js",
      name: "hello",
    });
    gen.addMapping({
      generated: { line: 2, column: 4 },
      original: { line: 3, column: 2 },
      source: "input.js",
    });
    gen.setSourceContent("input.js", "function hello() {\n  return 1;\n  return 2;\n}");

    const map = gen.toJSON();
    const consumer = new SourceMapConsumer(map);

    // Check first mapping
    const pos1 = consumer.originalPositionFor({ line: 1, column: 0 });
    assert.equal(pos1.source, "input.js");
    assert.equal(pos1.line, 1);
    assert.equal(pos1.column, 0);
    assert.equal(pos1.name, "hello");

    // Check second mapping
    const pos2 = consumer.originalPositionFor({ line: 2, column: 4 });
    assert.equal(pos2.source, "input.js");
    assert.equal(pos2.line, 3);
    assert.equal(pos2.column, 2);
    assert.equal(pos2.name, null);

    // Check reverse lookup
    const gen1 = consumer.generatedPositionFor({ source: "input.js", line: 1, column: 0 });
    assert.equal(gen1.line, 1);
    assert.equal(gen1.column, 0);

    // Check source content
    const content = consumer.sourceContentFor("input.js");
    assert.equal(content, "function hello() {\n  return 1;\n  return 2;\n}");

    // Check eachMapping
    const mappings = [];
    consumer.eachMapping((m) => mappings.push(m));
    assert.equal(mappings.length, 2);

    consumer.destroy();
    gen.destroy();
  });
});

// ── Multi-source consumer ────────────────────────────────────────

describe("multi-source consumer", () => {
  it("handles multiple sources", () => {
    const consumer = new SourceMapConsumer(MULTI_SOURCE_MAP);

    // First line maps to a.js
    const pos1 = consumer.originalPositionFor({ line: 1, column: 0 });
    assert.equal(pos1.source, "a.js");
    assert.equal(pos1.name, "x");

    // Second line maps to b.js
    const pos2 = consumer.originalPositionFor({ line: 2, column: 0 });
    assert.equal(pos2.source, "b.js");

    // Source content
    const contentA = consumer.sourceContentFor("a.js");
    assert.equal(contentA, "// a.js\nconst x = 1;");
    const contentB = consumer.sourceContentFor("b.js");
    assert.equal(contentB, "// b.js\nconst y = 2;");

    consumer.destroy();
  });
});

// ── API compatibility with source-map v0.6 ──────────────────────

describe("API compatibility with source-map v0.6", () => {
  it("exports all expected classes and constants", () => {
    assert.equal(typeof SourceMapConsumer, "function");
    assert.equal(typeof SourceMapGenerator, "function");
    assert.equal(typeof GREATEST_LOWER_BOUND, "number");
    assert.equal(typeof LEAST_UPPER_BOUND, "number");
  });

  it("SourceMapConsumer has expected methods", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    assert.equal(typeof consumer.originalPositionFor, "function");
    assert.equal(typeof consumer.generatedPositionFor, "function");
    assert.equal(typeof consumer.eachMapping, "function");
    assert.equal(typeof consumer.sourceContentFor, "function");
    assert.equal(typeof consumer.destroy, "function");
    consumer.destroy();
  });

  it("SourceMapGenerator has expected methods", () => {
    const gen = new SourceMapGenerator();
    assert.equal(typeof gen.addMapping, "function");
    assert.equal(typeof gen.setSourceContent, "function");
    assert.equal(typeof gen.toJSON, "function");
    assert.equal(typeof gen.toString, "function");
    assert.equal(typeof gen.applySourceMap, "function");
    gen.destroy();
  });

  it("originalPositionFor result has all expected fields", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.originalPositionFor({ line: 1, column: 0 });
    assert.ok("source" in pos);
    assert.ok("line" in pos);
    assert.ok("column" in pos);
    assert.ok("name" in pos);
    consumer.destroy();
  });

  it("generatedPositionFor result has all expected fields", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.generatedPositionFor({ source: "input.js", line: 1, column: 0 });
    assert.ok("line" in pos);
    assert.ok("column" in pos);
    assert.ok("lastColumn" in pos);
    consumer.destroy();
  });

  it("eachMapping result has all expected fields", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    consumer.eachMapping((m) => {
      assert.ok("generatedLine" in m);
      assert.ok("generatedColumn" in m);
      assert.ok("source" in m);
      assert.ok("originalLine" in m);
      assert.ok("originalColumn" in m);
      assert.ok("name" in m);
      assert.ok("lastGeneratedColumn" in m);
    });
    consumer.destroy();
  });

  it("null result has all null fields (not undefined)", () => {
    const consumer = new SourceMapConsumer(SIMPLE_MAP);
    const pos = consumer.originalPositionFor({ line: 999, column: 0 });
    assert.equal(pos.source, null);
    assert.equal(pos.line, null);
    assert.equal(pos.column, null);
    assert.equal(pos.name, null);
    consumer.destroy();
  });
});

// ── Edge cases ───────────────────────────────────────────────────

describe("edge cases", () => {
  it("handles empty mappings", () => {
    const consumer = new SourceMapConsumer({
      version: 3,
      sources: [],
      names: [],
      mappings: "",
    });
    const pos = consumer.originalPositionFor({ line: 1, column: 0 });
    assert.equal(pos.source, null);
    consumer.destroy();
  });

  it("handles source map with only semicolons", () => {
    const consumer = new SourceMapConsumer({
      version: 3,
      sources: ["a.js"],
      names: [],
      mappings: ";;;",
    });
    const pos = consumer.originalPositionFor({ line: 1, column: 0 });
    assert.equal(pos.source, null);
    consumer.destroy();
  });

  it("generator with sourceRoot includes it in output", () => {
    const gen = new SourceMapGenerator({ file: "out.js", sourceRoot: "src/" });
    gen.addMapping({
      generated: { line: 1, column: 0 },
      original: { line: 1, column: 0 },
      source: "input.js",
    });
    const map = gen.toJSON();
    assert.equal(map.sourceRoot, "src/");
    gen.destroy();
  });
});
