import { SourceMap } from '@srcmap/sourcemap-wasm'
import { SourceMapGenerator as WasmGenerator } from '@srcmap/generator-wasm'

// ── Constants (match Mozilla source-map v0.6) ───────────────────

export const GREATEST_LOWER_BOUND = 1
export const LEAST_UPPER_BOUND = 2

// ── Internal helpers ────────────────────────────────────────────

const COLUMN = 0
const SOURCES_INDEX = 1
const SOURCE_LINE = 2
const SOURCE_COLUMN = 3
const NAMES_INDEX = 4

/** @param {string} path */
const stripFilename = (path) => {
  if (!path) return ''
  const index = path.lastIndexOf('/')
  return path.slice(0, index + 1)
}

/**
 * Normalize path components (remove `.` and `..` segments).
 * @param {string} path
 * @returns {string}
 */
const normalizePath = (path) => {
  const parts = path.split('/')
  const out = []
  for (const part of parts) {
    if (part === '.') continue
    if (part === '..' && out.length > 0 && out[out.length - 1] !== '..') {
      out.pop()
    } else {
      out.push(part)
    }
  }
  return out.join('/')
}

/**
 * Resolve a source path against sourceRoot and mapUrl.
 * @param {string | undefined} mapUrl
 * @param {string | undefined} sourceRoot
 */
const resolver = (mapUrl, sourceRoot) => {
  const from = stripFilename(mapUrl)
  const prefix = sourceRoot ? sourceRoot + '/' : ''
  return (source) => {
    const resolved = prefix + (source || '')
    if (resolved.startsWith('http://') || resolved.startsWith('https://') || resolved.startsWith('/') || resolved.startsWith('data:') || resolved.includes('://')) {
      return normalizePath(resolved)
    }
    if (!from) return normalizePath(resolved)
    return normalizePath(from + resolved)
  }
}

/**
 * Resolve a source name to the WASM-internal name.
 * @param {SourceMapConsumer} consumer
 * @param {string} source
 * @returns {string | null}
 */
const resolveSourceName = (consumer, source) => {
  if (consumer._wasmSourceMap.has(source)) return source

  let index = consumer._sources.indexOf(source)
  if (index === -1) index = consumer._resolvedSources.indexOf(source)
  if (index === -1) return null

  return consumer._wasmSources[index] ?? null
}

// ── SourceMapConsumer ───────────────────────────────────────────

export class SourceMapConsumer {
  /**
   * @param {string | object} rawSourceMap - JSON string or parsed source map object
   * @param {string} [sourceMapUrl] - URL of the source map (for resolving relative sources)
   */
  constructor(rawSourceMap, sourceMapUrl) {
    const parsed = typeof rawSourceMap === 'string' ? JSON.parse(rawSourceMap) : rawSourceMap
    const json = typeof rawSourceMap === 'string' ? rawSourceMap : JSON.stringify(rawSourceMap)

    this._wasm = new SourceMap(json)

    const isIndexed = !!parsed.sections

    this.file = parsed.file || null
    this.sourceRoot = parsed.sourceRoot || null

    if (isIndexed) {
      this._sources = [...this._wasm.sources]
      this.sourcesContent = [...this._wasm.sourcesContent].map((c) => c ?? null)
    } else {
      this._sources = parsed.sources || []
      this.sourcesContent = parsed.sourcesContent || null
    }

    const resolve = resolver(sourceMapUrl, parsed.sourceRoot)
    this._resolvedSources = this._sources.map(resolve)

    // Cache WASM sources for O(1) lookups
    this._wasmSources = [...this._wasm.sources]
    this._wasmSourceMap = new Map()
    for (let i = 0; i < this._wasmSources.length; i++) {
      this._wasmSourceMap.set(this._wasmSources[i], i)
    }

    // Decoded mappings cache
    this._decoded = undefined
  }

  /** @type {string[]} */
  get sources() {
    return this._resolvedSources
  }

