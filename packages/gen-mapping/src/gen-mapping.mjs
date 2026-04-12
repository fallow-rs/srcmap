import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { SourceMapGenerator } = require("@srcmap/generator-wasm");
const { SourceMap } = require("@srcmap/sourcemap-wasm");

// ── Internal constants ──────────────────────────────────────────

const NO_NAME = -1;

// ── GenMapping class ────────────────────────────────────────────

export class GenMapping {
  /**
   * @param {{ file?: string | null, sourceRoot?: string | null }} [opts]
   */
  constructor({ file, sourceRoot } = {}) {
    this._wasm = new SourceMapGenerator(file ?? undefined);
    if (sourceRoot) this._wasm.setSourceRoot(sourceRoot);

    this.file = file ?? undefined;
    this.sourceRoot = sourceRoot ?? undefined;

    // JS-side tracking for source name → index mapping
    this._sources = [];
    this._sourceIndexMap = new Map();
    this._names = [];
    this._nameIndexMap = new Map();
    this._sourcesContent = [];

    // Dedup state for maybeAddMapping (per generated line)
    this._lastLine = -1;
    this._lastSourcesIndex = -1;
    this._lastSourceLine = -1;
    this._lastSourceColumn = -1;
    this._lastNamesIndex = NO_NAME;
    this._lastWasSourceless = false;
  }

  free() {
    if (this._wasm) {
      this._wasm.free();
      this._wasm = null;
    }
  }

  [Symbol.dispose]() {
    this.free();
  }
}

// ── Internal helpers ────────────────────────────────────────────

/**
 * Register a source and return its index, deduplicating by name.
 * @param {GenMapping} map
 * @param {string} source
 * @returns {number}
 */
const putSource = (map, source) => {
  const existing = map._sourceIndexMap.get(source);
  if (existing !== undefined) return existing;
  const idx = map._wasm.addSource(source);
  map._sources.push(source);
  map._sourceIndexMap.set(source, idx);
  // Ensure sourcesContent array is in sync
  if (map._sourcesContent.length <= idx) {
    map._sourcesContent[idx] = null;
  }
  return idx;
};

/**
 * Register a name and return its index, deduplicating by name.
 * @param {GenMapping} map
 * @param {string} name
 * @returns {number}
 */
const putName = (map, name) => {
  const existing = map._nameIndexMap.get(name);
  if (existing !== undefined) return existing;
  const idx = map._wasm.addName(name);
  map._names.push(name);
  map._nameIndexMap.set(name, idx);
  return idx;
};

/**
 * Add a mapping through the internal path, with optional dedup.
 * @param {boolean} skippable - Whether to skip redundant mappings
 * @param {GenMapping} map
 * @param {object} mapping
 */
const addMappingInternal = (skippable, map, mapping) => {
  const { generated, source, original, name, content } = mapping;
  const genLine = generated.line - 1;
  const genColumn = generated.column;

  if (!source) {
    if (skippable) {
      // Skip sourceless mapping if at start of line or previous was also sourceless
      if (map._lastLine === genLine && map._lastWasSourceless) return;
      if (map._lastLine !== genLine) {
        // First mapping on a new line — sourceless at index 0 is skippable
        map._lastLine = genLine;
        map._lastWasSourceless = true;
        map._lastSourcesIndex = -1;
        map._lastSourceLine = -1;
        map._lastSourceColumn = -1;
        map._lastNamesIndex = NO_NAME;
        return;
      }
    }
    map._wasm.addGeneratedMapping(genLine, genColumn);
    map._lastLine = genLine;
    map._lastWasSourceless = true;
    map._lastSourcesIndex = -1;
    map._lastSourceLine = -1;
    map._lastSourceColumn = -1;
    map._lastNamesIndex = NO_NAME;
    return;
  }

  const sourcesIndex = putSource(map, source);
  const sourceLine = original.line - 1;
  const sourceColumn = original.column;
  const namesIndex = name ? putName(map, name) : NO_NAME;

  if (content !== undefined && content !== null) {
    map._sourcesContent[sourcesIndex] = content;
    map._wasm.setSourceContent(sourcesIndex, content);
  }

  if (skippable) {
    if (map._lastLine === genLine && !map._lastWasSourceless) {
      // Previous was a source mapping on the same line — check for dups
      if (
        sourcesIndex === map._lastSourcesIndex &&
        sourceLine === map._lastSourceLine &&
        sourceColumn === map._lastSourceColumn &&
        namesIndex === map._lastNamesIndex
      ) {
        return;
      }
    }
  }

  if (namesIndex !== NO_NAME) {
    map._wasm.addNamedMapping(
      genLine,
      genColumn,
      sourcesIndex,
      sourceLine,
      sourceColumn,
      namesIndex,
    );
  } else {
    map._wasm.addMapping(genLine, genColumn, sourcesIndex, sourceLine, sourceColumn);
  }

  map._lastLine = genLine;
  map._lastWasSourceless = false;
  map._lastSourcesIndex = sourcesIndex;
  map._lastSourceLine = sourceLine;
  map._lastSourceColumn = sourceColumn;
  map._lastNamesIndex = namesIndex;
};

