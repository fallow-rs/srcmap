import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { remap: wasmRemap } = require("@srcmap/remapping-wasm");

// ── SourceMap class ─────────────────────────────────────────────

/**
 * Source map result class with the same interface as @jridgewell/remapping's SourceMap.
 * Provides .version, .file, .mappings, .names, .sources, .sourcesContent,
 * plus toString() and toJSON() methods.
 */
class SourceMap {
  /**
   * @param {object} raw - Parsed source map object
   */
  constructor(raw) {
    this.version = raw.version;
    this.file = raw.file ?? undefined;
    this.mappings = raw.mappings;
    this.names = raw.names || [];
    this.sources = raw.sources || [];
    this.sourcesContent = raw.sourcesContent || undefined;
    this.sourceRoot = raw.sourceRoot;
    this.ignoreList = raw.ignoreList;
  }

  toString() {
    return JSON.stringify(this);
  }

  toJSON() {
    const result = {
      version: this.version,
      file: this.file,
      mappings: this.mappings,
      names: this.names,
      sources: this.sources,
      sourcesContent: this.sourcesContent,
    };
    if (this.sourceRoot != null) {
      result.sourceRoot = this.sourceRoot;
    }
    if (this.ignoreList != null && this.ignoreList.length > 0) {
      result.ignoreList = this.ignoreList;
    }
    return result;
  }
}

// ── Helpers ──────────────────────────────────────────────────────

/**
 * Convert a source map input (object, string, or SourceMap) to a JSON string
 * suitable for the WASM remap function.
 * @param {object | string} input
 * @returns {string}
 */
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
  for (const line of decoded) {
    const segments = [];
    let prevCol = 0,
      prevSource = 0,
      prevOrigLine = 0,
      prevOrigCol = 0,
      prevName = 0;
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

const toJsonString = (input) => {
  if (typeof input === "string") return input;
  if (input && Array.isArray(input.mappings)) {
    return JSON.stringify({ ...input, mappings: encodeMappings(input.mappings) });
  }
  return JSON.stringify(input);
};

/**
 * Wrap the user's loader to convert its return values to JSON strings.
 * @param {Function} loader
 * @returns {Function}
 */
const wrapLoader = (loader) => (source) => {
  const result = loader(source);
  if (result == null) return null;
  return toJsonString(result);
};

const remapArray = (maps, loader) => {
  if (maps.length === 0) {
    return JSON.stringify({ version: 3, sources: [], names: [], mappings: "" });
  }

  if (maps.length === 1) {
    return wasmRemap(toJsonString(maps[0]), wrapLoader(loader));
  }

  // For an array [map0, map1, map2, ...], map0 is the outermost (final stage)
  // and mapN is the innermost (closest to original source).
  // We remap map0 through map1, then that result through map2, etc.
  let current = toJsonString(maps[0]);
  for (let i = 1; i < maps.length; i++) {
    const innerMap = toJsonString(maps[i]);
    current = wasmRemap(current, () => innerMap);
  }

  // After composing the chain, do a final remap pass with the user's loader
  // to resolve any remaining upstream sources.
  return wasmRemap(current, wrapLoader(loader));
};

// ── Main API ────────────────────────────────────────────────────

/**
 * Remap/compose source maps. Drop-in replacement for @jridgewell/remapping.
 *
 * Supports two calling conventions:
 * - `remapping(singleMap, loader)` — remap a single source map through a loader
 * - `remapping([map1, map2, ...], loader)` — compose an array of source maps
 *
 * @param {object | string | Array<object | string>} input
 * @param {Function} loader - (sourcefile: string) => SourceMap | string | null | undefined
 * @param {object} [options]
 * @returns {SourceMap}
 */
const remapping = (input, loader, _options) => {
  let resultJson;

  if (Array.isArray(input)) {
    resultJson = remapArray(input, loader);
  } else {
    resultJson = wasmRemap(toJsonString(input), wrapLoader(loader));
  }

  const parsed = JSON.parse(resultJson);
  return new SourceMap(parsed);
};

export default remapping;
export { remapping, SourceMap };
