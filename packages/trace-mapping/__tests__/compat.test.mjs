/**
 * Cross-validation tests comparing @srcmap/trace-mapping against
 * @jridgewell/trace-mapping to verify drop-in compatibility.
 */
import { describe, it } from 'node:test'
import assert from 'node:assert/strict'
import {
  TraceMap as SrcmapTraceMap,
  originalPositionFor as srcmapOriginal,
  generatedPositionFor as srcmapGenerated,
  eachMapping as srcmapEachMapping,
  encodedMappings as srcmapEncodedMappings,
  decodedMappings as srcmapDecodedMappings,
  sourceContentFor as srcmapSourceContentFor,
  isIgnored as srcmapIsIgnored,
} from '../src/trace-mapping.mjs'

import {
  TraceMap as JrTraceMap,
  originalPositionFor as jrOriginal,
  generatedPositionFor as jrGenerated,
  eachMapping as jrEachMapping,
  encodedMappings as jrEncodedMappings,
  decodedMappings as jrDecodedMappings,
  sourceContentFor as jrSourceContentFor,
  isIgnored as jrIsIgnored,
} from '../../../benchmarks/node_modules/@jridgewell/trace-mapping/dist/trace-mapping.mjs'

import { encode } from '../../../benchmarks/node_modules/@jridgewell/sourcemap-codec/dist/sourcemap-codec.mjs'

// ── Test fixtures ────────────────────────────────────────────────

const SIMPLE_MAP = JSON.stringify({
  version: 3,
  file: 'output.js',
  sources: ['input.js'],
  sourcesContent: ['const foo = 1;\nconst bar = 2;'],
  names: ['foo', 'bar'],
  mappings: 'AAAAA,SACIC',
})

const MULTI_SOURCE_MAP = JSON.stringify({
  version: 3,
  sources: ['a.js', 'b.js'],
  sourcesContent: ['// a\nconst x = 1;', '// b\nconst y = 2;'],
  names: ['x', 'y', 'z'],
  mappings: 'AAAAA;ACAAC,KACCC',
})

const IGNORE_LIST_MAP = JSON.stringify({
  version: 3,
  sources: ['app.js', 'node_modules/lib.js'],
  sourcesContent: ['app code', 'lib code'],
  names: [],
  mappings: 'AAAA;ACAA',
  ignoreList: [1],
})

/**
 * Generate a realistic source map for stress-testing.
 */
const generateSourceMap = (lines, segsPerLine, numSources) => {
  const sources = Array.from({ length: numSources }, (_, i) => `src/file${i}.js`)
  const names = Array.from({ length: 20 }, (_, i) => `var${i}`)
  const sourcesContent = sources.map(
    (_, i) => `// source file ${i}\n${'const x = 1;\n'.repeat(lines)}`
  )

  const mappings = []
  let src = 0
  let srcLine = 0
  let srcCol = 0
  let name = 0

  for (let line = 0; line < lines; line++) {
    const lineSegs = []
    let genCol = 0

    for (let s = 0; s < segsPerLine; s++) {
      genCol += 2 + ((s * 3) % 20)
      if (s % 7 === 0) src = (src + 1) % numSources
      srcLine += 1
      srcCol = ((s * 5 + 1) % 30)

      if (s % 4 === 0) {
        name = (name + 1) % names.length
        lineSegs.push([genCol, src, srcLine, srcCol, name])
      } else {
        lineSegs.push([genCol, src, srcLine, srcCol])
      }
    }
    mappings.push(lineSegs)
  }

  return JSON.stringify({
    version: 3,
    sources,
    sourcesContent,
    names,
    mappings: encode(mappings),
  })
}

const LARGE_MAP = generateSourceMap(200, 20, 5)

const TEST_MAPS = [
  { name: 'simple', json: SIMPLE_MAP },
  { name: 'multi-source', json: MULTI_SOURCE_MAP },
  { name: 'ignore-list', json: IGNORE_LIST_MAP },
  { name: 'large (200 lines)', json: LARGE_MAP },
]

// ── Cross-validation ─────────────────────────────────────────────

