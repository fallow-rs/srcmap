import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { remap: wasmRemap, RemapTree } = require("@srcmap/remapping-wasm");
const resolveUri = require("@jridgewell/resolve-uri");

/**
 * Result class with the fields of the @jridgewell/remapping `SourceMap`.
 *
 * @param {object} raw - A source map with encoded `mappings`.
 * @param {{ excludeContent?: boolean, decodedMappings?: boolean }} [options]
 */
class SourceMap {
  constructor(raw, options = {}) {
    this.version = 3;
    this.file = raw.file ?? undefined;
    this.mappings = options.decodedMappings ? decodeMappings(raw.mappings) : raw.mappings;
    this.names = raw.names || [];
    this.ignoreList = raw.ignoreList || [];
    this.sourceRoot = raw.sourceRoot ?? undefined;
    this.sources = raw.sources || [];
    if (!options.excludeContent) {
      this.sourcesContent = raw.sourcesContent || this.sources.map(() => null);
    }
  }

  toString() {
    return JSON.stringify(this);
  }

  toJSON() {
    return {
      version: this.version,
      file: this.file,
      mappings: this.mappings,
      names: this.names,
      ignoreList: this.ignoreList,
      sourceRoot: this.sourceRoot,
      sources: this.sources,
      sourcesContent: this.sourcesContent,
    };
  }
}

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const encodeVlq = (value) => {
  let vlq = value < 0 ? (-value << 1) | 1 : value << 1;
  let result = "";
  do {
    let digit = vlq & 0x1f;
    vlq >>>= 5;
    if (vlq > 0) digit |= 0x20;
    result += BASE64[digit];
  } while (vlq > 0);
  return result;
};

const encodeMappings = (decoded) => {
  const lines = [];
  let prevSource = 0,
    prevOrigLine = 0,
    prevOrigCol = 0,
    prevName = 0;
  for (const line of decoded) {
    const segments = [];
    let prevCol = 0;
    for (const seg of line) {
      let str = encodeVlq(seg[0] - prevCol);
      prevCol = seg[0];
      if (seg.length > 1) {
        str += encodeVlq(seg[1] - prevSource);
        str += encodeVlq(seg[2] - prevOrigLine);
        str += encodeVlq(seg[3] - prevOrigCol);
        prevSource = seg[1];
        prevOrigLine = seg[2];
        prevOrigCol = seg[3];
        if (seg.length > 4) {
          str += encodeVlq(seg[4] - prevName);
          prevName = seg[4];
        }
      }
      segments.push(str);
    }
    lines.push(segments.join(","));
  }
  return lines.join(";");
};

const BASE64_VALUES = new Map([...BASE64].map((char, index) => [char, index]));
const VLQ_CONTINUATION = 0x20;
const VLQ_DIGIT_MASK = 0x1f;
const VLQ_SHIFT = 5;

const decodeSegment = (segment, state) => {
  const fields = [];
  let pos = 0;
  while (pos < segment.length) {
    let value = 0;
    let shift = 0;
    let digit;
    do {
      digit = BASE64_VALUES.get(segment[pos++]);
      value |= (digit & VLQ_DIGIT_MASK) << shift;
      shift += VLQ_SHIFT;
    } while (digit & VLQ_CONTINUATION);
    const index = fields.length;
    state[index] += value & 1 ? -(value >>> 1) : value >>> 1;
    fields.push(state[index]);
  }
  return fields;
};

const decodeMappings = (encoded) => {
  if (encoded === "") return [];
  const state = [0, 0, 0, 0, 0];
  return encoded.split(";").map((line) => {
    state[0] = 0;
    if (line === "") return [];
    return line.split(",").map((segment) => decodeSegment(segment, state));
  });
};

const NO_UPSTREAM = () => null;

// An indexed map (with `sections`) has no top-level `sources`. A WASM remap without
// upstream maps flattens it, so the loader can then see each of its sources.
// @jridgewell/remapping rejects indexed maps.
// Marks a map whose `sources` are already resolved.
const RESOLVED = Symbol("resolved sources");

