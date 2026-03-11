import { describe, it } from 'node:test'
import assert from 'node:assert/strict'
import { SourceMap, resultPtr, wasmMemory } from '../pkg/srcmap_sourcemap_wasm.js'

const SIMPLE_MAP = JSON.stringify({
  version: 3,
  sources: ['input.js'],
  names: ['foo', 'bar'],
  mappings: 'AAAAA,SACIC',
})

const MULTI_SOURCE_MAP = JSON.stringify({
  version: 3,
  sources: ['a.js', 'b.js'],
  names: ['x', 'y', 'z'],
  mappings: 'AAAAA;ACAAC,KACCC',
})

describe('SourceMap constructor', () => {
  it('parses a valid source map', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.ok(sm)
    sm.free()
  })

  it('throws on invalid JSON', () => {
    assert.throws(() => new SourceMap('not json'))
  })
})

describe('sources and names', () => {
  it('returns source file list', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.deepEqual(sm.sources, ['input.js'])
    sm.free()
  })

  it('returns names list', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.deepEqual(sm.names, ['foo', 'bar'])
    sm.free()
  })
})

describe('mappingCount and lineCount', () => {
  it('reports correct mapping count', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.equal(sm.mappingCount, 2)
    sm.free()
  })

  it('reports correct line count', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.ok(sm.lineCount >= 1)
    sm.free()
  })
})

describe('originalPositionFor', () => {
  it('looks up first segment', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const pos = sm.originalPositionFor(0, 0)
    assert.ok(pos)
    assert.equal(pos.source, 'input.js')
    assert.equal(pos.line, 0)
    assert.equal(pos.column, 0)
    assert.equal(pos.name, 'foo')
    sm.free()
  })

  it('returns null for unmapped position', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const pos = sm.originalPositionFor(999, 999)
    assert.equal(pos, null)
    sm.free()
  })

  it('resolves across multiple sources', () => {
    const sm = new SourceMap(MULTI_SOURCE_MAP)
    const pos = sm.originalPositionFor(1, 0)
    assert.ok(pos)
    assert.equal(pos.source, 'b.js')
    sm.free()
  })
})

describe('generatedPositionFor', () => {
  it('reverse-looks up a position', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const pos = sm.generatedPositionFor('input.js', 0, 0)
    assert.ok(pos)
    assert.equal(pos.line, 0)
    assert.equal(pos.column, 0)
    sm.free()
  })

  it('returns null for unknown source', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const pos = sm.generatedPositionFor('nonexistent.js', 0, 0)
    assert.equal(pos, null)
    sm.free()
  })
})

describe('originalPositionsFor (batch)', () => {
  it('batch-resolves positions', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const results = sm.originalPositionsFor(new Int32Array([0, 0]))
    assert.ok(results instanceof Int32Array)
    assert.equal(results.length, 4)
    assert.ok(results[0] >= 0) // valid source index
    assert.equal(results[1], 0) // line
    assert.equal(results[2], 0) // column
    sm.free()
  })

  it('returns -1 for unmapped batch positions', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const results = sm.originalPositionsFor(new Int32Array([999, 999]))
    assert.equal(results[0], -1)
    sm.free()
  })
})

describe('debugId', () => {
  it('returns debugId when present', () => {
    const map = JSON.stringify({
      version: 3,
      sources: ['a.js'],
      names: [],
      mappings: 'AAAA',
      debugId: '85314830-023f-4cf1-a267-535f4e37bb17',
    })
    const sm = new SourceMap(map)
    assert.equal(sm.debugId, '85314830-023f-4cf1-a267-535f4e37bb17')
    sm.free()
  })

  it('returns undefined when debugId is absent', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.equal(sm.debugId, undefined)
    sm.free()
  })
})

describe('file and sourceRoot getters', () => {
  it('returns file when present', () => {
    const map = JSON.stringify({
      version: 3,
      file: 'output.js',
      sources: ['a.js'],
      names: [],
      mappings: 'AAAA',
    })
    const sm = new SourceMap(map)
    assert.equal(sm.file, 'output.js')
    sm.free()
  })

  it('returns undefined when file is absent', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.equal(sm.file, undefined)
    sm.free()
  })

  it('returns sourceRoot when present', () => {
    const map = JSON.stringify({
      version: 3,
      sourceRoot: 'src/',
      sources: ['a.js'],
      names: [],
      mappings: 'AAAA',
    })
    const sm = new SourceMap(map)
    assert.equal(sm.sourceRoot, 'src/')
    sm.free()
  })
})

describe('sourcesContent getter', () => {
  it('returns sources content array', () => {
    const map = JSON.stringify({
      version: 3,
      sources: ['a.js'],
      sourcesContent: ['const x = 1;'],
      names: [],
      mappings: 'AAAA',
    })
    const sm = new SourceMap(map)
    assert.deepEqual(sm.sourcesContent, ['const x = 1;'])
    sm.free()
  })

  it('returns null for missing content entries', () => {
    const map = JSON.stringify({
      version: 3,
      sources: ['a.js', 'b.js'],
      sourcesContent: [null, 'const y = 2;'],
      names: [],
      mappings: 'AAAA',
    })
    const sm = new SourceMap(map)
    assert.equal(sm.sourcesContent[0], null)
    assert.equal(sm.sourcesContent[1], 'const y = 2;')
    sm.free()
  })
})

