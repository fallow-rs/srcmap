// Comparison of srcmap-scopes with the Chrome DevTools codec
// (@chrome-devtools/source-map-scopes-codec), in both directions.
//
// Each test that has a comment with "Difference:" records a case where the two
// decoders give different results. The comment says which side follows the
// draft specification (https://tc39.es/ecma426/branch/proposal-scopes/).

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { decode, DecodeMode, encode } from "@chrome-devtools/source-map-scopes-codec";

import { decodeScopes, encodeScopes } from "../pkg/srcmap_scopes_wasm.js";

// ── Structure helpers ────────────────────────────────────────────

const pos = (line, column) => ({ line, column });

const scope = (start, end, fields = {}) => ({
  start,
  end,
  name: null,
  kind: null,
  isStackFrame: false,
  variables: [],
  children: [],
  ...fields,
});

const range = (start, end, fields = {}) => ({
  start,
  end,
  isStackFrame: false,
  isHidden: false,
  definition: null,
  callSite: null,
  bindings: [],
  children: [],
  ...fields,
});

const definition = (sourceIndex, scopeIndex) => ({ sourceIndex, scopeIndex });
const expression = (text) => ({ type: "expression", expression: text });
const UNAVAILABLE = { type: "unavailable" };
const subRanges = (...entries) => ({
  type: "subRanges",
  subRanges: entries.map(([text, from]) => ({ expression: text, from })),
});

/** The scopes of one source in pre-order. */
const flatten = (roots) => roots.flatMap((root) => [root, ...flatten(root.children)]);

/** Convert the structure of srcmap to the input of the codec encoder. */
const toCodec = (info) => {
  const flat = [];
  const convertScope = (source, target) => {
    const result = {
      start: source.start,
      end: source.end,
      isStackFrame: source.isStackFrame,
      variables: source.variables,
      children: [],
    };
    if (source.name !== null) result.name = source.name;
    if (source.kind !== null) result.kind = source.kind;
    target.push(result);
    result.children = source.children.map((child) => convertScope(child, target));
    return result;
  };
  const scopes = info.scopes.map((roots) => {
    const target = [];
    flat.push(target);
    return roots.length === 0 ? null : roots.map((root) => convertScope(root, target));
  });

  const convertBinding = (binding, parent) => {
    if (binding.type === "expression") return binding.expression;
    if (binding.type === "unavailable") return null;
    return binding.subRanges.map((sub, index, all) => ({
      from: sub.from,
      to: index + 1 < all.length ? all[index + 1].from : parent.end,
      ...(sub.expression === null ? {} : { value: sub.expression }),
    }));
  };
  const convertRange = (source) => {
    const result = {
      start: source.start,
      end: source.end,
      isStackFrame: source.isStackFrame,
      isHidden: source.isHidden,
      values: source.bindings.map((binding) => convertBinding(binding, source)),
      children: source.children.map(convertRange),
    };
    if (source.definition !== null) {
      result.originalScope = flat[source.definition.sourceIndex][source.definition.scopeIndex];
    }
    if (source.callSite !== null) result.callSite = source.callSite;
    return result;
  };

  return { scopes, ranges: info.ranges.map(convertRange) };
};

/** Convert the output of the codec decoder to the structure of srcmap. */
const fromCodec = (info) => {
  const locations = new Map();
  info.scopes.forEach((roots, sourceIndex) => {
    flatten(roots ?? []).forEach((item, scopeIndex) => {
      locations.set(item, definition(sourceIndex, scopeIndex));
    });
  });

  const convertScope = (source) =>
    scope(source.start, source.end, {
      name: source.name ?? null,
      kind: source.kind ?? null,
      isStackFrame: source.isStackFrame,
      variables: source.variables,
      children: source.children.map(convertScope),
    });
  const convertBinding = (value) => {
    if (typeof value === "string") return expression(value);
    if (value === null || value === undefined) return UNAVAILABLE;
    return subRanges(...value.map((sub) => [sub.value ?? null, sub.from]));
  };
  const convertRange = (source) =>
    range(source.start, source.end, {
      isStackFrame: source.isStackFrame,
      isHidden: source.isHidden,
      definition: source.originalScope ? locations.get(source.originalScope) : null,
      callSite: source.callSite ?? null,
      bindings: source.values.map(convertBinding),
      children: source.children.map(convertRange),
    });

  return {
    scopes: info.scopes.map((roots) => (roots ?? []).map(convertScope)),
    ranges: info.ranges.map(convertRange),
  };
};

