import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { findUnclassifiedBindings, loadBindingPolicyInputs } from "./binding-manifest-policy.mjs";

const ROOT_URL = new URL("../../", import.meta.url);
const EXPERIMENTAL_BINDINGS = [
  "srcmap-generator-napi",
  "srcmap-remapping-napi",
  "srcmap-scopes-wasm",
];

describe("Rust binding manifest policy", () => {
  it("classifies every workspace binding crate", async () => {
    const inputs = await loadBindingPolicyInputs(ROOT_URL);

    assert.deepEqual(findUnclassifiedBindings(inputs), []);
  });

  it("identifies experimental bindings when publish protection is removed", async () => {
    const inputs = await loadBindingPolicyInputs(ROOT_URL);
    const packages = inputs.packages.map((pkg) =>
      EXPERIMENTAL_BINDINGS.includes(pkg.name) ? { ...pkg, publish: null } : pkg,
    );

    assert.deepEqual(findUnclassifiedBindings({ ...inputs, packages }), EXPERIMENTAL_BINDINGS);
  });

  it("recognizes release paths for published bindings", async () => {
    const inputs = await loadBindingPolicyInputs(ROOT_URL);
    const packages = inputs.packages.map((pkg) =>
      pkg.name.endsWith("-napi") || pkg.name.endsWith("-wasm") ? { ...pkg, publish: null } : pkg,
    );

    assert.deepEqual(findUnclassifiedBindings({ ...inputs, packages }), EXPERIMENTAL_BINDINGS);
  });
});
