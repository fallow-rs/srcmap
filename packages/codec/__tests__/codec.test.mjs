import { describe, it } from 'node:test'
import assert from 'node:assert/strict'
import { decode, encode } from '../index.js'

describe('decode', () => {
  it('decodes a simple mappings string', () => {
    const result = decode('AAAA;AACA,GAAG;AACA,IAAI,EAAE')
    assert.ok(Array.isArray(result))
    assert.equal(result.length, 3)
    // First line: one segment [0,0,0,0]
    assert.deepEqual(result[0], [[0, 0, 0, 0]])
    // Second line: two segments
    assert.equal(result[1].length, 2)
    assert.deepEqual(result[1][0], [0, 0, 1, 0])
  })

  it('decodes empty mappings', () => {
    const result = decode('')
    assert.ok(Array.isArray(result))
    assert.equal(result.length, 0)
  })

  it('decodes semicolons-only (empty lines)', () => {
    const result = decode(';;;')
    assert.equal(result.length, 4)
    for (const line of result) {
      assert.deepEqual(line, [])
    }
  })

  it('decodes segments with 1 field (generated column only)', () => {
    const result = decode('A')
    assert.equal(result.length, 1)
    assert.equal(result[0].length, 1)
    assert.equal(result[0][0].length, 1)
    assert.equal(result[0][0][0], 0)
  })

  it('decodes segments with 5 fields (including name)', () => {
    const result = decode('AAAAA')
    assert.equal(result[0][0].length, 5)
    assert.deepEqual(result[0][0], [0, 0, 0, 0, 0])
  })

  it('decodes and accumulates deltas to absolute values', () => {
    // C = genCol 1, E = genCol delta +2 → absolute genCol 3
    const result = decode('CAAA,EAAA')
    assert.equal(result[0].length, 2)
    assert.equal(result[0][0][0], 1)
    assert.equal(result[0][1][0], 3)
  })

  it('decodes negative deltas', () => {
    // D = -1 as a single-field segment
    const result = decode('D')
    assert.equal(result[0][0][0], -1)
  })
})

describe('encode', () => {
  it('encodes a simple mapping', () => {
    const result = encode([[[0, 0, 0, 0]]])
    assert.equal(typeof result, 'string')
    assert.equal(result, 'AAAA')
  })

  it('encodes empty mappings', () => {
    const result = encode([])
    assert.equal(result, '')
  })

  it('encodes empty lines', () => {
    const result = encode([[], [], [], []])
    assert.equal(result, ';;;')
  })

  it('encodes segments with names', () => {
    const result = encode([[[0, 0, 0, 0, 0]]])
    assert.equal(result, 'AAAAA')
  })

  it('encodes multiple segments per line', () => {
    const result = encode([[[0, 0, 0, 0], [3, 0, 0, 3]]])
    assert.ok(result.length > 0)
    assert.ok(!result.includes(';'))
    assert.ok(result.includes(','))
  })
})

describe('roundtrip', () => {
  it('decode(encode(x)) preserves data', () => {
    const original = [
      [[0, 0, 0, 0], [4, 0, 0, 4, 0]],
      [[0, 0, 1, 0], [8, 0, 0, 8]],
      [],
      [[2, 1, 5, 3]],
    ]
    const encoded = encode(original)
    const decoded = decode(encoded)
    assert.deepEqual(decoded, original)
  })

  it('encode(decode(x)) preserves string', () => {
    const original = 'AAAA,IAAIE;AACA,QAAQ;AACA'
    const decoded = decode(original)
    const reencoded = encode(decoded)
    assert.equal(reencoded, original)
  })

  it('handles large mappings', () => {
    const lines = []
    for (let i = 0; i < 100; i++) {
      const segments = []
      for (let j = 0; j < 50; j++) {
        segments.push([j * 2, j % 3, i, j * 4])
      }
      lines.push(segments)
    }
    const encoded = encode(lines)
    const decoded = decode(encoded)
    assert.deepEqual(decoded, lines)
  })
})