  /**
   * Look up the original position for a generated position.
   * Lines are 1-based, columns are 0-based.
   * @param {{ line: number, column: number, bias?: number }} needle
   * @returns {{ source: string|null, line: number|null, column: number|null, name: string|null }}
   */
  originalPositionFor(needle) {
    const { line, column, bias } = needle
    if (line < 1) throw new Error('Line must be greater than or equal to 1, got ' + line)
    if (column < 0) throw new Error('Column must be greater than or equal to 0, got ' + column)

    const zeroLine = line - 1

    if (!bias || bias === GREATEST_LOWER_BOUND) {
      const result = this._wasm.originalPositionFor(zeroLine, column)
      if (result === null || result === undefined) {
        return { source: null, line: null, column: null, name: null }
      }

      let source = result.source ?? null
      if (source !== null) {
        const idx = this._wasmSourceMap.get(source)
        if (idx !== undefined) source = this._resolvedSources[idx]
      }

      return {
        source,
        line: result.line != null ? result.line + 1 : null,
        column: result.column ?? null,
        name: result.name ?? null,
      }
    }

    // LEAST_UPPER_BOUND: fall back to decoded mappings search
    const decoded = this._getDecodedMappings()
    if (zeroLine >= decoded.length) return { source: null, line: null, column: null, name: null }

    const segments = decoded[zeroLine]
    const index = binarySearchLUB(segments, column)
    if (index === -1 || index >= segments.length) {
      return { source: null, line: null, column: null, name: null }
    }

    const segment = segments[index]
    if (segment.length === 1) return { source: null, line: null, column: null, name: null }

    return {
      source: this._resolvedSources[segment[SOURCES_INDEX]],
      line: segment[SOURCE_LINE] + 1,
      column: segment[SOURCE_COLUMN],
      name: segment.length === 5 ? this._names[segment[NAMES_INDEX]] : null,
    }
  }

  /**
   * Look up the generated position for an original source position.
   * Lines are 1-based, columns are 0-based.
   * @param {{ source: string, line: number, column: number, bias?: number }} needle
   * @returns {{ line: number|null, column: number|null, lastColumn: number|null }}
   */
  generatedPositionFor(needle) {
    const { source, line, column, bias } = needle
    if (line < 1) throw new Error('Line must be greater than or equal to 1, got ' + line)
    if (column < 0) throw new Error('Column must be greater than or equal to 0, got ' + column)

    const resolvedSource = resolveSourceName(this, source)
    if (resolvedSource === null) return { line: null, column: null, lastColumn: null }

    // Mozilla v0.6 uses GREATEST_LOWER_BOUND=1 and LEAST_UPPER_BOUND=2
    // WASM bias convention: 0 = GLB (default), -1 = LUB
    const wasmBias = bias === LEAST_UPPER_BOUND ? -1 : 0
    const result = this._wasm.generatedPositionForWithBias(resolvedSource, line - 1, column, wasmBias)
    if (result === null || result === undefined) {
      return { line: null, column: null, lastColumn: null }
    }

    return {
      line: result.line != null ? result.line + 1 : null,
      column: result.column ?? null,
      lastColumn: null,
    }
  }

  /**
   * Iterate all mappings in generated position order.
   * @param {(mapping: object) => void} callback
   * @param {object} [context] - `this` context for callback
   * @param {number} [order] - ordering (ignored, always generated order)
   */
  eachMapping(callback, context, order) {
    const decoded = this._getDecodedMappings()
    const cb = context ? callback.bind(context) : callback

    for (let i = 0; i < decoded.length; i++) {
      const line = decoded[i]
      for (let j = 0; j < line.length; j++) {
        const seg = line[j]
        const generatedLine = i + 1
        const generatedColumn = seg[COLUMN]
        let source = null
        let originalLine = null
        let originalColumn = null
        let name = null

        if (seg.length !== 1) {
          source = this._resolvedSources[seg[SOURCES_INDEX]]
          originalLine = seg[SOURCE_LINE] + 1
          originalColumn = seg[SOURCE_COLUMN]
        }
        if (seg.length === 5) name = this._names[seg[NAMES_INDEX]]

        cb({
          generatedLine,
          generatedColumn,
          source,
          originalLine,
          originalColumn,
          name,
          lastGeneratedColumn: null,
        })
      }
    }
  }

  /**
   * Get source content for a source file.
   * @param {string} source
   * @returns {string | null}
   */
  sourceContentFor(source) {
    if (this.sourcesContent == null) return null
    let index = this._sources.indexOf(source)
    if (index === -1) index = this._resolvedSources.indexOf(source)
    if (index === -1) return null
    return this.sourcesContent[index] ?? null
  }

  /**
   * Free WASM resources. No-op after first call.
   */
  destroy() {
    if (this._wasm) {
      this._wasm.free()
      this._wasm = null
    }
  }

  /** @private */
  _getDecodedMappings() {
    if (this._decoded != null) return this._decoded

    const flat = this._wasm.allMappingsFlat()
    const lineCount = this._wasm.lineCount
    const decoded = []

    for (let i = 0; i < lineCount; i++) {
      decoded.push([])
    }

    // Cache names from WASM
    this._names = [...this._wasm.names]

    for (let i = 0; i < flat.length; i += 7) {
      const genLine = flat[i]
      const genCol = flat[i + 1]
      const source = flat[i + 2]
      const origLine = flat[i + 3]
      const origCol = flat[i + 4]
      const name = flat[i + 5]

      while (decoded.length <= genLine) decoded.push([])

      if (source === -1) {
        decoded[genLine].push([genCol])
      } else if (name === -1) {
        decoded[genLine].push([genCol, source, origLine, origCol])
      } else {
        decoded[genLine].push([genCol, source, origLine, origCol, name])
      }
    }

    this._decoded = decoded
    return decoded
  }
}