const mapOf = (scopes, ranges, names) => ({
  version: 3,
  sources: scopes.map((_, index) => `source${index}.js`),
  names,
  mappings: "",
  scopes,
  ranges,
});

const codecDecode = (scopes, ranges, names, mode = DecodeMode.LAX) =>
  fromCodec(decode(mapOf(scopes, ranges, names), { mode }));

const srcmapDecode = (scopes, ranges, names) => decodeScopes(scopes, ranges, names, scopes.length);

/**
 * Check both directions for one structure:
 * - srcmap decodes what the codec encodes
 * - the codec decodes what srcmap encodes, also in strict mode
 * - each result is the structure
 */
const assertBothDirections = (info) => {
  const fromCodecEncoder = encode(toCodec(info));
  assert.deepEqual(
    srcmapDecode(fromCodecEncoder.scopes, fromCodecEncoder.ranges, fromCodecEncoder.names),
    info,
    "srcmap decodes the output of the codec",
  );

  const fromSrcmapEncoder = encodeScopes(JSON.stringify(info), []);
  for (const mode of [DecodeMode.LAX, DecodeMode.STRICT]) {
    assert.deepEqual(
      codecDecode(
        fromSrcmapEncoder.scopes,
        fromSrcmapEncoder.ranges,
        fromSrcmapEncoder.names,
        mode,
      ),
      info,
      "the codec decodes the output of srcmap",
    );
  }
  assert.deepEqual(
    srcmapDecode(fromSrcmapEncoder.scopes, fromSrcmapEncoder.ranges, fromSrcmapEncoder.names),
    info,
    "srcmap decodes the output of srcmap",
  );

  return { fromCodecEncoder, fromSrcmapEncoder };
};

/** Check that the two encoders write the same strings and the same names. */
const assertSameEncoding = ({ fromCodecEncoder, fromSrcmapEncoder }) => {
  assert.deepEqual(fromSrcmapEncoder.scopes, fromCodecEncoder.scopes);
  assert.deepEqual(fromSrcmapEncoder.ranges, fromCodecEncoder.ranges);
  assert.deepEqual(fromSrcmapEncoder.names, fromCodecEncoder.names);
};

// ── Structures ───────────────────────────────────────────────────

const functionScope = (name, start, end, fields = {}) =>
  scope(start, end, { name, kind: "Function", isStackFrame: true, ...fields });

/** A module with a function, and a block in that function. */
const nestedScopes = () => [
  scope(pos(0, 0), pos(20, 0), {
    kind: "Global",
    variables: ["add", "result"],
    children: [
      functionScope("add", pos(1, 9), pos(6, 1), {
        variables: ["a", "b"],
        children: [scope(pos(2, 4), pos(4, 5), { kind: "Block", variables: ["tmp"] })],
      }),
      functionScope("main", pos(8, 0), pos(12, 1)),
    ],
  }),
];

// ── Tests ────────────────────────────────────────────────────────