// A `TraceMap` of @jridgewell/trace-mapping keeps its mappings in `_encoded` or
// `_decoded`, and its resolved sources in `resolvedSources`.
const fromTraceMap = (traceMap) => ({
  version: 3,
  file: traceMap.file,
  names: traceMap.names,
  sources: traceMap.resolvedSources,
  sourcesContent: traceMap.sourcesContent,
  ignoreList: traceMap.ignoreList,
  mappings: traceMap._encoded ?? traceMap._decoded,
  [RESOLVED]: true,
});

const toRawMap = (input) => {
  const parsed = typeof input === "string" ? JSON.parse(input) : input;
  if (parsed == null) return parsed;
  if (parsed._decodedMemo) return fromTraceMap(parsed);
  if (Array.isArray(parsed.sections)) {
    return JSON.parse(wasmRemap(JSON.stringify(parsed), NO_UPSTREAM));
  }
  if (typeof parsed.mappings !== "string" && !Array.isArray(parsed.mappings)) {
    throw new Error(`invalid source map: ${JSON.stringify(parsed)}`);
  }
  return parsed;
};

const stripFilename = (path) => (path ? path.slice(0, path.lastIndexOf("/") + 1) : "");

// The same source resolution as the `TraceMap` of @jridgewell/trace-mapping: join
// `sourceRoot` with a slash, then resolve against the directory of `mapUrl`.
const resolveSources = (map, mapUrl) => {
  if (map[RESOLVED]) return map.sources;
  const from = stripFilename(mapUrl);
  const prefix = map.sourceRoot ? map.sourceRoot + "/" : "";
  return (map.sources || []).map((source) => resolveUri(prefix + (source || ""), from));
};

// @jridgewell/remapping reads neither `version` nor `rangeMappings`. A decoded map
// from `MagicString.generateDecodedMap()` has no `version` and holds `rangeMappings`
// as arrays, so the JSON for WASM sets `version` and drops `rangeMappings`. The
// sources in it are already resolved, so it drops `sourceRoot` too.
const toWasmJson = (map, fields) =>
  JSON.stringify({
    ...map,
    version: 3,
    mappings: Array.isArray(map.mappings) ? encodeMappings(map.mappings) : map.mappings,
    sourceRoot: undefined,
    rangeMappings: undefined,
    x_google_ignoreList: undefined,
    ...fields,
  });

// Each map except the innermost one must have one source, as in @jridgewell/remapping.
const checkTransformationMaps = (maps) => {
  for (let i = 0; i < maps.length - 1; i++) {
    if (maps[i]?.sources?.length > 1) {
      throw new Error(
        `Transformation map ${i} must have exactly one source file.\n` +
          "Did you specify these with the most recent transformation maps first?",
      );
    }
  }
};

const NO_NAME_INDEX = -1;

// The same check as `skipSource` of @jridgewell/gen-mapping: a segment that repeats
// the previous segment on its line adds nothing.
const repeatsPrevious = (line, segment) => {
  const prev = line.at(-1);
  if (prev === undefined || prev.length === 1) return false;
  return (
    prev[1] === segment[1] &&
    prev[2] === segment[2] &&
    prev[3] === segment[3] &&
    (prev[4] ?? NO_NAME_INDEX) === (segment[4] ?? NO_NAME_INDEX)
  );
};

// Gives each source of `map` the index of its group. As in jridgewell, the content
// of a group is the last content that a segment of the group sets.
const mergeSources = (map, groupOf, groups, restore) => {
  const contents = map.sourcesContent;
  const lines = decodeMappings(map.mappings).map((line) => {
    const merged = [];
    for (const segment of line) {
      if (segment.length === 1) {
        merged.push(segment);
        continue;
      }
      const group = groupOf[segment[1]];
      if (contents?.[segment[1]] != null) group.content = contents[segment[1]];
      const next = [segment[0], group.index, ...segment.slice(2)];
      if (!repeatsPrevious(merged, next)) merged.push(next);
    }
    return merged;
  });
  map.mappings = encodeMappings(lines);
  map.sources = groups.map((group) => restore(group.name));
  if (contents) map.sourcesContent = groups.map((group) => group.content);
  if (map.ignoreList) {
    map.ignoreList = [...new Set(map.ignoreList.map((i) => groupOf[i].index))];
  }
  return map;
};

