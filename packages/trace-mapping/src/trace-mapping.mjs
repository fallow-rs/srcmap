import { SourceMap } from '../../sourcemap-wasm/pkg/srcmap_sourcemap_wasm.js'

// ── Constants ────────────────────────────────────────────────────

export const LEAST_UPPER_BOUND = -1
export const GREATEST_LOWER_BOUND = 1

// ── Internal helpers ─────────────────────────────────────────────

const LINE_GTR_ZERO = '`line` must be greater than 0 (lines start at line 1)'
const COL_GTR_EQ_ZERO = '`column` must be greater than or equal to 0 (columns start at column 0)'

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
 * Resolve a source path against sourceRoot and mapUrl.
 * Simple path resolution without a full URI resolver.
 * @param {string | undefined} mapUrl
 * @param {string | undefined} sourceRoot
 */
const resolver = (mapUrl, sourceRoot) => {
  const from = stripFilename(mapUrl)
  const prefix = sourceRoot ? sourceRoot + '/' : ''
  return (source) => {
    const resolved = prefix + (source || '')
    // Simple resolution: if absolute URL or path, return as-is
    if (resolved.startsWith('http://') || resolved.startsWith('https://') || resolved.startsWith('/')) {
      return resolved
    }
    if (!from) return resolved
    return from + resolved
  }
}

// ── TraceMap class ───────────────────────────────────────────────

export class TraceMap {
  /**
   * @param {string | object} map - JSON string or parsed source map object
   * @param {string} [mapUrl] - URL of the source map (for resolving relative sources)
   */
  constructor(map, mapUrl) {
    // If already a TraceMap, return as-is
    if (map instanceof TraceMap) {
      Object.assign(this, map)
      return
    }

    const parsed = typeof map === 'string' ? JSON.parse(map) : map
    const json = typeof map === 'string' ? map : JSON.stringify(map)

    // WASM SourceMap handles both regular and indexed source maps natively
    this._wasm = new SourceMap(json)

    const isIndexed = !!parsed.sections

    this.version = parsed.version
    this.file = parsed.file

    if (isIndexed) {
      // For indexed maps, WASM flattens sections — get metadata from WASM
      this.sources = [...this._wasm.sources]
      this.names = [...this._wasm.names]
      this.sourcesContent = [...this._wasm.sourcesContent].map((c) => c ?? null)
      if (this.sourcesContent.every((c) => c === null)) this.sourcesContent = undefined
      this.ignoreList = this._wasm.ignoreList.length > 0 ? [...this._wasm.ignoreList] : undefined
      this.sourceRoot = undefined
    } else {
      this.names = parsed.names || []
      this.sourceRoot = parsed.sourceRoot
      this.sources = parsed.sources || []
      this.sourcesContent = parsed.sourcesContent || undefined
      this.ignoreList = parsed.ignoreList || parsed.x_google_ignoreList || undefined
    }

    const resolve = resolver(mapUrl, this.sourceRoot)
    this.resolvedSources = this.sources.map(resolve)

    // Store raw mappings for encodedMappings()
    if (isIndexed) {
      this._encoded = undefined
      this._decoded = undefined
    } else if (typeof parsed.mappings === 'string') {
      this._encoded = parsed.mappings
      this._decoded = undefined
    } else if (Array.isArray(parsed.mappings)) {
      this._encoded = undefined
      this._decoded = parsed.mappings
    } else {
      this._encoded = ''
      this._decoded = undefined
    }
  }

  free() {
    if (this._wasm) {
      this._wasm.free()
      this._wasm = null
    }
  }

  [Symbol.dispose]() {
    this.free()
  }
}

// ── Free functions ───────────────────────────────────────────────

/**
 * Return the VLQ-encoded mappings string.
 * @param {TraceMap} map
 * @returns {string}
 */
export const encodedMappings = (map) => {
  if (map._encoded != null) return map._encoded
  // Re-encode from WASM if we only have decoded
  map._encoded = map._wasm.encodedMappings()
  return map._encoded
}

/**
 * Return the decoded mappings as SourceMapSegment[][].
 * @param {TraceMap} map
 * @returns {Array<Array<number[]>>}
 */
