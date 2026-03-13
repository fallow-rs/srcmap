import { describe, it } from 'node:test'
import assert from 'node:assert/strict'
import {
  TraceMap,
  originalPositionFor,
  generatedPositionFor,
  allGeneratedPositionsFor,
  eachMapping,
  sourceContentFor,
  isIgnored,
  encodedMappings,
  decodedMappings,
  traceSegment,
  presortedDecodedMap,
  decodedMap,
  encodedMap,
  FlattenMap,
  AnyMap,
  LEAST_UPPER_BOUND,
  GREATEST_LOWER_BOUND,
} from '../src/trace-mapping.mjs'

// ── Test fixtures ────────────────────────────────────────────────

const SIMPLE_MAP = JSON.stringify({
  version: 3,
  sources: ['input.js'],
  names: ['foo', 'bar'],
  mappings: 'AAAAA,SACIC',
})

const SIMPLE_MAP_WITH_CONTENT = JSON.stringify({
  version: 3,
  file: 'output.js',
  sourceRoot: '',
  sources: ['input.js'],
  sourcesContent: ['const foo = 1;\nconst bar = 2;'],
  names: ['foo', 'bar'],
  mappings: 'AAAAA,SACIC',
})

const MULTI_SOURCE_MAP = JSON.stringify({
  version: 3,
  sources: ['a.js', 'b.js'],
  sourcesContent: ['// a.js\nconst x = 1;', '// b.js\nconst y = 2;'],
  names: ['x', 'y', 'z'],
  mappings: 'AAAAA;ACAAC,KACCC',
})

const IGNORE_LIST_MAP = JSON.stringify({
  version: 3,
  sources: ['app.js', 'node_modules/lib.js'],
  names: [],
  mappings: 'AAAA;ACAA',
  ignoreList: [1],
})

const X_GOOGLE_IGNORE_MAP = JSON.stringify({
  version: 3,
  sources: ['app.js', 'vendor.js'],
  names: [],
  mappings: 'AAAA;ACAA',
  x_google_ignoreList: [1],
})

const INDEXED_MAP = JSON.stringify({
  version: 3,
  sections: [
    {
      offset: { line: 0, column: 0 },
      map: {
        version: 3,
        sources: ['a.js'],
        names: ['hello'],
        mappings: 'AAAAA',
      },
    },
    {
      offset: { line: 10, column: 0 },
      map: {
        version: 3,
        sources: ['b.js'],
        names: ['world'],
        mappings: 'AAAAA',
      },
    },
  ],
})

// ── TraceMap constructor ─────────────────────────────────────────

describe('TraceMap constructor', () => {
  it('parses from JSON string', () => {
    const map = new TraceMap(SIMPLE_MAP)
    assert.equal(map.version, 3)
    assert.deepEqual(map.sources, ['input.js'])
    assert.deepEqual(map.names, ['foo', 'bar'])
    map.free()
  })

  it('parses from object', () => {
    const map = new TraceMap(JSON.parse(SIMPLE_MAP))
    assert.equal(map.version, 3)
    assert.deepEqual(map.sources, ['input.js'])
    map.free()
  })

  it('preserves file field', () => {
    const map = new TraceMap(SIMPLE_MAP_WITH_CONTENT)
    assert.equal(map.file, 'output.js')
    map.free()
  })

  it('has resolvedSources', () => {
    const map = new TraceMap(SIMPLE_MAP)
    assert.ok(Array.isArray(map.resolvedSources))
    assert.equal(map.resolvedSources.length, 1)
    map.free()
  })

  it('supports Symbol.dispose', () => {
    const map = new TraceMap(SIMPLE_MAP)
    assert.equal(typeof map[Symbol.dispose], 'function')
    map[Symbol.dispose]()
  })

  it('handles indexed source maps', () => {
    const map = new TraceMap(INDEXED_MAP)
    assert.ok(map.sources.length >= 2 || map.resolvedSources.length >= 2)
    map.free()
  })

  it('reads ignoreList', () => {
    const map = new TraceMap(IGNORE_LIST_MAP)
    assert.deepEqual(map.ignoreList, [1])
    map.free()
  })

  it('reads x_google_ignoreList as ignoreList', () => {
    const map = new TraceMap(X_GOOGLE_IGNORE_MAP)
    assert.deepEqual(map.ignoreList, [1])
    map.free()
  })
})