// ── Free functions ──────────────────────────────────────────────

/**
 * Add a mapping to the source map.
 * Lines are 1-based, columns are 0-based.
 * @param {GenMapping} map
 * @param {{ generated: { line: number, column: number }, source?: string, original?: { line: number, column: number }, name?: string, content?: string | null }} mapping
 */
export const addMapping = (map, mapping) => {
  addMappingInternal(false, map, mapping);
};

/**
 * Add a mapping only if it differs from the previous mapping on the same line.
 * Requires mappings to be added in order.
 * @param {GenMapping} map
 * @param {{ generated: { line: number, column: number }, source?: string, original?: { line: number, column: number }, name?: string, content?: string | null }} mapping
 */
export const maybeAddMapping = (map, mapping) => {
  addMappingInternal(true, map, mapping);
};

/**
 * Set the source content for a source file by source name.
 * @param {GenMapping} map
 * @param {string} source - Source filename
 * @param {string | null} content - Source content
 */
export const setSourceContent = (map, source, content) => {
  const idx = putSource(map, source);
  map._sourcesContent[idx] = content;
  if (content != null) {
    map._wasm.setSourceContent(idx, content);
  }
};

/**
 * Mark a source as ignored (or not).
 * @param {GenMapping} map
 * @param {string} source - Source filename
 * @param {boolean} [ignore=true] - Whether to ignore
 */
export const setIgnore = (map, source, ignore = true) => {
  if (!ignore) return;
  const idx = putSource(map, source);
  map._wasm.addToIgnoreList(idx);
};

/**
 * Return all mappings as an array of Mapping objects.
 * Lines are 1-based, columns are 0-based.
 * @param {GenMapping} map
 * @returns {Array<{ generated: { line: number, column: number }, source?: string, original?: { line: number, column: number }, name?: string }>}
 */
export const allMappings = (map) => {
  const json = map._wasm.toJSON();
  const parsed = JSON.parse(json);
  const sm = new SourceMap(json);

  try {
    const flat = sm.allMappingsFlat();
    const out = [];
    const sources = parsed.sources || [];
    const names = parsed.names || [];

    // flat format: [genLine, genCol, source, origLine, origCol, name, isRange, ...]
    for (let i = 0; i < flat.length; i += 7) {
      const genLine = flat[i];
      const genCol = flat[i + 1];
      const sourceIdx = flat[i + 2];
      const origLine = flat[i + 3];
      const origCol = flat[i + 4];
      const nameIdx = flat[i + 5];

      const generated = { line: genLine + 1, column: genCol };
      let source, original, name;

      if (sourceIdx !== -1) {
        source = sources[sourceIdx];
        original = { line: origLine + 1, column: origCol };
        if (nameIdx !== -1) {
          name = names[nameIdx];
        }
      }

      out.push({ generated, source, original, name });
    }

    return out;
  } finally {
    sm.free();
  }
};

/**
 * Return the source map as an encoded source map object (with VLQ string mappings).
 * @param {GenMapping} map
 * @returns {object}
 */
