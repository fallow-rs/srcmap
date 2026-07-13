import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { it } from "node:test";
import init, { LazySourceMap, SourceMap, resultPtr, wasmMemory } from "../browser/index.mjs";

it("exposes the browser API as static ESM exports before initialization", () => {
  assert.equal(typeof init, "function");
  assert.equal(typeof SourceMap, "function");
  assert.equal(typeof LazySourceMap, "function");
  assert.equal(typeof resultPtr, "function");
  assert.equal(typeof wasmMemory, "function");
});

it("publishes declarations for the browser API", () => {
  const packageJson = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));

  assert.equal(packageJson.exports["./browser"].types, "./browser/index.d.ts");

  const declarations = readFileSync(new URL("../browser/index.d.ts", import.meta.url), "utf8");
  assert.match(declarations, /export \{ LazySourceMap, SourceMap, resultPtr, wasmMemory \}/);
  assert.match(declarations, /Promise<void>/);
});

it("initializes the generated module only once", async () => {
  const wasm = readFileSync(new URL("../web/srcmap_sourcemap_wasm_bg.wasm", import.meta.url));

  const input = { module_or_path: wasm };
  const first = init(input);
  const second = init(input);

  assert.equal(second, first);
  await first;
});