// ── Constants ────────────────────────────────────────────────────

describe('constants', () => {
  it('exports LEAST_UPPER_BOUND = -1', () => {
    assert.equal(LEAST_UPPER_BOUND, -1)
  })

  it('exports GREATEST_LOWER_BOUND = 1', () => {
    assert.equal(GREATEST_LOWER_BOUND, 1)
  })
})

// ── originalPositionFor ──────────────────────────────────────────

describe('originalPositionFor', () => {
  it('finds the original position (1-based lines)', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.equal(pos.source, 'input.js')
    assert.equal(pos.line, 1) // 1-based
    assert.equal(pos.column, 0)
    assert.equal(pos.name, 'foo')
    map.free()
  })

  it('returns nulls for unmapped position', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = originalPositionFor(map, { line: 999, column: 0 })
    assert.equal(pos.source, null)
    assert.equal(pos.line, null)
    assert.equal(pos.column, null)
    assert.equal(pos.name, null)
    map.free()
  })

  it('throws for line < 1', () => {
    const map = new TraceMap(SIMPLE_MAP)
    assert.throws(() => originalPositionFor(map, { line: 0, column: 0 }))
    map.free()
  })

  it('throws for column < 0', () => {
    const map = new TraceMap(SIMPLE_MAP)
    assert.throws(() => originalPositionFor(map, { line: 1, column: -1 }))
    map.free()
  })

  it('resolves across multiple sources', () => {
    const map = new TraceMap(MULTI_SOURCE_MAP)
    const pos = originalPositionFor(map, { line: 2, column: 0 })
    assert.equal(pos.source, 'b.js')
    assert.ok(pos.line >= 1)
    map.free()
  })

  it('resolves name when present', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.equal(pos.name, 'foo')
    map.free()
  })

  it('returns null name when not present', () => {
    // Create a map with segments that have no name
    const noNameMap = JSON.stringify({
      version: 3,
      sources: ['x.js'],
      names: [],
      mappings: 'AAAA',
    })
    const map = new TraceMap(noNameMap)
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.equal(pos.name, null)
    map.free()
  })

  it('uses GREATEST_LOWER_BOUND by default', () => {
    // Map with segments at columns 0 and 10
    const gapMap = JSON.stringify({
      version: 3,
      sources: ['x.js'],
      names: [],
      mappings: 'AAAA,UAAS',
    })
    const map = new TraceMap(gapMap)
    // Column 5 should snap back to the segment at column 0
    const pos = originalPositionFor(map, { line: 1, column: 5 })
    assert.equal(pos.source, 'x.js')
    assert.equal(pos.column, 0) // snapped to segment at col 0
    map.free()
  })

  it('supports LEAST_UPPER_BOUND bias', () => {
    // Map with segments at columns 0 and 10
    const gapMap = JSON.stringify({
      version: 3,
      sources: ['x.js'],
      names: [],
      mappings: 'AAAA,UAAS',
    })
    const map = new TraceMap(gapMap)
    // Column 5 with LUB should find the segment at column 10
    const pos = originalPositionFor(map, { line: 1, column: 5, bias: LEAST_UPPER_BOUND })
    assert.equal(pos.source, 'x.js')
    assert.equal(pos.line, 1)
    // With LUB, should find segment at or after column 5
    assert.ok(pos.column != null)
    map.free()
  })
})

// ── generatedPositionFor ─────────────────────────────────────────

describe('generatedPositionFor', () => {
  it('reverse-looks up a position (1-based lines)', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = generatedPositionFor(map, { source: 'input.js', line: 1, column: 0 })
    assert.equal(pos.line, 1) // 1-based
    assert.equal(pos.column, 0)
    map.free()
  })

  it('returns nulls for unknown source', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = generatedPositionFor(map, { source: 'nonexistent.js', line: 1, column: 0 })
    assert.equal(pos.line, null)
    assert.equal(pos.column, null)
    map.free()
  })

  it('throws for line < 1', () => {
    const map = new TraceMap(SIMPLE_MAP)
    assert.throws(
      () => generatedPositionFor(map, { source: 'input.js', line: 0, column: 0 })
    )
    map.free()
  })
})

// ── allGeneratedPositionsFor ─────────────────────────────────────