/**
 * The maps of one `remapping` call. Each map that the loader gives, and each outer
 * map of a chain, gets a unique key. The key is the source name of that map in the
 * map above it, so the WASM remap can find the map by key.
 */
class UpstreamMaps {
  constructor() {
    this.prefix = `\u0000srcmap-upstream-${Math.random().toString(36).slice(2)}-`;
    this.nodes = new Map();
    this.json = new Map();
    this.height = 0;
    this.ignoredNames = new Set();
    this.plainNames = new Set();
    this.standIns = new Map();
    // The value of each name that `leafName` gives for a source that is not a string.
    this.values = new Map();
    this.valueNames = new Map();
  }

  // jridgewell keeps a `ctx.source` that is not a string, and finds a source by its
  // property key. A falsy value gives sourceless segments, and an ignored one stays
  // in `sources`. The WASM remap has one empty source, so the first ignored falsy
  // value in loader order gives its value. Each other value gets a unique name, so
  // that the WASM remap keeps it apart from a string with the same string form.
  // `restoreValues` merges them after the last pass.
  leafName(source, ignored) {
    if (!source) {
      if (ignored && !this.values.has("")) this.values.set("", source);
      return "";
    }
    if (typeof source === "string") return source;
    let name = this.valueNames.get(source);
    if (name === undefined) {
      name = `${this.prefix}value-${this.valueNames.size}`;
      this.valueNames.set(source, name);
      this.values.set(name, source);
    }
    return name;
  }

  hasValues() {
    return this.values.size > 1 || (this.values.size === 1 && this.values.get("") !== "");
  }

  // Merges the sources that share a property key, as `put` of jridgewell does. The
  // first source in the result is the first that a segment registered, so it gives
  // the value. jridgewell throws for a value without a string form only when a
  // segment uses it.
  restoreValues(map) {
    const keyOf = (name) => {
      if (!this.values.has(name)) return name;
      const value = this.values.get(name);
      if (typeof value === "symbol") return value;
      try {
        return String(value);
      } catch {
        throw new TypeError("Cannot convert object to primitive value");
      }
    };
    const groups = new Map();
    const groupOf = map.sources.map((name, i) => {
      const key = keyOf(name);
      let group = groups.get(key);
      if (group === undefined) {
        group = { index: groups.size, name, content: map.sourcesContent?.[i] ?? null };
        groups.set(key, group);
      }
      return group;
    });
    const restore = (name) => (this.values.has(name) ? this.values.get(name) : name);
    if (groups.size === map.sources.length) {
      map.sources = map.sources.map(restore);
      return map;
    }
    return mergeSources(map, groupOf, [...groups.values()], restore);
  }

  add(node, depth) {
    const key = this.prefix + this.nodes.size;
    this.nodes.set(key, node);
    this.height = Math.max(this.height, depth);
    return key;
  }

  noteLeaf(name, ignored) {
    (ignored ? this.ignoredNames : this.plainNames).add(name);
  }

  // A pass merges the sources that share a name, and the next pass registers an
  // ignored source at the first segment of the merged source. jridgewell registers
  // it at the first segment of an ignored copy, and keeps an ignored empty source.
  // With more than one pass, such a source gets a stand-in name, so that each copy
  // stays a separate source until `restoreStandIns`.
  needsStandIn(name) {
    if (this.height < 2 || !this.ignoredNames.has(name)) return false;
    return name === "" || this.plainNames.has(name);
  }

  standIn(name) {
    const standIn = `${this.prefix}ignored-${name}`;
    this.standIns.set(standIn, name);
    return standIn;
  }

  // The source names of `node` for the WASM remap.
  sourcesOf(node) {
    return node.sources.map((source, i) =>
      node.ignored.has(i) && this.needsStandIn(source) ? this.standIn(source) : source,
    );
  }

  toJson(node) {
    return toWasmJson(node.map, { ...node.fields, sources: this.sourcesOf(node) });
  }

  // Call after the whole tree exists, when the pass count is known.
  serialize(root) {
    for (const [key, node] of this.nodes) this.json.set(key, this.toJson(node));
    return this.toJson(root);
  }

