'use strict'

let wasmRemap
try {
  wasmRemap = require('@srcmap/remapping-wasm').remap
} catch {
  // Fallback for monorepo development
  wasmRemap = require('../../remapping-wasm/pkg/srcmap_remapping_wasm.js').remap
}

// ── SourceMap class ─────────────────────────────────────────────

/**
 * Source map result class with the same interface as @jridgewell/remapping's SourceMap.
 */
class SourceMap {
  constructor(raw) {
    this.version = raw.version
    this.file = raw.file ?? undefined
    this.mappings = raw.mappings
    this.names = raw.names || []
    this.sources = raw.sources || []
    this.sourcesContent = raw.sourcesContent || undefined
    this.sourceRoot = raw.sourceRoot
    this.ignoreList = raw.ignoreList
  }

  toString() {
    return JSON.stringify(this)
  }

  toJSON() {
    const result = {
      version: this.version,
      file: this.file,
      mappings: this.mappings,
      names: this.names,
      sources: this.sources,
      sourcesContent: this.sourcesContent,
    }
    if (this.sourceRoot != null) {
      result.sourceRoot = this.sourceRoot
    }
    if (this.ignoreList != null && this.ignoreList.length > 0) {
      result.ignoreList = this.ignoreList
    }
    return result
  }
}

// ── Helpers ──────────────────────────────────────────────────────

const toJsonString = (input) => {
  if (typeof input === 'string') return input
  return JSON.stringify(input)
}

const wrapLoader = (loader) => (source) => {
  const result = loader(source)
  if (result == null) return null
  return toJsonString(result)
}

const remapArray = (maps, loader) => {
  if (maps.length === 0) {
    return JSON.stringify({ version: 3, sources: [], names: [], mappings: '' })
  }

  if (maps.length === 1) {
    return wasmRemap(toJsonString(maps[0]), wrapLoader(loader))
  }

  let current = toJsonString(maps[0])
  for (let i = 1; i < maps.length; i++) {
    const innerMap = toJsonString(maps[i])
    current = wasmRemap(current, () => innerMap)
  }

  return wasmRemap(current, wrapLoader(loader))
}

// ── Main API ────────────────────────────────────────────────────

const remapping = (input, loader, options) => {
  let resultJson

  if (Array.isArray(input)) {
    resultJson = remapArray(input, loader)
  } else {
    resultJson = wasmRemap(toJsonString(input), wrapLoader(loader))
  }

  const parsed = JSON.parse(resultJson)
  return new SourceMap(parsed)
}

module.exports = remapping
module.exports.default = remapping
module.exports.remapping = remapping
module.exports.SourceMap = SourceMap