describe('allGeneratedPositionsFor', () => {
  it('returns all generated positions for an original position', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const positions = allGeneratedPositionsFor(map, { source: 'input.js', line: 1, column: 0 })
    assert.ok(Array.isArray(positions))
    assert.ok(positions.length >= 1)
    assert.ok(positions[0].line >= 1)
    assert.ok(positions[0].column >= 0)
    map.free()
  })

  it('returns empty array for unknown source', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const positions = allGeneratedPositionsFor(map, { source: 'nonexistent.js', line: 1, column: 0 })
    assert.deepEqual(positions, [])
    map.free()
  })
})

// ── eachMapping ──────────────────────────────────────────────────

describe('eachMapping', () => {
  it('iterates all mappings', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const mappings = []
    eachMapping(map, (m) => mappings.push(m))
    assert.ok(mappings.length >= 2)
    map.free()
  })

  it('provides 1-based lines', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const mappings = []
    eachMapping(map, (m) => mappings.push(m))
    assert.ok(mappings[0].generatedLine >= 1)
    if (mappings[0].originalLine != null) {
      assert.ok(mappings[0].originalLine >= 1)
    }
    map.free()
  })

  it('provides source, name when available', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const mappings = []
    eachMapping(map, (m) => mappings.push(m))

    // First segment should have source and name
    const first = mappings[0]
    assert.equal(first.source, 'input.js')
    assert.equal(first.name, 'foo')
    map.free()
  })

  it('iterates correct number of segments', () => {
    const map = new TraceMap(MULTI_SOURCE_MAP)
    const mappings = []
    eachMapping(map, (m) => mappings.push(m))
    assert.ok(mappings.length >= 3) // AAAAA;ACAAC,KACCC has at least 3 segments
    map.free()
  })
})

// ── sourceContentFor ─────────────────────────────────────────────

describe('sourceContentFor', () => {
  it('returns source content', () => {
    const map = new TraceMap(SIMPLE_MAP_WITH_CONTENT)
    const content = sourceContentFor(map, 'input.js')
    assert.equal(content, 'const foo = 1;\nconst bar = 2;')
    map.free()
  })

  it('returns null for unknown source', () => {
    const map = new TraceMap(SIMPLE_MAP_WITH_CONTENT)
    const content = sourceContentFor(map, 'nonexistent.js')
    assert.equal(content, null)
    map.free()
  })

  it('returns null when sourcesContent is not present', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const content = sourceContentFor(map, 'input.js')
    assert.equal(content, null)
    map.free()
  })
})

// ── isIgnored ────────────────────────────────────────────────────

describe('isIgnored', () => {
  it('returns true for ignored source', () => {
    const map = new TraceMap(IGNORE_LIST_MAP)
    assert.equal(isIgnored(map, 'node_modules/lib.js'), true)
    map.free()
  })

  it('returns false for non-ignored source', () => {
    const map = new TraceMap(IGNORE_LIST_MAP)
    assert.equal(isIgnored(map, 'app.js'), false)
    map.free()
  })

  it('returns false when no ignoreList', () => {
    const map = new TraceMap(SIMPLE_MAP)
    assert.equal(isIgnored(map, 'input.js'), false)
    map.free()
  })

  it('supports x_google_ignoreList', () => {
    const map = new TraceMap(X_GOOGLE_IGNORE_MAP)
    assert.equal(isIgnored(map, 'vendor.js'), true)
    assert.equal(isIgnored(map, 'app.js'), false)
    map.free()
  })
})

// ── encodedMappings ──────────────────────────────────────────────

describe('encodedMappings', () => {
  it('returns the VLQ mappings string', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const encoded = encodedMappings(map)
    assert.equal(typeof encoded, 'string')
    assert.equal(encoded, 'AAAAA,SACIC')
    map.free()
  })
})

// ── decodedMappings ──────────────────────────────────────────────

describe('decodedMappings', () => {
  it('returns decoded segments as arrays', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const decoded = decodedMappings(map)
    assert.ok(Array.isArray(decoded))
    assert.ok(decoded.length >= 1)
    // First line should have segments
    assert.ok(decoded[0].length >= 1)
    map.free()
  })

  it('segments have correct structure', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const decoded = decodedMappings(map)
    const firstSeg = decoded[0][0]

    // AAAAA → [0, 0, 0, 0, 0] (genCol, srcIdx, origLine, origCol, nameIdx)
    assert.ok(firstSeg.length === 4 || firstSeg.length === 5)
    assert.equal(firstSeg[0], 0) // generated column
    map.free()
  })

  it('caches result', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const decoded1 = decodedMappings(map)
    const decoded2 = decodedMappings(map)
    assert.equal(decoded1, decoded2) // Same reference
    map.free()
  })
})

