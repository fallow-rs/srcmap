import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { remap: wasmRemap } = require("@srcmap/remapping-wasm");
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

  toJson(node) {
    const sources = node.sources.map((source, i) =>
      node.ignored.has(i) && this.needsStandIn(source) ? this.standIn(source) : source,
    );
    return toWasmJson(node.map, { ...node.fields, sources });
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
