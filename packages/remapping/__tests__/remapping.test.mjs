import { describe, it } from 'node:test'
import assert from 'node:assert/strict'
import remapping, { SourceMap } from '../src/remapping.mjs'

// ── Test fixtures ────────────────────────────────────────────────

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

// ── SourceMap class ──────────────────────────────────────────────

describe('SourceMap', () => {
  it('has version, sources, names, mappings, sourcesContent', () => {
    const sm = new SourceMap(ORIGINAL_MAP)
    assert.equal(sm.version, 3)
    assert.deepEqual(sm.sources, ['original.ts'])
    assert.deepEqual(sm.names, [])
    assert.equal(sm.mappings, 'AAAA')
    assert.deepEqual(sm.sourcesContent, ['const x = 1;'])
  })

  it('toString() returns JSON string', () => {
    const sm = new SourceMap(ORIGINAL_MAP)
    const str = sm.toString()
    const parsed = JSON.parse(str)
    assert.equal(parsed.version, 3)
    assert.deepEqual(parsed.sources, ['original.ts'])
  })

  it('toJSON() returns plain object', () => {
    const sm = new SourceMap(ORIGINAL_MAP)
    const json = sm.toJSON()
    assert.equal(json.version, 3)
    assert.deepEqual(json.sources, ['original.ts'])
    assert.equal(json.mappings, 'AAAA')
  })

  it('file is undefined when not set', () => {
    const sm = new SourceMap(ORIGINAL_MAP)
    assert.equal(sm.file, undefined)
  })

  it('file is set when provided', () => {
    const sm = new SourceMap({ ...ORIGINAL_MAP, file: 'output.js' })
    assert.equal(sm.file, 'output.js')
  })
})

// ── Single map input ─────────────────────────────────────────────

describe('remapping(singleMap, loader)', () => {
  it('remaps through a single upstream map', () => {
    const result = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === 'intermediate.js') return INNER_MAP
      return null
    })

    assert.ok(result instanceof SourceMap)
    assert.equal(result.version, 3)
    assert.deepEqual(result.sources, ['original.js'])
  })

  it('passes through sources with no upstream', () => {
    const result = remapping(ORIGINAL_MAP, () => null)
    assert.deepEqual(result.sources, ['original.ts'])
  })

  it('accepts JSON string input', () => {
    const result = remapping(JSON.stringify(INTERMEDIATE_MAP), (source) => {
      if (source === 'intermediate.js') return INNER_MAP
      return null
    })

    assert.deepEqual(result.sources, ['original.js'])
  })

  it('accepts JSON string from loader', () => {
    const result = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === 'intermediate.js') return JSON.stringify(INNER_MAP)
      return null
    })

    assert.deepEqual(result.sources, ['original.js'])
  })

  it('propagates sourcesContent from upstream', () => {
    const outer = {
      version: 3,
      sources: ['compiled.js'],
      names: [],
      mappings: 'AAAA',
    }
    const inner = {
      version: 3,
      sources: ['original.ts'],
      sourcesContent: ['const x = 1;'],
      names: [],
      mappings: 'AAAA',
    }

    const result = remapping(outer, () => inner)
    assert.deepEqual(result.sourcesContent, ['const x = 1;'])
  })

  it('preserves names from outer map when upstream has none', () => {
    const outer = {
      version: 3,
      sources: ['compiled.js'],
      names: ['myFunc'],
      mappings: 'AAAAA',
    }
    const inner = {
      version: 3,
      sources: ['original.ts'],
      names: [],
      mappings: 'AAAA',
    }

    const result = remapping(outer, () => inner)
    assert.ok(result.names.includes('myFunc'))
  })

  it('handles partial upstream maps', () => {
    const outer = {
      version: 3,
      sources: ['compiled.js', 'passthrough.js'],
      names: [],
      mappings: 'AAAA,KCCA',
    }
    const inner = {
      version: 3,
      sources: ['original.ts'],
      names: [],
      mappings: 'AAAA',
    }

    const result = remapping(outer, (source) => {
      if (source === 'compiled.js') return inner
      return null
    })

    assert.ok(result.sources.includes('original.ts'))
    assert.ok(result.sources.includes('passthrough.js'))
  })

  it('loader returning undefined is treated as no upstream', () => {
    const result = remapping(ORIGINAL_MAP, () => undefined)
    assert.deepEqual(result.sources, ['original.ts'])
  })
})

// ── Array input ──────────────────────────────────────────────────