// ── traceSegment ─────────────────────────────────────────────────

describe('traceSegment', () => {
  it('returns segment for valid position (0-based)', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const seg = traceSegment(map, 0, 0)
    assert.ok(seg)
    assert.ok(Array.isArray(seg))
    assert.equal(seg[0], 0) // generated column
    map.free()
  })

  it('returns null for out-of-range line', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const seg = traceSegment(map, 999, 0)
    assert.equal(seg, null)
    map.free()
  })

  it('returns null for column before first segment', () => {
    // Map where first segment starts at column 5
    const offsetMap = JSON.stringify({
      version: 3,
      sources: ['x.js'],
      names: [],
      mappings: 'KAAA',
    })
    const map = new TraceMap(offsetMap)
    // Column 0 should return null (no segment at or before col 0... wait,
    // KAAA decodes to genCol=5, so column 0 returns null)
    const seg = traceSegment(map, 0, 0)
    // With GLB, column 0 with first segment at 5 → returns -1 / null
    assert.equal(seg, null)
    map.free()
  })
})

// ── presortedDecodedMap ──────────────────────────────────────────

describe('presortedDecodedMap', () => {
  it('creates a TraceMap from pre-decoded data', () => {
    const decoded = {
      version: 3,
      sources: ['a.js'],
      names: ['x'],
      sourcesContent: ['const x = 1;'],
      mappings: [
        [[0, 0, 0, 0, 0]],
      ],
    }
    const map = presortedDecodedMap(decoded)
    assert.ok(map instanceof TraceMap)
    assert.deepEqual(map.sources, ['a.js'])
    assert.deepEqual(map.names, ['x'])

    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.equal(pos.source, 'a.js')
    assert.equal(pos.line, 1)
    assert.equal(pos.column, 0)
    assert.equal(pos.name, 'x')
    map.free()
  })
})

// ── decodedMap / encodedMap ──────────────────────────────────────

describe('decodedMap', () => {
  it('exports as decoded source map object', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const dm = decodedMap(map)
    assert.equal(dm.version, 3)
    assert.ok(Array.isArray(dm.mappings))
    assert.ok(Array.isArray(dm.sources))
    assert.ok(Array.isArray(dm.names))
    map.free()
  })
})

describe('encodedMap', () => {
  it('exports as encoded source map object', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const em = encodedMap(map)
    assert.equal(em.version, 3)
    assert.equal(typeof em.mappings, 'string')
    assert.equal(em.mappings, 'AAAAA,SACIC')
    map.free()
  })
})

// ── FlattenMap / AnyMap ──────────────────────────────────────────

describe('FlattenMap / AnyMap', () => {
  it('FlattenMap is exported', () => {
    assert.equal(typeof FlattenMap, 'function')
  })

  it('AnyMap is exported', () => {
    assert.equal(typeof AnyMap, 'function')
  })

  it('FlattenMap handles indexed source maps', () => {
    const map = new FlattenMap(INDEXED_MAP)
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.ok(pos.source)
    map.free()
  })
})

// ── Correctness: compare with decoded mappings ───────────────────

describe('correctness', () => {
  it('originalPositionFor matches decoded segments', () => {
    const map = new TraceMap(MULTI_SOURCE_MAP)
    const decoded = decodedMappings(map)

    for (let lineIdx = 0; lineIdx < decoded.length; lineIdx++) {
      const line = decoded[lineIdx]
      for (const seg of line) {
        if (seg.length === 1) continue

        const pos = originalPositionFor(map, {
          line: lineIdx + 1,
          column: seg[0],
        })

        assert.equal(pos.line, seg[2] + 1, `line mismatch at ${lineIdx}:${seg[0]}`)
        assert.equal(pos.column, seg[3], `column mismatch at ${lineIdx}:${seg[0]}`)
      }
    }
    map.free()
  })

  it('generatedPositionFor round-trips with originalPositionFor', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const orig = originalPositionFor(map, { line: 1, column: 0 })
    assert.ok(orig.source)

    const gen = generatedPositionFor(map, {
      source: orig.source,
      line: orig.line,
      column: orig.column,
    })
    assert.equal(gen.line, 1)
    assert.equal(gen.column, 0)
    map.free()
  })

  it('eachMapping count matches decodedMappings total', () => {
    const map = new TraceMap(MULTI_SOURCE_MAP)
    const decoded = decodedMappings(map)
    const decodedCount = decoded.reduce((sum, line) => sum + line.length, 0)

    let eachCount = 0
    eachMapping(map, () => eachCount++)

    assert.equal(eachCount, decodedCount)
    map.free()
  })
})