export const decodedMappings = (map) => {
  if (map._decoded != null) return map._decoded

  const flat = map._wasm.allMappingsFlat()
  const lineCount = map._wasm.lineCount
  const decoded = []

  for (let i = 0; i < lineCount; i++) {
    decoded.push([])
  }

  // flat format: [genLine, genCol, source, origLine, origCol, name, ...]
  for (let i = 0; i < flat.length; i += 6) {
    const genLine = flat[i]
    const genCol = flat[i + 1]
    const source = flat[i + 2]
    const origLine = flat[i + 3]
    const origCol = flat[i + 4]
    const name = flat[i + 5]

    // Ensure line array exists
    while (decoded.length <= genLine) decoded.push([])

    if (source === -1) {
      decoded[genLine].push([genCol])
    } else if (name === -1) {
      decoded[genLine].push([genCol, source, origLine, origCol])
    } else {
      decoded[genLine].push([genCol, source, origLine, origCol, name])
    }
  }

  map._decoded = decoded
  return decoded
}

/**
 * Low-level segment lookup (0-based line and column).
 * @param {TraceMap} map
 * @param {number} line - 0-based line
 * @param {number} column - 0-based column
 * @returns {number[] | null}
 */
export const traceSegment = (map, line, column) => {
  const decoded = decodedMappings(map)
  if (line >= decoded.length) return null
  const segments = decoded[line]
  const index = binarySearch(segments, column)
  if (index === -1) return null
  return segments[index]
}

/**
 * Look up the original position for a generated position.
 * Lines are 1-based, columns are 0-based.
 * @param {TraceMap} map
 * @param {{ line: number, column: number, bias?: number }} needle
 * @returns {{ source: string|null, line: number|null, column: number|null, name: string|null }}
 */
export const originalPositionFor = (map, needle) => {
  let { line, column, bias } = needle
  line--
  if (line < 0) throw new Error(LINE_GTR_ZERO)
  if (column < 0) throw new Error(COL_GTR_EQ_ZERO)

  // Use WASM for the fast path (GREATEST_LOWER_BOUND, which is the default)
  if (!bias || bias === GREATEST_LOWER_BOUND) {
    const result = map._wasm.originalPositionFor(line, column)
    if (result === null || result === undefined) {
      return { source: null, line: null, column: null, name: null }
    }

    // WASM returns 0-based lines, trace-mapping returns 1-based
    return {
      source: result.source ?? null,
      line: result.line != null ? result.line + 1 : null,
      column: result.column ?? null,
      name: result.name ?? null,
    }
  }

  // LEAST_UPPER_BOUND: fall back to decoded mappings search
  const decoded = decodedMappings(map)
  if (line >= decoded.length) return { source: null, line: null, column: null, name: null }

  const segments = decoded[line]
  const index = binarySearchLUB(segments, column)
  if (index === -1 || index >= segments.length) {
    return { source: null, line: null, column: null, name: null }
  }

  const segment = segments[index]
  if (segment.length === 1) return { source: null, line: null, column: null, name: null }

  return {
    source: map.resolvedSources[segment[SOURCES_INDEX]],
    line: segment[SOURCE_LINE] + 1,
    column: segment[SOURCE_COLUMN],
    name: segment.length === 5 ? map.names[segment[NAMES_INDEX]] : null,
  }
}

/**
 * Look up the generated position for an original source position.
 * Lines are 1-based, columns are 0-based.
 * @param {TraceMap} map
 * @param {{ source: string, line: number, column: number, bias?: number }} needle
 * @returns {{ line: number|null, column: number|null }}
 */
export const generatedPositionFor = (map, needle) => {
  const { source, line, column, bias } = needle
  if (line < 1) throw new Error(LINE_GTR_ZERO)
  if (column < 0) throw new Error(COL_GTR_EQ_ZERO)

  // Resolve source name: try raw sources first, then resolved
  const resolvedSource = resolveSourceName(map, source)
  if (resolvedSource === null) return { line: null, column: null }

  // Use WASM (0-based lines internally)
  const result = map._wasm.generatedPositionFor(resolvedSource, line - 1, column)
  if (result === null || result === undefined) {
    return { line: null, column: null }
  }

  // WASM returns 0-based, trace-mapping returns 1-based
  return {
    line: result.line != null ? result.line + 1 : null,
    column: result.column ?? null,
  }
}

