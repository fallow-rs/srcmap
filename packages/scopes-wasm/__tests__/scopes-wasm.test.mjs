import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { decodeLegacyScopes, decodeScopes, encodeScopes } from "../pkg/srcmap_scopes_wasm.js";

const INFO = {
  scopes: [
    [
      {
        start: { line: 0, column: 0 },
        end: { line: 5, column: 27 },
        name: null,
        kind: "module",
        isStackFrame: false,
        variables: ["result"],
        children: [
          {
            start: { line: 1, column: 0 },
            end: { line: 3, column: 1 },
            name: "add",
            kind: "function",
            isStackFrame: true,
            variables: ["a", "b"],
            children: [],
          },
        ],
      },
    ],
    [],
  ],
  ranges: [
    {
      start: { line: 0, column: 0 },
      end: { line: 4, column: 1 },
      isStackFrame: false,
      isHidden: false,
      definition: { sourceIndex: 0, scopeIndex: 0 },
      callSite: null,
      bindings: [{ type: "expression", expression: "result" }],
      children: [
        {
          start: { line: 1, column: 0 },
          end: { line: 3, column: 22 },
          isStackFrame: true,
          isHidden: false,
          definition: { sourceIndex: 0, scopeIndex: 1 },
          callSite: { sourceIndex: 0, line: 5, column: 14 },
          bindings: [
            { type: "expression", expression: "_a" },
            {
              type: "subRanges",
              subRanges: [
                { expression: "_b", from: { line: 1, column: 0 } },
                { expression: null, from: { line: 2, column: 4 } },
              ],
            },
          ],
          children: [],
        },
      ],
    },
  ],
};

// The fields that srcmap 0.6.0 wrote for the same data, in the older layout.
const LEGACY_SCOPES = "BCAAA,DC,BHBAEG,DGC,CCB,CCb,,ECAA,GC,EHBAC,GHI,HBABE,IAFO,FCW,FBB";
const NAMES = ["module", "result", "add", "function", "a", "b", "_a", "_b"];

describe("encodeScopes", () => {
  it("writes one scopes entry for each source and an array of ranges", () => {
    const encoded = encodeScopes(JSON.stringify(INFO), []);

    assert.deepEqual(encoded.scopes, ["BCAAA,DC,BHBAEG,DGC,CCB,CCb", null]);
    assert.deepEqual(encoded.ranges, ["ECAAA,GC,EHBAAC,GHI,IAFO,HBBEA,FCW,FBB"]);
    assert.deepEqual(encoded.names, NAMES);
  });

  it("keeps the names that the caller gives", () => {
    const encoded = encodeScopes(JSON.stringify(INFO), ["first", "add"]);

    assert.deepEqual(encoded.names.slice(0, 2), ["first", "add"]);
    assert.deepEqual(decodeScopes(encoded.scopes, encoded.ranges, encoded.names, 2), INFO);
  });

  it("reads null as a source without scopes", () => {
    const encoded = encodeScopes(JSON.stringify({ scopes: [null, INFO.scopes[0]] }), []);

    assert.equal(encoded.scopes[0], null);
    assert.equal(typeof encoded.scopes[1], "string");
    assert.deepEqual(encoded.ranges, []);
  });

  it("rejects JSON that is not scope information", () => {
    assert.throws(() => encodeScopes('{"scopes":[[{}]]}', []), /missing field/);
  });
});

describe("decodeScopes", () => {
  it("decodes what encodeScopes writes", () => {
    const encoded = encodeScopes(JSON.stringify(INFO), []);

    assert.deepEqual(decodeScopes(encoded.scopes, encoded.ranges, encoded.names, 2), INFO);
  });

  it("gives each source an entry", () => {
    assert.deepEqual(decodeScopes([], [], [], 2), { scopes: [[], []], ranges: [] });
    assert.deepEqual(decodeScopes([null, ""], [], [], 2), { scopes: [[], []], ranges: [] });
  });

  it("throws on a string that it cannot decode", () => {
    assert.throws(() => decodeScopes(["BAAA"], [], [], 1), /never closed/);
    assert.throws(() => decodeScopes([], ["!!"], [], 0), /VLQ decode error/);
  });
});

describe("decodeLegacyScopes", () => {
  it("reads the older layout into the same structure", () => {
    assert.deepEqual(decodeLegacyScopes(LEGACY_SCOPES, NAMES, 2), INFO);
  });
});