// trace-mapping normalizes paths (resolves ./ segments), srcmap returns raw paths
const normalizePath = (s) => s?.replace(/\/\.\//g, '/') ?? null

describe('cross-validation with @jridgewell/trace-mapping', () => {
  for (const { name, json } of TEST_MAPS) {
    describe(name, () => {
      it('originalPositionFor matches for random lookups', () => {
        const srcmap = new SrcmapTraceMap(json)
        const jr = new JrTraceMap(json)

        const maxLine = srcmap._wasm.lineCount

        let checked = 0
        for (
          let line = 0;
          line < maxLine && checked < 500;
          line += Math.max(1, Math.floor(maxLine / 50))
        ) {
          for (let col = 0; col < 200; col += 10) {
            const expected = jrOriginal(jr, { line: line + 1, column: col })
            const actual = srcmapOriginal(srcmap, { line: line + 1, column: col })

            const expectedNull = expected.source === null
            const actualNull = actual.source === null

            assert.equal(
              actualNull,
              expectedNull,
              `null mismatch at ${line}:${col}: jr=${JSON.stringify(expected)}, srcmap=${JSON.stringify(actual)}`
            )

            if (!expectedNull && !actualNull) {
              assert.equal(
                normalizePath(actual.source),
                normalizePath(expected.source),
                `source mismatch at ${line}:${col}`
              )
              assert.equal(actual.line, expected.line, `line mismatch at ${line}:${col}`)
              assert.equal(actual.column, expected.column, `column mismatch at ${line}:${col}`)
              assert.equal(actual.name, expected.name, `name mismatch at ${line}:${col}`)
            }

            checked++
          }
        }

        assert.ok(checked > 0, 'should have checked at least one lookup')
        srcmap.free()
      })

      it('generatedPositionFor matches', () => {
        const srcmap = new SrcmapTraceMap(json)
        const jr = new JrTraceMap(json)

        // Get some source positions to test
        const positions = []
        jrEachMapping(jr, (m) => {
          if (m.source && positions.length < 50) {
            positions.push({
              source: m.source,
              line: m.originalLine,
              column: m.originalColumn,
            })
          }
        })

        for (const pos of positions) {
          const expected = jrGenerated(jr, pos)
          const actual = srcmapGenerated(srcmap, pos)

          // Both should be found or both null
          if (expected.line !== null) {
            assert.equal(
              actual.line,
              expected.line,
              `generated line mismatch for ${pos.source}:${pos.line}:${pos.column}`
            )
            assert.equal(
              actual.column,
              expected.column,
              `generated column mismatch for ${pos.source}:${pos.line}:${pos.column}`
            )
          }
        }

        srcmap.free()
      })

      it('eachMapping produces same count', () => {
        const srcmap = new SrcmapTraceMap(json)
        const jr = new JrTraceMap(json)

        let srcmapCount = 0
        let jrCount = 0
        srcmapEachMapping(srcmap, () => srcmapCount++)
        jrEachMapping(jr, () => jrCount++)

        assert.equal(srcmapCount, jrCount, 'mapping count should match')
        srcmap.free()
      })

      it('eachMapping produces same data', () => {
        const srcmap = new SrcmapTraceMap(json)
        const jr = new JrTraceMap(json)

        const srcmapMappings = []
        const jrMappings = []
        srcmapEachMapping(srcmap, (m) => srcmapMappings.push(m))
        jrEachMapping(jr, (m) => jrMappings.push(m))

        assert.equal(srcmapMappings.length, jrMappings.length)

        for (let i = 0; i < jrMappings.length; i++) {
          const sm = srcmapMappings[i]
          const jm = jrMappings[i]

          assert.equal(sm.generatedLine, jm.generatedLine, `generatedLine[${i}]`)
          assert.equal(sm.generatedColumn, jm.generatedColumn, `generatedColumn[${i}]`)
          assert.equal(
            normalizePath(sm.source),
            normalizePath(jm.source),
            `source[${i}]`
          )
          assert.equal(sm.originalLine, jm.originalLine, `originalLine[${i}]`)
          assert.equal(sm.originalColumn, jm.originalColumn, `originalColumn[${i}]`)
          assert.equal(sm.name, jm.name, `name[${i}]`)
        }

        srcmap.free()
      })

      it('sourceContentFor matches', () => {
        const parsed = JSON.parse(json)
        if (!parsed.sourcesContent) return

        const srcmap = new SrcmapTraceMap(json)
        const jr = new JrTraceMap(json)

        for (const source of parsed.sources || []) {
          if (!source) continue
          const expected = jrSourceContentFor(jr, source)
          const actual = srcmapSourceContentFor(srcmap, source)
          assert.equal(actual, expected, `sourceContentFor(${source})`)
        }

        srcmap.free()
      })

      it('isIgnored matches', () => {
        const parsed = JSON.parse(json)
        const srcmap = new SrcmapTraceMap(json)
        const jr = new JrTraceMap(json)

        for (const source of parsed.sources || []) {
          if (!source) continue
          const expected = jrIsIgnored(jr, source)
          const actual = srcmapIsIgnored(srcmap, source)
          assert.equal(actual, expected, `isIgnored(${source})`)
        }

        srcmap.free()
      })
    })
  }
})

describe('decoded/encoded mappings cross-validation', () => {
  it('decodedMappings structure matches', () => {
    const srcmap = new SrcmapTraceMap(SIMPLE_MAP)
    const jr = new JrTraceMap(SIMPLE_MAP)

    const srcmapDecoded = srcmapDecodedMappings(srcmap)
    const jrDecoded = jrDecodedMappings(jr)

    assert.equal(srcmapDecoded.length, jrDecoded.length, 'line count')

    for (let i = 0; i < jrDecoded.length; i++) {
      assert.equal(
        srcmapDecoded[i].length,
        jrDecoded[i].length,
        `segment count on line ${i}`
      )

      for (let j = 0; j < jrDecoded[i].length; j++) {
        const srcmapSeg = srcmapDecoded[i][j]
        const jrSeg = jrDecoded[i][j]

        assert.equal(srcmapSeg.length, jrSeg.length, `segment length at [${i}][${j}]`)
        for (let k = 0; k < jrSeg.length; k++) {
          assert.equal(srcmapSeg[k], jrSeg[k], `segment[${i}][${j}][${k}]`)
        }
      }
    }

    srcmap.free()
  })

  it('encodedMappings matches', () => {
    const srcmap = new SrcmapTraceMap(SIMPLE_MAP)
    const jr = new JrTraceMap(SIMPLE_MAP)

    const srcmapEncoded = srcmapEncodedMappings(srcmap)
    const jrEncoded = jrEncodedMappings(jr)

    assert.equal(srcmapEncoded, jrEncoded)
    srcmap.free()
  })
})