  get(key) {
    return this.json.get(key) ?? null;
  }
}

// The parts path: `RemapTree` of the WASM package takes each map as parts, so
// that no JSON of a whole map crosses the boundary. It must give the result of
// the JSON path, so each map that it takes must give the same JSON parse result
// in Rust. A map that it cannot take with certainty goes to the JSON path.

// The WASM remap reads these keys of a map. Its JSON sets `version`, `sources`,
// `sourcesContent` and `ignoreList`, and drops the other keys of this list.
const PARTS_KEYS = new Set([
  "version",
  "file",
  "names",
  "mappings",
  "sources",
  "sourcesContent",
  "ignoreList",
  "sourceRoot",
  "rangeMappings",
  "x_google_ignoreList",
]);

// Values per segment in the decoded mappings for `RemapTree.addDecoded`.
const SEGMENT_STRIDE = 6;
const NO_INDEX = -1;
// `encodeVlq` gives the right VLQ only for values below 2^30.
const VLQ_VALUE_MASK = 0x3fffffff;

const isWellFormed = String.prototype.isWellFormed
  ? (text) => text.isWellFormed()
  : (text) => !/\p{Surrogate}/u.test(text);

// The JSON of a string with a lone surrogate does not parse in Rust, and a
// string that crosses the boundary loses it. Such a string goes to the JSON path.
const isPartsString = (value) => typeof value === "string" && isWellFormed(value);

const isPartsStrings = (values) => values.every(isPartsString);

// True for an integer from 0 up to `VLQ_VALUE_LIMIT`. The mask also rejects a
// fraction, NaN and a value that is not a number.
const isVlqValue = (value) => typeof value === "number" && (value & VLQ_VALUE_MASK) === value;

// The segments go to one buffer for all calls, so that a call allocates no array.
let segmentBuffer = new Int32Array(SEGMENT_STRIDE * 1024);

const growSegmentBuffer = (size) => {
  const grown = new Int32Array(Math.max(size, segmentBuffer.length * 2));
  grown.set(segmentBuffer);
  segmentBuffer = grown;
};

// True for a segment with a source that the parts path can take. The JSON path
// rejects a source index outside `sources`, so this path does not take one.
const isSourcedSegment = (segment, sourceCount) => {
  const fields = segment.length;
  if (fields !== 4 && fields !== 5) return false;
  // A name index of -1 in the JSON path is a name outside `names`, not "no name".
  if (fields === 5 && !isVlqValue(segment[4])) return false;
  const source = segment[1];
  return (
    isVlqValue(source) && source < sourceCount && isVlqValue(segment[2]) && isVlqValue(segment[3])
  );
};

// Writes `segment` at `at` of `flat`. Returns false when the JSON path must take it.
const writeSegment = (flat, at, lineIndex, segment, sourceCount) => {
  if (!Array.isArray(segment) || !isVlqValue(segment[0])) return false;
  flat[at] = lineIndex;
  flat[at + 1] = segment[0];
  if (segment.length === 1) {
    flat[at + 2] = NO_INDEX;
    flat[at + 5] = NO_INDEX;
    return true;
  }
  if (!isSourcedSegment(segment, sourceCount)) return false;
  flat[at + 2] = segment[1];
  flat[at + 3] = segment[2];
  flat[at + 4] = segment[3];
  flat[at + 5] = segment.length === 5 ? segment[4] : NO_INDEX;
  return true;
};

// The segments of decoded `mappings` for `RemapTree.addDecoded`, or null when the
// JSON path must take them. The result is a view of `segmentBuffer`, valid up to
// the next call.
const flattenDecoded = (lines, sourceCount) => {
  let at = 0;
  for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
    const line = lines[lineIndex];
    if (!Array.isArray(line)) return null;
    const end = at + line.length * SEGMENT_STRIDE;
    if (end > segmentBuffer.length) growSegmentBuffer(end);
    for (let i = 0; i < line.length; i++, at += SEGMENT_STRIDE) {
      if (!writeSegment(segmentBuffer, at, lineIndex, line[i], sourceCount)) return null;
    }
  }
  return segmentBuffer.subarray(0, at);
};