describe('remapping([maps], loader)', () => {
  it('composes two source maps', () => {
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
    assert.ok(result instanceof SourceMap)
    assert.deepEqual(result.sources, ['original.ts'])
  })

  it('composes three source maps', () => {
    const step3 = {
      version: 3,
      sources: ['step2.js'],
      names: [],
      mappings: 'AAAA',
    }
    const step2 = {
      version: 3,
      sources: ['step1.js'],
      names: [],
      mappings: 'AAAA',
    }
    const step1 = {
      version: 3,
      sources: ['original.ts'],
      names: [],
      mappings: 'AAAA',
    }

    const result = remapping([step3, step2, step1], () => null)
    assert.deepEqual(result.sources, ['original.ts'])
  })

  it('handles single-element array', () => {
    const result = remapping([ORIGINAL_MAP], () => null)
    assert.deepEqual(result.sources, ['original.ts'])
  })

  it('propagates sourcesContent through chain', () => {
    const outer = {
      version: 3,
      sources: ['intermediate.js'],
      names: [],
      mappings: 'AAAA',
    }
    const inner = {
      version: 3,
      sources: ['original.ts'],
      sourcesContent: ['const x = 1;'],
      names: [],
      mappings: 'AAAA',
    }

    const result = remapping([outer, inner], () => null)
    assert.deepEqual(result.sourcesContent, ['const x = 1;'])
  })

  it('consults loader after chain exhaustion', () => {
    const outer = {
      version: 3,
      sources: ['intermediate.js'],
      names: [],
      mappings: 'AAAA',
    }
    const inner = {
      version: 3,
      sources: ['another-intermediate.js'],
      names: [],
      mappings: 'AAAA',
    }
    const deepest = {
      version: 3,
      sources: ['original.ts'],
      sourcesContent: ['const x = 1;'],
      names: [],
      mappings: 'AAAA',
    }

    const result = remapping([outer, inner], (source) => {
      if (source === 'another-intermediate.js') return deepest
      return null
    })

    assert.deepEqual(result.sources, ['original.ts'])
  })

  it('works with JSON string inputs in array', () => {
    const outer = JSON.stringify({
      version: 3,
      sources: ['intermediate.js'],
      names: [],
      mappings: 'AAAA',
    })
    const inner = JSON.stringify({
      version: 3,
      sources: ['original.ts'],
      names: [],
      mappings: 'AAAA',
    })

    const result = remapping([outer, inner], () => null)
    assert.deepEqual(result.sources, ['original.ts'])
  })
})

// ── Vite-style usage ─────────────────────────────────────────────

describe('Vite combineSourcemaps pattern', () => {
  it('remapping(sourcemapList, () => null) — simple chain', () => {
    // Vite's fast path: array of maps, no loader
    const maps = [
      {
        version: 3,
        sources: ['step1.js'],
        names: [],
        mappings: 'AAAA',
      },
      {
        version: 3,
        sources: ['original.vue'],
        sourcesContent: ['<template>hello</template>'],
        names: [],
        mappings: 'AAAA',
      },
    ]

    const result = remapping(maps, () => null)
    assert.ok(result instanceof SourceMap)
    assert.deepEqual(result.sources, ['original.vue'])
  })

  it('remapping(map, loader) — loader-based chain', () => {
    // Vite's loader path for multi-source chains
    const sourcemapList = [
      {
        version: 3,
        sources: ['test.vue'],
        names: [],
        mappings: 'AAAA',
      },
      {
        version: 3,
        sources: ['test.vue'],
        sourcesContent: ['<script>export default {}</script>'],
        names: [],
        mappings: 'AAAA',
      },
    ]

    const filename = 'test.vue'
    let mapIndex = 1

    const result = remapping(sourcemapList[0], (sourcefile) => {
      if (sourcefile === filename && sourcemapList[mapIndex]) {
        return sourcemapList[mapIndex++]
      }
      return null
    })

    assert.ok(result instanceof SourceMap)
  })
})

// ── Return value interface ───────────────────────────────────────

describe('return value interface', () => {
  it('result has all expected properties', () => {
    const result = remapping(ORIGINAL_MAP, () => null)

    assert.equal(typeof result.version, 'number')
    assert.ok(Array.isArray(result.sources))
    assert.ok(Array.isArray(result.names))
    assert.equal(typeof result.mappings, 'string')
    assert.equal(typeof result.toString, 'function')
    assert.equal(typeof result.toJSON, 'function')
  })

  it('toString() produces valid JSON', () => {
    const result = remapping(ORIGINAL_MAP, () => null)
    const str = result.toString()
    const parsed = JSON.parse(str)
    assert.equal(parsed.version, 3)
  })

  it('toJSON() produces serializable object', () => {
    const result = remapping(ORIGINAL_MAP, () => null)
    const json = result.toJSON()
    assert.equal(json.version, 3)
    assert.ok(Array.isArray(json.sources))

    // Re-stringify round-trips
    const reparsed = JSON.parse(JSON.stringify(json))
    assert.equal(reparsed.version, 3)
  })

  it('result can be passed as input to another remapping call', () => {
    const step1 = remapping(INTERMEDIATE_MAP, (source) => {
      if (source === 'intermediate.js') return INNER_MAP
      return null
    })

    // Use the result as input to another remapping
    const step2 = remapping(step1, () => null)
    assert.ok(step2 instanceof SourceMap)
    assert.deepEqual(step2.sources, ['original.js'])
  })
})

// ── Error handling ───────────────────────────────────────────────

describe('error handling', () => {
  it('throws on invalid JSON string input', () => {
    assert.throws(() => remapping('not json', () => null))
  })

  it('handles empty array gracefully', () => {
    const result = remapping([], () => null)
    assert.ok(result instanceof SourceMap)
    assert.equal(result.version, 3)
  })
})

// ── Default export ───────────────────────────────────────────────

describe('exports', () => {
  it('default export is the remapping function', () => {
    assert.equal(typeof remapping, 'function')
  })

  it('SourceMap is a named export', () => {
    assert.equal(typeof SourceMap, 'function')
  })
})
