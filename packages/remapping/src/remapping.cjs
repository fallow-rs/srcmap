"use strict";

let wasmRemap;
try {
  wasmRemap = require("@srcmap/remapping-wasm").remap;
} catch {
  // Fallback for monorepo development
  wasmRemap = require("../../remapping-wasm/pkg/srcmap_remapping_wasm.js").remap;
}
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

/**
 * The maps of one `remapping` call. Each map that the loader gives, and each outer
 * map of a chain, gets a unique key. The key is the source name of that map in the
 * map above it, so the WASM remap can find the map by key.
 */
class UpstreamMaps {
  constructor() {
    this.prefix = `\u0000srcmap-upstream-${Math.random().toString(36).slice(2)}-`;
    this.maps = new Map();
    this.height = 0;
    this.ignoredEmpty = null;
  }

  // An ignored empty source gets a stand-in name, so that it stays a real source
  // through each pass. `restoreIgnoredEmpty` turns it back into `""`.
  ignoredEmptyName() {
    this.ignoredEmpty ??= this.prefix + "ignored-empty";
    return this.ignoredEmpty;
  }

  add(json, depth) {
    const key = this.prefix + this.maps.size;
    this.maps.set(key, json);
    this.height = Math.max(this.height, depth);
    return key;
  }

  get(key) {
    return this.maps.get(key) ?? null;
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
  const ignored = [];
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
    if (!(ctx.ignore !== undefined ? ctx.ignore : ignoreList.includes(i))) return ctx.source;
    ignored.push(i);
    return ctx.source === "" ? upstreams.ignoredEmptyName() : ctx.source;
  });
  return toWasmJson(map, { sources, sourcesContent, ignoreList: ignored });
};

// jridgewell adds an ignored empty source to `sources` and `ignoreList`, and makes
// its segments sourceless. A last WASM pass over the empty name does the same.
const restoreIgnoredEmpty = (json, name) => {
  const map = JSON.parse(json);
  map.sources = map.sources.map((source) => (source === name ? "" : source));
  return wasmRemap(JSON.stringify(map), NO_UPSTREAM);
};

// A WASM remap pass resolves one level of keys, so the remap runs once per level.
const remapAll = (input, loader) => {
  const maps = (Array.isArray(input) ? input : [input]).map(toRawMap);
  checkTransformationMaps(maps);
  if (maps.length === 0) {
    return JSON.stringify({ version: 3, sources: [], names: [], mappings: "" });
  }

  const upstreams = new UpstreamMaps();
  let current = buildTree(maps.at(-1), loader, "", 0, upstreams);
  for (let i = maps.length - 2; i >= 0; i--) {
    const key = upstreams.add(current, upstreams.height + 1);
    const outer = maps[i];
    current = toWasmJson(outer, {
      sources: (outer.sources || []).map(() => key),
      sourcesContent: undefined,
      ignoreList: undefined,
    });
  }

  const lookup = (source) => upstreams.get(source);
  let result = wasmRemap(current, lookup);
  for (let level = 1; level < upstreams.height; level++) {
    result = wasmRemap(result, lookup);
  }
  return upstreams.ignoredEmpty ? restoreIgnoredEmpty(result, upstreams.ignoredEmpty) : result;
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
  return new SourceMap(JSON.parse(remapAll(input, loader)), opts);
};

module.exports = remapping;
module.exports.default = remapping;
module.exports.remapping = remapping;
module.exports.SourceMap = SourceMap;