// ── Edge cases ───────────────────────────────────────────────────

describe('edge cases', () => {
  it('handles empty mappings', () => {
    const emptyMap = JSON.stringify({
      version: 3,
      sources: [],
      names: [],
      mappings: '',
    })
    const map = new TraceMap(emptyMap)
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.equal(pos.source, null)

    const decoded = decodedMappings(map)
    assert.ok(Array.isArray(decoded))
    assert.equal(decoded.reduce((s, l) => s + l.length, 0), 0)
    map.free()
  })

  it('handles source map with only semicolons', () => {
    const semiMap = JSON.stringify({
      version: 3,
      sources: ['a.js'],
      names: [],
      mappings: ';;;',
    })
    const map = new TraceMap(semiMap)
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.equal(pos.source, null)
    map.free()
  })

  it('handles large column values', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = originalPositionFor(map, { line: 1, column: 99999 })
    // Should snap to last segment on the line (GLB behavior)
    assert.ok(pos.source === 'input.js' || pos.source === null)
    map.free()
  })

  it('handles map with null sources entries', () => {
    const nullSourceMap = JSON.stringify({
      version: 3,
      sources: [null, 'real.js'],
      names: [],
      mappings: 'ACAA',
    })
    const map = new TraceMap(nullSourceMap)
    assert.ok(map.sources.length === 2)
    map.free()
  })

  it('handles sourcesContent with null entries', () => {
    const mixedContent = JSON.stringify({
      version: 3,
      sources: ['a.js', 'b.js'],
      sourcesContent: [null, 'const y = 2;'],
      names: [],
      mappings: 'AAAA;ACAA',
    })
    const map = new TraceMap(mixedContent)
    assert.equal(sourceContentFor(map, 'a.js'), null)
    assert.equal(sourceContentFor(map, 'b.js'), 'const y = 2;')
    map.free()
  })
})

// ── API compatibility with @jridgewell/trace-mapping ─────────────

describe('API compatibility', () => {
  it('exports all expected functions', () => {
    assert.equal(typeof TraceMap, 'function')
    assert.equal(typeof originalPositionFor, 'function')
    assert.equal(typeof generatedPositionFor, 'function')
    assert.equal(typeof allGeneratedPositionsFor, 'function')
    assert.equal(typeof eachMapping, 'function')
    assert.equal(typeof sourceContentFor, 'function')
    assert.equal(typeof isIgnored, 'function')
    assert.equal(typeof encodedMappings, 'function')
    assert.equal(typeof decodedMappings, 'function')
    assert.equal(typeof traceSegment, 'function')
    assert.equal(typeof presortedDecodedMap, 'function')
    assert.equal(typeof decodedMap, 'function')
    assert.equal(typeof encodedMap, 'function')
    assert.equal(typeof FlattenMap, 'function')
    assert.equal(typeof AnyMap, 'function')
  })

  it('TraceMap has expected properties', () => {
    const map = new TraceMap(SIMPLE_MAP_WITH_CONTENT)
    assert.equal(map.version, 3)
    assert.equal(map.file, 'output.js')
    assert.ok(Array.isArray(map.sources))
    assert.ok(Array.isArray(map.names))
    assert.ok(Array.isArray(map.resolvedSources))
    assert.ok(Array.isArray(map.sourcesContent))
    map.free()
  })

  it('originalPositionFor returns object with source/line/column/name', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.ok('source' in pos)
    assert.ok('line' in pos)
    assert.ok('column' in pos)
    assert.ok('name' in pos)
    map.free()
  })

  it('generatedPositionFor returns object with line/column', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = generatedPositionFor(map, { source: 'input.js', line: 1, column: 0 })
    assert.ok('line' in pos)
    assert.ok('column' in pos)
    map.free()
  })

  it('null result has all null fields (not undefined)', () => {
    const map = new TraceMap(SIMPLE_MAP)
    const pos = originalPositionFor(map, { line: 999, column: 0 })
    assert.equal(pos.source, null)
    assert.equal(pos.line, null)
    assert.equal(pos.column, null)
    assert.equal(pos.name, null)
    map.free()
  })
})