// True when the parts path can take the fields of `map` other than `mappings`.
const isPartsMap = (map, names, sources) => {
  for (const key of Object.keys(map)) {
    if (!PARTS_KEYS.has(key) && map[key] !== undefined) return false;
  }
  if (map.file != null && !isPartsString(map.file)) return false;
  return Array.isArray(names) && isPartsStrings(names) && isPartsStrings(sources);
};

// The content tokens for `RemapTree`, or null when the JSON path must take the
// contents. Each content goes to `contents`, and the token is its index.
const contentTokens = (contentList, contents) => {
  const tokens = new Int32Array(contentList.length);
  for (let i = 0; i < contentList.length; i++) {
    const content = contentList[i];
    if (content === null) {
      tokens[i] = NO_INDEX;
    } else if (isPartsString(content)) {
      tokens[i] = contents.push(content) - 1;
    } else {
      return null;
    }
  }
  return tokens;
};

const utf8 = new TextEncoder();
let mappingBytes = new Uint8Array(64 * 1024);

// The UTF-8 bytes of `text`, as a view of `mappingBytes` that is valid up to the
// next call. The WASM glue copies a string one character at a time, and
// `encodeInto` copies it in native code, which is faster for a long VLQ string.
const toUtf8 = (text) => {
  if (mappingBytes.length < text.length) mappingBytes = new Uint8Array(text.length * 2);
  let { read, written } = utf8.encodeInto(text, mappingBytes);
  if (read < text.length) {
    // A character outside ASCII takes up to 3 bytes.
    mappingBytes = new Uint8Array(text.length * 3);
    ({ written } = utf8.encodeInto(text, mappingBytes));
  }
  return mappingBytes.subarray(0, written);
};

// Adds a map with the other fields in `args` and `mappings` to `tree`. Returns
// false when the JSON path must take the mappings.
const addMappings = (tree, args, mappings, sourceCount) => {
  if (!Array.isArray(mappings)) {
    if (!isPartsString(mappings)) return false;
    tree.addEncoded(...args, toUtf8(mappings));
    return true;
  }
  const segments = flattenDecoded(mappings, sourceCount);
  if (segments === null) return false;
  tree.addDecoded(...args, segments);
  return true;
};

// Adds `node` to `tree` with `RemapTree.addEncoded` or `addDecoded`. Returns false
// when the JSON path must take the node.
const addNode = (tree, key, node, sources, contents) => {
  const { map, fields } = node;
  const names = map.names ?? [];
  if (!isPartsMap(map, names, sources)) return false;
  const tokens = contentTokens(fields.sourcesContent ?? [], contents);
  if (tokens === null) return false;
  const args = [key, map.file ?? undefined, sources, tokens, names, fields.ignoreList ?? []];
  return addMappings(tree, args, map.mappings, sources.length);
};

// Reads the result of `tree` as the JSON path reads the result JSON.
const readResult = (tree, contents) => {
  const map = {
    version: 3,
    file: tree.resultFile(),
    sources: tree.resultSources(),
    names: tree.resultNames(),
    mappings: tree.resultMappings(),
    ignoreList: Array.from(tree.resultIgnoreList()),
  };
  const tokens = tree.resultContents();
  if (tokens.some((token) => token !== NO_INDEX)) {
    map.sourcesContent = Array.from(tokens, (token) =>
      token === NO_INDEX ? null : contents[token],
    );
  }
  return map;
};

// Runs the remap of `remapAll` with `RemapTree`. Returns the result map, or null
// when the JSON path must do the remap.
const remapParts = (root, upstreams) => {
  if (upstreams.hasValues()) return null;
  const nodes = [...upstreams.nodes, [undefined, root]];
  const sourceLists = nodes.map(([, node]) => upstreams.sourcesOf(node));
  if (upstreams.standIns.size > 0) return null;

  const contents = [];
  const tree = new RemapTree();
  try {
    for (let i = 0; i < nodes.length; i++) {
      const [key, node] = nodes[i];
      if (!addNode(tree, key, node, sourceLists[i], contents)) return null;
    }
    tree.remap(Math.max(1, upstreams.height));
    return readResult(tree, contents);
  } finally {
    tree.free();
  }
};

