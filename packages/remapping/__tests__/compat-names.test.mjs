/**
 * Compares the name handling of @srcmap/remapping with @jridgewell/remapping.
 *
 * jridgewell reads a name as `names[index]` and adds it only when the value is
 * truthy. A name index outside `names` and an empty name thus give a segment
 * without a name. An upstream 5-field segment sets the name also when its
 * index is outside `names`, so the name of the outer segment is not used.
 */
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import srcmapRemapping from "../src/remapping.mjs";
import jrRemapping from "@jridgewell/remapping";

const clone = (value) => JSON.parse(JSON.stringify(value));

const loaderFor = (upstreams) => (source) => {
  const map = upstreams[source];
  return map ? clone(map) : null;
};

const compare = (input, upstreams = {}) => {
  const jr = jrRemapping(clone(input), loaderFor(upstreams));
  const srcmap = srcmapRemapping(clone(input), loaderFor(upstreams));
  assert.deepEqual(
    { sources: srcmap.sources, names: srcmap.names, mappings: srcmap.mappings },
    { sources: jr.sources, names: jr.names, mappings: jr.mappings },
  );
  return srcmap;
};

const map = (fields) => ({ version: 3, sources: ["a.js"], names: [], ...fields });

describe("name index outside names, compared with @jridgewell/remapping", () => {
  it("gives no name for an outer segment with name index 0 and no names", () => {
    const result = compare(map({ mappings: "AAAAA" }));
    assert.equal(result.mappings, "AAAA");
    assert.deepEqual(result.names, []);
  });

  it("gives no name for an outer name index past the end of names", () => {
    compare(map({ names: ["x"], mappings: "AAAAE,CAAAD" }));
  });

  it("gives no name for a negative outer name index", () => {
    compare(map({ names: ["x"], mappings: "AAAAD,CAAAC" }));
  });

  it("gives no name when the outer map has no names field", () => {
    compare({ version: 3, sources: ["a.js"], mappings: "AAAAA" });
  });

  it("gives no name for decoded mappings with a name index outside names", () => {
    compare(map({ names: ["x"], mappings: [[[0, 0, 0, 0, 3]]] }));
  });

  it("gives no name for an upstream segment when the outer segment has no name", () => {
    compare(map({ mappings: "AAAA" }), {
      "a.js": map({ sources: ["orig.js"], mappings: "AAAAA" }),
    });
  });

  it("does not use the outer name when the upstream name index is outside names", () => {
    const result = compare(map({ names: ["outer"], mappings: "AAAAA" }), {
      "a.js": map({ sources: ["orig.js"], mappings: "AAAAA" }),
    });
    assert.deepEqual(result.names, []);
  });

  it("uses an upstream 4-field segment with an outer name index outside names", () => {
    compare(map({ mappings: "AAAAA" }), {
      "a.js": map({ sources: ["orig.js"], mappings: "AAAA" }),
    });
  });

  it("drops a segment that equals the previous one when neither has a name", () => {
    // The first segment traces to an upstream name outside names, the second
    // to a 4-field upstream segment. Both have no name and the same position.
    const result = compare(map({ names: ["outer"], mappings: "AAAAA,CAAC" }), {
      "a.js": map({ sources: ["orig.js"], mappings: "AAAAA,CAAA" }),
    });
    assert.equal(result.mappings, "AAAA");
  });

  it("keeps valid names next to name indices outside names", () => {
    compare(map({ names: ["outer"], mappings: "AAAAA,EAACC,EAACD;AAAAE" }), {
      "a.js": map({
        sources: ["orig.js"],
        names: ["inner"],
        mappings: "AAAAA,CAACC,CAACD,CAACA;AACAA",
      }),
    });
  });

  it("matches an array of maps with name indices outside names", () => {
    compare([
      map({ names: ["top"], mappings: "AAAAA,CAAAC,CAAAE" }),
      map({ sources: ["mid.js"], names: [], mappings: "AAAAA,CAAC" }),
      map({ sources: ["orig.js"], names: ["deep"], mappings: "AAAA,CAAAA" }),
    ]);
  });

  it("lets a deeper valid name replace a name index outside names", () => {
    compare(map({ names: ["outer"], mappings: "AAAAA" }), {
      "a.js": map({ sources: ["mid.js"], mappings: "AAAAC" }),
      "mid.js": map({ sources: ["orig.js"], names: ["deep"], mappings: "AAAAA" }),
    });
  });

  it("gives no name through a deeper 4-field map after a name index outside names", () => {
    compare(map({ names: ["outer"], mappings: "AAAAA" }), {
      "a.js": map({ sources: ["mid.js"], mappings: "AAAAC" }),
      "mid.js": map({ sources: ["orig.js"], mappings: "AAAA" }),
    });
  });

  it("matches a name index outside names on a line with sourceless segments", () => {
    compare(map({ names: ["x"], mappings: "A,CAAAE,C,CAACC;AAAAD" }));
  });
});

