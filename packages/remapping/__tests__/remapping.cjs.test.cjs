'use strict'

const { describe, it } = require('node:test')
const assert = require('node:assert/strict')
const remapping = require('../src/remapping.cjs')
const { SourceMap } = require('../src/remapping.cjs')

const ORIGINAL_MAP = {
  version: 3,
  sources: ['original.ts'],
  sourcesContent: ['const x = 1;'],
  names: [],
  mappings: 'AAAA',
}

const INTERMEDIATE_MAP = {
  version: 3,
  sources: ['intermediate.js'],
  names: [],
  mappings: 'AAAA;AACA',
}

const INNER_MAP = {
  version: 3,
  sources: ['original.js'],
  names: [],
  mappings: 'AACA;AACA',
}

describe('CJS: remapping', () => {
  it('default export is the remapping function', () => {
    assert.equal(typeof remapping, 'function')
  })

  it('named export: remapping', () => {
    assert.equal(typeof remapping.remapping, 'function')
  })

  it('named export: SourceMap', () => {
    assert.equal(typeof SourceMap, 'function')
  })

  it('remaps single map through loader', () => {
    const result = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === 'intermediate.js') return INNER_MAP
      return null
    })
    assert.ok(result instanceof SourceMap)
    assert.deepEqual(result.sources, ['original.js'])
  })

  it('composes array of maps', () => {
    const outer = {
      version: 3,
      sources: ['intermediate.js'],
      names: [],
      mappings: 'AAAA',
    }
    const inner = {
      version: 3,
      sources: ['original.ts'],
      names: [],
      mappings: 'AAAA',
    }
    const result = remapping([outer, inner], () => null)
    assert.deepEqual(result.sources, ['original.ts'])
  })

  it('passes through with no upstream', () => {
    const result = remapping(ORIGINAL_MAP, () => null)
    assert.deepEqual(result.sources, ['original.ts'])
  })

  it('result has toString and toJSON', () => {
    const result = remapping(ORIGINAL_MAP, () => null)
    assert.equal(typeof result.toString, 'function')
    assert.equal(typeof result.toJSON, 'function')

    const json = result.toJSON()
    assert.equal(json.version, 3)

    const str = result.toString()
    const parsed = JSON.parse(str)
    assert.equal(parsed.version, 3)
  })
})