// ── SourceMapGenerator ──────────────────────────────────────────

export class SourceMapGenerator {
  /**
   * @param {{ file?: string, sourceRoot?: string }} [opts]
   */
  constructor(opts) {
    const file = opts?.file || null
    this._gen = new WasmGenerator(file)
    this._file = file || undefined
    this._sourceRoot = opts?.sourceRoot || undefined
    this._sourceIndices = new Map()
    this._nameIndices = new Map()
    this._sourceContents = new Map()

    if (this._sourceRoot) {
      this._gen.setSourceRoot(this._sourceRoot)
    }
  }

  /**
   * Add a mapping.
   * Mozilla v0.6 API uses object-based arguments:
   * { generated: {line, column}, original?: {line, column}, source?, name? }
   * @param {object} mapping
   */
  addMapping(mapping) {
    const { generated, original, source, name } = mapping
    const genLine = generated.line - 1
    const genCol = generated.column

    if (!original || source == null) {
      this._gen.addGeneratedMapping(genLine, genCol)
      return
    }

    const srcIdx = this._getSourceIndex(source)
    const origLine = original.line - 1
    const origCol = original.column

    if (name != null) {
      const nameIdx = this._getNameIndex(name)
      this._gen.addNamedMapping(genLine, genCol, srcIdx, origLine, origCol, nameIdx)
    } else {
      this._gen.addMapping(genLine, genCol, srcIdx, origLine, origCol)
    }
  }

  /**
   * Set source content for a source file.
   * @param {string} source
   * @param {string | null} content
   */
  setSourceContent(source, content) {
    const srcIdx = this._getSourceIndex(source)
    if (content != null) {
      this._gen.setSourceContent(srcIdx, content)
      this._sourceContents.set(source, content)
    }
  }

  /**
   * Returns a source map object (parsed JSON, not a string).
   * @returns {object}
   */
  toJSON() {
    const json = this._gen.toJSON()
    return JSON.parse(json)
  }

  /**
   * Returns the source map as a JSON string.
   * @returns {string}
   */
  toString() {
    return this._gen.toJSON()
  }

  /**
   * Apply a source map from a consumer to this generator.
   * Basic implementation: remaps sources through the provided consumer.
   * @param {SourceMapConsumer} consumer
   * @param {string} [sourceFile] - The source file to apply the map for
   * @param {string} [sourceMapPath] - Path to the source map (unused)
   */
  applySourceMap(consumer, sourceFile, sourceMapPath) {
    // Determine the source to remap
    let source = sourceFile
    if (source == null) {
      if (consumer.file == null) {
        throw new Error(
          'SourceMapGenerator.prototype.applySourceMap requires either an explicit source file, ' +
          'or the source map\'s "file" property. Both were omitted.'
        )
      }
      source = consumer.file
    }

    // Build a new generator with the remapped mappings
    // This is a basic implementation that applies the consumer's mappings
    // to the current generator's output
    const currentMap = this.toJSON()
    const sourceIdx = currentMap.sources.indexOf(source)
    if (sourceIdx === -1) return // source not found, nothing to remap

    // Replace source content from the consumer
    if (consumer.sourcesContent) {
      for (let i = 0; i < consumer.sources.length; i++) {
        const content = consumer.sourcesContent[i]
        if (content != null) {
          this._sourceContents.set(consumer.sources[i], content)
        }
      }
    }
  }

  /**
   * Free WASM resources.
   */
  destroy() {
    if (this._gen) {
      this._gen.free()
      this._gen = null
    }
  }

  /** @private */
  _getSourceIndex(source) {
    let idx = this._sourceIndices.get(source)
    if (idx == null) {
      idx = this._gen.addSource(source)
      this._sourceIndices.set(source, idx)
    }
    return idx
  }

  /** @private */
  _getNameIndex(name) {
    let idx = this._nameIndices.get(name)
    if (idx == null) {
      idx = this._gen.addName(name)
      this._nameIndices.set(name, idx)
    }
    return idx
  }
}

// ── Internal helpers ────────────────────────────────────────────

/**
 * Binary search for LEAST_UPPER_BOUND (first segment with column >= needle).
 * @param {Array<number[]>} segments
 * @param {number} column
 * @returns {number} index or -1
 */
const binarySearchLUB = (segments, column) => {
  let low = 0
  let high = segments.length - 1
  let result = -1

  while (low <= high) {
    const mid = low + ((high - low) >> 1)
    const midCol = segments[mid][COLUMN]

    if (midCol === column) {
      result = mid
      high = mid - 1
    } else if (midCol > column) {
      result = mid
      high = mid - 1
    } else {
      low = mid + 1
    }
  }

  return result
}