describe("empty names, compared with @jridgewell/remapping", () => {
  it("gives no name for an outer segment with an empty name", () => {
    compare(map({ names: [""], mappings: "AAAAA" }));
  });

  it("does not use the outer name when the upstream name is empty", () => {
    compare(map({ names: ["outer"], mappings: "AAAAA" }), {
      "a.js": map({ sources: ["orig.js"], names: [""], mappings: "AAAAA" }),
    });
  });

  it("keeps other names next to an empty name", () => {
    compare(map({ names: ["", "kept"], mappings: "AAAAA,CAAAC,CAAAD" }));
  });
});

describe("source index outside sources", () => {
  it("throws for an outer source index outside sources, as jridgewell does", () => {
    const input = map({ mappings: "ACAA" });
    assert.throws(() => jrRemapping(clone(input), () => null));
    assert.throws(() => srcmapRemapping(clone(input), () => null));
  });

  it("throws for an upstream source index outside sources, as jridgewell does", () => {
    const input = map({ mappings: "AAAA" });
    const upstreams = { "a.js": map({ sources: ["orig.js"], mappings: "ACAA" }) };
    assert.throws(() => jrRemapping(clone(input), loaderFor(upstreams)));
    assert.throws(() => srcmapRemapping(clone(input), loaderFor(upstreams)));
  });
});

describe("names entries that are not strings, compared with @jridgewell/remapping", () => {
  // jridgewell adds a name only when `names[index]` is truthy, so null, false
  // and 0 give no name. A truthy value that is not a string is a known difference.
  it("gives no name for a null entry", () => {
    const result = compare(map({ names: [null], mappings: "AAAAA" }));
    assert.equal(result.mappings, "AAAA");
  });

  it("gives no name for a false or 0 entry", () => {
    compare(map({ names: [false], mappings: "AAAAA" }));
    compare(map({ names: [0], mappings: "AAAAA" }));
  });

  it("accepts a null entry that no segment uses", () => {
    compare(map({ names: [null], mappings: "AAAA" }));
  });

  it("keeps valid names next to a null entry", () => {
    const result = compare(map({ names: [null, "x"], mappings: "AAAAA,CAAAC" }));
    assert.deepEqual(result.names, ["x"]);
    compare(map({ names: [null, false, 0, "x"], mappings: "AAAAA,CAACC,CAACC,CAACC" }));
  });

  it("does not use the outer name when the upstream entry is null", () => {
    const result = compare(map({ names: ["outer"], mappings: "AAAAA" }), {
      "a.js": map({ sources: ["orig.js"], names: [null], mappings: "AAAAA" }),
    });
    assert.deepEqual(result.names, []);
  });

  it("throws for a truthy entry that is not a string", () => {
    assert.throws(() => srcmapRemapping(map({ names: [1], mappings: "AAAAA" }), () => null));
  });
});

describe("names that is not an array, compared with @jridgewell/remapping", () => {
  it("gives no name when names is null", () => {
    compare(map({ names: null, mappings: "AAAAA" }));
  });

  it("gives no name when names is a number, a boolean or an empty string", () => {
    compare(map({ names: 5, mappings: "AAAAA" }));
    compare(map({ names: true, mappings: "AAAAA" }));
    compare(map({ names: "", mappings: "AAAAA" }));
  });

  it("gives no name when the upstream names is null", () => {
    compare(map({ names: ["outer"], mappings: "AAAAA" }), {
      "a.js": map({ sources: ["orig.js"], names: null, mappings: "AAAAA" }),
    });
  });
});