describe("srcmap and the Chrome DevTools codec agree", () => {
  it("on nested scopes, function scopes and block scopes", () => {
    const info = {
      scopes: [nestedScopes()],
      ranges: [
        range(pos(0, 0), pos(9, 0), {
          definition: definition(0, 0),
          bindings: [expression("n"), expression("r")],
          children: [
            range(pos(0, 10), pos(3, 1), {
              isStackFrame: true,
              definition: definition(0, 1),
              bindings: [expression("x"), expression("y")],
              children: [
                range(pos(1, 2), pos(1, 30), {
                  definition: definition(0, 2),
                  bindings: [UNAVAILABLE],
                }),
              ],
            }),
            range(pos(4, 0), pos(8, 1), { isStackFrame: true, definition: definition(0, 3) }),
          ],
        }),
      ],
    };
    assertSameEncoding(assertBothDirections(info));
  });

  it("on a scope without a kind, a scope without a name, and several root scopes", () => {
    const info = {
      scopes: [
        [
          scope(pos(0, 0), pos(1, 0)),
          scope(pos(1, 4), pos(1, 9), { name: "onlyName" }),
          scope(pos(3, 0), pos(5, 2), { kind: "Block", isStackFrame: true }),
        ],
      ],
      ranges: [range(pos(0, 0), pos(0, 5), { definition: definition(0, 2) })],
    };
    assertSameEncoding(assertBothDirections(info));
  });

  it("on hidden ranges and ranges without a definition", () => {
    const info = {
      scopes: [nestedScopes()],
      ranges: [
        range(pos(0, 0), pos(0, 80), {
          children: [
            range(pos(0, 5), pos(0, 40), {
              isStackFrame: true,
              isHidden: true,
              definition: definition(0, 2),
              bindings: [expression("t")],
            }),
            range(pos(0, 45), pos(0, 60), { isStackFrame: true, isHidden: true }),
          ],
        }),
      ],
    };
    assertSameEncoding(assertBothDirections(info));
  });

  it("on bindings and sub-range bindings", () => {
    const info = {
      scopes: [nestedScopes()],
      ranges: [
        range(pos(2, 4), pos(6, 0), {
          isStackFrame: true,
          definition: definition(0, 1),
          bindings: [
            subRanges(["a1", pos(2, 4)], [null, pos(2, 20)], ["a2", pos(4, 2)], ["a1", pos(4, 9)]),
            subRanges([null, pos(2, 4)], ["b", pos(3, 0)]),
          ],
          children: [
            range(pos(3, 0), pos(3, 9), {
              definition: definition(0, 2),
              bindings: [subRanges(["t", pos(3, 0)], ["u", pos(3, 4)])],
            }),
          ],
        }),
      ],
    };
    assertSameEncoding(assertBothDirections(info));
  });

  it("on call sites", () => {
    const info = {
      scopes: [nestedScopes(), [functionScope("helper", pos(0, 0), pos(3, 1))]],
      ranges: [
        range(pos(0, 0), pos(5, 0), {
          definition: definition(0, 0),
          bindings: [UNAVAILABLE, expression("r")],
          children: [
            range(pos(1, 0), pos(2, 0), {
              definition: definition(0, 1),
              callSite: { sourceIndex: 0, line: 10, column: 15 },
              bindings: [expression("x"), expression("y")],
              children: [
                range(pos(1, 4), pos(1, 20), {
                  definition: definition(1, 0),
                  callSite: { sourceIndex: 0, line: 3, column: 11 },
                }),
              ],
            }),
          ],
        }),
      ],
    };
    assertSameEncoding(assertBothDirections(info));
  });

  it("on a call site and sub-range bindings in one range", () => {
    const info = {
      scopes: [nestedScopes()],
      ranges: [
        range(pos(0, 0), pos(0, 50), {
          definition: definition(0, 1),
          callSite: { sourceIndex: 0, line: 10, column: 15 },
          bindings: [subRanges(["x", pos(0, 0)], [null, pos(0, 25)]), expression("y")],
        }),
      ],
    };
    const { fromCodecEncoder, fromSrcmapEncoder } = assertBothDirections(info);
    // Difference: the grammar of the specification has the call site item
    // before the sub-range items (GeneratedRangeTree). srcmap writes that
    // order. The codec writes the sub-range items first. Each decoder reads
    // both orders.
    assert.deepEqual(fromSrcmapEncoder.ranges, ["ECAAC,GKL,IAKP,HAAZA,FyB"]);
    assert.deepEqual(fromCodecEncoder.ranges, ["ECAAC,GKL,HAAZA,IAKP,FyB"]);
  });

  it("on several sources and a source without scopes", () => {
    const info = {
      scopes: [
        nestedScopes(),
        [],
        [functionScope("helper", pos(0, 0), pos(3, 1), { variables: ["h"] })],
        [],
      ],
      ranges: [
        // The definitions go to a later source, stay there, and go back.
        range(pos(0, 0), pos(0, 9), { definition: definition(0, 3) }),
        range(pos(1, 0), pos(1, 9), {
          isStackFrame: true,
          definition: definition(2, 0),
          bindings: [expression("h")],
        }),
        range(pos(2, 0), pos(2, 9), { definition: definition(2, 0), bindings: [UNAVAILABLE] }),
        range(pos(3, 0), pos(3, 9), { definition: definition(0, 1) }),
        range(pos(4, 0), pos(4, 9), { definition: definition(0, 0) }),
      ],
    };
    const encoded = assertBothDirections(info);
    assertSameEncoding(encoded);
    // A source without scopes is `null`.
    assert.deepEqual(
      encoded.fromSrcmapEncoder.scopes.map((entry) => entry === null),
      [false, true, false, true],
    );
  });

  it("on a variable whose name index is past the end of names", () => {
    // The variables have index 0, 5 and 1. `names` has two entries.
    const scopes = ["BAAA,DAKJ,CCA"];
    const ranges = ["ECAAA,GBCA,FCA"];
    const names = ["a", "b"];
    const expected = {
      scopes: [[scope(pos(0, 0), pos(2, 0), { variables: ["a", "", "b"] })]],
      ranges: [
        range(pos(0, 0), pos(2, 0), {
          definition: definition(0, 0),
          bindings: [expression("a"), expression("b"), UNAVAILABLE],
        }),
      ],
    };
    assert.deepEqual(srcmapDecode(scopes, ranges, names), expected);
    assert.deepEqual(codecDecode(scopes, ranges, names), expected);
  });

  it("on vendor extension items", () => {
    // The codec has no API that writes a vendor extension item (tag V). Both
    // decoders ignore the item at the top level and in a scope or a range.
    const scopes = ["VA,BAAA,VAKK,BABA,CAB,VA,CBA,VAC"];
    const ranges = ["VA,EAA,VAKK,EAB,FB,VA,FB,VAC"];
    const names = ["vendor"];
    const expected = {
      scopes: [[scope(pos(0, 0), pos(2, 0), { children: [scope(pos(1, 0), pos(1, 1))] })]],
      ranges: [range(pos(0, 0), pos(0, 3), { children: [range(pos(0, 1), pos(0, 2))] })],
    };
    assert.deepEqual(srcmapDecode(scopes, ranges, names), expected);
    assert.deepEqual(codecDecode(scopes, ranges, names), expected);
    assert.deepEqual(codecDecode(scopes, ranges, names, DecodeMode.STRICT), expected);
  });

  it("on several ranges strings and on more values at the end of an item", () => {
    const scopes = ["BAAAKK,CBAKK", ""];
    const ranges = ["ECAAAKK,FB", "EAK,FB"];
    const expected = {
      scopes: [[scope(pos(0, 0), pos(1, 0))], []],
      ranges: [
        range(pos(0, 0), pos(0, 1), { definition: definition(0, 0) }),
        range(pos(0, 10), pos(0, 11)),
      ],
    };
    assert.deepEqual(srcmapDecode(scopes, ranges, []), expected);
    assert.deepEqual(codecDecode(scopes, ranges, []), expected);
  });
});