/**
 * Find all generated positions for an original source position.
 * Lines are 1-based, columns are 0-based.
 * @param {TraceMap} map
 * @param {{ source: string, line: number, column: number, bias?: number }} needle
 * @returns {Array<{ line: number|null, column: number|null }>}
 */
export const allGeneratedPositionsFor = (map, needle) => {
  const { source, line, column } = needle
  if (line < 1) throw new Error(LINE_GTR_ZERO)
  if (column < 0) throw new Error(COL_GTR_EQ_ZERO)

  const resolvedSource = resolveSourceName(map, source)
  if (resolvedSource === null) return []

  const results = map._wasm.allGeneratedPositionsFor(resolvedSource, line - 1, column)
  return results.map((r) => ({
    line: r.line != null ? r.line + 1 : null,
    column: r.column ?? null,
  }))
}

/**
 * Iterate all mappings in generated position order.
 * Lines are 1-based in the callback.
 * @param {TraceMap} map
 * @param {(mapping: object) => void} cb
 */
export const eachMapping = (map, cb) => {
  const decoded = decodedMappings(map)
  const { names, resolvedSources } = map

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
        source = resolvedSources[seg[SOURCES_INDEX]]
        originalLine = seg[SOURCE_LINE] + 1
        originalColumn = seg[SOURCE_COLUMN]
      }
      if (seg.length === 5) name = names[seg[NAMES_INDEX]]

      cb({
        generatedLine,
        generatedColumn,
        source,
        originalLine,
        originalColumn,
        name,
      })
    }
  }
}

/**
 * Get source content for a source file.
 * @param {TraceMap} map
 * @param {string} source
 * @returns {string | null}
 */
export const sourceContentFor = (map, source) => {
  const { sourcesContent } = map
  if (sourcesContent == null) return null
  const index = sourceIndexOf(map, source)
  if (index === -1) return null
  return sourcesContent[index] ?? null
}

/**
 * Check if a source is in the ignoreList.
 * @param {TraceMap} map
 * @param {string} source
 * @returns {boolean}
 */
export const isIgnored = (map, source) => {
  const { ignoreList } = map
  if (ignoreList == null) return false
  const index = sourceIndexOf(map, source)
  if (index === -1) return false
  return ignoreList.includes(index)
}

/**
 * Create a TraceMap from a pre-sorted decoded source map (skip sorting).
 * @param {object} map - Decoded source map object with array mappings
 * @param {string} [mapUrl]
 * @returns {TraceMap}
 */
export const presortedDecodedMap = (map, mapUrl) => {
  // Convert decoded mappings to encoded for WASM consumption
  const encoded = encodeDecodedMappings(map.mappings)
  const raw = {
    version: map.version,
    file: map.file,
    names: map.names,
    sourceRoot: map.sourceRoot,
    sources: map.sources,
    sourcesContent: map.sourcesContent,
    mappings: encoded,
    ignoreList: map.ignoreList,
  }
  const tracer = new TraceMap(raw, mapUrl)
  tracer._decoded = map.mappings
  return tracer
}

/**
 * Export as a decoded source map object.
 * @param {TraceMap} map
 * @returns {object}
 */
export const decodedMap = (map) => ({
  version: map.version,
  file: map.file,
  names: map.names,
  sourceRoot: map.sourceRoot,
  sources: map.sources,
  sourcesContent: map.sourcesContent,
  mappings: decodedMappings(map),
  ignoreList: map.ignoreList,
})

/**
 * Export as an encoded source map object.
 * @param {TraceMap} map
 * @returns {object}
 */
export const encodedMap = (map) => ({
  version: map.version,
  file: map.file,
  names: map.names,
  sourceRoot: map.sourceRoot,
  sources: map.sources,
  sourcesContent: map.sourcesContent,
  mappings: encodedMappings(map),
  ignoreList: map.ignoreList,
})

