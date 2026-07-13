import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { createDeterministicLookups, setFailureExitCode } from "./workload.mjs";

afterEach(() => {
  process.exitCode = undefined;
});

test("identical seeds create identical lookups", () => {
  const first = createDeterministicLookups(100, 80, 200, 12345);
  const second = createDeterministicLookups(100, 80, 200, 12345);

  assert.deepEqual(first, second);
});

test("different seeds create different lookups", () => {
  const first = createDeterministicLookups(100, 80, 200, 12345);
  const second = createDeterministicLookups(100, 80, 200, 54321);

  assert.notDeepEqual(first, second);
});

test("lookups stay within the configured bounds", () => {
  const lookups = createDeterministicLookups(1_000, 80, 200, 12345);

  assert.equal(lookups.length, 1_000);
  for (const { line, column } of lookups) {
    assert.ok(line >= 0 && line < 80);
    assert.ok(column >= 0 && column < 200);
  }
});

test("a failed WASM correctness result sets the process exit code", () => {
  setFailureExitCode([
    { wasmPass: true, napiPass: true },
    { wasmPass: false, napiPass: true },
  ]);

  assert.equal(process.exitCode, 1);
});

test("a failed NAPI correctness result sets the process exit code", () => {
  setFailureExitCode([
    { wasmPass: true, napiPass: true },
    { wasmPass: true, napiPass: false },
  ]);

  assert.equal(process.exitCode, 1);
});

test("passing correctness results leave the process exit code unchanged", () => {
  setFailureExitCode([{ wasmPass: true, napiPass: true }]);

  assert.equal(process.exitCode, undefined);
});
