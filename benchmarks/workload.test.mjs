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

test("rejects invalid count values", () => {
  for (const count of [-1, 1.5, Number.NaN]) {
    assert.throws(() => createDeterministicLookups(count, 80, 200, 12345), {
      name: "RangeError",
      message: "count must be a non-negative integer",
    });
  }
});

test("rejects invalid maxLine values", () => {
  for (const maxLine of [-1, 0, 1.5, Number.NaN]) {
    assert.throws(() => createDeterministicLookups(100, maxLine, 200, 12345), {
      name: "RangeError",
      message: "maxLine must be a positive integer",
    });
  }
});

test("rejects invalid maxColumn values", () => {
  for (const maxColumn of [-1, 0, 1.5, Number.NaN]) {
    assert.throws(() => createDeterministicLookups(100, 80, maxColumn, 12345), {
      name: "RangeError",
      message: "maxColumn must be a positive integer",
    });
  }
});

test("accepts zero count", () => {
  assert.deepEqual(createDeterministicLookups(0, 80, 200, 12345), []);
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