/**
 * FlattenMap handles sectioned/indexed source maps.
 * Since srcmap's WASM SourceMap handles indexed maps natively,
 * this is just an alias for TraceMap.
 */
export const FlattenMap = TraceMap
export const AnyMap = TraceMap

// ── Internal helpers ─────────────────────────────────────────────

/**
 * Find source index by name, checking both sources and resolvedSources.
 * @param {TraceMap} map
 * @param {string} source
 * @returns {number}
 */
const sourceIndexOf = (map, source) => {
  let index = map.sources.indexOf(source)
  if (index === -1) index = map.resolvedSources.indexOf(source)
  return index
}

/**
 * Resolve a source name to the WASM-internal name.
 * The WASM SourceMap stores sources with sourceRoot prepended.
 * @param {TraceMap} map
 * @param {string} source
 * @returns {string | null}
 */
const resolveSourceName = (map, source) => {
  // Try direct match against WASM sources
  const wasmSources = map._wasm.sources
  if (wasmSources.includes(source)) return source

  // Try matching via index (raw sources → WASM sources)
  let index = map.sources.indexOf(source)
  if (index === -1) index = map.resolvedSources.indexOf(source)
  if (index === -1) return null

  return wasmSources[index] ?? null
}

/**
 * Binary search for GREATEST_LOWER_BOUND (last segment with column <= needle).
 * @param {Array<number[]>} segments
 * @param {number} column
 * @returns {number} index or -1
 */
const binarySearch = (segments, column) => {
  let low = 0
  let high = segments.length - 1
  let result = -1

  while (low <= high) {
    const mid = low + ((high - low) >> 1)
    const midCol = segments[mid][COLUMN]

    if (midCol === column) {
      // Find the last segment with this column (lower bound)
      result = mid
      // Continue searching right for potential later same-column segments
      // Actually for GREATEST_LOWER_BOUND with exact match, return last match
      low = mid + 1
    } else if (midCol < column) {
      result = mid
      low = mid + 1
    } else {
      high = mid - 1
    }
  }

  return result
}

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
      high = mid - 1 // Find the first match
    } else if (midCol > column) {
      result = mid
      high = mid - 1
    } else {
      low = mid + 1
    }
  }

  return result
}

/**
 * Encode decoded mappings to VLQ string.
 * Minimal implementation for presortedDecodedMap.
 * @param {Array<Array<number[]>>} decoded
 * @returns {string}
 */
const encodeDecodedMappings = (decoded) => {
  const parts = []

  for (let i = 0; i < decoded.length; i++) {
    const line = decoded[i]
    if (line.length === 0) {
      parts.push('')
      continue
    }

    const segments = []
    let prevCol = 0
    let prevSource = 0
    let prevOrigLine = 0
    let prevOrigCol = 0
    let prevName = 0

    for (const seg of line) {
      let encoded = vlqEncode(seg[COLUMN] - prevCol)
      prevCol = seg[COLUMN]

      if (seg.length > 1) {
        encoded += vlqEncode(seg[SOURCES_INDEX] - prevSource)
        prevSource = seg[SOURCES_INDEX]
        encoded += vlqEncode(seg[SOURCE_LINE] - prevOrigLine)
        prevOrigLine = seg[SOURCE_LINE]
        encoded += vlqEncode(seg[SOURCE_COLUMN] - prevOrigCol)
        prevOrigCol = seg[SOURCE_COLUMN]

        if (seg.length === 5) {
          encoded += vlqEncode(seg[NAMES_INDEX] - prevName)
          prevName = seg[NAMES_INDEX]
        }
      }

      segments.push(encoded)
    }

    parts.push(segments.join(','))
  }

  return parts.join(';')
}

const B64_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/**
 * Encode a single VLQ value.
 * @param {number} value
 * @returns {string}
 */
const vlqEncode = (value) => {
  let vlq = value < 0 ? (-value << 1) + 1 : value << 1
  let result = ''
  do {
    let digit = vlq & 0x1f
    vlq >>>= 5
    if (vlq > 0) digit |= 0x20
    result += B64_CHARS[digit]
  } while (vlq > 0)
  return result
}
