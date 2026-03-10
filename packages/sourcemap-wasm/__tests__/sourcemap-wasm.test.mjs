import { describe, it } from 'node:test'
import assert from 'node:assert/strict'
import { SourceMap } from '../pkg/srcmap_sourcemap_wasm.js'

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