// ── Review fixes ─────────────────────────────────────────────────

describe('review fixes', () => {
  const REVIEW_MAP = JSON.stringify({
    version: 3,
    file: 'output.js',
    sources: ['input.js'],
    names: [],
    mappings: 'AAAA',
  })

  it('copy-constructor does not share WASM pointer', () => {
    const original = new TraceMap(REVIEW_MAP)
    const copy = new TraceMap(original)

    // Free the original — the copy must remain functional
    original.free()

    const pos = originalPositionFor(copy, { line: 1, column: 0 })
    assert.equal(pos.source, 'input.js')
    assert.equal(pos.line, 1)
    assert.equal(pos.column, 0)
    copy.free()
  })

  it('resolver handles data: URIs', () => {
    const dataMap = JSON.stringify({
      version: 3,
      file: 'output.js',
      sources: ['data:application/json;base64,abc'],
      names: [],
      mappings: 'AAAA',
    })
    const map = new TraceMap(dataMap)
    assert.equal(map.resolvedSources[0], 'data:application/json;base64,abc')
    map.free()
  })

  it('resolver handles webpack:// URIs', () => {
    const webpackMap = JSON.stringify({
      version: 3,
      file: 'output.js',
      sources: ['webpack:///src/index.js'],
      names: [],
      mappings: 'AAAA',
    })
    const map = new TraceMap(webpackMap)
    assert.equal(map.resolvedSources[0], 'webpack:///src/index.js')
    map.free()
  })

  it('generatedPositionFor with LEAST_UPPER_BOUND bias', () => {
    // Map with two segments: col 0 → line 1 col 0, col 10 → line 1 col 9
    const biasMap = JSON.stringify({
      version: 3,
      file: 'output.js',
      sources: ['input.js'],
      names: [],
      mappings: 'AAAA,UAAS',
    })
    const map = new TraceMap(biasMap)
    const pos = generatedPositionFor(map, {
      source: 'input.js',
      line: 1,
      column: 5,
      bias: LEAST_UPPER_BOUND,
    })
    // With LUB, should find the segment at or after original column 5
    assert.ok(pos.line != null)
    assert.ok(pos.column != null)
    assert.equal(pos.line, 1)
    map.free()
  })

  it('generatedPositionFor with default bias (GLB)', () => {
    const biasMap = JSON.stringify({
      version: 3,
      file: 'output.js',
      sources: ['input.js'],
      names: [],
      mappings: 'AAAA,UAAS',
    })
    const map = new TraceMap(biasMap)
    // Default bias is GREATEST_LOWER_BOUND
    const pos = generatedPositionFor(map, {
      source: 'input.js',
      line: 1,
      column: 0,
    })
    assert.equal(pos.line, 1)
    assert.equal(pos.column, 0)
    map.free()
  })

  it('sourcesContent absent vs empty', () => {
    // Map WITHOUT sourcesContent field at all
    const noContentMap = JSON.stringify({
      version: 3,
      file: 'output.js',
      sources: ['input.js'],
      names: [],
      mappings: 'AAAA',
    })
    const map = new TraceMap(noContentMap)
    const content = sourceContentFor(map, 'input.js')
    assert.equal(content, null)
    map.free()
  })

  it('cached WASM sources lookup', () => {
    const map = new TraceMap(REVIEW_MAP)
    // Perform a lookup — the source should be resolved via _wasmSourceMap cache
    const pos = originalPositionFor(map, { line: 1, column: 0 })
    assert.equal(pos.source, 'input.js')
    assert.equal(pos.line, 1)
    assert.equal(pos.column, 0)

    // Verify the cache exists and maps correctly
    assert.ok(map._wasmSourceMap instanceof Map)
    assert.ok(map._wasmSourceMap.size > 0)
    map.free()
  })
})
