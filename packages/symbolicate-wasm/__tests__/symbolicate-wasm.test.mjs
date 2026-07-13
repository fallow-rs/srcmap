import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { parseStackTrace, symbolicate } from "../pkg/srcmap_symbolicate_wasm.js";

const SOURCE_MAP = JSON.stringify({
  version: 3,
  sources: ["src/app.ts"],
  names: ["originalName"],
  mappings: "AAAAA",
});

const parseResult = (stack, loader) => JSON.parse(symbolicate(stack, loader));

describe("parseStackTrace", () => {
  it("loads the generated Node wrapper and returns frame objects", () => {
    const frames = parseStackTrace(
      "Error: test\n    at handleClick (bundle.js:10:4)\n    at bundle.js:20:8",
    );

    assert.deepEqual(frames, [
      {
        functionName: "handleClick",
        file: "bundle.js",
        line: 10,
        column: 4,
      },
      {
        functionName: null,
        file: "bundle.js",
        line: 20,
        column: 8,
      },
    ]);
  });
});

describe("symbolicate", () => {
  it("resolves a frame and returns the documented output shape", () => {
    const result = parseResult("Error: test\n    at compiled (bundle.js:1:1)", (file) => {
      assert.equal(file, "bundle.js");
      return SOURCE_MAP;
    });

    assert.deepEqual(result, {
      message: "Error: test",
      frames: [
        {
          functionName: "originalName",
          file: "src/app.ts",
          line: 1,
          column: 1,
          symbolicated: true,
        },
      ],
    });
  });

  it("preserves frames when a map is missing", () => {
    const result = parseResult("Error: test\n    at compiled (missing.js:2:3)", () => null);

    assert.deepEqual(result.frames, [
      {
        functionName: "compiled",
        file: "missing.js",
        line: 2,
        column: 3,
        symbolicated: false,
      },
    ]);
  });

  it("treats malformed loader values as missing maps", () => {
    const stack = "Error: test\n    at compiled (bundle.js:1:1)";

    for (const value of [42, { version: 3 }, "not json"]) {
      const result = parseResult(stack, () => value);
      assert.equal(result.frames[0].symbolicated, false);
    }
  });

  it("loads each repeated source file once", () => {
    let calls = 0;
    const stack = "Error: test\n    at first (bundle.js:1:1)\n    at second (bundle.js:1:1)";

    const result = parseResult(stack, () => {
      calls += 1;
      return SOURCE_MAP;
    });

    assert.equal(calls, 1);
    assert.equal(result.frames.length, 2);
    assert.ok(result.frames.every((frame) => frame.symbolicated));
  });
});

it("exposes the generated web module without browser globals", async () => {
  const module = await import("../web/srcmap_symbolicate_wasm.js");

  assert.equal(typeof module.default, "function");
  assert.equal(typeof module.parseStackTrace, "function");
  assert.equal(typeof module.symbolicate, "function");
});