export const toEncodedMap = (map) => {
  const json = map._wasm.toJSON();
  const parsed = JSON.parse(json);

  return {
    version: 3,
    file: parsed.file || undefined,
    sourceRoot: parsed.sourceRoot || undefined,
    sources: parsed.sources || [],
    sourcesContent:
      parsed.sourcesContent || map._sourcesContent.slice(0, (parsed.sources || []).length),
    names: parsed.names || [],
    mappings: parsed.mappings || "",
    ignoreList: parsed.ignoreList || [],
  };
};

/**
 * Return the source map as a decoded source map object (with decoded mappings array).
 * @param {GenMapping} map
 * @returns {object}
 */
export const toDecodedMap = (map) => {
  const json = map._wasm.toJSON();
  const parsed = JSON.parse(json);
  const sm = new SourceMap(json);

  try {
    const flat = sm.allMappingsFlat();
    const lineCount = sm.lineCount;
    const decoded = [];

    for (let i = 0; i < lineCount; i++) {
      decoded.push([]);
    }

    // flat format: [genLine, genCol, source, origLine, origCol, name, isRange, ...]
    for (let i = 0; i < flat.length; i += 7) {
      const genLine = flat[i];
      const genCol = flat[i + 1];
      const sourceIdx = flat[i + 2];
      const origLine = flat[i + 3];
      const origCol = flat[i + 4];
      const nameIdx = flat[i + 5];

      while (decoded.length <= genLine) decoded.push([]);

      if (sourceIdx === -1) {
        decoded[genLine].push([genCol]);
      } else if (nameIdx === -1) {
        decoded[genLine].push([genCol, sourceIdx, origLine, origCol]);
      } else {
        decoded[genLine].push([genCol, sourceIdx, origLine, origCol, nameIdx]);
      }
    }

    // Remove trailing empty lines (match jridgewell behavior)
    while (decoded.length > 0 && decoded[decoded.length - 1].length === 0) {
      decoded.pop();
    }

    return {
      version: 3,
      file: parsed.file || undefined,
      sourceRoot: parsed.sourceRoot || undefined,
      sources: parsed.sources || [],
      sourcesContent:
        parsed.sourcesContent || map._sourcesContent.slice(0, (parsed.sources || []).length),
      names: parsed.names || [],
      mappings: decoded,
      ignoreList: parsed.ignoreList || [],
    };
  } finally {
    sm.free();
  }
};

/**
 * Construct a GenMapping from an existing source map input.
 * @param {string | object} input - Source map JSON string or object
 * @returns {GenMapping}
 */
export const fromMap = (input) => {
  const parsed = typeof input === "string" ? JSON.parse(input) : input;
  const gen = new GenMapping({
    file: parsed.file,
    sourceRoot: parsed.sourceRoot,
  });

  const sources = parsed.sources || [];
  const names = parsed.names || [];
  const sourcesContent = parsed.sourcesContent || [];

  // Register all sources
  for (let i = 0; i < sources.length; i++) {
    putSource(gen, sources[i]);
    if (sourcesContent[i] != null) {
      gen._sourcesContent[i] = sourcesContent[i];
      gen._wasm.setSourceContent(i, sourcesContent[i]);
    }
  }

  // Register all names
  for (let i = 0; i < names.length; i++) {
    putName(gen, names[i]);
  }

  // Parse the source map to get decoded mappings
  const json = typeof input === "string" ? input : JSON.stringify(input);
  const sm = new SourceMap(json);

  try {
    const flat = sm.allMappingsFlat();

    for (let i = 0; i < flat.length; i += 7) {
      const genLine = flat[i];
      const genCol = flat[i + 1];
      const sourceIdx = flat[i + 2];
      const origLine = flat[i + 3];
      const origCol = flat[i + 4];
      const nameIdx = flat[i + 5];

      if (sourceIdx === -1) {
        gen._wasm.addGeneratedMapping(genLine, genCol);
      } else if (nameIdx === -1) {
        gen._wasm.addMapping(genLine, genCol, sourceIdx, origLine, origCol);
      } else {
        gen._wasm.addNamedMapping(genLine, genCol, sourceIdx, origLine, origCol, nameIdx);
      }
    }
  } finally {
    sm.free();
  }

  // Handle ignoreList
  if (parsed.ignoreList) {
    for (const idx of parsed.ignoreList) {
      gen._wasm.addToIgnoreList(idx);
    }
  }

  return gen;
};
