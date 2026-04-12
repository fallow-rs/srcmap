"use strict";

let SourceMap;
try {
  SourceMap = require("@srcmap/sourcemap-wasm").SourceMap;
} catch {
  // Fallback for monorepo development
  SourceMap = require("../../sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js").SourceMap;
}

// ── Constants ────────────────────────────────────────────────────

const LEAST_UPPER_BOUND = -1;
const GREATEST_LOWER_BOUND = 1;

// ── Internal helpers ─────────────────────────────────────────────

const LINE_GTR_ZERO = "`line` must be greater than 0 (lines start at line 1)";
const COL_GTR_EQ_ZERO = "`column` must be greater than or equal to 0 (columns start at column 0)";

const COLUMN = 0;
const SOURCES_INDEX = 1;
const SOURCE_LINE = 2;
const SOURCE_COLUMN = 3;
const NAMES_INDEX = 4;

const stripFilename = (path) => {
  if (!path) return "";
  const index = path.lastIndexOf("/");
  return path.slice(0, index + 1);
};

const normalizePath = (path) => {
  // Normalize /./ and /../ components in paths (matches resolve-uri behavior)
  const parts = path.split("/");
  const out = [];
  for (const part of parts) {
    if (part === ".") continue;
    if (part === ".." && out.length > 0 && out[out.length - 1] !== "..") {
      out.pop();
    } else {
      out.push(part);
    }
  }
  return out.join("/");
};

const resolver = (mapUrl, sourceRoot) => {
  const from = stripFilename(mapUrl);
  const prefix = sourceRoot ? sourceRoot + "/" : "";
  return (source) => {
    const resolved = prefix + (source || "");
    // Absolute URIs (http, https, data, webpack, node, etc.) pass through
    if (
      resolved.startsWith("http://") ||
      resolved.startsWith("https://") ||
      resolved.startsWith("/") ||
      resolved.startsWith("data:") ||
      resolved.includes("://")
    ) {
      return normalizePath(resolved);
    }
    if (!from) return normalizePath(resolved);
    return normalizePath(from + resolved);
  };
};

// ── TraceMap class ───────────────────────────────────────────────

class TraceMap {
  constructor(map, mapUrl) {
    // If already a TraceMap, extract a plain object to avoid sharing the WASM
    // pointer (Object.assign would cause double-free on .free())
    if (map instanceof TraceMap) {
      map = {
        version: map.version,
        file: map.file,
        names: [...map.names],
        sourceRoot: map.sourceRoot,
        sources: [...map.sources],
        sourcesContent: map.sourcesContent ? [...map.sourcesContent] : undefined,
        mappings: map._encoded ?? map._wasm.encodedMappings(),
        ignoreList: map.ignoreList ? [...map.ignoreList] : undefined,
      };
    }

    const parsed = typeof map === "string" ? JSON.parse(map) : map;
    const json = typeof map === "string" ? map : JSON.stringify(map);

    this._wasm = new SourceMap(json);

    const isIndexed = !!parsed.sections;

    this.version = parsed.version;
    this.file = parsed.file;

    if (isIndexed) {
      this.sources = [...this._wasm.sources];
      this.names = [...this._wasm.names];
      this.sourcesContent = [...this._wasm.sourcesContent].map((c) => c ?? null);
      this.ignoreList = this._wasm.ignoreList.length > 0 ? [...this._wasm.ignoreList] : undefined;
      this.sourceRoot = undefined;
    } else {
      this.names = parsed.names || [];
      this.sourceRoot = parsed.sourceRoot;
      this.sources = parsed.sources || [];
      this.sourcesContent = parsed.sourcesContent || undefined;
      this.ignoreList = parsed.ignoreList || parsed.x_google_ignoreList || undefined;
    }

    const resolve = resolver(mapUrl, this.sourceRoot);
    this.resolvedSources = this.sources.map(resolve);

    // Cache WASM sources array for O(1) lookups (avoids re-allocating on every access)
    this._wasmSources = [...this._wasm.sources];
    // Build source name → index map for fast reverse lookups
    this._wasmSourceMap = new Map();
    for (let i = 0; i < this._wasmSources.length; i++) {
      this._wasmSourceMap.set(this._wasmSources[i], i);
    }

    if (isIndexed) {
      this._encoded = undefined;
      this._decoded = undefined;
    } else if (typeof parsed.mappings === "string") {
      this._encoded = parsed.mappings;
      this._decoded = undefined;
    } else if (Array.isArray(parsed.mappings)) {
      this._encoded = undefined;
      this._decoded = parsed.mappings;
    } else {
      this._encoded = "";
      this._decoded = undefined;
    }
  }

