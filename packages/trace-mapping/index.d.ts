/**
 * @srcmap/trace-mapping — drop-in replacement for @jridgewell/trace-mapping
 * powered by Rust via WebAssembly.
 */

// ── Segment types ────────────────────────────────────────────────

/** A segment with only a generated column (unmapped). */
export type UnmappedSegment = [generatedColumn: number]

/** A segment with source location but no name. */
export type MappedSegment = [
  generatedColumn: number,
  sourcesIndex: number,
  sourceLine: number,
  sourceColumn: number,
]

/** A segment with source location and name. */
export type NamedMappedSegment = [
  generatedColumn: number,
  sourcesIndex: number,
  sourceLine: number,
  sourceColumn: number,
  namesIndex: number,
]

export type SourceMapSegment = UnmappedSegment | MappedSegment | NamedMappedSegment

// ── Input types ──────────────────────────────────────────────────

export interface SourceMapV3 {
  version: 3
  file?: string | null
  sourceRoot?: string
  sources: (string | null)[]
  sourcesContent?: (string | null)[]
  names: string[]
  ignoreList?: number[]
  x_google_ignoreList?: number[]
}

export interface EncodedSourceMap extends SourceMapV3 {
  mappings: string
}

export interface DecodedSourceMap extends SourceMapV3 {
  mappings: SourceMapSegment[][]
}

export interface Section {
  offset: { line: number; column: number }
  map: EncodedSourceMap | DecodedSourceMap
}

export interface SectionedSourceMap {
  version: 3
  file?: string | null
  sections: Section[]
}

export type SectionedSourceMapInput = SourceMapInput | SectionedSourceMap
export type SourceMapInput = EncodedSourceMap | DecodedSourceMap | string

// ── Needle types ─────────────────────────────────────────────────

export interface Needle {
  line: number
  column: number
  bias?: typeof LEAST_UPPER_BOUND | typeof GREATEST_LOWER_BOUND
}

export interface SourceNeedle {
  source: string
  line: number
  column: number
  bias?: typeof LEAST_UPPER_BOUND | typeof GREATEST_LOWER_BOUND
}

// ── Result types ─────────────────────────────────────────────────

export interface OriginalMapping {
  source: string | null
  line: number | null
  column: number | null
  name: string | null
}

export type InvalidOriginalMapping = {
  source: null
  line: null
  column: null
  name: null
}

export interface GeneratedMapping {
  line: number | null
  column: number | null
}

export type InvalidGeneratedMapping = {
  line: null
  column: null
}

export interface EachMapping {
  generatedLine: number
  generatedColumn: number
  source: string | null
  originalLine: number | null
  originalColumn: number | null
  name: string | null
}

// ── Constants ────────────────────────────────────────────────────

export declare const LEAST_UPPER_BOUND: -1
export declare const GREATEST_LOWER_BOUND: 1

// ── TraceMap ─────────────────────────────────────────────────────

export declare class TraceMap {
  constructor(map: SectionedSourceMapInput, mapUrl?: string | null)

  version: SourceMapV3['version']
  file: SourceMapV3['file']
  names: SourceMapV3['names']
  sourceRoot: SourceMapV3['sourceRoot']
  sources: SourceMapV3['sources']
  sourcesContent: SourceMapV3['sourcesContent']
  ignoreList: SourceMapV3['ignoreList']
  resolvedSources: string[]

  free(): void
  [Symbol.dispose](): void
}

// ── Free functions ───────────────────────────────────────────────

export declare function encodedMappings(map: TraceMap): string

export declare function decodedMappings(
  map: TraceMap,
): readonly SourceMapSegment[][]

export declare function traceSegment(
  map: TraceMap,
  line: number,
  column: number,
): Readonly<SourceMapSegment> | null

export declare function originalPositionFor(
  map: TraceMap,
  needle: Needle,
): OriginalMapping | InvalidOriginalMapping

export declare function generatedPositionFor(
  map: TraceMap,
  needle: SourceNeedle,
): GeneratedMapping | InvalidGeneratedMapping

export declare function allGeneratedPositionsFor(
  map: TraceMap,
  needle: SourceNeedle,
): GeneratedMapping[]

export declare function eachMapping(
  map: TraceMap,
  cb: (mapping: EachMapping) => void,
): void

export declare function sourceContentFor(
  map: TraceMap,
  source: string,
): string | null

export declare function isIgnored(map: TraceMap, source: string): boolean

export declare function presortedDecodedMap(
  map: DecodedSourceMap,
  mapUrl?: string,
): TraceMap

export declare function decodedMap(
  map: TraceMap,
): Omit<DecodedSourceMap, 'mappings'> & {
  mappings: readonly SourceMapSegment[][]
}

export declare function encodedMap(map: TraceMap): EncodedSourceMap

/** Handles sectioned/indexed source maps. Alias for TraceMap (srcmap handles indexed maps natively). */
export declare const FlattenMap: typeof TraceMap
/** Alias for FlattenMap. */
export declare const AnyMap: typeof TraceMap
