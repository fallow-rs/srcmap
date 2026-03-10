import { describe, it } from 'node:test'
import assert from 'node:assert/strict'
import { SourceMapGenerator } from '../pkg/srcmap_generator_wasm.js'

describe('SourceMapGenerator constructor', () => {
  it('creates a generator with a file name', () => {
    const gen = new SourceMapGenerator('output.js')
    assert.ok(gen)
    gen.free()
  })

  it('creates a generator without a file name', () => {
    const gen = new SourceMapGenerator()
    assert.ok(gen)
    gen.free()
  })
})

describe('sources and names', () => {
  it('registers sources with deduplication', () => {
    const gen = new SourceMapGenerator()
    const idx1 = gen.addSource('a.js')
    const idx2 = gen.addSource('b.js')
    const idx3 = gen.addSource('a.js')
    assert.equal(idx1, 0)
    assert.equal(idx2, 1)
    assert.equal(idx3, 0) // deduplicated
    gen.free()
  })

  it('registers names with deduplication', () => {
    const gen = new SourceMapGenerator()
    const idx1 = gen.addName('foo')
    const idx2 = gen.addName('bar')
    const idx3 = gen.addName('foo')
    assert.equal(idx1, 0)
    assert.equal(idx2, 1)
    assert.equal(idx3, 0)
    gen.free()
  })
})

describe('addMapping', () => {
  it('adds a simple mapping and produces valid JSON', () => {
    const gen = new SourceMapGenerator('output.js')
    const src = gen.addSource('input.js')
    gen.addMapping(0, 0, src, 0, 0)

    const json = gen.toJSON()
    const map = JSON.parse(json)

    assert.equal(map.version, 3)
    assert.equal(map.file, 'output.js')
    assert.deepEqual(map.sources, ['input.js'])
    assert.ok(map.mappings.length > 0)
    gen.free()
  })

  it('adds mappings across multiple lines', () => {
    const gen = new SourceMapGenerator()
    const src = gen.addSource('input.js')
    gen.addMapping(0, 0, src, 0, 0)
    gen.addMapping(1, 4, src, 1, 2)
    gen.addMapping(2, 0, src, 2, 0)

    assert.equal(gen.mappingCount, 3)
    const map = JSON.parse(gen.toJSON())
    assert.ok(map.mappings.includes(';')) // multiple lines
    gen.free()
  })
})

describe('addNamedMapping', () => {
  it('adds a mapping with a name', () => {
    const gen = new SourceMapGenerator()
    const src = gen.addSource('input.js')
    const name = gen.addName('myFunction')
    gen.addNamedMapping(0, 0, src, 0, 0, name)

    const map = JSON.parse(gen.toJSON())
    assert.deepEqual(map.names, ['myFunction'])
    gen.free()
  })
})

describe('addGeneratedMapping', () => {
  it('adds a generated-only mapping (no source)', () => {
    const gen = new SourceMapGenerator()
    gen.addGeneratedMapping(0, 0)

    assert.equal(gen.mappingCount, 1)
    const map = JSON.parse(gen.toJSON())
    assert.ok(map.mappings.length > 0)
    gen.free()
  })
})

describe('maybeAddMapping', () => {
  it('skips redundant mappings', () => {
    const gen = new SourceMapGenerator()
    const src = gen.addSource('input.js')

    assert.equal(gen.maybeAddMapping(0, 0, src, 10, 0), true) // added
    assert.equal(gen.maybeAddMapping(0, 5, src, 10, 0), false) // redundant
    assert.equal(gen.maybeAddMapping(0, 10, src, 11, 0), true) // different

    assert.equal(gen.mappingCount, 2)
    gen.free()
  })
})

describe('sourceContent', () => {
  it('embeds source content', () => {
    const gen = new SourceMapGenerator()
    const src = gen.addSource('input.js')
    gen.setSourceContent(src, 'var x = 1;')
    gen.addMapping(0, 0, src, 0, 0)

    const map = JSON.parse(gen.toJSON())
    assert.deepEqual(map.sourcesContent, ['var x = 1;'])
    gen.free()
  })
})

describe('sourceRoot', () => {
  it('sets the source root', () => {
    const gen = new SourceMapGenerator()
    gen.setSourceRoot('src/')
    gen.addSource('input.js')
    gen.addGeneratedMapping(0, 0)

    const map = JSON.parse(gen.toJSON())
    assert.equal(map.sourceRoot, 'src/')
    gen.free()
  })
})

describe('ignoreList', () => {
  it('adds sources to the ignore list', () => {
    const gen = new SourceMapGenerator()
    const _app = gen.addSource('app.js')
    const lib = gen.addSource('node_modules/lib.js')
    gen.addToIgnoreList(lib)
    gen.addMapping(0, 0, lib, 0, 0)

    const map = JSON.parse(gen.toJSON())
    assert.deepEqual(map.ignoreList, [1])
    gen.free()
  })
})

describe('debugId', () => {
  it('sets and outputs debugId', () => {
    const gen = new SourceMapGenerator()
    gen.setDebugId('85314830-023f-4cf1-a267-535f4e37bb17')
    gen.addSource('input.js')
    gen.addGeneratedMapping(0, 0)

    const map = JSON.parse(gen.toJSON())
    assert.equal(map.debugId, '85314830-023f-4cf1-a267-535f4e37bb17')
    gen.free()
  })

  it('omits debugId when not set', () => {
    const gen = new SourceMapGenerator()
    gen.addGeneratedMapping(0, 0)

    const map = JSON.parse(gen.toJSON())
    assert.equal(map.debugId, undefined)
    gen.free()
  })
})

describe('large roundtrip', () => {
  it('handles 1000 mappings correctly', () => {
    const gen = new SourceMapGenerator('bundle.js')

    for (let i = 0; i < 5; i++) {
      gen.addSource(`src/file${i}.js`)
    }
    for (let i = 0; i < 10; i++) {
      gen.addName(`var${i}`)
    }

    for (let line = 0; line < 100; line++) {
      for (let col = 0; col < 10; col++) {
        const src = (line * 10 + col) % 5
        if (col % 3 === 0) {
          gen.addNamedMapping(line, col * 10, src, line, col * 5, col % 10)
        } else {
          gen.addMapping(line, col * 10, src, line, col * 5)
        }
      }
    }

    assert.equal(gen.mappingCount, 1000)

    const map = JSON.parse(gen.toJSON())
    assert.equal(map.version, 3)
    assert.equal(map.sources.length, 5)
    assert.equal(map.names.length, 10)
    gen.free()
  })
})