  free() {
    if (this._wasm) {
      this._wasm.free();
      this._wasm = null;
    }
  }
}

// ── Free functions ───────────────────────────────────────────────

const encodedMappings = (map) => {
  if (map._encoded != null) return map._encoded;
  map._encoded = map._wasm.encodedMappings();
  return map._encoded;
};

const decodedMappings = (map) => {
  if (map._decoded != null) return map._decoded;

  const flat = map._wasm.allMappingsFlat();
  const lineCount = map._wasm.lineCount;
  const decoded = [];

  for (let i = 0; i < lineCount; i++) {
    decoded.push([]);
  }

  for (let i = 0; i < flat.length; i += 7) {
    const genLine = flat[i];
    const genCol = flat[i + 1];
    const source = flat[i + 2];
    const origLine = flat[i + 3];
    const origCol = flat[i + 4];
    const name = flat[i + 5];
    // flat[i + 6] is is_range_mapping (ignored for decoded output)

    while (decoded.length <= genLine) decoded.push([]);

    if (source === -1) {
      decoded[genLine].push([genCol]);
    } else if (name === -1) {
      decoded[genLine].push([genCol, source, origLine, origCol]);
    } else {
      decoded[genLine].push([genCol, source, origLine, origCol, name]);
    }
  }

  map._decoded = decoded;
  return decoded;
};

const traceSegment = (map, line, column) => {
  const decoded = decodedMappings(map);
  if (line >= decoded.length) return null;
  const segments = decoded[line];
  const index = binarySearch(segments, column);
  if (index === -1) return null;
  return segments[index];
};

const originalPositionFor = (map, needle) => {
  // Auto-wrap duck-typed objects (e.g. Vite's DecodedMap) into a TraceMap
  if (!(map instanceof TraceMap) && !map._wasm) {
    map = new TraceMap(map);
  }

  let { line, column, bias } = needle;
  line--;
  if (line < 0) throw new Error(LINE_GTR_ZERO);
  if (column < 0) throw new Error(COL_GTR_EQ_ZERO);

  if (!bias || bias === GREATEST_LOWER_BOUND) {
    const result = map._wasm.originalPositionFor(line, column);
    if (result === null || result === undefined) {
      return { source: null, line: null, column: null, name: null };
    }

    // Map WASM source name to resolvedSources
    let source = result.source ?? null;
    if (source !== null) {
      const idx = map._wasmSourceMap.get(source);
      if (idx !== undefined) source = map.resolvedSources[idx];
    }

    return {
      source,
      line: result.line != null ? result.line + 1 : null,
      column: result.column ?? null,
      name: result.name ?? null,
    };
  }

  const decoded = decodedMappings(map);
  if (line >= decoded.length) return { source: null, line: null, column: null, name: null };

  const segments = decoded[line];
  const index = binarySearchLUB(segments, column);
  if (index === -1 || index >= segments.length) {
    return { source: null, line: null, column: null, name: null };
  }

  const segment = segments[index];
  if (segment.length === 1) return { source: null, line: null, column: null, name: null };

  return {
    source: map.resolvedSources[segment[SOURCES_INDEX]],
    line: segment[SOURCE_LINE] + 1,
    column: segment[SOURCE_COLUMN],
    name: segment.length === 5 ? map.names[segment[NAMES_INDEX]] : null,
  };
};

