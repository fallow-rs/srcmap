"use strict";

let SourceMapGenerator;
try {
  SourceMapGenerator = require("@srcmap/generator-wasm").SourceMapGenerator;
} catch {
  SourceMapGenerator =
    require("../../generator-wasm/pkg/srcmap_generator_wasm.js").SourceMapGenerator;
}

let SourceMap;
try {
  SourceMap = require("@srcmap/sourcemap-wasm").SourceMap;
} catch {
  SourceMap = require("../../sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js").SourceMap;
}

// ── Internal constants ──────────────────────────────────────────

const NO_NAME = -1;

// ── GenMapping class ────────────────────────────────────────────

class GenMapping {
  constructor({ file, sourceRoot } = {}) {
    this._wasm = new SourceMapGenerator(file ?? undefined);
    if (sourceRoot) this._wasm.setSourceRoot(sourceRoot);

    this.file = file ?? undefined;
    this.sourceRoot = sourceRoot ?? undefined;

    this._sources = [];
    this._sourceIndexMap = new Map();
    this._names = [];
    this._nameIndexMap = new Map();
    this._sourcesContent = [];

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
}

// ── Internal helpers ────────────────────────────────────────────

const putSource = (map, source) => {
  const existing = map._sourceIndexMap.get(source);
  if (existing !== undefined) return existing;
  const idx = map._wasm.addSource(source);
  map._sources.push(source);
  map._sourceIndexMap.set(source, idx);
  if (map._sourcesContent.length <= idx) {
    map._sourcesContent[idx] = null;
  }
  return idx;
};

const putName = (map, name) => {
  const existing = map._nameIndexMap.get(name);
  if (existing !== undefined) return existing;
  const idx = map._wasm.addName(name);
  map._names.push(name);
  map._nameIndexMap.set(name, idx);
  return idx;
};

const addMappingInternal = (skippable, map, mapping) => {
  const { generated, source, original, name, content } = mapping;
  const genLine = generated.line - 1;
  const genColumn = generated.column;

  if (!source) {
    if (skippable) {
      if (map._lastLine === genLine && map._lastWasSourceless) return;
      if (map._lastLine !== genLine) {
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

const addMapping = (map, mapping) => {
  addMappingInternal(false, map, mapping);
};

const maybeAddMapping = (map, mapping) => {
  addMappingInternal(true, map, mapping);
};

const setSourceContent = (map, source, content) => {
  const idx = putSource(map, source);
  map._sourcesContent[idx] = content;
  if (content != null) {
    map._wasm.setSourceContent(idx, content);
  }
};

const setIgnore = (map, source, ignore = true) => {
  if (!ignore) return;
  const idx = putSource(map, source);
  map._wasm.addToIgnoreList(idx);
};

const allMappings = (map) => {
  const json = map._wasm.toJSON();
  const parsed = JSON.parse(json);
  const sm = new SourceMap(json);

  try {
    const flat = sm.allMappingsFlat();
    const out = [];
    const sources = parsed.sources || [];
    const names = parsed.names || [];

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

const toEncodedMap = (map) => {
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

const toDecodedMap = (map) => {
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

const fromMap = (input) => {
  const parsed = typeof input === "string" ? JSON.parse(input) : input;
  const gen = new GenMapping({
    file: parsed.file,
    sourceRoot: parsed.sourceRoot,
  });

  const sources = parsed.sources || [];
  const names = parsed.names || [];
  const sourcesContent = parsed.sourcesContent || [];

  for (let i = 0; i < sources.length; i++) {
    putSource(gen, sources[i]);
    if (sourcesContent[i] != null) {
      gen._sourcesContent[i] = sourcesContent[i];
      gen._wasm.setSourceContent(i, sourcesContent[i]);
    }
  }

  for (let i = 0; i < names.length; i++) {
    putName(gen, names[i]);
  }

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

  if (parsed.ignoreList) {
    for (const idx of parsed.ignoreList) {
      gen._wasm.addToIgnoreList(idx);
    }
  }

  return gen;
};

module.exports = {
  GenMapping,
  addMapping,
  maybeAddMapping,
  setSourceContent,
  setIgnore,
  allMappings,
  toEncodedMap,
  toDecodedMap,
  fromMap,
};