describe("srcmap and the Chrome DevTools codec differ", () => {
  const twoScopes = "BACB,BADE,CEB,CKA";
  const twoScopesDecoded = [
    scope(pos(2, 1), pos(19, 0), { children: [scope(pos(5, 4), pos(9, 1))] }),
  ];

  it("on a call site relative to a parent range (tag J)", () => {
    // Difference: the specification has the J item (GeneratedRangeRelativeCallSite).
    // Version 2.0.0 of the codec does not know the tag. It ignores the item, or
    // it throws in strict mode. srcmap reads the item, and it writes only the
    // absolute I item, so that the codec can read the output of srcmap.
    const scopes = [twoScopes];
    const ranges = ["ECAAA,ECBAC,JDJ,FB,FB"];
    const inner = (callSite) =>
      range(pos(0, 1), pos(0, 2), { definition: definition(0, 1), callSite });
    const outer = (callSite) =>
      range(pos(0, 0), pos(0, 3), { definition: definition(0, 0), children: [inner(callSite)] });

    assert.deepEqual(srcmapDecode(scopes, ranges, []), {
      scopes: [twoScopesDecoded],
      ranges: [outer({ sourceIndex: 0, line: 5, column: 9 })],
    });
    assert.deepEqual(codecDecode(scopes, ranges, []), {
      scopes: [twoScopesDecoded],
      ranges: [outer(null)],
    });
    assert.throws(() => codecDecode(scopes, ranges, [], DecodeMode.STRICT), /illegal item tag 9/);
  });

  it("on the hidden flag of a range that is not a stack frame", () => {
    // Difference: the specification says that flag 0x8 is valid only with flag
    // 0x4, and that the range has the stack frame type "none" without it.
    // srcmap follows the specification. The codec keeps `isHidden`.
    const ranges = ["EIA,FB"];
    assert.deepEqual(srcmapDecode([], ranges, []).ranges, [range(pos(0, 0), pos(0, 1))]);
    assert.deepEqual(codecDecode([], ranges, []).ranges, [
      range(pos(0, 0), pos(0, 1), { isHidden: true }),
    ]);
  });

  it("on a scope name and a scope kind past the end of names", () => {
    // Difference: the specification (RelativeName) gives null for an index past
    // the end of `names`, and only a variable becomes the empty string. srcmap
    // follows the specification. The codec gives the empty string for a name
    // and a kind too, and it throws in strict mode.
    const scopes = ["BDAAKM,CBA"];
    assert.deepEqual(srcmapDecode(scopes, [], ["a"]).scopes, [[scope(pos(0, 0), pos(1, 0))]]);
    assert.deepEqual(codecDecode(scopes, [], ["a"]).scopes, [
      [scope(pos(0, 0), pos(1, 0), { name: "", kind: "" })],
    ]);
    assert.throws(() => codecDecode(scopes, [], ["a"], DecodeMode.STRICT), /Illegal index/);
  });

  it("on a binding expression past the end of names", () => {
    // Difference: the specification (BindingExpression) gives null, which is a
    // variable that is not available. srcmap follows the specification. The
    // codec gives the empty expression.
    const ranges = ["EAA,GBK,FC"];
    assert.deepEqual(srcmapDecode([], ranges, ["a"]).ranges[0].bindings, [
      expression("a"),
      UNAVAILABLE,
    ]);
    assert.deepEqual(codecDecode([], ranges, ["a"]).ranges[0].bindings, [
      expression("a"),
      expression(""),
    ]);
  });

  it("on an index that goes below zero", () => {
    // Difference: the specification (AccumulateIndex) sets an index below zero
    // to zero. srcmap follows the specification. The codec keeps the negative
    // index, which gives the empty string and no definition.
    const scopes = ["BBAAD,CBA"];
    const ranges = ["ECAAD,FB"];
    const names = ["a"];
    assert.deepEqual(srcmapDecode(scopes, ranges, names), {
      scopes: [[scope(pos(0, 0), pos(1, 0), { name: "a" })]],
      ranges: [range(pos(0, 0), pos(0, 1), { definition: definition(0, 0) })],
    });
    assert.deepEqual(codecDecode(scopes, ranges, names), {
      scopes: [[scope(pos(0, 0), pos(1, 0), { name: "" })]],
      ranges: [range(pos(0, 0), pos(0, 1))],
    });
  });

  it("on two sub-range items for one variable", () => {
    // Difference: the specification adds the bindings of each sub-range item to
    // the list of the variable. srcmap follows the specification. The codec
    // ignores the second item, and it throws in strict mode.
    const ranges = ["EAA,GB,HAAKA,HAAUC,Fe"];
    const names = ["a", "b"];
    assert.deepEqual(srcmapDecode([], ranges, names).ranges[0].bindings, [
      subRanges(["a", pos(0, 0)], [null, pos(0, 10)], ["b", pos(0, 20)]),
    ]);
    assert.deepEqual(codecDecode([], ranges, names).ranges[0].bindings, [
      subRanges(["a", pos(0, 0)], [null, pos(0, 10)]),
    ]);
    assert.throws(() => codecDecode([], ranges, names, DecodeMode.STRICT), /multiple/);
  });

  it("on a definition that is not a decoded scope", () => {
    // The specification does not say what a decoder does with a scope index
    // past the last scope of the source. srcmap-scopes keeps the two indices,
    // and `SourceMap::from_json` rejects the map. The codec gives a range
    // without a definition, and it throws in strict mode.
    const scopes = [twoScopes];
    const ranges = ["ECAAK,FB"];
    assert.deepEqual(srcmapDecode(scopes, ranges, []).ranges, [
      range(pos(0, 0), pos(0, 1), { definition: definition(0, 5) }),
    ]);
    assert.deepEqual(codecDecode(scopes, ranges, []).ranges, [range(pos(0, 0), pos(0, 1))]);
    assert.throws(() => codecDecode(scopes, ranges, [], DecodeMode.STRICT), /Invalid definition/);
  });

  it("on input that does not match the grammar", () => {
    // The specification lets a decoder report an error when the parse fails.
    // srcmap reports the error. The codec gives the scopes that it can read
    // when it is not in strict mode.
    for (const [scopes, ranges, message] of [
      [["BAAA"], [], /never closed/],
      [["CAA"], [], /without matching start/],
      [[], ["EAA"], /never closed/],
      [[], ["FA"], /without matching start/],
    ]) {
      assert.throws(() => srcmapDecode(scopes, ranges, []), message);
      assert.doesNotThrow(() => codecDecode(scopes, ranges, []));
      assert.throws(() => codecDecode(scopes, ranges, [], DecodeMode.STRICT));
    }
  });
});