const generatedPositionFor = (map, needle) => {
  if (!(map instanceof TraceMap) && !map._wasm) {
    map = new TraceMap(map);
  }
  const { source, line, column, bias } = needle;
  if (line < 1) throw new Error(LINE_GTR_ZERO);
  if (column < 0) throw new Error(COL_GTR_EQ_ZERO);

  const resolvedSource = resolveSourceName(map, source);
  if (resolvedSource === null) return { line: null, column: null };

  // WASM bias convention: 0 = GLB (default), -1 = LUB
  const wasmBias = bias === LEAST_UPPER_BOUND ? -1 : 0;
  const result = map._wasm.generatedPositionForWithBias(resolvedSource, line - 1, column, wasmBias);
  if (result === null || result === undefined) {
    return { line: null, column: null };
  }

  return {
    line: result.line != null ? result.line + 1 : null,
    column: result.column ?? null,
  };
};

const allGeneratedPositionsFor = (map, needle) => {
  if (!(map instanceof TraceMap) && !map._wasm) {
    map = new TraceMap(map);
  }
  const { source, line, column } = needle;
  if (line < 1) throw new Error(LINE_GTR_ZERO);
  if (column < 0) throw new Error(COL_GTR_EQ_ZERO);

  const resolvedSource = resolveSourceName(map, source);
  if (resolvedSource === null) return [];

  const results = map._wasm.allGeneratedPositionsFor(resolvedSource, line - 1, column);
  return results.map((r) => ({
    line: r.line != null ? r.line + 1 : null,
    column: r.column ?? null,
  }));
};

const eachMapping = (map, cb) => {
  if (!(map instanceof TraceMap) && !map._wasm) {
    map = new TraceMap(map);
  }
  const decoded = decodedMappings(map);
  const { names, resolvedSources } = map;

  for (let i = 0; i < decoded.length; i++) {
    const line = decoded[i];
    for (let j = 0; j < line.length; j++) {
      const seg = line[j];
      const generatedLine = i + 1;
      const generatedColumn = seg[COLUMN];
      let source = null;
      let originalLine = null;
      let originalColumn = null;
      let name = null;

      if (seg.length !== 1) {
        source = resolvedSources[seg[SOURCES_INDEX]];
        originalLine = seg[SOURCE_LINE] + 1;
        originalColumn = seg[SOURCE_COLUMN];
      }
      if (seg.length === 5) name = names[seg[NAMES_INDEX]];

      cb({
        generatedLine,
        generatedColumn,
        source,
        originalLine,
        originalColumn,
        name,
      });
    }
  }
};

const sourceContentFor = (map, source) => {
  const { sourcesContent } = map;
  if (sourcesContent == null) return null;
  const index = sourceIndexOf(map, source);
  if (index === -1) return null;
  return sourcesContent[index] ?? null;
};

const isIgnored = (map, source) => {
  const { ignoreList } = map;
  if (ignoreList == null) return false;
  const index = sourceIndexOf(map, source);
  if (index === -1) return false;
  return ignoreList.includes(index);
};

const presortedDecodedMap = (map, mapUrl) => {
  const encoded = encodeDecodedMappings(map.mappings);
  const raw = {
    version: map.version,
    file: map.file,
    names: map.names,
    sourceRoot: map.sourceRoot,
    sources: map.sources,
    sourcesContent: map.sourcesContent,
    mappings: encoded,
    ignoreList: map.ignoreList,
  };
  const tracer = new TraceMap(raw, mapUrl);
  tracer._decoded = map.mappings;
  return tracer;
};

const decodedMap = (map) => ({
  version: map.version,
  file: map.file,
  names: map.names,
  sourceRoot: map.sourceRoot,
  sources: map.sources,
  sourcesContent: map.sourcesContent,
  mappings: decodedMappings(map),
  ignoreList: map.ignoreList,
});