// Calls the loader for each source in the order of @jridgewell/remapping: by source
// index, depth first. The loader can change `ctx.source`, `ctx.content` and
// `ctx.ignore` of a source that has no upstream map.
const buildTree = (input, loader, importer, importerDepth, upstreams) => {
  const map = toRawMap(input);
  const depth = importerDepth + 1;
  const ignoreList = map.ignoreList || map.x_google_ignoreList || [];
  const sourcesContent = [];
  const ignored = new Set();
  const sources = resolveSources(map, importer).map((resolved, i) => {
    const ctx = { importer, depth, source: resolved, content: undefined, ignore: undefined };
    const upstream = loader(ctx.source, ctx);
    if (upstream) {
      sourcesContent.push(null);
      return upstreams.add(buildTree(upstream, loader, ctx.source, depth, upstreams), depth);
    }
    sourcesContent.push(
      ctx.content !== undefined ? ctx.content : (map.sourcesContent?.[i] ?? null),
    );
    const isIgnored = ctx.ignore !== undefined ? ctx.ignore : ignoreList.includes(i);
    if (isIgnored) ignored.add(i);
    const name = upstreams.leafName(ctx.source, isIgnored);
    upstreams.noteLeaf(name, isIgnored);
    return name;
  });
  return { map, sources, ignored, fields: { sourcesContent, ignoreList: [...ignored] } };
};

// The last pass renames each stand-in back. The copies of one name then merge in
// segment order, and an empty name becomes a sourceless source, as in jridgewell.
const restoreStandIns = (json, standIns) => {
  const map = JSON.parse(json);
  map.sources = map.sources.map((source) => standIns.get(source) ?? source);
  return wasmRemap(JSON.stringify(map), NO_UPSTREAM);
};

// A WASM remap pass resolves one level of keys, so the remap runs once per level.
// Returns the parsed result map.
const remapAll = (input, loader) => {
  const maps = (Array.isArray(input) ? input : [input]).map(toRawMap);
  checkTransformationMaps(maps);
  if (maps.length === 0) {
    // jridgewell throws a TypeError here too, before it calls the loader.
    throw new TypeError("remapping needs at least one source map, but got an empty array");
  }

  const upstreams = new UpstreamMaps();
  let current = buildTree(maps.at(-1), loader, "", 0, upstreams);
  for (let i = maps.length - 2; i >= 0; i--) {
    const key = upstreams.add(current, upstreams.height + 1);
    const outer = maps[i];
    current = {
      map: outer,
      sources: (outer.sources || []).map(() => key),
      ignored: new Set(),
      fields: { sourcesContent: undefined, ignoreList: undefined },
    };
  }

  const parts = remapParts(current, upstreams);
  if (parts !== null) return parts;

  const lookup = (source) => upstreams.get(source);
  let result = wasmRemap(upstreams.serialize(current), lookup);
  for (let level = 1; level < upstreams.height; level++) {
    result = wasmRemap(result, lookup);
  }
  if (upstreams.standIns.size > 0) result = restoreStandIns(result, upstreams.standIns);
  const map = JSON.parse(result);
  return upstreams.hasValues() ? upstreams.restoreValues(map) : map;
};

/**
 * Remap/compose source maps. Takes the arguments of @jridgewell/remapping.
 *
 * Supports two calling conventions:
 * - `remapping(singleMap, loader)`: remap a single source map through a loader
 * - `remapping([map1, map2, ...], loader)`: compose an array of source maps
 *
 * @param {object | string | Array<object | string>} input
 * @param {Function} loader - (sourcefile: string, ctx: LoaderContext) => SourceMap | string | null | undefined
 * @param {boolean | { excludeContent?: boolean, decodedMappings?: boolean }} [options]
 *   `true` means `{ excludeContent: true }`.
 * @returns {SourceMap}
 */
const remapping = (input, loader, options) => {
  const opts =
    typeof options === "object" && options !== null
      ? options
      : { excludeContent: !!options, decodedMappings: false };
  return new SourceMap(remapAll(input, loader), opts);
};

export default remapping;
export { remapping, SourceMap };