describe('ignoreList getter', () => {
  it('returns ignore list', () => {
    const map = JSON.stringify({
      version: 3,
      sources: ['app.js', 'lib.js'],
      names: [],
      mappings: 'AAAA',
      ignoreList: [1],
    })
    const sm = new SourceMap(map)
    assert.deepEqual([...sm.ignoreList], [1])
    sm.free()
  })
})

describe('sourceContentFor', () => {
  it('returns content for valid index', () => {
    const map = JSON.stringify({
      version: 3,
      sources: ['a.js'],
      sourcesContent: ['const x = 1;'],
      names: [],
      mappings: 'AAAA',
    })
    const sm = new SourceMap(map)
    assert.equal(sm.sourceContentFor(0), 'const x = 1;')
    sm.free()
  })

  it('returns null for out-of-range index', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    assert.equal(sm.sourceContentFor(999), null)
    sm.free()
  })
})

describe('isIgnoredIndex', () => {
  it('returns true for ignored source index', () => {
    const map = JSON.stringify({
      version: 3,
      sources: ['app.js', 'lib.js'],
      names: [],
      mappings: 'AAAA',
      ignoreList: [1],
    })
    const sm = new SourceMap(map)
    assert.equal(sm.isIgnoredIndex(0), false)
    assert.equal(sm.isIgnoredIndex(1), true)
    sm.free()
  })
})

describe('allGeneratedPositionsFor', () => {
  it('returns positions for a known mapping', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const positions = sm.allGeneratedPositionsFor('input.js', 0, 0)
    assert.ok(Array.isArray(positions))
    assert.ok(positions.length >= 1)
    assert.equal(positions[0].line, 0)
    assert.equal(positions[0].column, 0)
    sm.free()
  })

  it('returns empty for unknown source', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const positions = sm.allGeneratedPositionsFor('nonexistent.js', 0, 0)
    assert.deepEqual(positions, [])
    sm.free()
  })
})

describe('allMappingsFlat', () => {
  it('returns flat array of all mappings', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const flat = sm.allMappingsFlat()
    assert.ok(flat instanceof Int32Array)
    // 2 segments * 6 fields each = 12
    assert.equal(flat.length, 12)
    // First mapping: genLine=0, genCol=0
    assert.equal(flat[0], 0) // genLine
    assert.equal(flat[1], 0) // genCol
    assert.ok(flat[2] >= 0) // source index
    sm.free()
  })
})

describe('encodedMappings', () => {
  it('returns VLQ mappings string', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const encoded = sm.encodedMappings()
    assert.equal(typeof encoded, 'string')
    assert.equal(encoded, 'AAAAA,SACIC')
    sm.free()
  })
})

describe('originalPositionFlat', () => {
  it('returns flat array for mapped position', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const flat = sm.originalPositionFlat(0, 0)
    assert.ok(flat instanceof Int32Array)
    assert.equal(flat.length, 4)
    assert.equal(flat[0], 0) // source index
    assert.equal(flat[1], 0) // line
    assert.equal(flat[2], 0) // column
    assert.equal(flat[3], 0) // name index (foo)
    sm.free()
  })

  it('returns [-1,-1,-1,-1] for unmapped position', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const flat = sm.originalPositionFlat(999, 999)
    assert.deepEqual([...flat], [-1, -1, -1, -1])
    sm.free()
  })
})

describe('originalPositionBuf (zero-alloc)', () => {
  const bufOffset = resultPtr()

  // Helper: create a fresh view (needed after WASM memory may have grown)
  const getView = () => new Int32Array(wasmMemory().buffer, bufOffset, 4)

  it('writes result to static buffer and returns true', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const found = sm.originalPositionBuf(0, 0)
    const view = getView()
    assert.equal(found, true)
    assert.equal(view[0], 0) // source index
    assert.equal(view[1], 0) // line
    assert.equal(view[2], 0) // column
    assert.equal(view[3], 0) // name index (foo)
    sm.free()
  })

  it('returns false for unmapped position', () => {
    const sm = new SourceMap(SIMPLE_MAP)
    const found = sm.originalPositionBuf(999, 999)
    assert.equal(found, false)
    sm.free()
  })

  it('matches originalPositionFor results', () => {
    const sm = new SourceMap(MULTI_SOURCE_MAP)
    const obj = sm.originalPositionFor(1, 0)
    const found = sm.originalPositionBuf(1, 0)
    const view = getView()
    assert.equal(found, true)
    assert.equal(sm.source(view[0]), obj.source)
    assert.equal(view[1], obj.line)
    assert.equal(view[2], obj.column)
    sm.free()
  })
})

describe('indexed source maps', () => {
  it('parses an indexed (sectioned) source map', () => {
    const indexedMap = JSON.stringify({
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

    const sm = new SourceMap(indexedMap)
    assert.ok(sm.sources.includes('a.js'))
    assert.ok(sm.sources.includes('b.js'))

    const pos0 = sm.originalPositionFor(0, 0)
    assert.ok(pos0)
    assert.equal(pos0.source, 'a.js')

    const pos10 = sm.originalPositionFor(10, 0)
    assert.ok(pos10)
    assert.equal(pos10.source, 'b.js')
    sm.free()
  })
})