const encodedMap = (map) => ({
  version: map.version,
  file: map.file,
  names: map.names,
  sourceRoot: map.sourceRoot,
  sources: map.sources,
  sourcesContent: map.sourcesContent,
  mappings: encodedMappings(map),
  ignoreList: map.ignoreList,
});

const FlattenMap = TraceMap;
const AnyMap = TraceMap;

// ── Internal helpers ─────────────────────────────────────────────

const sourceIndexOf = (map, source) => {
  let index = map.sources.indexOf(source);
  if (index === -1) index = map.resolvedSources.indexOf(source);
  return index;
};

const resolveSourceName = (map, source) => {
  // Try direct match against cached WASM sources (O(1) via Map)
  if (map._wasmSourceMap.has(source)) return source;

  // Try matching via index (raw sources → WASM sources)
  let index = map.sources.indexOf(source);
  if (index === -1) index = map.resolvedSources.indexOf(source);
  if (index === -1) return null;

  return map._wasmSources[index] ?? null;
};

const binarySearch = (segments, column) => {
  let low = 0;
  let high = segments.length - 1;
  let result = -1;

  while (low <= high) {
    const mid = low + ((high - low) >> 1);
    const midCol = segments[mid][COLUMN];

    if (midCol === column) {
      result = mid;
      low = mid + 1;
    } else if (midCol < column) {
      result = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }

  return result;
};

const binarySearchLUB = (segments, column) => {
  let low = 0;
  let high = segments.length - 1;
  let result = -1;

  while (low <= high) {
    const mid = low + ((high - low) >> 1);
    const midCol = segments[mid][COLUMN];

    if (midCol === column) {
      result = mid;
      high = mid - 1;
    } else if (midCol > column) {
      result = mid;
      high = mid - 1;
    } else {
      low = mid + 1;
    }
  }

  return result;
};

const B64_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const vlqEncode = (value) => {
  let vlq = value < 0 ? (-value << 1) + 1 : value << 1;
  let result = "";
  do {
    let digit = vlq & 0x1f;
    vlq >>>= 5;
    if (vlq > 0) digit |= 0x20;
    result += B64_CHARS[digit];
  } while (vlq > 0);
  return result;
};

const encodeDecodedMappings = (decoded) => {
  const parts = [];

  for (let i = 0; i < decoded.length; i++) {
    const line = decoded[i];
    if (line.length === 0) {
      parts.push("");
      continue;
    }

    const segments = [];
    let prevCol = 0;
    let prevSource = 0;
    let prevOrigLine = 0;
    let prevOrigCol = 0;
    let prevName = 0;

    for (const seg of line) {
      let encoded = vlqEncode(seg[COLUMN] - prevCol);
      prevCol = seg[COLUMN];

      if (seg.length > 1) {
        encoded += vlqEncode(seg[SOURCES_INDEX] - prevSource);
        prevSource = seg[SOURCES_INDEX];
        encoded += vlqEncode(seg[SOURCE_LINE] - prevOrigLine);
        prevOrigLine = seg[SOURCE_LINE];
        encoded += vlqEncode(seg[SOURCE_COLUMN] - prevOrigCol);
        prevOrigCol = seg[SOURCE_COLUMN];

        if (seg.length === 5) {
          encoded += vlqEncode(seg[NAMES_INDEX] - prevName);
          prevName = seg[NAMES_INDEX];
        }
      }

      segments.push(encoded);
    }

    parts.push(segments.join(","));
  }

  return parts.join(";");
};

module.exports = {
  LEAST_UPPER_BOUND,
  GREATEST_LOWER_BOUND,
  TraceMap,
  AnyMap,
  FlattenMap,
  encodedMappings,
  decodedMappings,
  traceSegment,
  originalPositionFor,
  generatedPositionFor,
  allGeneratedPositionsFor,
  eachMapping,
  sourceContentFor,
  isIgnored,
  presortedDecodedMap,
  decodedMap,
  encodedMap,
};
